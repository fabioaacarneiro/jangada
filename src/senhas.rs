//! O gerenciador de senhas da Jangada: as senhas ficam no chaveiro do sistema (veja
//! `chaveiro`). Na página, um script pequeno avisa quando há um campo de senha (a Jangada
//! preenche com a senha guardada do site) e quando um formulário com senha é enviado (a
//! Jangada oferece guardar). A conversa com a página é pelo console, com uma marca que
//! muda a cada vez que a Jangada abre.

use std::cell::RefCell;
use std::sync::OnceLock;

thread_local! {
    /// Senhas enviadas esperando a pergunta "Guardar a senha?": (origem, usuário, senha).
    pub static PARA_GUARDAR: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
}

/// A marca desta vez (a página não a conhece de antemão).
fn marca() -> &'static str {
    static M: OnceLock<String> = OnceLock::new();
    M.get_or_init(|| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        format!("\u{1}JANGADA:s{:x}{:x}:", t ^ 0x9e37_79b9_7f4a_7c15, std::process::id())
    })
}

/// Oferecer para guardar e preencher (Preferências ▸ Senhas).
pub static LIGADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn ligado() -> bool {
    LIGADO.load(std::sync::atomic::Ordering::Relaxed)
}

fn js_str(t: &str) -> String {
    serde_json::to_string(t).unwrap_or_else(|_| "\"\"".into()).replace('<', "\\u003c")
}

/// O script da página (o de cima): pede o preenchimento quando há campo de senha e avisa
/// os envios com senha. None para páginas que não são da web.
pub fn script_da_pagina(url: &str) -> Option<String> {
    if !ligado() || crate::ajustes::privada() || !url.starts_with("http") {
        return None;
    }
    let m = js_str(marca());
    Some(format!(
        r#"(function(){{if(window.__jangadaSenhas)return;window.__jangadaSenhas=1;var M={m};
function senha(r){{return (r||document).querySelector('input[type=password]')}}
function usuario(p){{var l=Array.prototype.slice.call(document.querySelectorAll('input'));var i=l.indexOf(p);for(var k=i-1;k>=0;k--){{var t=(l[k].type||'text').toLowerCase();if((t=='text'||t=='email'||t=='tel')&&l[k].offsetParent!==null)return l[k]}}return null}}
var pedido=false;function pedir(){{if(!pedido&&senha()){{pedido=true;console.debug(M+'p')}}}}
pedir();new MutationObserver(function(){{pedir()}}).observe(document.documentElement,{{childList:true,subtree:true}});
var ultimo='';function avisar(){{var p=senha();if(!p||!p.value)return;var u=usuario(p);var v=(u?u.value:'')+'\u0002'+p.value;if(v==ultimo)return;ultimo=v;console.debug(M+'g'+v)}}
document.addEventListener('submit',avisar,true);
document.addEventListener('click',function(e){{var b=e.target&&e.target.closest&&e.target.closest('button,input[type=submit],[role=button]');if(b)setTimeout(avisar,0)}},true);
document.addEventListener('keydown',function(e){{if(e.key=='Enter'&&e.target&&e.target.tagName=='INPUT')avisar()}},true);
window.__jangadaPreencher=function(u,s){{var p=senha();if(!p)return;function por(el,v){{var d=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value');d.set.call(el,v);el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}))}}var c=usuario(p);if(c&&u&&!c.value)por(c,u);if(!p.value)por(p,s)}};}})();"#
    ))
}

/// Uma mensagem da página: Some(o pedido) se é das senhas.
pub enum Pedido {
    /// Há campo de senha: preencher.
    Preencher,
    /// Um envio com senha: (usuário, senha).
    Guardar(String, String),
}

pub fn ler(msg: &str) -> Option<Pedido> {
    let resto = msg.strip_prefix(marca())?;
    if resto == "p" {
        return Some(Pedido::Preencher);
    }
    let (u, s) = resto.strip_prefix('g')?.split_once('\u{2}')?;
    (!s.is_empty()).then(|| Pedido::Guardar(u.to_string(), s.to_string()))
}

/// O script que preenche (usuário e senha guardados da origem), ou None.
pub fn script_preencher(origem: &str) -> Option<String> {
    let c = crate::chaveiro::Chaveiro::abrir().ok()?;
    let l = c.logins().ok()?.into_iter().find(|l| l.origem == origem)?;
    let s = String::from_utf8(c.segredo(&l.caminho).ok()?).ok()?;
    Some(format!("window.__jangadaPreencher&&window.__jangadaPreencher({},{});", js_str(&l.usuario), js_str(&s)))
}

/// Um envio com senha: entra na fila da pergunta, se for novo (outra senha, outro
/// usuário) e se o site não está em "Nunca".
pub fn enviado(origem: &str, usuario: &str, senha: &str) {
    if crate::ajustes::privada() || crate::ajustes::nunca_guardar().contains(origem) {
        return;
    }
    if let Ok(c) = crate::chaveiro::Chaveiro::abrir() {
        if let Some(l) = c.logins().ok().and_then(|v| v.into_iter().find(|l| l.origem == origem && l.usuario == usuario)) {
            if c.segredo(&l.caminho).ok().is_some_and(|s| s == senha.as_bytes()) {
                return;
            }
        }
    }
    PARA_GUARDAR.with(|p| {
        let mut p = p.borrow_mut();
        p.retain(|(o, u, _)| !(o == origem && u == usuario));
        p.push((origem.to_string(), usuario.to_string(), senha.to_string()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mensagens_da_pagina() {
        let m = marca().to_string();
        assert!(matches!(ler(&format!("{m}p")), Some(Pedido::Preencher)));
        match ler(&format!("{m}gfulano@x.com\u{2}s3nha")) {
            Some(Pedido::Guardar(u, s)) => assert_eq!((u.as_str(), s.as_str()), ("fulano@x.com", "s3nha")),
            _ => panic!("devia pedir para guardar"),
        }
        // Sem a marca desta vez (uma página tentando se passar pela Jangada): nada.
        assert!(ler("\u{1}JANGADA:s0:p").is_none());
        assert!(script_da_pagina("https://exemplo.com/").is_some_and(|s| s.contains("__jangadaPreencher")));
        assert!(script_da_pagina("file:///x").is_none());
    }
}
