//! Os ajustes da Jangada (Preferências), os favoritos, o histórico e as abas da última
//! vez, em arquivos de texto simples em ~/.config/jangada.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use glove_ui::gtk::glib;

/// Buscadores: (nome, endereço da busca, começo da página).
pub const BUSCADORES: [(&str, &str); 6] = [
    ("DuckDuckGo", "https://duckduckgo.com/?q="),
    ("Google", "https://www.google.com/search?q="),
    ("Bing", "https://www.bing.com/search?q="),
    ("Ecosia", "https://www.ecosia.org/search?q="),
    ("Brave Search", "https://search.brave.com/search?q="),
    ("Startpage", "https://www.startpage.com/do/search?q="),
];

/// Ligados nas Preferências, lidos pelo motor fora da linha principal (bloqueador,
/// "Não Rastrear").
pub static BLOQUEAR: AtomicBool = AtomicBool::new(true);
pub static NAO_RASTREAR: AtomicBool = AtomicBool::new(true);
pub static BLOQUEAR_JANELAS: AtomicBool = AtomicBool::new(true);
pub static COOKIES_TERCEIROS: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Debug, PartialEq)]
pub struct Ajustes {
    /// Página das abas novas e do botão de início: "" = a da Jangada.
    pub inicio: String,
    /// Ao abrir a Jangada, as abas da última vez (senão, a página inicial).
    pub reabrir_abas: bool,
    /// Índice em `BUSCADORES`.
    pub busca: usize,
    pub barra_favoritos: bool,
    /// Extensões da Jangada.
    pub bloqueador: bool,
    pub nao_rastrear: bool,
    pub escuro_nas_paginas: bool,
    pub bloquear_janelas: bool,
    /// Barrar os cookies de terceiros (os que seguem a pessoa de site em site).
    pub cookies_terceiros: bool,
    /// Oferecer para guardar as senhas e preencher as guardadas.
    pub senhas: bool,
    /// Avançado. Usar a placa de vídeo (o padrão): páginas pesadas, como o feed das
    /// redes sociais, ficam muito mais leves. Desligada, o processador desenha tudo.
    pub placa_de_video: bool,
    pub endereco_completo: bool,
    /// Zoom das páginas (nível do motor: 0 = 100%).
    pub zoom: f64,
    /// A página inicial: fundo ("", "cor:#…" ou "imagem:…"), atalhos e busca.
    pub inicio_fundo: String,
    pub inicio_atalhos: bool,
    pub inicio_busca: bool,
    /// Ferramentas do desenvolvedor: abre a porta local de depuração (vale ao reabrir).
    pub ferramentas_dev: bool,
}

impl Default for Ajustes {
    fn default() -> Self {
        Self { inicio: String::new(), reabrir_abas: false, busca: 0, barra_favoritos: true, bloqueador: true, nao_rastrear: true, escuro_nas_paginas: false, bloquear_janelas: true, cookies_terceiros: true, senhas: true, placa_de_video: true, endereco_completo: false, zoom: 0.0, inicio_fundo: String::new(), inicio_atalhos: true, inicio_busca: true, ferramentas_dev: false }
    }
}

pub fn pasta() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let p = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("jangada");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn sim(v: &str) -> bool {
    v == "yes" || v == "1" || v == "true"
}

/// The settings keys that had Portuguese names up to Jangada 2.0: (then, now).
const OLD_KEYS: [(&str, &str); 17] = [
    ("inicio", "home"),
    ("reabrir_abas", "reopen_tabs"),
    ("busca", "search_engine"),
    ("barra_favoritos", "bookmarks_bar"),
    ("bloqueador", "shields"),
    ("nao_rastrear", "do_not_track"),
    ("escuro_nas_paginas", "dark_pages"),
    ("bloquear_janelas", "block_popups"),
    ("cookies_terceiros", "block_third_party_cookies"),
    ("senhas", "passwords"),
    ("usar_placa_de_video", "use_gpu"),
    ("endereco_completo", "full_address"),
    ("zoom", "zoom"),
    ("inicio_fundo", "home_background"),
    ("inicio_atalhos", "home_shortcuts"),
    ("inicio_busca", "home_search"),
    ("ferramentas_dev", "dev_tools"),
];

/// The old settings file in today's keys and values ("sim"/"nao" are "yes"/"no"; the home
/// background is "color:…" or "image:…"). Lines it does not know are dropped.
fn convert_old_settings(text: &str) -> String {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = OLD_KEYS.iter().find(|(old, _)| *old == key.trim()).map(|(_, new)| *new)?;
            let value = match value.trim() {
                "sim" => "yes".to_string(),
                "nao" => "no".to_string(),
                v if key == "home_background" => match v.split_once(':') {
                    Some(("cor", rest)) => format!("color:{rest}"),
                    Some(("imagem", rest)) => format!("image:{rest}"),
                    _ => v.to_string(),
                },
                v => v.to_string(),
            };
            Some(format!("{key} = {value}\n"))
        })
        .collect()
}

/// The stored site permissions with today's names for each kind.
fn convert_old_permissions(text: &str) -> String {
    text.lines()
        .map(|line| {
            let mut fields: Vec<&str> = line.split('\t').collect();
            if let Some(kind) = fields.get_mut(1) {
                *kind = match *kind {
                    "microfone" => "microphone",
                    "localizacao" => "location",
                    "notificacoes" => "notifications",
                    "transferencia" => "clipboard",
                    other => other,
                };
            }
            fields.join("\t") + "\n"
        })
        .collect()
}

/// Up to Jangada 2.0 the data files, the settings keys and some stored values had Portuguese
/// names. What this computer still has under those names moves to the names of today: each
/// file once, and only when the new one does not exist yet.
pub fn migrate_old_names() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let rename = |from: PathBuf, to: PathBuf| {
        if from.exists() && !to.exists() {
            let _ = std::fs::rename(from, to);
        }
    };
    let convert = |from: PathBuf, to: PathBuf, f: fn(&str) -> String| {
        if from.is_file() && !to.exists() {
            if let Ok(text) = std::fs::read_to_string(&from) {
                if std::fs::write(&to, f(&text)).is_ok() {
                    let _ = std::fs::remove_file(&from);
                }
            }
        }
    };
    // Settings, bookmarks, history, last session, permissions.
    let config = pasta();
    convert(config.join("ajustes.conf"), config.join("settings.conf"), convert_old_settings);
    convert(config.join("permissoes.tsv"), config.join("permissions.tsv"), convert_old_permissions);
    for (old, new) in [("favoritos.tsv", "bookmarks.tsv"), ("historico.tsv", "history.tsv"), ("sessao.tsv", "session.tsv"), ("senhas-nunca.txt", "passwords-never.txt"), ("escudos-fora.txt", "shields-off.txt")] {
        rename(config.join(old), config.join(new));
    }
    // Installed extensions (and the list of the ones turned off); the home page is rebuilt.
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("jangada");
    rename(data.join("extensoes"), data.join("extensions"));
    rename(data.join("extensions/ativas.txt"), data.join("extensions/disabled.txt"));
    let _ = std::fs::remove_file(data.join("inicio.html"));
    // The shields: the downloaded lists and the compiled filters.
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("jangada");
    rename(cache.join("escudos"), cache.join("shields"));
    rename(cache.join("shields/filtros.txt"), cache.join("shields/filters.txt"));
    rename(cache.join("shields/compilados"), cache.join("shields/compiled"));
}

impl Ajustes {
    pub fn carregar() -> Self {
        let mut a = Self::default();
        let Ok(t) = std::fs::read_to_string(pasta().join("settings.conf")) else { return a };
        for l in t.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "home" => a.inicio = v.to_string(),
                "reopen_tabs" => a.reabrir_abas = sim(v),
                "search_engine" => a.busca = v.parse::<usize>().unwrap_or(0).min(BUSCADORES.len() - 1),
                "bookmarks_bar" => a.barra_favoritos = sim(v),
                "shields" => a.bloqueador = sim(v),
                "do_not_track" => a.nao_rastrear = sim(v),
                "dark_pages" => a.escuro_nas_paginas = sim(v),
                "block_popups" => a.bloquear_janelas = sim(v),
                "block_third_party_cookies" => a.cookies_terceiros = sim(v),
                "passwords" => a.senhas = sim(v),
                // Chave nova: até a 1.12 o padrão era desligada e ficou gravado "nao" para
                // quem salvou as preferências; assim todos passam ao padrão novo (ligada).
                "use_gpu" => a.placa_de_video = sim(v),
                "full_address" => a.endereco_completo = sim(v),
                "zoom" => a.zoom = v.parse().unwrap_or(0.0),
                "home_background" => a.inicio_fundo = v.to_string(),
                "home_shortcuts" => a.inicio_atalhos = sim(v),
                "home_search" => a.inicio_busca = sim(v),
                "dev_tools" => a.ferramentas_dev = sim(v),
                _ => {}
            }
        }
        a
    }

    pub fn salvar(&self) {
        let b = |v: bool| if v { "yes" } else { "no" };
        let t = format!(
            "# Jangada settings (Jangada ▸ Preferences…)\nhome = {}\nreopen_tabs = {}\nsearch_engine = {}\nbookmarks_bar = {}\nshields = {}\ndo_not_track = {}\ndark_pages = {}\nblock_popups = {}\nblock_third_party_cookies = {}\npasswords = {}\nuse_gpu = {}\nfull_address = {}\nzoom = {}\nhome_background = {}\nhome_shortcuts = {}\nhome_search = {}\ndev_tools = {}\n",
            self.inicio, b(self.reabrir_abas), self.busca, b(self.barra_favoritos), b(self.bloqueador), b(self.nao_rastrear), b(self.escuro_nas_paginas), b(self.bloquear_janelas), b(self.cookies_terceiros), b(self.senhas), b(self.placa_de_video), b(self.endereco_completo), self.zoom, self.inicio_fundo, b(self.inicio_atalhos), b(self.inicio_busca), b(self.ferramentas_dev)
        );
        let _ = std::fs::write(pasta().join("settings.conf"), t);
        self.aplicar();
    }

    /// O que o motor lê sozinho (fora da linha principal).
    pub fn aplicar(&self) {
        BLOQUEAR.store(self.bloqueador, Ordering::Relaxed);
        NAO_RASTREAR.store(self.nao_rastrear, Ordering::Relaxed);
        BLOQUEAR_JANELAS.store(self.bloquear_janelas, Ordering::Relaxed);
        COOKIES_TERCEIROS.store(self.cookies_terceiros, Ordering::Relaxed);
        crate::senhas::LIGADO.store(self.senhas, Ordering::Relaxed);
        crate::motor::aplicar_cookies(self.cookies_terceiros);
    }

    pub fn opcoes_inicio(&self) -> crate::inicio::Opcoes {
        crate::inicio::Opcoes { fundo: self.inicio_fundo.clone(), atalhos: self.inicio_atalhos, busca: self.inicio_busca }
    }

    pub fn busca_url(&self) -> &'static str {
        BUSCADORES[self.busca.min(BUSCADORES.len() - 1)].1
    }
}

// ---------------------------------------------------------------------------
// Favoritos, histórico e as abas da última vez: (título, endereço) por linha.
// ---------------------------------------------------------------------------

fn ler_lista(nome: &str) -> Vec<(String, String)> {
    std::fs::read_to_string(pasta().join(nome))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(t, u)| (t.to_string(), u.to_string())))
        .filter(|(_, u)| !u.is_empty())
        .collect()
}

/// Janela privada (`--private`): nada do que é visitado ou guardado como uso (favoritos,
/// histórico, abas da última vez) é gravado. Ajustes e a própria escolha de escudos valem.
pub static PRIVADA: AtomicBool = AtomicBool::new(false);

pub fn privada() -> bool {
    PRIVADA.load(Ordering::Relaxed)
}

fn gravar_lista(nome: &str, v: &[(String, String)]) {
    if privada() {
        return;
    }
    let t: String = v.iter().map(|(t, u)| format!("{}\t{}\n", t.replace(['\t', '\n'], " "), u.replace(['\t', '\n'], ""))).collect();
    let _ = std::fs::write(pasta().join(nome), t);
}

pub fn favoritos() -> Vec<(String, String)> {
    ler_lista("bookmarks.tsv")
}

pub fn gravar_favoritos(v: &[(String, String)]) {
    gravar_lista("bookmarks.tsv", v);
}

thread_local! {
    /// As visitas à espera de gravação (título, endereço), da mais velha para a mais nova.
    static VISITAS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    static VISITAS_AGENDADAS: Cell<bool> = const { Cell::new(false) };
}

/// Quanto as visitas esperam antes de irem para o arquivo.
const ESPERA_DAS_VISITAS: std::time::Duration = std::time::Duration::from_secs(2);

/// O histórico, do mais novo para o mais velho.
pub fn historico() -> Vec<(String, String)> {
    gravar_visitas();
    ler_lista("history.tsv")
}

/// The file's mark (modification time and size): it changes when the history is written.
pub type HistoryMark = Option<(std::time::SystemTime, u64)>;

/// The history, only when it changed since `mark` (which is updated). Reading the whole
/// file (up to 2000 visits, often hundreds of KB) every few seconds cost for nothing.
pub fn history_if_changed(mark: &mut HistoryMark) -> Option<Vec<(String, String)>> {
    gravar_visitas();
    let now = std::fs::metadata(pasta().join("history.tsv")).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
    if now.is_some() && now == *mark {
        return None;
    }
    *mark = now;
    Some(ler_lista("history.tsv"))
}

/// Uma visita: vai para o topo (sem repetir o endereço), no máximo 2000. A gravação
/// espera um instante e junta o que chegar: um site que troca o título o tempo todo
/// (aviso de mensagem nova, contador) regravava o arquivo inteiro a cada troca.
pub fn visitar(titulo: &str, url: &str) {
    if privada() || url.is_empty() || url.starts_with("data:") || url.starts_with("file:") || url.starts_with("devtools:") {
        return;
    }
    anotar_visita(titulo, url);
    if !VISITAS_AGENDADAS.with(|a| a.replace(true)) {
        glib::timeout_add_local_once(ESPERA_DAS_VISITAS, || {
            VISITAS_AGENDADAS.with(|a| a.set(false));
            gravar_visitas();
        });
    }
}

/// A visita entra na fila (a mais nova do mesmo endereço fica no lugar da anterior).
fn anotar_visita(titulo: &str, url: &str) {
    VISITAS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|(_, u)| u != url);
        v.push((titulo.to_string(), url.to_string()));
    });
}

/// As visitas da fila no histórico dado (a mais nova no topo, sem repetir, até 2000).
fn juntar_visitas(mut v: Vec<(String, String)>, novas: Vec<(String, String)>) -> Vec<(String, String)> {
    for (t, u) in novas {
        v.retain(|(_, x)| *x != u);
        v.insert(0, (t, u));
    }
    v.truncate(2000);
    v
}

/// Grava as visitas à espera (também antes de ler o histórico e ao fechar a Jangada).
pub fn gravar_visitas() {
    let novas = VISITAS.with(|v| std::mem::take(&mut *v.borrow_mut()));
    if novas.is_empty() {
        return;
    }
    gravar_lista("history.tsv", &juntar_visitas(ler_lista("history.tsv"), novas));
}

/// As permissões guardadas dos sites (um por linha: origem, tipo, permitido 1/0).
fn permissoes_guardadas() -> Vec<(String, String, bool)> {
    std::fs::read_to_string(pasta().join("permissions.tsv"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut c = l.split('\t');
            Some((c.next()?.to_string(), c.next()?.to_string(), c.next()? == "1"))
        })
        .collect()
}

/// A decisão guardada para o site e o tipo (camera, microfone, localizacao, notificacoes):
/// Some(permitido) ou None (ainda não foi decidido).
pub fn permissao(origem: &str, tipo: &str) -> Option<bool> {
    permissoes_guardadas().into_iter().find(|(o, t, _)| o == origem && t == tipo).map(|(_, _, p)| p)
}

/// Guarda a decisão para o site e o tipo.
pub fn gravar_permissao(origem: &str, tipo: &str, permitido: bool) {
    if privada() {
        return;
    }
    let mut v: Vec<(String, String, bool)> = permissoes_guardadas().into_iter().filter(|(o, t, _)| !(o == origem && t == tipo)).collect();
    v.push((origem.to_string(), tipo.to_string(), permitido));
    let t: String = v.iter().map(|(o, t, p)| format!("{o}\t{t}\t{}\n", if *p { 1 } else { 0 })).collect();
    let _ = std::fs::write(pasta().join("permissions.tsv"), t);
}

/// Quantos sites têm alguma permissão guardada.
pub fn quantas_permissoes() -> usize {
    permissoes_guardadas().len()
}

/// Esquece todas as permissões dos sites.
pub fn esquecer_permissoes() {
    let _ = std::fs::remove_file(pasta().join("permissions.tsv"));
}

/// Tira um site do histórico.
pub fn apagar_do_historico(url: &str) {
    let mut v = historico();
    v.retain(|(_, u)| u != url);
    gravar_lista("history.tsv", &v);
}

/// Junta visitas importadas ao fim do histórico (sem repetir endereço). Devolve quantas
/// entraram.
pub fn juntar_historico(novas: &[(String, String)]) -> usize {
    let mut v = historico();
    let mut n = 0;
    for (t, u) in novas {
        if v.len() >= 2000 {
            break;
        }
        if !v.iter().any(|(_, x)| x == u) {
            v.push((t.clone(), u.clone()));
            n += 1;
        }
    }
    gravar_lista("history.tsv", &v);
    n
}

pub fn limpar_historico() {
    VISITAS.with(|v| v.borrow_mut().clear());
    let _ = std::fs::remove_file(pasta().join("history.tsv"));
}

/// As abas da última vez: (título, endereço).
pub fn sessao() -> Vec<(String, String)> {
    ler_lista("session.tsv")
}

pub fn gravar_sessao(abas: &[(String, String)]) {
    gravar_lista("session.tsv", abas);
}

// ---------------------------------------------------------------------------
// Bloqueador de anúncios e rastreadores (a extensão que vem com a Jangada)
// ---------------------------------------------------------------------------

/// Domínios de anúncios e de rastreamento conhecidos (e os subdomínios deles).
pub const BLOQUEADOS: &[&str] = &[
    "doubleclick.net", "googlesyndication.com", "googleadservices.com", "google-analytics.com", "googletagmanager.com",
    "googletagservices.com", "adservice.google.com", "pagead2.googlesyndication.com", "analytics.google.com",
    "connect.facebook.net", "facebook.com/tr", "ads.linkedin.com", "analytics.twitter.com", "ads-twitter.com",
    "scorecardresearch.com", "quantserve.com", "taboola.com", "outbrain.com", "criteo.com", "criteo.net",
    "adnxs.com", "amazon-adsystem.com", "pubmatic.com", "rubiconproject.com", "openx.net", "casalemedia.com",
    "adsrvr.org", "moatads.com", "hotjar.com", "mixpanel.com", "segment.io", "segment.com", "fullstory.com",
    "yandex.ru/metrika", "mc.yandex.ru", "adform.net", "smartadserver.com", "teads.tv", "yieldmo.com",
    "media.net", "bidswitch.net", "3lift.com", "sharethrough.com", "zemanta.com", "advertising.com",
    "adcolony.com", "chartbeat.com", "chartbeat.net", "newrelic.com", "nr-data.net", "clarity.ms",
    "bat.bing.com", "ads.yahoo.com", "analytics.yahoo.com", "everesttech.net", "demdex.net", "omtrdc.net",
    "2o7.net", "krxd.net", "bluekai.com", "exelator.com", "agkn.com", "tapad.com", "rlcdn.com", "adsafeprotected.com",
    "doubleverify.com", "serving-sys.com", "flashtalking.com", "contextweb.com", "gumgum.com", "indexww.com",
    "lijit.com", "sovrn.com", "spotxchange.com", "springserve.com", "stickyadstv.com", "tremorhub.com",
    "unrulymedia.com", "vidazoo.com", "admixer.net", "mgid.com", "propellerads.com", "popads.net", "popcash.net",
    "exoclick.com", "juicyads.com", "trafficjunky.net", "hotjar.io", "mouseflow.com", "crazyegg.com", "luckyorange.com",
];

/// Os sites em que a Jangada não oferece guardar a senha ("Nunca Neste Site").
pub fn nunca_guardar() -> std::collections::HashSet<String> {
    std::fs::read_to_string(pasta().join("passwords-never.txt")).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

pub fn nunca_guardar_em(origem: &str) {
    let mut v = nunca_guardar();
    v.insert(origem.to_string());
    let mut l: Vec<String> = v.into_iter().collect();
    l.sort();
    let _ = std::fs::write(pasta().join("passwords-never.txt"), l.join("\n") + "\n");
}

/// Os sites com os escudos desligados (um por linha).
pub fn escudos_fora() -> std::collections::HashSet<String> {
    std::fs::read_to_string(pasta().join("shields-off.txt")).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

pub fn gravar_escudos_fora(v: &std::collections::HashSet<String>) {
    let mut l: Vec<&String> = v.iter().collect();
    l.sort();
    let t: String = l.iter().map(|s| format!("{s}\n")).collect();
    let _ = std::fs::write(pasta().join("shields-off.txt"), t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visitas_juntas_numa_gravacao() {
        // Um site que troca o título o tempo todo: fica só o último, uma vez.
        for n in 1..=50 {
            anotar_visita(&format!("Mensagem nova ({n})"), "https://chat.exemplo/");
        }
        anotar_visita("Outra", "https://outra.exemplo/");
        let fila = VISITAS.with(|v| std::mem::take(&mut *v.borrow_mut()));
        assert_eq!(fila.len(), 2);
        // The settings and the permissions of before, in today's names.
        assert_eq!(convert_old_settings("# Preferências\nreabrir_abas = sim\nbusca = 2\ninicio_fundo = cor:#eceef3\nsenhas = nao\noutra = 1\n"), "reopen_tabs = yes\nsearch_engine = 2\nhome_background = color:#eceef3\npasswords = no\n");
        assert_eq!(convert_old_settings("inicio_fundo = imagem:/home/x/foto.jpg\n"), "home_background = image:/home/x/foto.jpg\n");
        assert_eq!(convert_old_permissions("https://a.b\tmicrofone\t1\nhttps://a.b\tcamera\t0\n"), "https://a.b\tmicrophone\t1\nhttps://a.b\tcamera\t0\n");
        let velho = vec![("Antiga".to_string(), "https://antiga.exemplo/".to_string()), ("Chat".to_string(), "https://chat.exemplo/".to_string())];
        let h = juntar_visitas(velho, fila);
        assert_eq!(h.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["Outra", "Mensagem nova (50)", "Antiga"]);
    }
}
