//! Os escudos da Jangada: anúncios e rastreadores barrados pelo próprio motor, com as
//! mesmas listas do Brave e do uBlock (EasyList, EasyPrivacy, a lista do Peter Lowe, a
//! portuguesa da EasyList e os filtros do uBlock Origin). O motor de bloqueio do Brave (o
//! `adblock`) lê as listas e as converte para os filtros de conteúdo do WebKit (o formato
//! do Safari): o WebKit os compila, barra os pedidos e esconde os anúncios da página
//! sozinho, sem script nenhum rodando na página (os feeds longos não travam). Por site,
//! os escudos podem ser desligados.
//!
//! As listas ficam em ~/.cache/jangada/escudos (baixadas pelo curl ou pelo wget, renovadas
//! a cada semana); os filtros compilados ficam guardados ali (abrem na hora). Sem as
//! listas (sem internet na primeira vez), vale a lista curta embutida.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use adblock::lists::{FilterSet, ParseOptions};
use glove_ui::gtk::glib;

/// As listas: (arquivo, endereço).
const LISTAS: [(&str, &str); 5] = [
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt"),
    ("easylist-pt.txt", "https://easylist-downloads.adblockplus.org/easylistportuguese.txt"),
    ("peter-lowe.txt", "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext"),
    ("ublock.txt", "https://ublockorigin.github.io/uAssets/filters/filters.txt"),
];

/// Renovar as listas depois de uma semana.
const VALIDADE: Duration = Duration::from_secs(7 * 24 * 3600);

/// Regras por filtro (o WebKit recusa listas grandes demais).
const POR_FILTRO: usize = 40_000;

/// Os sites com os escudos desligados.
fn fora() -> &'static RwLock<HashSet<String>> {
    static F: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    F.get_or_init(|| RwLock::new(crate::ajustes::escudos_fora()))
}

fn pasta() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let p = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("jangada/shields");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// A lista curta embutida (sem as listas baixadas ainda).
fn regras_embutidas() -> String {
    crate::ajustes::BLOQUEADOS.iter().map(|d| if d.contains('/') { format!("||{d}\n") } else { format!("||{d}^\n") }).collect()
}

/// As regras no formato do WebKit (em pedaços: o WebKit aceita um tanto por filtro).
fn montar(textos: &[String]) -> Vec<String> {
    let mut fs = FilterSet::new(true);
    fs.add_filter_list(regras_embutidas(), ParseOptions::default());
    for t in textos {
        fs.add_filter_list(t.clone(), ParseOptions::default());
    }
    let Ok((regras, _)) = fs.into_content_blocking() else { return Vec::new() };
    regras.chunks(POR_FILTRO).filter_map(|c| serde_json::to_string(c).ok()).collect()
}

fn velho(p: &std::path::Path) -> bool {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_none_or(|d| d > VALIDADE)
}

/// Baixa um arquivo (curl ou wget). Verdadeiro se deu certo.
fn baixar(url: &str, destino: &std::path::Path) -> bool {
    let tmp = destino.with_extension("novo");
    let ok = std::process::Command::new("curl").args(["-fsSL", "--max-time", "60", "-o"]).arg(&tmp).arg(url).status().is_ok_and(|s| s.success())
        || std::process::Command::new("wget").args(["-q", "-T", "60", "-O"]).arg(&tmp).arg(url).status().is_ok_and(|s| s.success());
    ok && std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 1000) && std::fs::rename(&tmp, destino).is_ok()
}

fn textos_das_listas() -> Vec<String> {
    let dir = pasta();
    LISTAS.iter().filter_map(|(f, _)| std::fs::read_to_string(dir.join(f)).ok()).collect()
}

/// Liga os escudos. Primeiro os filtros já compilados (abrem na hora); se faltam ou as
/// listas estão velhas, uma linha à parte baixa as listas e converte, e o WebKit compila.
pub fn iniciar() {
    let dir = pasta();
    let store = webkit6::UserContentFilterStore::new(&dir.join("compiled").to_string_lossy());
    let ids: Vec<String> = std::fs::read_to_string(dir.join("filters.txt")).unwrap_or_default().lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    carregar(store.clone(), ids.clone(), Vec::new());
    let velhas = LISTAS.iter().any(|(f, _)| velho(&dir.join(f)));
    if !ids.is_empty() && !velhas {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();
    std::thread::spawn(move || {
        let mut novas = false;
        for (f, url) in LISTAS {
            let p = dir.join(f);
            if velho(&p) {
                novas |= baixar(url, &p);
            }
        }
        if !novas && !ids.is_empty() {
            return;
        }
        let _ = tx.send(montar(&textos_das_listas()));
    });
    // A conversão volta para a linha principal (o WebKit só compila nela).
    glib::timeout_add_local(Duration::from_millis(500), move || match rx.try_recv() {
        Ok(json) => {
            compilar(store.clone(), json);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

/// Converts the lists into rules in another thread (it takes a few seconds: on the first
/// opening, in the main thread, the window froze) and compiles them in the main one (the
/// WebKit only compiles there).
fn montar_e_compilar(store: webkit6::UserContentFilterStore) {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(montar(&textos_das_listas()));
    });
    glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
        Ok(json) => {
            compilar(store.clone(), json);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

/// Abre os filtros guardados (um por vez); sem nenhum, compila o que houver.
fn carregar(store: webkit6::UserContentFilterStore, mut faltam: Vec<String>, mut prontos: Vec<webkit6::UserContentFilter>) {
    if faltam.is_empty() {
        if prontos.is_empty() {
            montar_e_compilar(store);
        } else {
            crate::motor::usar_filtros(prontos);
        }
        return;
    }
    let id = faltam.remove(0);
    let s2 = store.clone();
    store.load(&id, None::<&webkit6::gio::Cancellable>, move |r| match r {
        Ok(f) => {
            prontos.push(f);
            carregar(s2, faltam, prontos);
        }
        // Um faltou (a pasta foi limpa): compila de novo, das listas que houver.
        Err(_) => montar_e_compilar(s2),
    });
}

/// O WebKit compila as regras (cada pedaço vira um filtro guardado) e elas passam a valer.
fn compilar(store: webkit6::UserContentFilterStore, pedacos: Vec<String>) {
    fn proximo(store: webkit6::UserContentFilterStore, mut resto: Vec<(String, String)>, mut prontos: Vec<webkit6::UserContentFilter>, ids: Vec<String>) {
        if resto.is_empty() {
            let _ = std::fs::write(pasta().join("filters.txt"), ids.join("\n"));
            crate::motor::usar_filtros(prontos);
            return;
        }
        let (id, json) = resto.remove(0);
        let s2 = store.clone();
        store.save(&id, &glib::Bytes::from_owned(json.into_bytes()), None::<&webkit6::gio::Cancellable>, move |r| {
            match r {
                Ok(f) => prontos.push(f),
                Err(e) => eprintln!("jangada: escudos: {e}"),
            }
            proximo(s2, resto, prontos, ids);
        });
    }
    let marca = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let resto: Vec<(String, String)> = pedacos.into_iter().enumerate().map(|(i, j)| (format!("escudos-{marca}-{i}"), j)).collect();
    let ids = resto.iter().map(|(i, _)| i.clone()).collect();
    proximo(store, resto, Vec::new(), ids);
}

/// O site (para as exceções): o host sem o "www.".
pub fn site(url: &str) -> String {
    let resto = url.split("://").nth(1).unwrap_or("");
    let h = resto.split(['/', '?', '#']).next().unwrap_or("").split('@').next_back().unwrap_or("");
    let h = h.split(':').next().unwrap_or("").to_ascii_lowercase();
    h.strip_prefix("www.").unwrap_or(&h).to_string()
}

fn ligados() -> bool {
    crate::ajustes::BLOQUEAR.load(std::sync::atomic::Ordering::Relaxed)
}

/// Os escudos valem neste site?
pub fn valem(site_: &str) -> bool {
    ligados() && !site_.is_empty() && !fora().read().unwrap().contains(site_)
}

/// Relê a lista dos sites sem escudos (depois de mudar fora daqui).
pub fn recarregar_fora() {
    *fora().write().unwrap() = crate::ajustes::escudos_fora();
}

/// Liga ou desliga os escudos num site (grava a lista).
pub fn alternar_site(site_: &str) {
    let mut f = fora().write().unwrap();
    if !f.remove(site_) {
        f.insert(site_.to_string());
    }
    crate::ajustes::gravar_escudos_fora(&f);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regras_no_formato_do_webkit() {
        let j = montar(&["||anuncios.exemplo^\n##.propaganda\nexemplo.org##.banner-topo".to_string()]);
        let tudo = j.join("");
        // O rastreador da lista embutida, o anúncio de rede e os elementos escondidos.
        assert!(tudo.contains("google-analytics"), "{tudo}");
        assert!(tudo.contains("anuncios"), "{tudo}");
        assert!(tudo.contains("css-display-none") && tudo.contains(".propaganda") && tudo.contains(".banner-topo"), "{tudo}");
        assert_eq!(site("https://www.YouTube.com:443/watch?v=1"), "youtube.com");
    }
}
