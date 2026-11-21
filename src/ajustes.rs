//! Os ajustes da Jangada (Preferências), os favoritos, o histórico e as abas da última
//! vez, em arquivos de texto simples em ~/.config/jangada.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Avançado.
    pub placa_de_video: bool,
    pub endereco_completo: bool,
    /// Zoom das páginas (nível do motor: 0 = 100%).
    pub zoom: f64,
}

impl Default for Ajustes {
    fn default() -> Self {
        Self { inicio: String::new(), reabrir_abas: false, busca: 0, barra_favoritos: true, bloqueador: true, nao_rastrear: true, escuro_nas_paginas: false, bloquear_janelas: true, placa_de_video: false, endereco_completo: false, zoom: 0.0 }
    }
}

pub fn pasta() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let p = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("jangada");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn sim(v: &str) -> bool {
    v == "sim" || v == "1" || v == "true"
}

impl Ajustes {
    pub fn carregar() -> Self {
        let mut a = Self::default();
        let Ok(t) = std::fs::read_to_string(pasta().join("ajustes.conf")) else { return a };
        for l in t.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "inicio" => a.inicio = v.to_string(),
                "reabrir_abas" => a.reabrir_abas = sim(v),
                "busca" => a.busca = v.parse::<usize>().unwrap_or(0).min(BUSCADORES.len() - 1),
                "barra_favoritos" => a.barra_favoritos = sim(v),
                "bloqueador" => a.bloqueador = sim(v),
                "nao_rastrear" => a.nao_rastrear = sim(v),
                "escuro_nas_paginas" => a.escuro_nas_paginas = sim(v),
                "bloquear_janelas" => a.bloquear_janelas = sim(v),
                "placa_de_video" => a.placa_de_video = sim(v),
                "endereco_completo" => a.endereco_completo = sim(v),
                "zoom" => a.zoom = v.parse().unwrap_or(0.0),
                _ => {}
            }
        }
        a
    }

    pub fn salvar(&self) {
        let b = |v: bool| if v { "sim" } else { "nao" };
        let t = format!(
            "# Preferências da Jangada (Jangada ▸ Preferências…)\ninicio = {}\nreabrir_abas = {}\nbusca = {}\nbarra_favoritos = {}\nbloqueador = {}\nnao_rastrear = {}\nescuro_nas_paginas = {}\nbloquear_janelas = {}\nplaca_de_video = {}\nendereco_completo = {}\nzoom = {}\n",
            self.inicio, b(self.reabrir_abas), self.busca, b(self.barra_favoritos), b(self.bloqueador), b(self.nao_rastrear), b(self.escuro_nas_paginas), b(self.bloquear_janelas), b(self.placa_de_video), b(self.endereco_completo), self.zoom
        );
        let _ = std::fs::write(pasta().join("ajustes.conf"), t);
        self.aplicar();
    }

    /// O que o motor lê sozinho (fora da linha principal).
    pub fn aplicar(&self) {
        BLOQUEAR.store(self.bloqueador, Ordering::Relaxed);
        NAO_RASTREAR.store(self.nao_rastrear, Ordering::Relaxed);
        BLOQUEAR_JANELAS.store(self.bloquear_janelas, Ordering::Relaxed);
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

fn gravar_lista(nome: &str, v: &[(String, String)]) {
    let t: String = v.iter().map(|(t, u)| format!("{}\t{}\n", t.replace(['\t', '\n'], " "), u.replace(['\t', '\n'], ""))).collect();
    let _ = std::fs::write(pasta().join(nome), t);
}

pub fn favoritos() -> Vec<(String, String)> {
    ler_lista("favoritos.tsv")
}

pub fn gravar_favoritos(v: &[(String, String)]) {
    gravar_lista("favoritos.tsv", v);
}

/// O histórico, do mais novo para o mais velho.
pub fn historico() -> Vec<(String, String)> {
    ler_lista("historico.tsv")
}

/// Uma visita: vai para o topo (sem repetir o endereço), no máximo 2000.
pub fn visitar(titulo: &str, url: &str) {
    if url.is_empty() || url.starts_with("data:") || url.starts_with("file:") {
        return;
    }
    let mut v = historico();
    v.retain(|(_, u)| u != url);
    v.insert(0, (titulo.to_string(), url.to_string()));
    v.truncate(2000);
    gravar_lista("historico.tsv", &v);
}

/// Tira um site do histórico.
pub fn apagar_do_historico(url: &str) {
    let mut v = historico();
    v.retain(|(_, u)| u != url);
    gravar_lista("historico.tsv", &v);
}

pub fn limpar_historico() {
    let _ = std::fs::remove_file(pasta().join("historico.tsv"));
}

pub fn sessao() -> Vec<String> {
    ler_lista("sessao.tsv").into_iter().map(|(_, u)| u).collect()
}

pub fn gravar_sessao(urls: &[String]) {
    let v: Vec<(String, String)> = urls.iter().map(|u| (String::new(), u.clone())).collect();
    gravar_lista("sessao.tsv", &v);
}

// ---------------------------------------------------------------------------
// Bloqueador de anúncios e rastreadores (a extensão que vem com a Jangada)
// ---------------------------------------------------------------------------

/// Domínios de anúncios e de rastreamento conhecidos (e os subdomínios deles).
const BLOQUEADOS: &[&str] = &[
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

/// O endereço é de anúncio ou rastreador conhecido?
pub fn bloqueado(url: &str) -> bool {
    let Some(resto) = url.split("://").nth(1) else { return false };
    let host = resto.split(['/', '?', '#', ':']).next().unwrap_or("").to_ascii_lowercase();
    let caminho = resto.to_ascii_lowercase();
    BLOQUEADOS.iter().any(|d| {
        if d.contains('/') {
            caminho.starts_with(d) || caminho.contains(&format!(".{d}"))
        } else {
            host == *d || host.ends_with(&format!(".{d}"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloqueador() {
        assert!(bloqueado("https://securepubads.g.doubleclick.net/tag/js/gpt.js"));
        assert!(bloqueado("https://www.google-analytics.com/analytics.js"));
        assert!(bloqueado("https://connect.facebook.net/en_US/fbevents.js"));
        assert!(!bloqueado("https://www.google.com/search?q=doubleclick.net"));
        assert!(!bloqueado("https://pt.wikipedia.org/wiki/Jangada"));
        assert!(!bloqueado("https://notdoubleclick.net/x"));
    }
}
