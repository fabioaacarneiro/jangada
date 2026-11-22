//! As páginas da própria Jangada (HTML feito aqui, nada vem do Chromium): a página
//! inicial, com o logo, a busca e os atalhos dos sites, e a página de erro, quando um
//! site não abre. Seguem o tema do Glove (claro ou escuro).

use std::path::{Path, PathBuf};

use glove_ui::paint::{draw::Canvas, icons, tiny_skia, Fonts};

/// Sites da página inicial: (nome, endereço).
const ATALHOS: [(&str, &str); 8] = [
    ("Wikipédia", "https://pt.wikipedia.org"),
    ("YouTube", "https://www.youtube.com"),
    ("Gmail", "https://mail.google.com"),
    ("Notícias", "https://g1.globo.com"),
    ("GitHub", "https://github.com"),
    ("Mapas", "https://maps.google.com"),
    ("Mercado Livre", "https://www.mercadolivre.com.br"),
    ("gov.br", "https://www.gov.br"),
];

/// Base64 (para pôr as imagens dentro do HTML).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Texto seguro dentro do HTML.
fn esc(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Um ícone do Glove em PNG (`px` pixels).
fn png(fonts: &Fonts, icon: Option<icons::Icon>, px: u32) -> Option<Vec<u8>> {
    let mut c = Canvas::new(px as f32, px as f32, 1.0)?;
    match icon {
        Some(i) => icons::draw_bare(&mut c, fonts, i, 0.0, 0.0, px as f32, None),
        // A luva (o logo do Glove).
        None => icons::logo(&mut c, 0.0, 0.0, px as f32, glove_ui::paint::draw::rgba(40, 110, 220, 255)),
    }
    let size = tiny_skia::IntSize::from_wh(c.px_width(), c.px_height())?;
    tiny_skia::Pixmap::from_vec(c.data().to_vec(), size)?.encode_png().ok()
}

fn img(fonts: &Fonts, icon: Option<icons::Icon>, px: u32) -> String {
    png(fonts, icon, px).map(|d| format!("data:image/png;base64,{}", base64(&d))).unwrap_or_default()
}

/// O tema da página: (fundo, cartão, texto, texto apagado, borda).
fn cores(escuro: bool) -> (&'static str, &'static str, &'static str, &'static str, &'static str) {
    if escuro { ("#1e1e21", "#2c2c30", "#ececf0", "#9a9aa2", "#3c3c42") } else { ("#f2f2f5", "#ffffff", "#1d1d20", "#7a7a82", "#d6d6dc") }
}

/// O que a pessoa escolheu para a página inicial (Preferências ▸ Página Inicial).
#[derive(Clone, Debug, PartialEq)]
pub struct Opcoes {
    /// "" (o padrão do tema), "cor:#rrggbb" ou "imagem:/caminho/da/imagem.jpg".
    pub fundo: String,
    /// Os atalhos dos sites.
    pub atalhos: bool,
    /// A caixa de busca.
    pub busca: bool,
}

impl Default for Opcoes {
    fn default() -> Self {
        Self { fundo: String::new(), atalhos: true, busca: true }
    }
}

/// O fundo da página como CSS: a cor escolhida, a imagem (embutida, para a página ser um
/// arquivo só) ou o padrão do tema (None).
fn fundo_css(fundo: &str) -> Option<String> {
    if let Some(c) = fundo.strip_prefix("cor:") {
        let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit());
        return ok.then(|| format!("background:{c}"));
    }
    let caminho = fundo.strip_prefix("imagem:")?;
    // Imagens grandes não cabem bem numa página: até 8 MB, e acima disso fica o padrão.
    let dados = std::fs::metadata(caminho).ok().filter(|m| m.len() <= 8 << 20).and_then(|_| std::fs::read(caminho).ok())?;
    let tipo = if caminho.to_lowercase().ends_with(".png") { "png" } else if caminho.to_lowercase().ends_with(".webp") { "webp" } else { "jpeg" };
    Some(format!("background:url(data:image/{tipo};base64,{}) center/cover fixed no-repeat", base64(&dados)))
}

fn estilo(escuro: bool, fundo: &str) -> String {
    let (cor_padrao, cartao, texto, apagado, borda) = cores(escuro);
    let fundo_bg = fundo_css(fundo).unwrap_or_else(|| format!("background:{cor_padrao}"));
    format!(
        "*{{box-sizing:border-box}}html,body{{margin:0;height:100%}}\
body{{{fundo_bg};color:{texto};font-family:'Inter','DejaVu Sans',system-ui,sans-serif;display:flex;flex-direction:column;align-items:center}}\
main{{flex:1;display:flex;flex-direction:column;align-items:center;padding-top:9vh;width:100%}}\
.logo{{width:112px;height:112px}}h1{{font-weight:600;font-size:30px;margin:10px 0 22px;letter-spacing:.2px}}\
form{{width:min(600px,86vw)}}input{{width:100%;font-size:17px;padding:13px 20px;border-radius:24px;border:1px solid {borda};background:{cartao};color:{texto};outline:none;box-shadow:0 1px 3px rgba(0,0,0,.08)}}\
input:focus{{border-color:#2f7cf6;box-shadow:0 0 0 4px rgba(47,124,246,.25)}}\
.sites{{display:grid;grid-template-columns:repeat(4,104px);gap:20px 22px;margin-top:40px}}\
.sites a{{display:flex;flex-direction:column;align-items:center;gap:8px;text-decoration:none;color:{texto};font-size:12.5px;max-width:104px}}.sites .nome{{max-width:104px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}}\
.sites .ic{{width:60px;height:60px;border-radius:15px;background:{cartao};border:1px solid {borda};display:flex;align-items:center;justify-content:center;box-shadow:0 1px 3px rgba(0,0,0,.08);transition:transform .12s}}\
.sites a:hover .ic{{transform:scale(1.06)}}.sites img{{width:30px;height:30px}}\
footer{{padding:20px;font-size:11.5px;color:{apagado};display:flex;align-items:center;gap:7px}}footer img{{width:18px;height:18px}}\
.erro{{max-width:560px;text-align:center;padding:0 20px}}.erro p{{color:{apagado};line-height:1.5}}\
.erro code{{font-size:12px;color:{apagado}}}.botao{{display:inline-block;margin-top:18px;padding:9px 22px;border-radius:18px;background:#2f7cf6;color:#fff;text-decoration:none;font-size:14px}}"
    )
}

fn rodape(fonts: &Fonts) -> String {
    format!("<footer><img src=\"{}\" alt=\"\">Jangada · {}</footer>", img(fonts, None, 36), esc(glove_config::tr!("o navegador do Glove OS")))
}

/// Grava a página inicial em `dir` (refeita a cada vez que a Jangada abre: segue o tema,
/// o idioma, o buscador e os favoritos) e devolve o caminho. Os atalhos são os favoritos
/// (até 12); sem favoritos, uns sites conhecidos.
pub fn pagina_inicial(dir: &Path, fonts: &Fonts, escuro: bool, busca: &str, favoritos: &[(String, String)], opc: &Opcoes) -> PathBuf {
    let mut sites = String::new();
    let lista: Vec<(String, String)> = if favoritos.is_empty() {
        ATALHOS.iter().map(|(n, u)| (n.to_string(), u.to_string())).collect()
    } else {
        favoritos.iter().take(12).cloned().collect()
    };
    for (nome, url) in &lista {
        let host = url.split("//").nth(1).unwrap_or(url).split('/').next().unwrap_or("");
        let nome: String = nome.chars().take(22).collect();
        sites.push_str(&format!(
            "<a href=\"{}\"><span class=ic><img src=\"https://icons.duckduckgo.com/ip3/{}.ico\" alt=\"\"></span><span class=nome>{}</span></a>",
            esc(url),
            esc(host),
            esc(&nome)
        ));
    }
    // Sem a busca ou sem os atalhos, essas partes simplesmente não vão para a página.
    let caixa = if opc.busca {
        format!(
            "<form onsubmit=\"var q=this.q.value.trim();if(q)location.href='{busca}'+encodeURIComponent(q);return false\"><input name=q placeholder=\"{dica}\" autofocus autocomplete=off></form>",
            busca = esc(busca),
            dica = esc(glove_config::tr!("Buscar na web")),
        )
    } else {
        String::new()
    };
    let grade = if opc.atalhos { format!("<div class=sites>{sites}</div>") } else { String::new() };
    let html = format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\"><title>{titulo}</title><style>{css}</style></head><body><main>\
<img class=logo src=\"{logo}\" alt=\"\"><h1>Jangada</h1>{caixa}{grade}</main>{rodape}</body></html>",
        lang = if glove_config::idioma().starts_with("en") { "en" } else { "pt-BR" },
        titulo = esc(glove_config::tr!("Início")),
        css = estilo(escuro, &opc.fundo),
        caixa = caixa,
        grade = grade,
        logo = img(fonts, Some(icons::Icon::Jangada), 224),
        rodape = rodape(fonts),
    );
    let p = dir.join("inicio.html");
    let _ = std::fs::write(&p, html);
    p
}

/// A página de erro (o site não abriu), como endereço data: para o motor mostrar.
pub fn pagina_erro(fonts: &Fonts, escuro: bool, url: &str, codigo: &str) -> String {
    let host = url.split("//").nth(1).unwrap_or(url).split('/').next().unwrap_or(url);
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{titulo}</title><style>{css}</style></head><body><main><div class=erro>\
<img class=logo src=\"{logo}\" alt=\"\"><h1>{titulo}</h1><p>{texto}</p><code>{codigo}</code><br>\
<a class=botao href=\"{url}\">{tentar}</a></div></main>{rodape}</body></html>",
        titulo = esc(glove_config::tr!("Não foi possível abrir a página")),
        css = estilo(escuro, ""),
        logo = img(fonts, Some(icons::Icon::Jangada), 224),
        texto = esc(&glove_config::tr!("A Jangada não conseguiu chegar a {}. Confira o endereço e a conexão com a internet.", host)),
        codigo = esc(codigo),
        url = esc(url),
        tentar = esc(glove_config::tr!("Tentar de Novo")),
        rodape = rodape(fonts),
    );
    format!("data:text/html;charset=utf-8;base64,{}", base64(html.as_bytes()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn fundo_da_pagina_inicial() {
        assert_eq!(super::fundo_css("cor:#eceef3").as_deref(), Some("background:#eceef3"));
        // Cor inválida (não é #rrggbb) não vira CSS.
        assert!(super::fundo_css("cor:red").is_none());
        assert!(super::fundo_css("cor:#zzzzzz").is_none());
        // Imagem inexistente cai no padrão do tema.
        assert!(super::fundo_css("imagem:/nao/existe.jpg").is_none());
        assert!(super::fundo_css("").is_none());
    }

    #[test]
    fn base64_igual_ao_padrao() {
        assert_eq!(super::base64(b"Jangada"), "SmFuZ2FkYQ==");
        assert_eq!(super::base64(b"ab"), "YWI=");
        assert_eq!(super::base64(b"abc"), "YWJj");
    }
}
