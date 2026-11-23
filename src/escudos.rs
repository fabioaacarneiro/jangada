//! Os escudos da Jangada, como os do Brave (com o mesmo motor de bloqueio, o `adblock` do
//! Brave): anúncios e rastreadores barrados antes de carregar (EasyList, EasyPrivacy, a
//! lista do Peter Lowe, a portuguesa da EasyList e os filtros do uBlock Origin) e os
//! elementos de anúncio escondidos na página (filtros cosméticos, os do site e os
//! genéricos). Por aba, quantos itens foram barrados; por site, os escudos podem ser
//! desligados.
//!
//! As listas ficam em ~/.cache/jangada/escudos (baixadas pelo curl ou pelo wget, renovadas
//! a cada semana); o motor montado fica guardado pronto (escudos.dat) e abre na hora. Sem
//! as listas (sem internet na primeira vez), vale a lista curta embutida.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::Engine;

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

fn motor() -> &'static RwLock<Option<Engine>> {
    static M: OnceLock<RwLock<Option<Engine>>> = OnceLock::new();
    M.get_or_init(|| RwLock::new(None))
}

/// Por aba (o número do navegador no motor): o site aberto, quantos itens foram barrados
/// nele e as exceções cosméticas do site.
#[derive(Default)]
struct Aba {
    site: String,
    barrados: u32,
    excecoes: HashSet<String>,
}

fn abas() -> &'static Mutex<HashMap<i32, Aba>> {
    static A: OnceLock<Mutex<HashMap<i32, Aba>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Os sites com os escudos desligados.
fn fora() -> &'static RwLock<HashSet<String>> {
    static F: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    F.get_or_init(|| RwLock::new(crate::ajustes::escudos_fora()))
}

fn pasta() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let p = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("jangada/escudos");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// A lista curta embutida (sem as listas baixadas ainda).
fn regras_embutidas() -> String {
    crate::ajustes::BLOQUEADOS.iter().map(|d| if d.contains('/') { format!("||{d}\n") } else { format!("||{d}^\n") }).collect()
}

fn montar(textos: &[String]) -> Engine {
    let mut fs = FilterSet::new(false);
    fs.add_filter_list(regras_embutidas(), ParseOptions::default());
    for t in textos {
        fs.add_filter_list(t.clone(), ParseOptions::default());
    }
    Engine::new_with_filter_set(fs)
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

/// Liga os escudos (numa linha à parte: nada disso atrasa a abertura). Primeiro o motor
/// guardado (ou a lista embutida); depois, se as listas estão velhas, baixa e remonta.
pub fn iniciar() {
    std::thread::spawn(|| {
        let dir = pasta();
        let dat = dir.join("escudos.dat");
        let mut e = Engine::default();
        let pronto = std::fs::read(&dat).ok().is_some_and(|b| e.deserialize(&b).is_ok());
        let e = if pronto { e } else { montar(&[]) };
        *motor().write().unwrap() = Some(e);
        let velhas = LISTAS.iter().any(|(f, _)| velho(&dir.join(f)));
        if pronto && !velhas {
            return;
        }
        let mut novas = false;
        for (f, url) in LISTAS {
            let p = dir.join(f);
            if velho(&p) {
                novas |= baixar(url, &p);
            }
        }
        if !novas && pronto {
            return;
        }
        let textos: Vec<String> = LISTAS.iter().filter_map(|(f, _)| std::fs::read_to_string(dir.join(f)).ok()).collect();
        if textos.is_empty() && pronto {
            return;
        }
        let e = montar(&textos);
        let _ = std::fs::write(&dat, e.serialize());
        *motor().write().unwrap() = Some(e);
    });
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

/// Um pedido da página: barrar? `tipo` no formato do adblock ("script", "image"…);
/// `aba` é o número do navegador. A navegação da aba (tipo "document") zera a contagem.
pub fn barrar(aba: i32, url: &str, tipo: &str) -> bool {
    if tipo == "document" {
        let mut a = abas().lock().unwrap();
        let e = a.entry(aba).or_default();
        e.site = site(url);
        e.barrados = 0;
        e.excecoes.clear();
        return false;
    }
    let origem = abas().lock().unwrap().get(&aba).map(|a| a.site.clone()).unwrap_or_default();
    if !valem(&origem) {
        return false;
    }
    let fonte = format!("https://{origem}/");
    let Ok(req) = Request::new(url, &fonte, tipo, "get") else { return false };
    let barra = motor().read().unwrap().as_ref().is_some_and(|e| e.check_network_request(&req).should_block());
    if barra {
        if let Some(a) = abas().lock().unwrap().get_mut(&aba) {
            a.barrados += 1;
        }
    }
    barra
}

/// Quantos itens foram barrados na aba.
pub fn barrados(aba: i32) -> u32 {
    abas().lock().unwrap().get(&aba).map_or(0, |a| a.barrados)
}

/// A aba fechou.
pub fn esquecer(aba: i32) {
    abas().lock().unwrap().remove(&aba);
}

/// Texto JavaScript entre aspas.
fn js_str(t: &str) -> String {
    let mut o = String::with_capacity(t.len() + 2);
    o.push('"');
    for c in t.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            '<' => o.push_str("\\u003c"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A marca das mensagens dos escudos (o script da página fala com a Jangada pelo console).
pub const MARCA: &str = "\u{1}JANGADA:e:";

/// O script dos escudos para a página carregada: o estilo que esconde os anúncios do site
/// e, sem `generichide`, a leitura das classes e ids da página (os filtros genéricos
/// chegam depois, por `cosmetico_generico`). None: nada a fazer.
pub fn script_da_pagina(aba: i32, url: &str) -> Option<String> {
    let s = site(url);
    if !url.starts_with("http") || !valem(&s) {
        return None;
    }
    let r = motor().read().unwrap().as_ref()?.url_cosmetic_resources(url);
    if let Some(a) = abas().lock().unwrap().get_mut(&aba) {
        a.excecoes = r.exceptions.clone();
    }
    let css = if r.hide_selectors.is_empty() {
        String::new()
    } else {
        format!("{}{{display:none!important}}", r.hide_selectors.iter().cloned().collect::<Vec<_>>().join(","))
    };
    // Só os elementos que chegaram (num feed infinito, ler a página inteira a cada 0,7 s
    // travava o navegador: 7 s travado a cada 45 s de rolagem num teste), aos poucos,
    // quando o navegador está livre.
    let generico = if r.generichide { String::new() } else { GENERICO.replace("@MARCA@", &js_str(MARCA)) };
    Some(format!(
        "(function(){{if(window.__jangadaEscudos)return;window.__jangadaEscudos=1;var st=document.createElement('style');st.id='__jangada_escudos';st.textContent={};(document.head||document.documentElement).appendChild(st);{}}})();",
        js_str(&css),
        generico
    ))
}

/// A leitura das classes e ids novos da página (os filtros genéricos dos escudos). Cada
/// elemento que chega entra numa fila, lida quando o navegador está livre
/// (`requestIdleCallback`); o que é novo vai para a Jangada pelo console.
const GENERICO: &str = r#"var visto={},fila=[document.documentElement],agendado=false;
function olhar(e,c,i){if(e.id&&!visto['#'+e.id]){visto['#'+e.id]=1;i.push(e.id)}var l=e.classList;if(l)for(var k=0;k<l.length;k++){var x=l[k];if(!visto['.'+x]){visto['.'+x]=1;c.push(x)}}}
function processar(p){var c=[],i=[];while(fila.length&&(!p||p.didTimeout||p.timeRemaining()>2)){var e=fila.shift();if(!e.isConnected)continue;olhar(e,c,i);var d=e.querySelectorAll('[class],[id]');for(var k=0;k<d.length;k++)olhar(d[k],c,i)}agendado=false;if(fila.length)agendar();if(c.length||i.length)console.debug(@MARCA@+c.join(' ')+''+i.join(' '))}
function agendar(){if(agendado)return;agendado=true;(window.requestIdleCallback||function(f){return setTimeout(f,200)})(processar,{timeout:1500})}
agendar();new MutationObserver(function(rs){for(var r=0;r<rs.length;r++){var a=rs[r].addedNodes;for(var k=0;k<a.length;k++)if(a[k].nodeType==1)fila.push(a[k])}agendar()}).observe(document.documentElement,{childList:true,subtree:true});"#;

/// As classes e ids que a página mandou (o texto depois da `MARCA`): o estilo genérico a
/// acrescentar, ou None.
pub fn cosmetico_generico(aba: i32, msg: &str) -> Option<String> {
    let (c, i) = msg.split_once('\u{2}')?;
    let excecoes = abas().lock().unwrap().get(&aba).map(|a| a.excecoes.clone()).unwrap_or_default();
    let sel = motor().read().unwrap().as_ref()?.hidden_class_id_selectors(c.split_whitespace(), i.split_whitespace(), &excecoes);
    if sel.is_empty() {
        return None;
    }
    let css = format!("{}{{display:none!important}}", sel.join(","));
    // Numa folha nova: regravar a folha toda a cada resposta fazia o navegador refazer o
    // estilo da página inteira (e a folha só crescia).
    Some(format!("(function(){{var st=document.createElement('style');st.className='__jangada_escudos';st.textContent={};(document.head||document.documentElement).appendChild(st);}})();", js_str(&css)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motor_embutido_barra_rastreador() {
        *motor().write().unwrap() = Some(montar(&[]));
        crate::ajustes::BLOQUEAR.store(true, std::sync::atomic::Ordering::Relaxed);
        barrar(7, "https://exemplo.com.br/", "document");
        assert!(barrar(7, "https://www.google-analytics.com/analytics.js", "script"));
        assert!(!barrar(7, "https://exemplo.com.br/app.js", "script"));
        assert_eq!(barrados(7), 1);
        assert_eq!(site("https://www.YouTube.com:443/watch?v=1"), "youtube.com");
    }

    #[test]
    fn filtros_de_lista() {
        let e = montar(&["||anuncios.exemplo^\n##.propaganda\nexemplo.org##.banner-topo".to_string()]);
        let req = Request::new("https://anuncios.exemplo/x.js", "https://site.org/", "script", "get").unwrap();
        assert!(e.check_network_request(&req).should_block());
        assert!(e.url_cosmetic_resources("https://exemplo.org/").hide_selectors.contains(".banner-topo"));
        assert!(!e.hidden_class_id_selectors(["propaganda"], Vec::<String>::new(), &HashSet::new()).is_empty());
    }
}
