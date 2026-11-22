//! Jangada: o navegador do Glove OS. O motor é o do Chromium (CEF: os recursos da web de
//! hoje); a janela, as abas, a barra de endereço, as Preferências e as páginas da própria
//! Jangada (início, erro) são do Glove, no visual clássico ou no moderno. Feita para pesar
//! pouco: sem o processo da placa de vídeo, poucos processos de página e nada de fundo.
//!
//!   jangada [ENDEREÇO…]   abre os endereços (numa janela que já esteja aberta, se houver)

mod ajustes;
mod chaveiro;
mod escudos;
mod importar;
mod inicio;
mod motor;
mod senhas;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ajustes::Ajustes;
use cef::{ImplBrowser, ImplBrowserHost, ImplCommandLine, ImplCookieManager, ImplFrame, ImplJsdialogCallback};
use glove_config::tr;
use glove_ui::{
    paint::{draw::*, icons::Glyph, tiny_skia, Color, Fonts},
    run, App, Column, Cursor, Menu, MenuItem, Rect, TableRow, TableState, TableStyle, Ui, WindowOptions, SEGMENT_H, SEGMENT_W,
};

use motor::{Pagina, Shared};
use std::rc::Rc;

pub const VERSAO: &str = env!("CARGO_PKG_VERSION");

const BARRA_H: f32 = 40.0;
/// Altura do campo de endereço.
const CAMPO_H: f32 = 24.0;
const ABAS_H: f32 = 32.0;
const FAVS_H: f32 = 26.0;
const ACHAR_H: f32 = 32.0;
const STATUS_H: f32 = 20.0;
/// A janela das Preferências.
const JANELA_PREFS: u32 = 1;
/// Onde se inicia a sessão do Google (volta para a página da conta).
const GOOGLE_LOGIN: &str = "https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fmyaccount.google.com%2F";
const JANELA_HIST: u32 = 2;
const JANELA_DOWN: u32 = 3;
const JANELA_IMPORTAR: u32 = 4;

/// Zoom das páginas (porcentagem) e o nível do motor (cada nível = ×1,2).
const ZOOMS: [u32; 7] = [67, 80, 90, 100, 110, 125, 150];
fn nivel(pct: u32) -> f64 {
    (pct as f64 / 100.0).ln() / 1.2f64.ln()
}

pub fn pasta_downloads() -> PathBuf {
    glove_config::arquivos::user_dir("DOWNLOAD").unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads"))
}

fn socket_path() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into())).join("jangada.sock")
}

/// A pasta dos dados do motor (perfil, cookies, a página inicial). A do glove-web (o
/// nome de antes) vira a da Jangada.
fn pasta_dados() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let nova = base.join("jangada");
    let velha = base.join("glove-web");
    if !nova.exists() && velha.is_dir() {
        let _ = std::fs::rename(&velha, &nova);
    }
    let _ = std::fs::create_dir_all(&nova);
    nova
}

/// O que se digitou na barra: endereço (com ou sem https://) ou busca. Vazio = início.
pub fn endereco(t: &str, inicio: &str, busca: &str) -> String {
    let t = t.trim();
    if t.is_empty() {
        return inicio.to_string();
    }
    if t.contains("://") || t.starts_with("about:") || t.starts_with("data:") {
        return t.to_string();
    }
    if t.starts_with('/') {
        return format!("file://{t}");
    }
    let parece = !t.contains(' ') && (t.contains('.') || t.starts_with("localhost")) && !t.ends_with('.');
    if parece {
        return format!("https://{t}");
    }
    format!("{busca}{}", codificar(t))
}

fn codificar(t: &str) -> String {
    t.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else if b == b' ' { "+".into() } else { format!("%{b:02X}") }).collect()
}

/// O endereço inteiro, sem o https:// do começo e com os acentos de volta
/// (%C3%A9 → é), se o resultado for texto válido.
pub fn mostrar(url: &str) -> String {
    let u = url.strip_prefix("https://").unwrap_or(url).trim_end_matches('/');
    let b = u.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&u[i + 1..i + 3], 16) {
                // Espaços e barras codificados ficam como estão (mudariam o endereço).
                if v >= 0x80 {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| u.to_string())
}

/// Só o site (como a barra do Safari mostra fora de foco): "pt.wikipedia.org".
pub fn site(url: &str) -> String {
    let u = url.split("://").nth(1).unwrap_or(url);
    let h = u.split(['/', '?', '#']).next().unwrap_or(u);
    h.strip_prefix("www.").unwrap_or(h).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sugestoes_da_barra() {
        let fav = vec![("Facebook".to_string(), "https://www.facebook.com/".to_string())];
        let hist = vec![
            ("GitHub - gui-python-gtk: Criando interfaces".to_string(), "https://github.com/natorsc/gui".to_string()),
            ("Entrar no Facebook".to_string(), "https://pt-br.facebook.com/login".to_string()),
        ];
        let v = sugestoes_para("face", &fav, &hist);
        // Primeiro a busca, depois o que começa palavra ou endereço, e não o que só contém.
        assert!(v[0].1 == "face");
        assert!(v.iter().any(|(_, u)| u.contains("pt-br.facebook.com")));
        // "interfaces" só contém "face" (não começa palavra): vem depois do Facebook.
        let pos = |p: &str| v.iter().position(|(_, u)| u.contains(p)).unwrap();
        assert!(pos("pt-br.facebook.com") < pos("github"));
        // Endereço completo não oferece a busca.
        let e = sugestoes_para("github.com", &fav, &hist);
        assert!(e.iter().all(|(_, u)| u != "github.com"));
    }

    #[test]
    fn barra_de_endereco() {
        let (ini, b) = ("file:///x/inicio.html", "https://duckduckgo.com/?q=");
        assert_eq!(endereco("wikipedia.org", ini, b), "https://wikipedia.org");
        assert_eq!(endereco("gatos fofos", ini, b), "https://duckduckgo.com/?q=gatos+fofos");
        assert_eq!(endereco("gatos", ini, "https://www.google.com/search?q="), "https://www.google.com/search?q=gatos");
        assert_eq!(endereco("http://a.b/c", ini, b), "http://a.b/c");
        assert_eq!(endereco("/home/x.html", ini, b), "file:///home/x.html");
        assert_eq!(endereco("  ", ini, b), ini);
        assert_eq!(mostrar("https://pt.wikipedia.org/wiki/Dia_Mundial_da_Televis%C3%A3o"), "pt.wikipedia.org/wiki/Dia_Mundial_da_Televisão");
        assert_eq!(mostrar("https://a.b/x%20y/"), "a.b/x%20y");
        assert_eq!(site("https://www.youtube.com/watch?v=1"), "youtube.com");
        assert_eq!(site("http://localhost:8080/a"), "localhost:8080");
        assert!((nivel(100)).abs() < 1e-9);
        assert!((nivel(120) - 1.0).abs() < 1e-9);
    }
}

struct Aba {
    p: Shared,
    /// A lista suspensa da página (<select>), pronta para a janela.
    popup: Option<Canvas>,
    /// O ícone do site, em 16 pontos.
    icone: Option<Canvas>,
    /// O zoom padrão já foi aplicado.
    zoom_ok: bool,
    /// O motor já sabe que a aba está escondida (abas de fundo não desenham).
    oculta: bool,
}

/// A seção aberta nas Preferências.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Secao {
    Geral,
    Contas,
    Senhas,
    Busca,
    Privacidade,
    Extensoes,
    Avancado,
}

struct Navegador {
    abas: Vec<Aba>,
    atual: usize,
    /// O texto da barra de endereço (editado) e se ela está sendo editada.
    barra: String,
    barra_foco: bool,
    focar_barra: bool,
    /// Área da página na janela (do quadro anterior).
    area: Rect,
    escala: f32,
    /// O mouse estava sobre a página (para avisar a saída).
    dentro: bool,
    apertado: bool,
    aviso: Option<(String, Instant)>,
    servidor: Option<UnixListener>,
    /// Última vez que algo mexeu (o motor trabalha mais rápido logo depois).
    ativo: Instant,
    /// O que o último menu de contexto pegou (o item escolhido chega depois).
    ctx_dados: Option<motor::Contexto>,
    /// A página inicial da Jangada (file://…/inicio.html) e onde ela é gravada.
    inicio: String,
    dados: PathBuf,
    escuro: bool,
    /// "Sobre a Jangada" aberto.
    sobre: bool,
    /// As teclas deste quadro já foram da barra de endereço (não vão para a página).
    teclas_da_barra: bool,
    aj: Ajustes,
    favoritos: Vec<(String, String)>,
    /// Sugestões da barra de endereço (o que está sendo digitado): (título, endereço), a
    /// escolhida com as setas, e a área onde a lista está desenhada (o mouse não passa para
    /// a página por baixo dela).
    sugestoes: Vec<(String, String)>,
    sugestao_sel: Option<usize>,
    sugestao_area: Option<Rect>,
    /// O histórico inteiro (para as sugestões; a lista do menu é só dos últimos).
    recentes_todos: Vec<(String, String)>,
    /// Os últimos do histórico (para o menu).
    recentes: Vec<(String, String)>,
    /// Buscar na página: aberto, o texto e se é para pôr o cursor nele.
    achar: Option<String>,
    focar_achar: bool,
    /// As Preferências abertas, a seção e o endereço editado da página inicial.
    prefs: bool,
    secao: Secao,
    inicio_editado: String,
    /// Pergunta das Preferências: 1 = limpar o histórico, 2 = apagar cookies e dados.
    pergunta: u8,
    /// Mudou algo que só vale reabrindo a Jangada.
    reabrir: bool,
    /// Quando o histórico do menu foi relido.
    hist_lido: Instant,
    /// O botão vermelho da barra (a janela não tem a barra de título do Glove).
    fechar_janela: bool,
    /// As abas fechadas (para reabrir: Ctrl+Shift+T), da mais velha para a mais nova.
    fechadas: Vec<String>,
    /// As abas da última vez que a Jangada fechou (Histórico ▸ Reabrir as Abas da Última
    /// Sessão) e o que já foi gravado desta vez.
    sessao_anterior: Vec<String>,
    sessao_gravada: Vec<String>,
    /// Assinatura do estado das abas (a janela só é refeita quando muda).
    assinatura: u64,
    /// A folha Imprimir (a página já em PDF).
    imprimir: Option<glove_ui::print::PrintSheet>,
    /// Janela do histórico: pedida, a busca, a lista filtrada, a escolha e a rolagem.
    hist_abrir: bool,
    hist_busca: String,
    hist_lista: Vec<(String, String)>,
    hist_sel: Option<usize>,
    hist_tabela: TableState,
    /// Janela dos downloads: pedida, a escolha e a rolagem.
    down_abrir: bool,
    down_sel: Option<usize>,
    down_tabela: TableState,
    /// Algo mudou fora dos eventos da janela (a batida redesenha).
    mudou: bool,
    /// Quando a sessão do Google foi conferida (Preferências ▸ Contas).
    google_lido: Instant,
    /// O caminho da imagem de fundo da página inicial (o que está sendo editado).
    fundo_imagem: String,
    /// Preferências ▸ Senhas: as guardadas (sem os segredos), a escolhida, a rolagem e a
    /// senha à vista.
    senhas_lista: Option<Vec<chaveiro::Login>>,
    senhas_sel: Option<usize>,
    senhas_tabela: TableState,
    senha_vista: Option<String>,
    senhas_erro: Option<String>,
    /// Importar de outro navegador: a janela pedida, os perfis achados, o escolhido, o que
    /// importar, o trabalho e o resultado.
    imp_abrir: bool,
    imp_fontes: Vec<importar::Fonte>,
    imp_escolha: usize,
    imp_partes: importar::Partes,
    imp_job: Option<std::sync::mpsc::Receiver<importar::Resultado>>,
    imp_msg: Option<String>,
}

impl Navegador {
    fn aba(&self) -> Option<&Aba> {
        self.abas.get(self.atual)
    }

    fn browser(&self) -> Option<cef::Browser> {
        self.aba().and_then(|a| a.p.borrow().browser.clone())
    }

    fn host(&self) -> Option<cef::BrowserHost> {
        self.browser().and_then(|b| b.host())
    }

    fn url_atual(&self) -> String {
        self.aba().map(|a| a.p.borrow().url.clone()).unwrap_or_default()
    }

    fn titulo_atual(&self) -> String {
        self.aba().map(|a| a.p.borrow().title.clone()).unwrap_or_default()
    }

    fn eh_inicio(&self, url: &str) -> bool {
        url == self.inicio
    }

    /// A página das abas novas e do início: a escolhida nas Preferências ou a da Jangada.
    fn pagina_inicial(&self) -> String {
        if self.aj.inicio.trim().is_empty() { self.inicio.clone() } else { endereco(&self.aj.inicio, &self.inicio, self.aj.busca_url()) }
    }

    /// Refaz a página inicial da Jangada (buscador e favoritos novos).
    fn refazer_inicio(&mut self, fonts: &Fonts) {
        inicio::pagina_inicial(&self.dados, fonts, self.escuro, self.aj.busca_url(), &self.favoritos, &self.aj.opcoes_inicio());
    }

    fn abrir(&mut self, url: &str) {
        self.abrir_em(url, false);
    }

    /// Abre uma aba; `fundo`: por trás (a escolhida continua a mesma).
    fn abrir_em(&mut self, url: &str, fundo: bool) {
        let p: Shared = std::rc::Rc::new(std::cell::RefCell::new(Pagina::new(self.escala)));
        {
            let mut pg = p.borrow_mut();
            pg.w = self.area.w.max(100.0) as i32;
            pg.h = self.area.h.max(100.0) as i32;
            pg.url = url.to_string();
        }
        if motor::nova_aba(&p, url) {
            self.abas.push(Aba { p, popup: None, icone: None, zoom_ok: false, oculta: false });
            if !fundo || self.abas.len() == 1 {
                self.mudar(self.abas.len() - 1);
            }
        }
    }

    /// Aba nova na página inicial, com o cursor na barra de endereço.
    fn nova_aba(&mut self) {
        let i = self.pagina_inicial();
        self.abrir(&i);
        self.focar_barra = true;
    }

    fn mudar(&mut self, i: usize) {
        if let Some(h) = self.host() {
            h.was_hidden(1);
        }
        if let Some(a) = self.abas.get_mut(self.atual) {
            a.oculta = true;
        }
        self.atual = i.min(self.abas.len().saturating_sub(1));
        if let Some(a) = self.abas.get_mut(self.atual) {
            a.oculta = false;
            // A imagem da aba volta inteira na próxima pintura.
            a.p.borrow_mut().dirty = true;
        }
        if let Some(h) = self.host() {
            h.was_hidden(0);
            h.set_focus(1);
        }
        self.mudou = true;
        self.barra_foco = false;
    }

    fn fechar(&mut self, i: usize) {
        if i >= self.abas.len() {
            return;
        }
        let a = self.abas.remove(i);
        let url = a.p.borrow().url.clone();
        if !url.is_empty() && !url.starts_with("data:") && !self.eh_inicio(&url) {
            self.fechadas.push(url);
            if self.fechadas.len() > 30 {
                self.fechadas.remove(0);
            }
        }
        let b = a.p.borrow().browser.clone();
        if let Some(b) = &b {
            escudos::esquecer(b.identifier());
        }
        if let Some(h) = b.and_then(|b| b.host()) {
            h.close_browser(1);
        }
        if self.abas.is_empty() {
            self.nova_aba();
        } else {
            self.mudar(self.atual.min(self.abas.len() - 1));
        }
    }

    fn carregar(&mut self, url: &str) {
        let url = endereco(url, &self.inicio, self.aj.busca_url());
        match self.browser().and_then(|b| b.main_frame()) {
            Some(f) => f.load_url(Some(&url.as_str().into())),
            // O motor ainda está criando a aba: carrega quando ela ficar pronta.
            None => {
                if let Some(a) = self.aba() {
                    a.p.borrow_mut().pendente = Some(url.clone());
                }
            }
        }
        if let Some(a) = self.aba() {
            a.p.borrow_mut().url = url.clone();
        }
        if let Some(h) = self.host() {
            h.set_focus(1);
        }
    }

    fn eh_favorito(&self, url: &str) -> bool {
        !url.is_empty() && self.favoritos.iter().any(|(_, u)| u == url)
    }

    /// A página atual entra nos favoritos (ou sai, se já estava).
    fn alternar_favorito(&mut self) {
        let url = self.url_atual();
        if url.is_empty() || self.eh_inicio(&url) || url.starts_with("data:") {
            return;
        }
        if self.eh_favorito(&url) {
            self.favoritos.retain(|(_, u)| *u != url);
            self.aviso = Some((tr!("Saiu dos Favoritos.").into(), Instant::now()));
        } else {
            let t = self.titulo_atual();
            self.favoritos.push((if t.is_empty() { site(&url) } else { t }, url));
            self.aviso = Some((tr!("Adicionada aos Favoritos.").into(), Instant::now()));
        }
        ajustes::gravar_favoritos(&self.favoritos);
    }

    /// Atalhos da Jangada (do teclado ou dos menus). `pagina`: o foco está na página
    /// (copiar, colar etc. vão para ela). Devolve se tratou.
    fn atalho(&mut self, combo: &str, pagina: bool) -> bool {
        let b = self.browser();
        let frame = || b.as_ref().and_then(|b| b.focused_frame().or_else(|| b.main_frame()));
        match combo {
            "Ctrl+T" => self.nova_aba(),
            "Ctrl+U" => {
                // O código-fonte da página (o próprio motor mostra o texto).
                let u = self.url_atual();
                if !u.is_empty() && !u.starts_with("data:") {
                    self.abrir(&format!("view-source:{u}"));
                }
            }
            "Ctrl+Shift+N" => {
                // Outra janela (um processo), com o armazenamento só em memória.
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--privada").spawn();
                }
            }
            "Ctrl+W" => self.fechar(self.atual),
            "Ctrl+L" | "F6" => self.focar_barra = true,
            "Ctrl+F" => {
                self.achar.get_or_insert_with(String::new);
                self.focar_achar = true;
            }
            "Ctrl+D" => self.alternar_favorito(),
            "Ctrl+P" => {
                if let Some(h) = self.host() {
                    motor::imprimir(&h, &self.dados);
                }
            }
            "Ctrl+Shift+T" => self.reabrir_aba(),
            "Ctrl+H" => self.hist_abrir = true,
            "Ctrl+J" => self.down_abrir = true,
            "Ctrl+," | "Ctrl+comma" => self.prefs = true,
            "Ctrl+Shift+B" => {
                self.aj.barra_favoritos = !self.aj.barra_favoritos;
                self.aj.salvar();
            }
            "Ctrl+R" | "F5" => {
                if let Some(b) = &b {
                    b.reload();
                }
            }
            "Ctrl+Shift+R" => {
                if let Some(b) = &b {
                    b.reload_ignore_cache();
                }
            }
            "Alt+Left" => {
                if let Some(b) = &b {
                    b.go_back();
                }
            }
            "Alt+Right" => {
                if let Some(b) = &b {
                    b.go_forward();
                }
            }
            "Alt+Home" => {
                let i = self.pagina_inicial();
                self.carregar(&i);
            }
            "Ctrl+Tab" if !self.abas.is_empty() => self.mudar((self.atual + 1) % self.abas.len()),
            "Ctrl+Shift+Tab" if !self.abas.is_empty() => self.mudar((self.atual + self.abas.len() - 1) % self.abas.len()),
            "Ctrl+=" | "Ctrl++" | "Ctrl+Shift+=" | "Ctrl+plus" | "Ctrl+equal" => self.zoom(0.5),
            "Ctrl+-" | "Ctrl+minus" => self.zoom(-0.5),
            "Ctrl+0" => {
                if let Some(h) = self.host() {
                    h.set_zoom_level(self.aj.zoom);
                }
            }
            "Ctrl+C" if pagina => frame().map(|f| f.copy()).unwrap_or(()),
            "Ctrl+X" if pagina => frame().map(|f| f.cut()).unwrap_or(()),
            "Ctrl+V" if pagina => frame().map(|f| f.paste()).unwrap_or(()),
            "Ctrl+A" if pagina => frame().map(|f| f.select_all()).unwrap_or(()),
            "Ctrl+Z" if pagina => frame().map(|f| f.undo()).unwrap_or(()),
            "Ctrl+Shift+Z" | "Ctrl+Y" if pagina => frame().map(|f| f.redo()).unwrap_or(()),
            _ => return false,
        }
        true
    }

    /// Reabre a última aba fechada.
    fn reabrir_aba(&mut self) {
        if let Some(u) = self.fechadas.pop() {
            self.abrir(&u);
        }
    }

    /// Reabre as abas da última vez que a Jangada fechou (as que já estão abertas
    /// não repetem).
    fn reabrir_sessao(&mut self) {
        let lista = std::mem::take(&mut self.sessao_anterior);
        let abertas: Vec<String> = self.abas.iter().map(|a| a.p.borrow().url.clone()).collect();
        // A aba vazia da página inicial dá lugar às da sessão.
        let so_inicio = self.abas.len() == 1 && abertas.first().is_some_and(|u| self.eh_inicio(u) || u.is_empty());
        for u in lista.iter().filter(|u| !abertas.contains(u)) {
            self.abrir(u);
        }
        if so_inicio && self.abas.len() > 1 {
            self.fechar(0);
            self.fechadas.pop();
        }
    }

    /// As abas abertas agora (para a próxima vez), gravadas quando mudam.
    fn gravar_sessao(&mut self) {
        let urls: Vec<String> = self.abas.iter().map(|a| a.p.borrow().url.clone()).filter(|u| !u.is_empty() && !u.starts_with("data:") && !self.eh_inicio(u)).collect();
        if urls != self.sessao_gravada {
            ajustes::gravar_sessao(&urls);
            self.sessao_gravada = urls;
        }
    }

    fn zoom(&self, d: f64) {
        if let Some(h) = self.host() {
            h.set_zoom_level((h.zoom_level() + d).clamp(-4.0, 6.0));
        }
    }

    /// Endereços vindos de outra chamada da Jangada (a segunda abre aqui).
    fn ler_servidor(&mut self) {
        let mut novos = Vec::new();
        if let Some(l) = &self.servidor {
            while let Ok((mut c, _)) = l.accept() {
                let _ = c.set_read_timeout(Some(Duration::from_millis(200)));
                let mut t = String::new();
                let _ = c.read_to_string(&mut t);
                novos.extend(t.lines().map(str::to_string));
            }
        }
        for u in novos {
            if u.trim().is_empty() {
                self.nova_aba();
            } else {
                self.abrir(&u);
            }
        }
    }

    /// Botão liso de barra (ícone num quadradinho que acende ao passar o mouse).
    fn botao(ui: &mut Ui, r: Rect, g: Glyph, ativo: bool) -> bool {
        let h = ui.hit(r);
        if ativo && h.hover {
            ui.c.fill(rrect(r.x, r.y, r.w, r.h, 6.0), &solid(rgba(0, 0, 0, if h.down { 34 } else { 18 })));
        }
        let cor = cor_glifo(ui, if ativo { 1.0 } else { 0.35 });
        sem_troca(ui.c, |c| glove_ui::paint::icons::glyph(c, g, r.x + r.w / 2.0, r.y + r.h / 2.0, 13.0, cor));
        ativo && h.clicked
    }

    /// A barra de cima, como a do Safari: os botões da janela e voltar/avançar/início à
    /// esquerda, o endereço no meio (fora de foco só o site, com o cadeado; a estrela dos
    /// favoritos e o recarregar dentro dele) e os downloads à direita. A janela não tem a
    /// barra de título do Glove: esta barra arrasta a janela (e dois cliques dão o zoom).
    fn barra(&mut self, ui: &mut Ui) {
        let w = ui.c.w;
        let modern = ui.theme.modern;
        if modern {
            ui.c.fill_rect(0.0, 0.0, w, BARRA_H, &solid(ui.theme.body_a));
        } else {
            ui.c.fill_rect(0.0, 0.0, w, BARRA_H, &vgrad(0.0, BARRA_H, &[(0.0, rgba(236, 236, 236, 255)), (1.0, rgba(212, 212, 216, 255))]));
        }
        let (back, fwd, loading, url) = self.aba().map(|a| {
            let p = a.p.borrow();
            (p.back, p.forward, p.loading, p.url.clone())
        }).unwrap_or_default();
        // Os botões da janela (fechar, minimizar, zoom), no canto como no Safari.
        if ui.window_buttons(8.0, BARRA_H / 2.0) {
            self.fechar_janela = true;
        }
        let by = (BARRA_H - 26.0) / 2.0;
        let x0 = 86.0;
        if Self::botao(ui, Rect::new(x0, by, 28.0, 26.0), Glyph::Back, back) {
            self.atalho("Alt+Left", false);
        }
        if Self::botao(ui, Rect::new(x0 + 30.0, by, 28.0, 26.0), Glyph::Forward, fwd) {
            self.atalho("Alt+Right", false);
        }
        if Self::botao(ui, Rect::new(x0 + 64.0, by, 28.0, 26.0), Glyph::Home, true) {
            self.atalho("Alt+Home", false);
        }
        // Os downloads, à direita (aparece depois do primeiro download).
        let mut dir = w - 10.0;
        let (baixando, tem) = motor::DOWNLOADS.with(|l| {
            let l = l.borrow();
            (l.iter().any(|d| !d.pronto && !d.cancelado), !l.is_empty())
        });
        if tem {
            let r = Rect::new(dir - 28.0, by, 28.0, 26.0);
            let h = ui.hit(r);
            if h.hover {
                ui.c.fill(rrect(r.x, r.y, r.w, r.h, 6.0), &solid(rgba(0, 0, 0, if h.down { 34 } else { 18 })));
            }
            let cor = if baixando { ui.theme.accent } else { cor_glifo(ui, 1.0) };
            sem_troca(ui.c, |c| seta_download(c, r.x + r.w / 2.0, r.y + r.h / 2.0, cor));
            if h.clicked {
                self.down_abrir = true;
            }
            dir -= 34.0;
        }
        // Os escudos (como o leão do Brave): quantos anúncios e rastreadores esta página
        // teve barrados; o clique abre o menu deles.
        {
            let site_ = escudos::site(&url);
            let web = url.starts_with("http");
            let n = self.browser().map(|b| escudos::barrados(b.identifier())).unwrap_or(0);
            let r = Rect::new(dir - 28.0, by, 28.0, 26.0);
            let h = ui.hit(r);
            if h.hover && web {
                ui.c.fill(rrect(r.x, r.y, r.w, r.h, 6.0), &solid(rgba(0, 0, 0, if h.down { 34 } else { 18 })));
            }
            let ativo = web && escudos::valem(&site_);
            let cor = if ativo { ui.theme.accent } else { cor_glifo(ui, 0.35) };
            sem_troca(ui.c, |c| escudo(c, r.x + r.w / 2.0, r.y + r.h / 2.0, cor, ativo));
            if ativo && n > 0 {
                let t = if n > 99 { "99+".to_string() } else { n.to_string() };
                let tw = ui.fonts.measure(&t, 8.5, true).max(7.0) + 5.0;
                let (bx, byy) = (r.x + r.w - tw + 1.0, r.y + 1.0);
                ui.c.fill(rrect(bx, byy, tw, 11.0, 5.5), &solid(rgba(230, 80, 30, 255)));
                sem_troca(ui.c, |c| ui.fonts.draw(c, &t, bx + (tw - ui.fonts.measure(&t, 8.5, true)) / 2.0, byy + 8.5, 8.5, true, Color::WHITE));
            }
            if h.clicked && web {
                let mut itens = vec![
                    MenuItem::new("esc-site", &tr!("Escudos Ligados em {}", site_), "").checked(escudos::valem(&site_)).enabled(self.aj.bloqueador),
                    MenuItem::new("esc-info", &tr!("{} anúncios e rastreadores barrados nesta página", n), "").enabled(false),
                    MenuItem::Separator,
                ];
                itens.push(MenuItem::new("esc-todos", tr!("Escudos em Todos os Sites"), "").checked(self.aj.bloqueador));
                itens.push(MenuItem::new("esc-cookies", tr!("Barrar Cookies de Terceiros"), "").checked(self.aj.cookies_terceiros));
                itens.push(MenuItem::Separator);
                itens.push(MenuItem::new("esc-prefs", tr!("Preferências dos Escudos…"), ""));
                ui.context_menu((r.x, r.y + r.h + 2.0), &itens);
            }
            dir -= 32.0;
        }
        // O endereço no meio (no máximo 640 pontos, centrado como no Safari).
        let livre = (x0 + 104.0, dir - 6.0);
        let fw = (livre.1 - livre.0).clamp(120.0, 640.0);
        let fx = ((w - fw) / 2.0).max(livre.0).min(livre.1 - fw);
        let fr = Rect::new(fx, ((BARRA_H - CAMPO_H) / 2.0).round(), fw, CAMPO_H);
        let autofocus = std::mem::take(&mut self.focar_barra);
        if autofocus {
            if let Some(h) = self.host() {
                h.set_focus(0);
            }
            // O campo começa vazio: digitar já troca o endereço; Esc devolve.
            self.barra.clear();
        }
        if self.barra_foco || autofocus {
            let (_, enter) = ui.text_field_in(fr, &mut self.barra, autofocus);
            self.barra_foco = ui.has_focus();
            if self.barra.is_empty() {
                let dica = if ajustes::privada() { tr!("Janela privada: nada fica guardado").to_string() } else { tr!("Buscar ou digitar um endereço").to_string() };
                ui.fonts.draw(ui.c, &dica, fr.x + 8.0, fr.y + CAMPO_H / 2.0 + 4.5, 12.5, false, rgba(150, 150, 156, 255));
            }
            if self.barra_foco && ui.keys().iter().any(|k| k.key == glove_ui::ui::Key::Escape) {
                self.barra_foco = false;
                if let Some(h) = self.host() {
                    h.set_focus(1);
                }
            }
            if enter || self.barra_foco {
                self.teclas_da_barra = true;
            }
            // As sugestões: o que já foi visitado ou é favorito, e a busca pelo que foi digitado.
            // No quadro do Enter o campo já pode ter limpado o texto: a lista é a do quadro
            // anterior (ela não é refeita nesse quadro).
            let digitado = self.barra.trim().to_string();
            if enter {
                // a lista e a escolha ficam como estavam
            } else if self.barra_foco && !digitado.is_empty() {
                let antes = self.sugestoes.clone();
                self.sugestoes = sugestoes_para(&digitado, &self.favoritos, &self.recentes_todos);
                if self.sugestoes != antes {
                    self.sugestao_sel = None;
                }
                // As setas escolhem uma sugestão (o campo de uma linha não as usa).
                let baixo = ui.keys().iter().filter(|k| k.key == glove_ui::ui::Key::Down).count();
                let cima = ui.keys().iter().filter(|k| k.key == glove_ui::ui::Key::Up).count();
                let n = self.sugestoes.len();
                if n > 0 {
                    for _ in 0..baixo {
                        self.sugestao_sel = Some(self.sugestao_sel.map_or(0, |i| (i + 1).min(n - 1)));
                    }
                    for _ in 0..cima {
                        self.sugestao_sel = match self.sugestao_sel {
                            Some(0) | None => None,
                            Some(i) => Some(i - 1),
                        };
                    }
                }
            } else {
                self.sugestoes.clear();
                self.sugestao_sel = None;
            }
            if enter {
                let t = match self.sugestao_sel.and_then(|i| self.sugestoes.get(i)) {
                    Some((_, u)) => u.clone(),
                    None => self.barra.clone(),
                };
                self.carregar(&t);
                self.barra_foco = false;
                self.sugestoes.clear();
                self.sugestao_sel = None;
            }
        } else {
            // Fora de foco: uma pílula com o site no meio (e o cadeado), como no Safari.
            let hit = ui.hit(fr);
            let fill = if ui.theme.dark { rgba(58, 58, 62, 255) } else if modern { rgba(0, 0, 0, 14) } else { Color::WHITE };
            ui.c.fill(rrect(fr.x, fr.y, fr.w, fr.h, 7.0), &solid(fill));
            if !modern {
                ui.c.stroke(rrect(fr.x + 0.5, fr.y + 0.5, fr.w - 1.0, fr.h - 1.0, 7.0), &solid(rgba(190, 190, 196, 255)), 1.0);
            }
            let vazio = self.eh_inicio(&url) || url.is_empty();
            let texto = if vazio { tr!("Buscar ou digitar um endereço").to_string() } else if self.aj.endereco_completo { mostrar(&url) } else { site(&mostrar(&url)) };
            let t = ui.fonts.ellipsize(&texto, 13.0, false, fr.w - 80.0);
            let tw = ui.fonts.measure(&t, 13.0, false);
            let seguro = url.starts_with("https://");
            let lock_w = if seguro && !vazio { 15.0 } else { 0.0 };
            let tx = fr.x + (fr.w - tw - lock_w) / 2.0 + lock_w;
            if lock_w > 0.0 {
                let cor = cor_glifo(ui, 0.7);
                sem_troca(ui.c, |c| cadeado(c, tx - 14.0, fr.y + (CAMPO_H - 10.0) / 2.0, cor));
            }
            ui.fonts.draw(ui.c, &t, tx, fr.y + CAMPO_H / 2.0 + 4.5, 13.0, false, if vazio { rgba(150, 150, 156, 255) } else { tinta() });
            if hit.clicked {
                self.focar_barra = true;
                ui.redraw = true;
            }
            if hit.hover {
                ui.set_cursor(Cursor::Text);
            }
            // A estrela dos favoritos, à esquerda dentro do campo.
            if !vazio {
                let sr = Rect::new(fr.x + 3.0, fr.y + (CAMPO_H - 20.0) / 2.0, 20.0, 20.0);
                let sh = ui.hit(sr);
                let fav = self.eh_favorito(&url);
                let cor = if fav { rgba(240, 170, 20, 255) } else { cor_glifo(ui, 0.65) };
                sem_troca(ui.c, |c| estrela(c, sr.x + 10.0, sr.y + 10.0, 6.5, fav, cor));
                if sh.clicked {
                    self.alternar_favorito();
                }
            }
            // Recarregar (ou parar, carregando), à direita dentro do campo.
            let rr = Rect::new(fr.x + fr.w - 23.0, fr.y + (CAMPO_H - 20.0) / 2.0, 20.0, 20.0);
            let h = ui.hit(rr);
            if h.hover {
                ui.c.fill(rrect(rr.x, rr.y, rr.w, rr.h, 5.0), &solid(rgba(0, 0, 0, 20)));
            }
            let cor = cor_glifo(ui, 0.8);
            sem_troca(ui.c, |c| glove_ui::paint::icons::glyph(c, if loading { Glyph::Stop } else { Glyph::RotateRight }, rr.x + 10.0, rr.y + 10.0, 11.0, cor));
            if h.clicked {
                if let Some(b) = self.browser() {
                    if loading { b.stop_load() } else { b.reload() }
                }
            }
        }
        // Carregando: uma faixa azul fina embaixo do campo, como no Safari.
        if loading {
            let t = (self.ativo.elapsed().as_millis() % 1200) as f32 / 1200.0;
            let bw = fr.w * 0.3;
            let x0 = (fr.x - bw + t * (fr.w + bw)).max(fr.x);
            let x1 = (fr.x - bw + t * (fr.w + bw) + bw).min(fr.x + fr.w);
            if x1 > x0 {
                ui.c.fill_rect(x0, fr.y + fr.h - 2.0, x1 - x0, 2.0, &solid(ui.theme.accent));
            }
        }
        // O resto da barra arrasta a janela.
        ui.toolbar_drag(Rect::new(0.0, 0.0, w, BARRA_H));
    }

    /// As abas, como as do Safari: todas da mesma largura numa faixa, a escolhida em
    /// destaque (a mais clara, com sombra), as outras um tom acima da faixa, o ícone do
    /// site e o título no meio, o × ao passar o mouse e um + logo depois da última. O
    /// espaço livre da faixa também arrasta a janela. Devolve a altura usada.
    fn faixa_abas(&mut self, ui: &mut Ui, y: f32) -> f32 {
        let w = ui.c.w;
        let escuro = ui.theme.dark;
        let modern = ui.theme.modern;
        // Faixa < aba de fundo < aba escolhida (do mais escuro para o mais claro).
        let (fundo, apagada, acesa, escolhida) = if escuro {
            (rgba(30, 30, 33, 255), rgba(42, 42, 46, 255), rgba(52, 52, 57, 255), rgba(70, 70, 75, 255))
        } else if modern {
            (rgba(218, 218, 222, 255), rgba(228, 228, 232, 255), rgba(236, 236, 240, 255), Color::WHITE)
        } else {
            (rgba(200, 200, 205, 255), rgba(214, 214, 219, 255), rgba(224, 224, 228, 255), rgba(246, 246, 248, 255))
        };
        ui.c.fill_rect(0.0, y, w, ABAS_H, &solid(fundo));
        ui.c.hline(0.0, y, w, if escuro { rgba(0, 0, 0, 120) } else { rgba(0, 0, 0, 30) });
        ui.c.hline(0.0, y + ABAS_H - 1.0, w, if escuro { rgba(0, 0, 0, 160) } else { rgba(150, 150, 156, 255) });
        let n = self.abas.len().max(1) as f32;
        let (x0, gap, mais) = (6.0, 4.0, 28.0);
        let tw = ((w - x0 * 2.0 - mais - gap * n) / n).clamp(80.0, 260.0);
        let mut acao: Option<(usize, bool)> = None;
        let mut fim = x0;
        for i in 0..self.abas.len() {
            let r = Rect::new(x0 + i as f32 * (tw + gap), y + 4.0, tw, ABAS_H - 8.0);
            if r.x + r.w > w - mais - x0 {
                break;
            }
            fim = r.x + r.w + gap;
            let h = ui.hit(r);
            let sel = i == self.atual;
            if sel {
                ui.c.fill(rrect(r.x, r.y + 1.0, r.w, r.h, 7.0), &solid(rgba(0, 0, 0, if escuro { 70 } else { 30 })));
                ui.c.fill(rrect(r.x, r.y, r.w, r.h, 7.0), &solid(escolhida));
            } else {
                ui.c.fill(rrect(r.x, r.y, r.w, r.h, 7.0), &solid(if h.hover { acesa } else { apagada }));
            }
            let (titulo, url) = {
                let p = self.abas[i].p.borrow();
                (p.title.clone(), p.url.clone())
            };
            let titulo = if !titulo.is_empty() { titulo } else if self.eh_inicio(&url) { tr!("Início").to_string() } else { site(&mostrar(&url)) };
            let tem_icone = self.abas[i].icone.is_some();
            let iw = if tem_icone { 20.0 } else { 0.0 };
            let t = ui.fonts.ellipsize(&titulo, 12.0, false, r.w - 48.0 - iw);
            let lw = ui.fonts.measure(&t, 12.0, false);
            let lx = r.x + (r.w - lw - iw) / 2.0 + iw;
            if let Some(ic) = &self.abas[i].icone {
                ui.c.draw_canvas(ic, lx - iw, r.y + (r.h - 16.0) / 2.0);
            }
            let cor = if sel { tinta() } else { rgba(90, 90, 96, 255) };
            ui.fonts.draw(ui.c, &t, lx, r.y + r.h / 2.0 + 4.5, 12.0, false, cor);
            // O × aparece ao passar o mouse (e na escolhida), à esquerda.
            if h.hover || sel {
                let cr = Rect::new(r.x + 6.0, r.y + (r.h - 16.0) / 2.0, 16.0, 16.0);
                let ch = ui.hit(cr);
                if ch.hover {
                    ui.c.fill(rrect(cr.x, cr.y, cr.w, cr.h, 4.0), &solid(rgba(0, 0, 0, 30)));
                }
                let cor = cor_glifo(ui, 0.8);
                sem_troca(ui.c, |c| glove_ui::paint::icons::glyph(c, Glyph::Stop, cr.x + 8.0, cr.y + 8.0, 9.0, cor));
                if ch.clicked {
                    acao = Some((i, true));
                    continue;
                }
            }
            // O botão do meio fecha a aba (como nos outros navegadores).
            if ui.middle_button(r) == Some(false) {
                acao = Some((i, true));
                continue;
            }
            if h.clicked {
                acao = Some((i, false));
            }
        }
        // O + da aba nova, logo depois da última.
        let pr = Rect::new(fim, y + 4.0, mais, ABAS_H - 8.0);
        if Self::botao(ui, pr, Glyph::Plus, true) {
            self.nova_aba();
        }
        match acao {
            Some((i, true)) => self.fechar(i),
            Some((i, false)) => self.mudar(i),
            None => {}
        }
        // O espaço livre arrasta a janela (e dois cliques dão o zoom).
        ui.toolbar_drag(Rect::new(0.0, y, w, ABAS_H));
        ABAS_H
    }

    /// A barra dos favoritos (Visualizar ▸ Barra de Favoritos), embaixo do endereço.
    fn barra_favoritos(&mut self, ui: &mut Ui, y: f32) -> f32 {
        if !self.aj.barra_favoritos || self.favoritos.is_empty() {
            return 0.0;
        }
        let w = ui.c.w;
        ui.c.fill_rect(0.0, y, w, FAVS_H, &solid(if ui.theme.modern { ui.theme.body_a } else { rgba(226, 226, 230, 255) }));
        ui.c.hline(0.0, y + FAVS_H - 1.0, w, rgba(180, 180, 186, 255));
        let mut x = 10.0;
        let mut ir = None;
        for (i, (titulo, _)) in self.favoritos.iter().enumerate() {
            let t = ui.fonts.ellipsize(titulo, 11.5, false, 160.0);
            let tw = ui.fonts.measure(&t, 11.5, false) + 16.0;
            if x + tw > w - 10.0 {
                break;
            }
            let r = Rect::new(x, y + 3.0, tw, FAVS_H - 6.0);
            let h = ui.hit(r);
            if h.hover {
                ui.c.fill(rrect(r.x, r.y, r.w, r.h, 5.0), &solid(rgba(0, 0, 0, 22)));
            }
            ui.fonts.draw(ui.c, &t, x + 8.0, y + FAVS_H / 2.0 + 4.0, 11.5, false, tinta());
            if h.clicked {
                ir = Some(i);
            }
            x += tw + 4.0;
        }
        if let Some(i) = ir {
            let u = self.favoritos[i].1.clone();
            self.carregar(&u);
        }
        FAVS_H
    }

    /// Buscar na página (Ctrl+F): o campo, quantos achou, anterior/próximo e Concluído.
    fn barra_achar(&mut self, ui: &mut Ui, y: f32) -> f32 {
        let Some(mut texto) = self.achar.clone() else { return 0.0 };
        let w = ui.c.w;
        ui.c.fill_rect(0.0, y, w, ACHAR_H, &solid(if ui.theme.modern { ui.theme.body_a } else { rgba(226, 226, 230, 255) }));
        ui.c.hline(0.0, y + ACHAR_H - 1.0, w, rgba(180, 180, 186, 255));
        let fr = Rect::new(w - 420.0, y + 5.0, 220.0, 22.0);
        let foco = std::mem::take(&mut self.focar_achar);
        if foco {
            if let Some(h) = self.host() {
                h.set_focus(0);
            }
        }
        let (mudou, enter) = ui.text_field_in(fr, &mut texto, foco);
        if ui.has_focus() {
            self.teclas_da_barra = true;
        }
        let (atual, total) = self.aba().map(|a| a.p.borrow().achados).unwrap_or_default();
        let info = if texto.is_empty() {
            String::new()
        } else if total == 0 {
            tr!("Nada encontrado").to_string()
        } else if atual > 0 {
            tr!("{} de {}", atual, total)
        } else {
            tr!("{} encontrados", total)
        };
        ui.fonts.draw(ui.c, &info, fr.x - 12.0 - ui.fonts.measure(&info, 11.5, false), y + 20.0, 11.5, false, rgba(90, 90, 96, 255));
        let anterior = ui.segmented_in(Rect::new(fr.x + fr.w + 8.0, y + (ACHAR_H - SEGMENT_H) / 2.0, 2.0 * SEGMENT_W, SEGMENT_H), &[Glyph::Back, Glyph::Forward], None, &[]);
        let mut fechar = false;
        ui.area(Rect::new(w - 110.0, y + 4.0, 100.0, 24.0), |ui| fechar = ui.small_button(tr!("Concluído")));
        let esc = ui.keys().iter().any(|k| k.key == glove_ui::ui::Key::Escape);
        if let Some(h) = self.host() {
            if mudou {
                if texto.is_empty() {
                    h.stop_finding(1);
                } else {
                    h.find(Some(&texto.as_str().into()), 1, 0, 0);
                }
            } else if enter || anterior.is_some() {
                let frente = anterior != Some(0) && !ui.mods().shift;
                if !texto.is_empty() {
                    h.find(Some(&texto.as_str().into()), frente as i32, 0, 1);
                }
            }
            if fechar || esc {
                h.stop_finding(1);
                h.set_focus(1);
            }
        }
        self.achar = if fechar || esc { None } else { Some(texto) };
        ACHAR_H
    }

    /// Copia a imagem nova da página (se houver) para os canvas da aba.
    fn atualizar_tela(&mut self) {
        let escala = self.escala;
        let zoom = self.aj.zoom;
        for (k, a) in self.abas.iter_mut().enumerate() {
            let mut p = a.p.borrow_mut();
            // Endereço pedido antes de a aba ficar pronta.
            if p.pendente.is_some() {
                if let Some(f) = p.browser.as_ref().and_then(|b| b.main_frame()) {
                    let u = p.pendente.take().unwrap_or_default();
                    f.load_url(Some(&u.as_str().into()));
                }
            }
            // O zoom padrão das Preferências, quando o motor já criou a aba.
            if !a.zoom_ok {
                if let Some(h) = p.browser.as_ref().and_then(|b| b.host()) {
                    if zoom != 0.0 {
                        h.set_zoom_level(zoom);
                    }
                    a.zoom_ok = true;
                }
            }
            // O ícone do site (qualquer aba): refeito em 16 pontos.
            if std::mem::take(&mut p.favicon_novo) {
                a.icone = p.favicon.as_ref().and_then(|(d, w, h)| icone_16(d, *w, *h, escala));
            }
            // Aba aberta por trás: o motor para de desenhá-la.
            if k != self.atual && !a.oculta {
                if let Some(h) = p.browser.as_ref().and_then(|b| b.host()) {
                    h.was_hidden(1);
                    a.oculta = true;
                }
            }
            if k != self.atual || !p.dirty {
                continue;
            }
            p.dirty = false;
            // A imagem da página já vem pronta do motor (`p.tela`); a lista suspensa, aqui.
            a.popup = None;
            if p.popup.is_some() && p.pw > 0 && p.ph > 0 && p.popup_frame.len() == (p.pw * p.ph * 4) as usize {
                if let Some(mut c) = Canvas::new(p.pw as f32 / escala, p.ph as f32 / escala, escala).filter(|c| c.px_width() == p.pw as u32 && c.px_height() == p.ph as u32) {
                    c.pixels_mut().copy_from_slice(&p.popup_frame);
                    a.popup = Some(c);
                }
            }
        }
    }

    /// O mouse e o teclado vão para a página (o que não é das barras).
    fn entrada(&mut self, ui: &mut Ui, area: Rect) {
        let Some(h) = self.host() else { return };
        let (px, py) = ui.pointer();
        // Sobre a lista de sugestões, o mouse não vai para a página.
        if self.sugestao_area.is_some_and(|r| r.contains((px, py))) {
            return;
        }
        let mods = ui.mods().clone();
        let flags = (if mods.shift { 1 << 1 } else { 0 }) | (if mods.ctrl { 1 << 2 } else { 0 }) | (if mods.alt { 1 << 3 } else { 0 }) | (if ui.mouse_down() { 1 << 4 } else { 0 });
        let ev = cef::MouseEvent { x: (px - area.x).round() as i32, y: (py - area.y).round() as i32, modifiers: flags };
        let inside = ui.pointer_inside() && area.contains((px, py));
        if inside {
            // O cursor que a página pediu (mãozinha nos links, barra nos textos).
            let c = self.aba().map(|a| a.p.borrow().cursor).unwrap_or_default();
            ui.set_cursor(c);
        }
        if inside || self.apertado {
            if ui.moved() {
                h.send_mouse_move_event(Some(&ev), 0);
                self.ativo = Instant::now();
            }
            self.dentro = true;
        } else if self.dentro {
            h.send_mouse_move_event(Some(&ev), 1);
            self.dentro = false;
        }
        if inside && ui.mouse_pressed() {
            h.set_focus(1);
            self.barra_foco = false;
            self.apertado = true;
            h.send_mouse_click_event(Some(&ev), cef::MouseButtonType::LEFT, 0, ui.click_count().max(1) as i32);
            self.ativo = Instant::now();
        }
        if self.apertado && ui.mouse_released() {
            self.apertado = false;
            h.send_mouse_click_event(Some(&ev), cef::MouseButtonType::LEFT, 1, ui.click_count().max(1) as i32);
        }
        if ui.right_clicked(area) {
            h.set_focus(1);
            h.send_mouse_click_event(Some(&ev), cef::MouseButtonType::RIGHT, 0, 1);
            h.send_mouse_click_event(Some(&ev), cef::MouseButtonType::RIGHT, 1, 1);
        }
        // Botão do meio: o motor abre o link numa aba nova (e rola com ele, como no Chrome).
        if let Some(desce) = ui.middle_button(area) {
            h.send_mouse_click_event(Some(&ev), cef::MouseButtonType::MIDDLE, (!desce) as i32, 1);
            self.ativo = Instant::now();
        }
        // A roda vem em pixels (para baixo = positivo); o motor quer o contrário e maior.
        // Shift + roda e a roda de lado do touchpad rolam para os lados.
        let (mut sx, mut sy) = (ui.scroll_x_in(area), ui.scroll_in(area));
        if mods.shift && sx == 0.0 {
            (sx, sy) = (sy, 0.0);
        }
        if sx != 0.0 || sy != 0.0 {
            h.send_mouse_wheel_event(Some(&ev), (-sx * 4.0).round() as i32, (-sy * 4.0).round() as i32);
            self.ativo = Instant::now();
        }
        // Teclado: com um campo da Jangada em foco, só os atalhos dela.
        let keys = ui.keys().to_vec();
        if self.barra_foco || std::mem::take(&mut self.teclas_da_barra) {
            for k in keys {
                if matches!(k.combo.as_str(), "Ctrl+T" | "Ctrl+W" | "Ctrl+Tab" | "Ctrl+Shift+Tab" | "Ctrl+R" | "F5" | "Ctrl+," | "Ctrl+comma" | "Ctrl+P" | "Ctrl+Shift+T" | "Ctrl+H" | "Ctrl+J" | "Ctrl+Shift+N" | "Ctrl+U") {
                    self.atalho(&k.combo, false);
                }
            }
            return;
        }
        let mut handled_text = false;
        for k in keys {
            use glove_ui::ui::Key;
            if self.atalho(&k.combo, true) {
                handled_text = true;
                continue;
            }
            let vk = match k.key {
                Key::Backspace => 8,
                Key::Tab => 9,
                Key::Enter => 13,
                Key::Escape => 27,
                Key::PageUp => 33,
                Key::PageDown => 34,
                Key::End => 35,
                Key::Home => 36,
                Key::Left => 37,
                Key::Up => 38,
                Key::Right => 39,
                Key::Down => 40,
                Key::Delete => 46,
                Key::Other => 0,
            };
            if vk == 0 {
                continue;
            }
            let mut e = cef::KeyEvent { size: std::mem::size_of::<cef::sys::cef_key_event_t>(), type_: cef::KeyEventType::RAWKEYDOWN, modifiers: flags, windows_key_code: vk, ..Default::default() };
            h.send_key_event(Some(&e));
            if vk == 13 || vk == 9 || vk == 8 {
                e.type_ = cef::KeyEventType::CHAR;
                e.character = if vk == 8 { 8 } else if vk == 9 { 9 } else { 13 };
                e.unmodified_character = e.character;
                h.send_key_event(Some(&e));
            }
            e.type_ = cef::KeyEventType::KEYUP;
            h.send_key_event(Some(&e));
            self.ativo = Instant::now();
        }
        if !handled_text {
            let text = ui.typed().to_string();
            for ch in text.chars().filter(|c| !c.is_control()) {
                let mut buf = [0u16; 2];
                for u in ch.encode_utf16(&mut buf).iter() {
                    let e = cef::KeyEvent { size: std::mem::size_of::<cef::sys::cef_key_event_t>(), type_: cef::KeyEventType::CHAR, modifiers: flags & !(1 << 4), windows_key_code: *u as i32, character: *u, unmodified_character: *u, ..Default::default() };
                    h.send_key_event(Some(&e));
                }
                self.ativo = Instant::now();
            }
        }
    }

    /// O menu de contexto que a página pediu (link, seleção, campo).
    fn menu_contexto(&mut self, ui: &mut Ui, area: Rect) {
        let Some(c) = self.aba().and_then(|a| a.p.borrow_mut().ctx.take()) else { return };
        let mut itens = Vec::new();
        if !c.link.is_empty() {
            itens.push(MenuItem::new("ctx-link-aba", tr!("Abrir Link em Nova Aba"), ""));
            itens.push(MenuItem::new("ctx-link-copiar", tr!("Copiar Link"), ""));
            itens.push(MenuItem::new("ctx-link-favorito", tr!("Adicionar Link aos Favoritos"), ""));
            itens.push(MenuItem::Separator);
        }
        if !c.image.is_empty() {
            itens.push(MenuItem::new("ctx-img-aba", tr!("Abrir Imagem em Nova Aba"), ""));
            itens.push(MenuItem::Separator);
        }
        if c.editable {
            itens.push(MenuItem::new("ctx-recortar", tr!("Recortar"), ""));
        }
        if c.editable || !c.selection.is_empty() {
            itens.push(MenuItem::new("ctx-copiar", tr!("Copiar"), ""));
        }
        if c.editable {
            itens.push(MenuItem::new("ctx-colar", tr!("Colar"), ""));
        }
        if !c.selection.is_empty() {
            itens.push(MenuItem::new("ctx-buscar", &tr!("Buscar “{}”", c.selection.chars().take(24).collect::<String>()), ""));
        }
        if itens.last().is_some_and(|i| !matches!(i, MenuItem::Separator)) {
            itens.push(MenuItem::Separator);
        }
        let (back, fwd) = self.aba().map(|a| (a.p.borrow().back, a.p.borrow().forward)).unwrap_or_default();
        itens.push(MenuItem::new("ctx-voltar", tr!("Voltar"), "").enabled(back));
        itens.push(MenuItem::new("ctx-avancar", tr!("Avançar"), "").enabled(fwd));
        itens.push(MenuItem::new("ctx-recarregar", tr!("Recarregar"), ""));
        itens.push(MenuItem::Separator);
        itens.push(MenuItem::new("ctx-pdf", tr!("Salvar Página como PDF"), ""));
        self.ctx_dados = Some(c.clone());
        ui.context_menu((area.x + c.x as f32, area.y + c.y as f32), &itens);
    }

    fn acao_contexto(&mut self, id: &str) {
        let Some(c) = self.ctx_dados.clone() else { return };
        let b = self.browser();
        let frame = || b.as_ref().and_then(|b| b.focused_frame().or_else(|| b.main_frame()));
        match id {
            "ctx-link-aba" => self.abrir(&c.link),
            "ctx-img-aba" => self.abrir(&c.image),
            "ctx-link-copiar" => copiar_texto(&c.link),
            "ctx-link-favorito" => {
                if !self.eh_favorito(&c.link) {
                    self.favoritos.push((site(&c.link), c.link.clone()));
                    ajustes::gravar_favoritos(&self.favoritos);
                    self.aviso = Some((tr!("Adicionada aos Favoritos.").into(), Instant::now()));
                }
            }
            "ctx-recortar" => frame().map(|f| f.cut()).unwrap_or(()),
            "ctx-copiar" => frame().map(|f| f.copy()).unwrap_or(()),
            "ctx-colar" => frame().map(|f| f.paste()).unwrap_or(()),
            "ctx-buscar" => {
                let u = format!("{}{}", self.aj.busca_url(), codificar(&c.selection));
                self.abrir(&u);
            }
            "ctx-voltar" => {
                self.atalho("Alt+Left", false);
            }
            "ctx-avancar" => {
                self.atalho("Alt+Right", false);
            }
            "ctx-recarregar" => {
                self.atalho("Ctrl+R", false);
            }
            "ctx-pdf" => {
                self.atalho("Ctrl+P", false);
            }
            _ => {}
        }
    }

    /// Itens dos menus da barra: favoritos e histórico.
    fn acao_menu(&mut self, id: &str) {
        if let Some(i) = id.strip_prefix("fav:").and_then(|n| n.parse::<usize>().ok()) {
            if let Some((_, u)) = self.favoritos.get(i).cloned() {
                self.carregar(&u);
            }
        } else if let Some(i) = id.strip_prefix("hist:").and_then(|n| n.parse::<usize>().ok()) {
            if let Some((_, u)) = self.recentes.get(i).cloned() {
                self.carregar(&u);
            }
        } else {
            match id {
                "sobre" => self.sobre = true,
                "reabrir-sessao" => self.reabrir_sessao(),
                "nova-privada" => {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).arg("--privada").spawn();
                    }
                }
                "importar" => self.imp_abrir = true,
                "codigo" => {
                    self.atalho("Ctrl+U", false);
                }
                "esc-site" => {
                    escudos::alternar_site(&escudos::site(&self.url_atual()));
                    self.atalho("Ctrl+R", false);
                }
                "esc-todos" | "ext-escudos" => {
                    self.aj.bloqueador = !self.aj.bloqueador;
                    self.aj.salvar();
                    self.atalho("Ctrl+R", false);
                }
                "esc-cookies" | "ext-cookies" => {
                    self.aj.cookies_terceiros = !self.aj.cookies_terceiros;
                    self.aj.salvar();
                }
                "ext-escuro" => {
                    self.aj.escuro_nas_paginas = !self.aj.escuro_nas_paginas;
                    self.aj.salvar();
                    self.reabrir = true;
                }
                "esc-prefs" | "ext-gerenciar" => {
                    self.prefs = true;
                    self.secao = Secao::Extensoes;
                }
                "google" => {
                    self.abrir(GOOGLE_LOGIN);
                    self.mudou = true;
                }
                "pdf" => {
                    if let Some(h) = self.host() {
                        motor::salvar_pdf(&h, &self.titulo_atual());
                    }
                }
                "limpar-historico" => {
                    self.prefs = true;
                    self.secao = Secao::Privacidade;
                    self.pergunta = 1;
                }
                _ => {}
            }
        }
    }

    /// Um site não abriu: a página de erro da Jangada no lugar da do motor.
    fn pagina_de_erro(&mut self, fonts: &Fonts) {
        let escuro = self.escuro;
        for a in &self.abas {
            let pedido = a.p.borrow_mut().erro_pedido.take();
            let Some((url, codigo)) = pedido else { continue };
            if url.starts_with("data:") || url.is_empty() {
                continue;
            }
            let dado = inicio::pagina_erro(fonts, escuro, &url, &codigo);
            a.p.borrow_mut().erro_url = Some(url);
            let b = a.p.borrow().browser.clone();
            if let Some(f) = b.and_then(|b| b.main_frame()) {
                f.load_url(Some(&dado.as_str().into()));
            }
        }
    }

    /// Importar de outro navegador (Chrome, Chromium, Brave, Edge, Vivaldi): o perfil, o
    /// que trazer e o resultado.
    fn janela_importar(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.c.w, ui.c.h);
        ui.c.fill_rect(0.0, 0.0, w, h, &solid(ui.theme.body_a));
        // O trabalho terminou: as sessões (cookies) vão para o motor aqui.
        if let Some(rx) = &self.imp_job {
            match rx.try_recv() {
                Ok(r) => {
                    let sessoes = motor::gravar_cookies(&r.cookies);
                    self.favoritos = ajustes::favoritos();
                    self.refazer_inicio(ui.fonts);
                    self.senhas_lista = None;
                    let mut m = tr!("Pronto: {} favoritos, {} sites do histórico, {} cookies de sessão e {} senhas.", r.favoritos, r.historico, sessoes, r.senhas);
                    for p in r.problemas {
                        m.push(' ');
                        m.push_str(&p);
                    }
                    self.imp_msg = Some(m);
                    self.imp_job = None;
                    self.mudou = true;
                    ui.redraw_all_windows();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.wake_after(Duration::from_millis(200)),
                Err(_) => self.imp_job = None,
            }
        }
        ui.area(Rect::new(20.0, 16.0, w - 40.0, h - 32.0), |ui| {
            if self.imp_fontes.is_empty() {
                ui.paragraph(tr!("Nenhum outro navegador da família do Chrome (Google Chrome, Chromium, Brave, Microsoft Edge, Vivaldi) foi encontrado nesta conta."));
                return;
            }
            ui.label_ex(tr!("Importar de"), 12.0, true, tinta());
            let nomes: Vec<String> = self.imp_fontes.iter().map(|f| f.nome.clone()).collect();
            ui.popup(&mut self.imp_escolha, &nomes, 300.0);
            ui.space(10.0);
            let ocupado = self.imp_job.is_some();
            ui.set_active(!ocupado);
            ui.checkbox(&mut self.imp_partes.favoritos, tr!("Favoritos"));
            ui.checkbox(&mut self.imp_partes.historico, tr!("Histórico"));
            ui.checkbox(&mut self.imp_partes.sessoes, tr!("Sessões abertas nos sites (cookies)"));
            ui.checkbox(&mut self.imp_partes.senhas, tr!("Senhas (vão para o chaveiro do sistema)"));
            ui.set_active(true);
            ui.space(8.0);
            ui.paragraph(tr!("Vem o que o navegador guarda neste computador. Feche-o antes, para trazer tudo. As extensões dele não funcionam na Jangada."));
            ui.space(6.0);
            ui.horizontal(|ui| {
                ui.set_active(!ocupado);
                if ui.default_button(tr!("Importar")) {
                    if let Some(f) = self.imp_fontes.get(self.imp_escolha).cloned() {
                        let partes = self.imp_partes;
                        let (tx, rx) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let _ = tx.send(importar::importar(&f, partes));
                        });
                        self.imp_job = Some(rx);
                        self.imp_msg = Some(tr!("Importando…").into());
                    }
                }
                ui.set_active(true);
            });
            if let Some(m) = &self.imp_msg {
                ui.space(4.0);
                ui.paragraph(m);
            }
        });
    }

    /// A janela Histórico (Ctrl+H): todos os sites visitados, com busca. Dois cliques
    /// abrem numa aba nova.
    fn janela_historico(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.c.w, ui.c.h);
        ui.c.fill_rect(0.0, 0.0, w, h, &solid(ui.theme.body_a));
        let antes = self.hist_busca.clone();
        ui.search_field_in(Rect::new(w - 260.0, 10.0, 250.0, 22.0), &mut self.hist_busca, tr!("Buscar no Histórico"));
        if self.hist_busca != antes {
            self.hist_sel = None;
            self.hist_tabela.scroll = 0.0;
        }
        let busca = self.hist_busca.to_lowercase();
        let lista: Vec<&(String, String)> = self.hist_lista.iter().filter(|(t, u)| busca.is_empty() || t.to_lowercase().contains(&busca) || u.to_lowercase().contains(&busca)).collect();
        let info = tr!("{} sites", lista.len());
        ui.fonts.draw(ui.c, &info, 14.0, 26.0, 12.0, true, tinta());
        let linhas: Vec<TableRow> = lista.iter().map(|(t, u)| TableRow { cells: vec![if t.is_empty() { site(u) } else { t.clone() }, mostrar(u)], ..Default::default() }).collect();
        let cols = [Column::new(tr!("Título"), 0.55), Column::new(tr!("Endereço"), 0.45)];
        let cols: Vec<Column> = cols.into_iter().map(|c| c.fixed()).collect();
        let area = Rect::new(10.0, 42.0, w - 20.0, h - 42.0 - 46.0);
        let r = ui.table(area, &cols, &linhas, self.hist_sel, &mut self.hist_tabela, &TableStyle { zebra: true, ..Default::default() });
        if let Some(i) = r.pressed {
            self.hist_sel = Some(i);
        }
        let mut abrir = r.double;
        let sel = self.hist_sel.and_then(|i| lista.get(i)).map(|(_, u)| u.clone());
        let mut apagar = false;
        let mut limpar = false;
        ui.area(Rect::new(10.0, h - 36.0, w - 20.0, 28.0), |ui| {
            ui.horizontal(|ui| {
                ui.set_active(sel.is_some());
                if ui.button(tr!("Abrir em Nova Aba")) {
                    abrir = self.hist_sel;
                }
                if ui.button(tr!("Apagar do Histórico")) {
                    apagar = true;
                }
                ui.set_active(true);
                if ui.button(tr!("Limpar o Histórico…")) {
                    limpar = true;
                }
            });
        });
        if let Some(u) = abrir.and_then(|i| lista.get(i)).map(|(_, u)| u.clone()) {
            self.abrir(&u);
            self.mudou = true;
            ui.redraw_all_windows();
        }
        if apagar {
            if let Some(u) = sel {
                ajustes::apagar_do_historico(&u);
                self.hist_lista.retain(|(_, x)| *x != u);
                self.recentes.retain(|(_, x)| *x != u);
                self.hist_sel = None;
            }
        }
        if limpar {
            self.prefs = true;
            self.secao = Secao::Privacidade;
            self.pergunta = 1;
            ui.redraw_all_windows();
        }
    }

    /// A janela Downloads (Ctrl+J): o que foi baixado desta vez, com o andamento.
    fn janela_downloads(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.c.w, ui.c.h);
        ui.c.fill_rect(0.0, 0.0, w, h, &solid(ui.theme.body_a));
        let linhas: Vec<TableRow> = motor::DOWNLOADS.with(|l| {
            l.borrow().iter().rev().map(|d| {
                let estado = if d.pronto {
                    tr!("Pronto").to_string()
                } else if d.cancelado {
                    tr!("Cancelado").to_string()
                } else if d.total > 0 {
                    format!("{}% · {}/s", d.recebido * 100 / d.total.max(1), tamanho(d.velocidade))
                } else {
                    tr!("Baixando… {}", tamanho(d.recebido))
                };
                let t = if d.total > 0 { tamanho(d.total) } else { tamanho(d.recebido) };
                TableRow { cells: vec![d.nome.clone(), t, estado], ..Default::default() }
            }).collect()
        });
        let cols = [Column::new(tr!("Nome"), 0.55).fixed(), Column::new(tr!("Tamanho"), 0.17).right().fixed(), Column::new(tr!("Estado"), 0.28).fixed()];
        let area = Rect::new(10.0, 10.0, w - 20.0, h - 10.0 - 46.0);
        let r = ui.table(area, &cols, &linhas, self.down_sel, &mut self.down_tabela, &TableStyle { zebra: true, ..Default::default() });
        if let Some(i) = r.pressed {
            self.down_sel = Some(i);
        }
        let n = linhas.len();
        // A escolha conta de trás para a frente (o mais novo em cima).
        let idx = self.down_sel.filter(|i| *i < n).map(|i| n - 1 - i);
        let (pronto, baixando, caminho) = motor::DOWNLOADS.with(|l| {
            idx.and_then(|i| l.borrow().get(i).map(|d| (d.pronto, !d.pronto && !d.cancelado, d.caminho.clone()))).unwrap_or_default()
        });
        let mut abrir = r.double.is_some() && pronto;
        let (mut pasta, mut cancelar, mut limpar) = (false, false, false);
        ui.area(Rect::new(10.0, h - 36.0, w - 20.0, 28.0), |ui| {
            ui.horizontal(|ui| {
                ui.set_active(pronto);
                abrir |= ui.button(tr!("Abrir"));
                ui.set_active(idx.is_some());
                pasta = ui.button(tr!("Mostrar na Pasta"));
                ui.set_active(baixando);
                cancelar = ui.button(tr!("Cancelar"));
                ui.set_active(n > 0);
                limpar = ui.button(tr!("Limpar a Lista"));
                ui.set_active(true);
            });
        });
        if abrir {
            let _ = std::process::Command::new("xdg-open").arg(&caminho).spawn();
        }
        if pasta {
            let alvo = caminho.parent().map(|p| p.to_path_buf()).unwrap_or_else(pasta_downloads);
            let _ = std::process::Command::new("glove-files").arg(&alvo).spawn().or_else(|_| std::process::Command::new("xdg-open").arg(&alvo).spawn());
        }
        if cancelar {
            motor::DOWNLOADS.with(|l| {
                if let Some(cb) = idx.and_then(|i| l.borrow().get(i).and_then(|d| d.cb.clone())) {
                    use cef::ImplDownloadItemCallback;
                    cb.cancel();
                }
            });
        }
        if limpar {
            motor::DOWNLOADS.with(|l| l.borrow_mut().retain(|d| !d.pronto && !d.cancelado));
            self.down_sel = None;
            self.mudou = true;
        }
    }

    /// A janela das Preferências: seções como as do Safari.
    fn preferencias(&mut self, ui: &mut Ui) {
        let w = ui.c.w;
        ui.c.fill_rect(0.0, 0.0, w, ui.c.h, &solid(ui.theme.body_a));
        ui.space(10.0);
        let secoes = [(Secao::Geral, tr!("Geral")), (Secao::Contas, tr!("Contas")), (Secao::Senhas, tr!("Senhas")), (Secao::Busca, tr!("Busca")), (Secao::Privacidade, tr!("Privacidade")), (Secao::Extensoes, tr!("Extensões")), (Secao::Avancado, tr!("Avançado"))];
        let nomes: Vec<&str> = secoes.iter().map(|s| s.1).collect();
        let mut i = secoes.iter().position(|s| s.0 == self.secao).unwrap_or(0);
        if ui.tabs(&mut i, &nomes) {
            self.secao = secoes[i].0;
            self.pergunta = 0;
        }
        ui.space(14.0);
        let antes = self.aj.clone();
        ui.area(Rect::new(28.0, ui.cursor().1, w - 56.0, ui.c.h - ui.cursor().1 - 12.0), |ui| match self.secao {
            Secao::Geral => {
                ui.label_ex(tr!("Página inicial e das abas novas"), 12.0, true, tinta());
                let opcoes = [tr!("A página da Jangada").to_string(), tr!("Um endereço").to_string()];
                let mut k = usize::from(!self.aj.inicio.trim().is_empty() || !self.inicio_editado.is_empty());
                if ui.popup(&mut k, &opcoes, 240.0) {
                    if k == 0 {
                        self.aj.inicio.clear();
                        self.inicio_editado.clear();
                    } else {
                        self.inicio_editado = if self.aj.inicio.is_empty() { "https://".into() } else { self.aj.inicio.clone() };
                    }
                }
                if k == 1 {
                    ui.space(4.0);
                    if self.inicio_editado.is_empty() {
                        self.inicio_editado = self.aj.inicio.clone();
                    }
                    if ui.text_field(&mut self.inicio_editado, 360.0) {
                        self.aj.inicio = self.inicio_editado.trim().to_string();
                    }
                    let atual = self.url_atual();
                    if !atual.is_empty() && !self.eh_inicio(&atual) && ui.small_button(tr!("Usar a Página Atual")) {
                        self.aj.inicio = atual.clone();
                        self.inicio_editado = atual;
                    }
                }
                ui.space(12.0);
                ui.checkbox(&mut self.aj.reabrir_abas, tr!("Ao abrir a Jangada, reabrir as abas da última vez"));
                ui.checkbox(&mut self.aj.barra_favoritos, tr!("Mostrar a barra de favoritos"));
                ui.space(12.0);
                ui.label_ex(tr!("Tamanho das páginas"), 12.0, true, tinta());
                let nomes: Vec<String> = ZOOMS.iter().map(|z| format!("{z}%")).collect();
                let mut z = ZOOMS.iter().position(|p| (nivel(*p) - self.aj.zoom).abs() < 0.05).unwrap_or(3);
                if ui.popup(&mut z, &nomes, 120.0) {
                    self.aj.zoom = nivel(ZOOMS[z]);
                    for a in &mut self.abas {
                        a.zoom_ok = false;
                    }
                }
                ui.space(12.0);
                ui.label_ex(tr!("Página inicial: fundo"), 12.0, true, tinta());
                // Cor pronta, ou a imagem pelo caminho dela (a página embute a imagem).
                let cores = [("", tr!("Padrão")), ("cor:#eceef3", tr!("Cinza claro")), ("cor:#dbe7f7", tr!("Azul claro")), ("cor:#2b2b31", tr!("Grafite"))];
                let imagem = self.aj.inicio_fundo.starts_with("imagem:");
                let mut k = cores.iter().position(|c| c.0 == self.aj.inicio_fundo).unwrap_or(if imagem { 4 } else { 0 });
                let mut nomes_fundo: Vec<String> = cores.iter().map(|c| c.1.to_string()).collect();
                nomes_fundo.push(tr!("Imagem…").into());
                if ui.popup(&mut k, &nomes_fundo, 160.0) {
                    if k < cores.len() {
                        self.aj.inicio_fundo = cores[k].0.to_string();
                    } else if !imagem {
                        self.aj.inicio_fundo = format!("imagem:{}", self.fundo_imagem);
                    }
                }
                if k == 4 {
                    ui.space(4.0);
                    if ui.text_field(&mut self.fundo_imagem, 360.0) {
                        self.aj.inicio_fundo = format!("imagem:{}", self.fundo_imagem.trim());
                    }
                    ui.small(&tr!("Caminho de uma imagem no computador (JPG, PNG ou WebP, até 8 MB)."));
                }
                ui.space(6.0);
                ui.checkbox(&mut self.aj.inicio_atalhos, tr!("Mostrar os atalhos dos sites"));
                ui.checkbox(&mut self.aj.inicio_busca, tr!("Mostrar a caixa de busca"));
            }
            Secao::Contas => {
                // A sessão do Google: os cookies da conta, conferidos de tempos em tempos.
                if self.google_lido.elapsed() > Duration::from_secs(1) {
                    motor::conferir_google();
                    self.google_lido = Instant::now();
                }
                ui.wake_after(Duration::from_millis(500));
                let logado = motor::GOOGLE.with(|g| g.get());
                ui.label_ex(tr!("Conta do Google"), 12.0, true, tinta());
                ui.space(4.0);
                ui.paragraph(if logado {
                    tr!("Você está com a sessão aberta numa conta do Google: Gmail, YouTube, Drive e os outros serviços já abrem com ela.")
                } else {
                    tr!("Inicie a sessão para usar o Gmail, o YouTube, o Drive e os outros serviços do Google com a sua conta.")
                });
                ui.space(6.0);
                if logado {
                    if ui.button(tr!("Gerenciar a Conta…")) {
                        self.abrir("https://myaccount.google.com/");
                    }
                    if ui.button(tr!("Sair da Conta do Google")) {
                        self.abrir("https://accounts.google.com/Logout");
                    }
                } else if ui.default_button(tr!("Iniciar Sessão no Google…")) {
                    self.abrir(GOOGLE_LOGIN);
                }
                ui.space(12.0);
                ui.paragraph(tr!("A sessão fica guardada na Jangada (como nos outros navegadores). A sincronização de favoritos e senhas do Chrome não existe no motor da Jangada."));
            }
            Secao::Senhas => {
                ui.checkbox(&mut self.aj.senhas, tr!("Oferecer para guardar as senhas e preencher as guardadas"));
                ui.space(4.0);
                if self.senhas_lista.is_none() {
                    match chaveiro::Chaveiro::abrir().and_then(|c| c.logins()) {
                        Ok(v) => {
                            self.senhas_lista = Some(v);
                            self.senhas_erro = None;
                        }
                        Err(e) => {
                            self.senhas_lista = Some(Vec::new());
                            self.senhas_erro = Some(tr!("O chaveiro do sistema não respondeu: {}", e));
                        }
                    }
                    self.senha_vista = None;
                }
                let lista = self.senhas_lista.clone().unwrap_or_default();
                let linhas: Vec<TableRow> = lista.iter().map(|l| TableRow { cells: vec![escudos::site(&l.origem), l.usuario.clone(), if self.senhas_sel.and_then(|i| lista.get(i)) == Some(l) { self.senha_vista.clone().unwrap_or_else(|| "••••••••".into()) } else { "••••••••".into() }], ..Default::default() }).collect();
                let cols = [Column::new(tr!("Site"), 0.4).fixed(), Column::new(tr!("Usuário"), 0.36).fixed(), Column::new(tr!("Senha"), 0.24).fixed()];
                let (x, y) = ui.cursor();
                let area = Rect::new(x, y, ui.available_width(), 170.0);
                let r = ui.table(area, &cols, &linhas, self.senhas_sel, &mut self.senhas_tabela, &TableStyle { zebra: true, ..Default::default() });
                ui.alloc(area.w, area.h);
                if let Some(i) = r.pressed {
                    if self.senhas_sel != Some(i) {
                        self.senha_vista = None;
                    }
                    self.senhas_sel = Some(i);
                }
                let sel = self.senhas_sel.and_then(|i| lista.get(i)).cloned();
                ui.space(4.0);
                ui.horizontal(|ui| {
                    ui.set_active(sel.is_some());
                    let mostrar = if self.senha_vista.is_some() { tr!("Esconder") } else { tr!("Mostrar") };
                    if ui.button(mostrar) {
                        self.senha_vista = match self.senha_vista.take() {
                            Some(_) => None,
                            None => sel.as_ref().and_then(|l| chaveiro::Chaveiro::abrir().ok()?.segredo(&l.caminho).ok()).and_then(|s| String::from_utf8(s).ok()),
                        };
                    }
                    if ui.button(tr!("Copiar a Senha")) {
                        if let Some(s) = sel.as_ref().and_then(|l| chaveiro::Chaveiro::abrir().ok()?.segredo(&l.caminho).ok()).and_then(|s| String::from_utf8(s).ok()) {
                            copiar_texto(&s);
                            self.aviso = Some((tr!("Senha copiada.").into(), Instant::now()));
                        }
                    }
                    if ui.button(tr!("Apagar")) {
                        if let Some(l) = &sel {
                            if let Ok(c) = chaveiro::Chaveiro::abrir() {
                                let _ = c.apagar(&l.caminho);
                            }
                            self.senhas_lista = None;
                            self.senhas_sel = None;
                        }
                    }
                    ui.set_active(true);
                    if ui.button(tr!("Importar…")) {
                        self.imp_abrir = true;
                        // Quem abre janelas é a principal: ela tem de passar por aqui.
                        ui.redraw_all_windows();
                    }
                });
                if let Some(e) = &self.senhas_erro {
                    ui.small(e);
                } else {
                    ui.small(&tr!("{} senhas no chaveiro do sistema (também no Acesso às Chaves).", lista.len()));
                }
            }
            Secao::Busca => {
                ui.label_ex(tr!("Buscador da barra de endereço e da página inicial"), 12.0, true, tinta());
                let nomes: Vec<String> = ajustes::BUSCADORES.iter().map(|b| b.0.to_string()).collect();
                let mut b = self.aj.busca;
                if ui.popup(&mut b, &nomes, 240.0) {
                    self.aj.busca = b;
                }
                ui.space(8.0);
                ui.paragraph(tr!("O DuckDuckGo, o Ecosia, o Brave Search e o Startpage não guardam o que você busca."));
            }
            Secao::Privacidade => {
                ui.checkbox(&mut self.aj.bloquear_janelas, tr!("Bloquear janelas que as páginas abrem sozinhas"));
                ui.checkbox(&mut self.aj.nao_rastrear, tr!("Pedir aos sites para não rastrear (Não Rastrear e GPC)"));
                ui.space(14.0);
                ui.label_ex(tr!("Dados guardados"), 12.0, true, tinta());
                ui.space(4.0);
                ui.horizontal(|ui| {
                    if ui.button(tr!("Limpar o Histórico…")) {
                        self.pergunta = 1;
                    }
                    if ui.button(tr!("Apagar Cookies e Dados dos Sites…")) {
                        self.pergunta = 2;
                    }
                });
            }
            Secao::Extensoes => {
                ui.label_ex(tr!("Extensões da Jangada"), 12.0, true, tinta());
                ui.space(6.0);
                ui.checkbox(&mut self.aj.bloqueador, tr!("Escudos: anúncios e rastreadores"));
                ui.paragraph(tr!("Com o motor de bloqueio do Brave e as listas EasyList, EasyPrivacy, a portuguesa, a do Peter Lowe e os filtros do uBlock Origin (renovadas toda semana): barra o que carregaria e esconde os espaços de anúncio na página."));
                let fora = ajustes::escudos_fora().len();
                if fora > 0 {
                    ui.horizontal(|ui| {
                        ui.small(&tr!("Desligados em {} sites.", fora));
                        if ui.small_button(tr!("Ligar em Todos")) {
                            ajustes::gravar_escudos_fora(&Default::default());
                            escudos::recarregar_fora();
                        }
                    });
                }
                ui.space(6.0);
                ui.checkbox(&mut self.aj.cookies_terceiros, tr!("Barrar cookies de terceiros"));
                ui.paragraph(tr!("Os cookies que seguem você de site em site (os de propaganda) ficam de fora."));
                ui.space(6.0);
                ui.checkbox(&mut self.aj.escuro_nas_paginas, tr!("Modo escuro nas páginas"));
                ui.paragraph(tr!("Escurece as páginas que não têm um modo escuro próprio (vale ao reabrir a Jangada)."));
                ui.space(10.0);
                ui.paragraph(tr!("Extensões da loja do Chrome: o motor da Jangada desenha a página sem janela própria (é o que deixa a janela, as abas e os menus serem do Glove), e nesse modo o Chromium não carrega extensões. Por isso as principais vêm prontas aqui."));
            }
            Secao::Avancado => {
                ui.checkbox(&mut self.aj.endereco_completo, tr!("Mostrar o endereço completo na barra"));
                ui.checkbox(&mut self.aj.placa_de_video, tr!("Usar a placa de vídeo (vale ao reabrir a Jangada)"));
                ui.paragraph(tr!("Sem ela a Jangada gasta menos memória; com ela, vídeos e jogos ficam mais leves para o processador."));
                ui.space(14.0);
                ui.label_ex(tr!("Downloads"), 12.0, true, tinta());
                let pasta = pasta_downloads();
                let home = std::env::var("HOME").unwrap_or_default();
                let mostra = pasta.display().to_string().replacen(&home, "~", 1);
                let mostra = ui.fonts.ellipsize(&mostra, 11.0, false, ui.available_width() - 8.0);
                ui.small(&mostra);
                if ui.small_button(tr!("Abrir a Pasta Downloads")) {
                    let _ = std::process::Command::new("glove-files").arg(&pasta).spawn().or_else(|_| std::process::Command::new("xdg-open").arg(&pasta).spawn());
                }
            }
        });
        // Perguntas (em janela própria, pelo alerta do Glove).
        match self.pergunta {
            1 => match ui.alert(glove_ui::AlertKind::Caution, tr!("Limpar todo o histórico?"), tr!("Os sites visitados saem do menu Histórico. Os favoritos ficam."), &[tr!("Cancelar"), tr!("Limpar")]) {
                Some(1) => {
                    ajustes::limpar_historico();
                    self.recentes.clear();
                    self.pergunta = 0;
                }
                Some(_) => self.pergunta = 0,
                None => {}
            },
            2 => match ui.alert(glove_ui::AlertKind::Caution, tr!("Apagar os cookies e os dados dos sites?"), tr!("Você sai das contas em que entrou nos sites."), &[tr!("Cancelar"), tr!("Apagar")]) {
                Some(1) => {
                    if let Some(m) = cef::cookie_manager_get_global_manager(None) {
                        m.delete_cookies(None, None, None);
                    }
                    self.aviso = Some((tr!("Cookies apagados.").into(), Instant::now()));
                    self.pergunta = 0;
                }
                Some(_) => self.pergunta = 0,
                None => {}
            },
            _ => {}
        }
        if self.aj != antes {
            let reabrir = self.aj.placa_de_video != antes.placa_de_video || self.aj.escuro_nas_paginas != antes.escuro_nas_paginas;
            self.reabrir |= reabrir;
            if self.aj.busca != antes.busca || self.aj.inicio_fundo != antes.inicio_fundo || self.aj.inicio_atalhos != antes.inicio_atalhos || self.aj.inicio_busca != antes.inicio_busca {
                self.refazer_inicio(ui.fonts);
            }
            self.aj.salvar();
        }
    }
}

/// A cor do texto comum (escura; o tema escuro clareia sozinho).
fn tinta() -> Color {
    rgba(30, 30, 34, 255)
}

/// A cor dos glifos das barras: `forca` 1 = ativo, menos = mais apagado. No tema escuro
/// eles clareiam (os glifos não passam pela troca automática de cores do kit).
/// O escudo (Escudos), centrado em (cx, cy); cheio quando ligado.
fn escudo(c: &mut Canvas, cx: f32, cy: f32, cor: Color, cheio: bool) {
    let (w, h) = (12.0, 14.0);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(cx, y);
    pb.line_to(x + w, y + 2.5);
    pb.cubic_to(x + w, y + 8.5, cx + 3.0, y + 12.0, cx, y + h);
    pb.cubic_to(cx - 3.0, y + 12.0, x, y + 8.5, x, y + 2.5);
    pb.close();
    let p = pb.finish();
    if cheio {
        c.fill(p.clone(), &solid(cor));
        // O visto branco por dentro.
        let mut v = tiny_skia::PathBuilder::new();
        v.move_to(cx - 3.0, cy);
        v.line_to(cx - 0.6, cy + 2.4);
        v.line_to(cx + 3.2, cy - 2.2);
        c.stroke(v.finish(), &solid(Color::WHITE), 1.5);
    } else {
        c.stroke(p, &solid(cor), 1.3);
    }
}

fn cor_glifo(ui: &Ui, forca: f32) -> Color {
    let (forte, fraco) = if ui.theme.dark { (228.0, 70.0) } else { (64.0, 200.0) };
    let v = (fraco + (forte - fraco) * forca.clamp(0.0, 1.0)) as u8;
    rgba(v, v, v.saturating_add(4), 255)
}

fn fr_sugestao_w(w: f32) -> f32 {
    (w - 240.0).clamp(120.0, 640.0)
}

fn fr_sugestao_x(w: f32) -> f32 {
    ((w - fr_sugestao_w(w)) / 2.0).max(0.0)
}

/// As sugestões para o que foi digitado: a busca no que for digitado (se não é um endereço),
/// depois os favoritos e o histórico que casam (título ou endereço), sem repetir endereço.
fn sugestoes_para(digitado: &str, favoritos: &[(String, String)], historico: &[(String, String)]) -> Vec<(String, String)> {
    let busca = digitado.to_lowercase();
    let mut v: Vec<(String, String)> = Vec::new();
    let eh_endereco = !digitado.contains(' ') && digitado.contains('.');
    if !eh_endereco {
        v.push((format!("{} {}", tr!("Pesquisar"), digitado), digitado.to_string()));
    }
    // Primeiro o que começa uma palavra do título ou o endereço (como o Safari), depois o
    // resto que contém o texto.
    let comeca_palavra = |t: &str, u: &str| {
        let (t, u) = (t.to_lowercase(), u.to_lowercase());
        let u = u.split_once("://").map_or(u.as_str(), |(_, r)| r).trim_start_matches("www.").to_string();
        u.starts_with(&busca) || t.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&busca))
    };
    let todos: Vec<&(String, String)> = favoritos.iter().chain(historico.iter()).collect();
    let mut ordem: Vec<&(String, String)> = todos.iter().copied().filter(|(t, u)| comeca_palavra(t, u)).collect();
    ordem.extend(todos.iter().copied().filter(|(t, u)| !comeca_palavra(t, u) && (t.to_lowercase().contains(&busca) || u.to_lowercase().contains(&busca))));
    for (t, u) in ordem {
        if v.len() >= 8 {
            break;
        }
        if !v.iter().any(|(_, x)| x == u) {
            v.push((if t.is_empty() { site(u) } else { t.clone() }, u.clone()));
        }
    }
    v
}

/// Desenha sem a troca automática de cores do tema escuro (os glifos já vêm na cor
/// certa para o tema; a troca escureceria os claros).
fn sem_troca(c: &mut Canvas, f: impl FnOnce(&mut Canvas)) {
    let antes = std::mem::replace(&mut c.dark_remap, false);
    f(c);
    c.dark_remap = antes;
}

/// Tamanho em bytes para gente ler (1,2 MB).
fn tamanho(b: i64) -> String {
    let b = b.max(0) as f64;
    let (v, u) = if b >= 1e9 { (b / 1e9, "GB") } else if b >= 1e6 { (b / 1e6, "MB") } else if b >= 1e3 { (b / 1e3, "KB") } else { (b, "bytes") };
    let t = if u == "bytes" { format!("{v:.0} {u}") } else { format!("{v:.1} {u}") };
    if glove_config::idioma().starts_with("pt") { t.replace('.', ",") } else { t }
}

/// A seta dos downloads (para baixo, sobre uma linha), centrada em (cx, cy).
fn seta_download(c: &mut Canvas, cx: f32, cy: f32, cor: Color) {
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(cx, cy - 6.5);
    pb.line_to(cx, cy + 3.0);
    pb.move_to(cx - 4.0, cy - 1.0);
    pb.line_to(cx, cy + 3.0);
    pb.line_to(cx + 4.0, cy - 1.0);
    c.stroke(pb.finish(), &solid(cor), 1.6);
    c.fill_rect(cx - 5.5, cy + 5.5, 11.0, 1.6, &solid(cor));
}

/// Cadeado pequeno (site seguro), 9 × 10 pontos com o canto em (x, y).
fn cadeado(c: &mut Canvas, x: f32, y: f32, cor: Color) {
    c.stroke(rrect(x + 1.8, y, 5.4, 6.0, 2.7), &solid(cor), 1.3);
    c.fill(rrect(x, y + 4.2, 9.0, 6.0, 1.5), &solid(cor));
}

/// Estrela de cinco pontas (favorito): cheia ou só o contorno.
fn estrela(c: &mut Canvas, cx: f32, cy: f32, r: f32, cheia: bool, cor: Color) {
    let mut pb = tiny_skia::PathBuilder::new();
    for k in 0..10 {
        let a = std::f32::consts::PI * (k as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
        let rr = if k % 2 == 0 { r } else { r * 0.45 };
        let (x, y) = (cx + a.cos() * rr, cy + a.sin() * rr);
        if k == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
    }
    pb.close();
    if cheia {
        c.fill(pb.finish(), &solid(cor));
    } else {
        c.stroke(pb.finish(), &solid(cor), 1.2);
    }
}

/// O ícone do site num canvas de 16 pontos (o motor manda no tamanho que tiver).
fn icone_16(data: &[u8], w: i32, h: i32, escala: f32) -> Option<Canvas> {
    let src = tiny_skia::PixmapRef::from_bytes(data, w as u32, h as u32)?;
    let mut c = Canvas::new(16.0, 16.0, escala)?;
    let px = c.px_width() as f32;
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
    c.draw_pixmap_t(src, &paint, tiny_skia::Transform::from_scale(px / w as f32, px / h as f32));
    Some(c)
}

/// Texto para a área de transferência (pelo wl-copy, se houver).
fn copiar_texto(t: &str) {
    if let Ok(mut c) = std::process::Command::new("wl-copy").stdin(std::process::Stdio::piped()).spawn() {
        if let Some(mut i) = c.stdin.take() {
            let _ = i.write_all(t.as_bytes());
        }
        let _ = c.wait();
    }
}

impl App for Navegador {
    fn tick(&self) -> Option<Duration> {
        // O motor pede quando quer trabalhar; com algo acontecendo, quadros a ~60/s.
        let pedido = motor::BOMBA.with(|b| b.get()) as u64;
        let mexendo = self.ativo.elapsed() < Duration::from_secs(2) || self.abas.iter().any(|a| a.p.borrow().loading);
        let ms = if mexendo { 16 } else { pedido.clamp(16, 120) };
        Some(Duration::from_millis(ms))
    }

    fn on_tick(&mut self) {
        cef::do_message_loop_work();
        let novas = motor::NOVAS_ABAS.with(|n| std::mem::take(&mut *n.borrow_mut()));
        for (u, fundo) in novas {
            self.abrir_em(&u, fundo);
            self.mudou = true;
        }
        if let Some(a) = motor::AVISOS.with(|a| a.borrow_mut().pop()) {
            self.aviso = Some((a, Instant::now()));
            self.mudou = true;
        }
        // A página virou PDF: a folha Imprimir.
        if let Some(pdf) = motor::IMPRIMIR.with(|i| i.borrow_mut().take()) {
            let nome = self.titulo_atual();
            self.imprimir = Some(glove_ui::print::PrintSheet::new(if nome.is_empty() { "Jangada" } else { &nome }, glove_ui::print::PrintContent::Pdf(pdf)));
            self.mudou = true;
        }
        let n = self.abas.len();
        self.ler_servidor();
        self.mudou |= n != self.abas.len();
    }

    /// Só redesenha quando algo das abas mudou (imagem nova, título, endereço, cursor…):
    /// a batida do motor é rápida e refazer a janela à toa pesa.
    fn tick_redraw(&mut self) -> bool {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.atual.hash(&mut h);
        self.abas.len().hash(&mut h);
        let mut carregando = false;
        for a in &self.abas {
            let p = a.p.borrow();
            (p.versao, p.dirty, &p.title, &p.url, p.loading, p.back, p.forward, &p.status, p.favicon_novo, p.achados).hash(&mut h);
            (p.ctx.is_some(), p.dialog.is_some(), p.erro_pedido.is_some(), p.pendente.is_some(), p.popup.is_some(), p.cursor as u8).hash(&mut h);
            carregando |= p.loading;
        }
        // O aviso de baixo some sozinho depois de 5 s.
        self.aviso.as_ref().map(|(_, t)| t.elapsed() < Duration::from_secs(5)).hash(&mut h);
        let assinatura = h.finish();
        let downloads = motor::DOWNLOADS_MUDOU.with(|m| m.replace(false)) || senhas::PARA_GUARDAR.with(|p| !p.borrow().is_empty()) || self.imp_job.is_some();
        let mudou = std::mem::take(&mut self.mudou) || self.imp_abrir || self.hist_abrir || self.down_abrir || downloads || assinatura != self.assinatura || carregando;
        self.assinatura = assinatura;
        mudou
    }

    fn close_window(&mut self, id: u32) -> bool {
        if id != 0 {
            return true;
        }
        self.close_requested()
    }

    fn window_closed(&mut self, id: u32) {
        let _ = id;
    }

    fn close_requested(&mut self) -> bool {
        // As abas da última vez (para "reabrir as abas").
        self.gravar_sessao();
        for a in &self.abas {
            let b = a.p.borrow().browser.clone();
            if let Some(h) = b.and_then(|b| b.host()) {
                h.close_browser(1);
            }
        }
        true
    }

    fn paints_background(&self) -> bool {
        true
    }

    fn menus(&self) -> Vec<Menu> {
        let favs: Vec<MenuItem> = self.favoritos.iter().enumerate().take(30).map(|(i, (t, _))| MenuItem::new(&format!("fav:{i}"), &t.chars().take(60).collect::<String>(), "")).collect();
        let hist: Vec<MenuItem> = self.recentes.iter().enumerate().map(|(i, (t, u))| {
            let t = if t.is_empty() { site(u) } else { t.chars().take(60).collect() };
            MenuItem::new(&format!("hist:{i}"), &t, "")
        }).collect();
        let fav_label = if self.eh_favorito(&self.url_atual()) { tr!("Tirar dos Favoritos") } else { tr!("Adicionar aos Favoritos") };
        let mut m_favs = vec![MenuItem::new("favorito", fav_label, "Ctrl+D"), MenuItem::new("barra-favoritos", if self.aj.barra_favoritos { tr!("Esconder a Barra de Favoritos") } else { tr!("Mostrar a Barra de Favoritos") }, "Ctrl+Shift+B")];
        if !favs.is_empty() {
            m_favs.push(MenuItem::Separator);
            m_favs.extend(favs);
        }
        let mut m_hist = vec![
            MenuItem::new("voltar", tr!("Voltar"), "Alt+Left"),
            MenuItem::new("avancar", tr!("Avançar"), "Alt+Right"),
            MenuItem::new("inicio", tr!("Início"), "Alt+Home"),
            MenuItem::Separator,
            MenuItem::new("reabrir-aba", tr!("Reabrir a Última Aba Fechada"), "Ctrl+Shift+T").enabled(!self.fechadas.is_empty()),
            MenuItem::new("reabrir-sessao", tr!("Reabrir as Abas da Última Sessão"), "").enabled(!self.sessao_anterior.is_empty()),
            MenuItem::Separator,
            MenuItem::new("todo-historico", tr!("Mostrar Todo o Histórico"), "Ctrl+H"),
        ];
        if !hist.is_empty() {
            m_hist.push(MenuItem::Separator);
            m_hist.extend(hist);
        }
        m_hist.push(MenuItem::Separator);
        m_hist.push(MenuItem::new("limpar-historico", tr!("Limpar o Histórico…"), ""));
        vec![
            Menu::app(vec![
                MenuItem::new("sobre", tr!("Sobre a Jangada"), ""),
                MenuItem::Separator,
                MenuItem::new("preferencias", tr!("Preferências…"), "Ctrl+,"),
                MenuItem::new("google", tr!("Iniciar Sessão no Google…"), ""),
            ]),
            Menu::new(tr!("Arquivo"), vec![
                MenuItem::new("nova-aba", tr!("Nova Aba"), "Ctrl+T"),
                MenuItem::new("nova-privada", tr!("Nova Janela Privada"), "Ctrl+Shift+N"),
                MenuItem::new("abrir-local", tr!("Abrir Endereço…"), "Ctrl+L"),
                MenuItem::Separator,
                MenuItem::new("importar", tr!("Importar de Outro Navegador…"), ""),
                MenuItem::Separator,
                MenuItem::new("pdf", tr!("Salvar como PDF"), ""),
                MenuItem::new("imprimir", tr!("Imprimir…"), "Ctrl+P"),
                MenuItem::Separator,
                MenuItem::new("fechar-aba", tr!("Fechar Aba"), "Ctrl+W"),
            ]),
            Menu::new(tr!("Editar"), vec![
                MenuItem::new("desfazer", tr!("Desfazer"), "Ctrl+Z"),
                MenuItem::new("refazer", tr!("Refazer"), "Ctrl+Shift+Z"),
                MenuItem::Separator,
                MenuItem::new("recortar", tr!("Recortar"), "Ctrl+X"),
                MenuItem::new("copiar", tr!("Copiar"), "Ctrl+C"),
                MenuItem::new("colar", tr!("Colar"), "Ctrl+V"),
                MenuItem::new("tudo", tr!("Selecionar Tudo"), "Ctrl+A"),
                MenuItem::Separator,
                MenuItem::new("achar", tr!("Buscar na Página…"), "Ctrl+F"),
            ]),
            Menu::new(tr!("Visualizar"), vec![
                MenuItem::new("recarregar", tr!("Recarregar a Página"), "Ctrl+R"),
                MenuItem::Separator,
                MenuItem::new("maior", tr!("Aumentar"), "Ctrl+="),
                MenuItem::new("menor", tr!("Diminuir"), "Ctrl+-"),
                MenuItem::new("real", tr!("Tamanho Padrão"), "Ctrl+0"),
                MenuItem::Separator,
                MenuItem::new("downloads", tr!("Mostrar os Downloads"), "Ctrl+J"),
                MenuItem::Separator,
                MenuItem::new("codigo", tr!("Ver Código-Fonte"), "Ctrl+U"),
            ]),
            Menu::new(tr!("Histórico"), m_hist),
            Menu::new(tr!("Favoritos"), m_favs),
            Menu::new(tr!("Extensões"), vec![
                MenuItem::new("ext-escudos", tr!("Escudos: Anúncios e Rastreadores"), "").checked(self.aj.bloqueador),
                MenuItem::new("ext-cookies", tr!("Barrar Cookies de Terceiros"), "").checked(self.aj.cookies_terceiros),
                MenuItem::new("ext-escuro", tr!("Modo Escuro nas Páginas"), "").checked(self.aj.escuro_nas_paginas),
                MenuItem::Separator,
                MenuItem::new("ext-gerenciar", tr!("Gerenciar Extensões…"), ""),
            ]),
        ]
    }

    fn ui(&mut self, ui: &mut Ui) {
        match ui.window_id() {
            JANELA_PREFS => return self.preferencias(ui),
            JANELA_HIST => return self.janela_historico(ui),
            JANELA_DOWN => return self.janela_downloads(ui),
            JANELA_IMPORTAR => return self.janela_importar(ui),
            0 => {}
            // Outra janela (do kit): o navegador só se desenha na principal.
            _ => return,
        }
        self.escala = ui.c.scale;
        self.escuro = ui.theme.dark;
        self.teclas_da_barra = false;
        for a in ui.menu_actions().to_vec() {
            match a.as_str() {
                "preferencias" => self.prefs = true,
                id if id.starts_with("ctx-") => self.acao_contexto(id),
                id => self.acao_menu(id),
            }
        }
        // As Preferências numa janela própria.
        if std::mem::take(&mut self.prefs) {
            self.secao = if self.pergunta != 0 { Secao::Privacidade } else { self.secao };
            ui.open_window(JANELA_PREFS, WindowOptions { title: tr!("Preferências da Jangada").into(), app_id: "glove.Navegador".into(), size: (600, 420), fixed: true, min_size: None });
        }
        if std::mem::take(&mut self.hist_abrir) {
            self.hist_lista = ajustes::historico();
            self.hist_busca.clear();
            self.hist_sel = None;
            ui.open_window(JANELA_HIST, WindowOptions { title: tr!("Histórico").into(), app_id: "glove.Navegador".into(), size: (720, 480), fixed: false, min_size: Some((420, 260)) });
        }
        if std::mem::take(&mut self.imp_abrir) {
            self.imp_fontes = importar::fontes();
            self.imp_escolha = 0;
            self.imp_msg = None;
            ui.open_window(JANELA_IMPORTAR, WindowOptions { title: tr!("Importar de Outro Navegador").into(), app_id: "glove.Navegador".into(), size: (460, 330), fixed: true, min_size: None });
        }
        if std::mem::take(&mut self.down_abrir) {
            ui.open_window(JANELA_DOWN, WindowOptions { title: tr!("Downloads").into(), app_id: "glove.Navegador".into(), size: (560, 340), fixed: false, min_size: Some((380, 200)) });
        }
        // O botão vermelho da barra.
        if std::mem::take(&mut self.fechar_janela) && self.close_requested() {
            ui.close = true;
            return;
        }
        self.pagina_de_erro(ui.fonts);
        // O histórico do menu (os últimos 12), relido a cada poucos segundos.
        if self.hist_lido.elapsed() > Duration::from_secs(3) {
            let todos = ajustes::historico();
            self.recentes = todos.iter().take(12).cloned().collect();
            self.recentes_todos = todos;
            self.hist_lido = Instant::now();
            // As abas abertas ficam gravadas (se a Jangada cair, a sessão não se perde).
            self.gravar_sessao();
        }
        let (w, h) = (ui.c.w, ui.c.h);
        ui.c.fill_rect(0.0, 0.0, w, h, &solid(Color::WHITE));
        // Com a folha Imprimir aberta, as barras não respondem (a folha é da janela).
        ui.set_enabled(self.imprimir.is_none());
        self.barra(ui);
        self.atualizar_tela();
        let mut top = BARRA_H;
        top += self.faixa_abas(ui, top);
        if top == BARRA_H {
            ui.c.hline(0.0, BARRA_H - 1.0, w, rgba(170, 170, 176, 255));
        }
        top += self.barra_favoritos(ui, top);
        top += self.barra_achar(ui, top);
        let status = self.aviso.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(5)).map(|(m, _)| m.clone()).or_else(|| {
            self.aba().map(|a| a.p.borrow().status.clone()).filter(|s| !s.is_empty())
        });
        let area = Rect::new(0.0, top, w, (h - top).max(1.0));
        // O tamanho da página acompanha a janela.
        if (area.w, area.h) != (self.area.w, self.area.h) || self.aba().is_some_and(|a| a.p.borrow().scale != self.escala) {
            for a in &self.abas {
                let mut p = a.p.borrow_mut();
                p.w = area.w as i32;
                p.h = area.h as i32;
                p.scale = self.escala;
            }
            self.area = area;
            for a in &self.abas {
                let b = a.p.borrow().browser.clone();
                if let Some(hh) = b.and_then(|b| b.host()) {
                    hh.notify_screen_info_changed();
                    hh.was_resized();
                }
            }
        }
        self.area = area;
        if let Some(a) = self.aba() {
            {
                // A página: copiada só quando chega imagem nova (a versão muda).
                let p = a.p.borrow();
                if let Some(c) = &p.tela {
                    let aba = (Rc::as_ptr(&a.p) as u64).rotate_left(17);
                    for (i, v) in p.faixas.iter().enumerate() {
                        let y0 = i as i32 * motor::FAIXA;
                        ui.c.draw_opaque_rows(c, area.x, area.y, (y0, y0 + motor::FAIXA), v ^ aba);
                    }
                }
            }
            let pop = a.p.borrow().popup.clone();
            if let (Some(c), Some(r)) = (&a.popup, pop) {
                ui.c.draw_canvas(c, area.x + r.x as f32, area.y + r.y as f32);
            }
        }
        // O link sob o mouse (ou um aviso), num balãozinho no canto de baixo.
        if let Some(t) = status {
            let t = ui.fonts.ellipsize(&mostrar(&t), 11.0, false, w * 0.6);
            let tw = ui.fonts.measure(&t, 11.0, false) + 14.0;
            let y = h - STATUS_H;
            ui.c.fill(rrect4(0.0, y, tw, STATUS_H, [0.0, 5.0, 0.0, 0.0]), &solid(rgba(240, 240, 242, 245)));
            ui.c.stroke(rrect4(0.5, y + 0.5, tw - 1.0, STATUS_H, [0.0, 5.0, 0.0, 0.0]), &solid(rgba(170, 170, 176, 255)), 1.0);
            ui.fonts.draw(ui.c, &t, 7.0, y + 14.0, 11.0, false, rgba(40, 40, 44, 255));
        }
        // O título da janela segue a página.
        let titulo = self.titulo_atual();
        let nome = if ajustes::privada() { "Jangada Privada" } else { "Jangada" };
        ui.set_title(&if titulo.is_empty() { nome.to_string() } else { format!("{titulo} — {nome}") });
        // alert()/confirm() da página: alerta do Glove (janela própria).
        let dialog = self.aba().and_then(|a| a.p.borrow().dialog.clone());
        let guardar = senhas::PARA_GUARDAR.with(|p| p.borrow().first().cloned());
        if let (Some((origem, usuario, senha)), None) = (guardar, &dialog) {
            let site_ = escudos::site(&origem);
            let quem = if usuario.is_empty() { site_.clone() } else { tr!("{} em {}", usuario, site_) };
            let texto = tr!("A senha fica no chaveiro do sistema, e a Jangada a preenche da próxima vez.");
            if let Some(i) = ui.alert(glove_ui::AlertKind::Note, &tr!("Guardar a senha de {}?", quem), &texto, &[tr!("Nunca"), tr!("Agora Não"), tr!("Guardar")]) {
                senhas::PARA_GUARDAR.with(|p| p.borrow_mut().retain(|(o, u, _)| !(*o == origem && *u == usuario)));
                match i {
                    0 => ajustes::nunca_guardar_em(&origem),
                    2 => {
                        std::thread::spawn(move || {
                            if let Ok(c) = chaveiro::Chaveiro::abrir() {
                                let _ = c.guardar(&origem, &usuario, &senha);
                            }
                        });
                        self.senhas_lista = None;
                        self.aviso = Some((tr!("Senha guardada.").into(), Instant::now()));
                    }
                    _ => {}
                }
            }
        } else if let Some((confirm, msg, cb)) = dialog {
            let botoes: Vec<&str> = if confirm { vec![tr!("Cancelar"), tr!("OK")] } else { vec![tr!("OK")] };
            let url = site(&self.url_atual());
            if let Some(i) = ui.alert(glove_ui::AlertKind::Note, &tr!("A página {} diz:", url), &msg, &botoes) {
                cb.cont(if confirm { (i == 1) as i32 } else { 1 }, None);
                if let Some(a) = self.aba() {
                    a.p.borrow_mut().dialog = None;
                }
            }
        } else if self.sobre {
            let texto = tr!("O navegador do Glove OS. Motor: Chromium {} (CEF). Leve para computadores modestos: sem o processo da placa de vídeo e sem nada rodando de fundo.", cef::sys::CHROME_VERSION_MAJOR);
            if ui.alert(glove_ui::AlertKind::Note, &format!("Jangada {VERSAO}"), &texto, &[tr!("OK")]).is_some() {
                self.sobre = false;
            }
        } else if self.reabrir {
            if ui.alert(glove_ui::AlertKind::Note, tr!("Reabra a Jangada para a mudança valer."), tr!("O modo escuro nas páginas e a placa de vídeo só mudam quando a Jangada abre."), &[tr!("OK")]).is_some() {
                self.reabrir = false;
            }
        }
        // A folha Imprimir (a página em PDF), por cima de tudo: a página não recebe nada.
        if let Some(mut folha) = self.imprimir.take() {
            folha.top = top;
            if !ui.print_sheet(&mut folha) {
                self.imprimir = Some(folha);
            }
            return;
        }
        // As sugestões da barra, por cima da página (a página não recebe o que cai nelas).
        self.sugestao_area = None;
        if self.barra_foco && !self.sugestoes.is_empty() {
            let n = self.sugestoes.len() as f32;
            let r = Rect::new(fr_sugestao_x(w), BARRA_H - 4.0, fr_sugestao_w(w), n * 26.0 + 8.0);
            self.sugestao_area = Some(r);
            ui.c.fill(rrect(r.x, r.y, r.w, r.h, 8.0), &solid(if ui.theme.dark { rgba(44, 44, 48, 250) } else { rgba(252, 252, 254, 250) }));
            ui.c.stroke(rrect(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 8.0), &solid(rgba(160, 160, 166, 255)), 1.0);
            let mut escolhida = None;
            for (i, (titulo, url)) in self.sugestoes.clone().iter().enumerate() {
                let linha = Rect::new(r.x + 4.0, r.y + 4.0 + i as f32 * 26.0, r.w - 8.0, 24.0);
                let h = ui.hit(linha);
                let ativa = h.hover || self.sugestao_sel == Some(i);
                if ativa {
                    ui.c.fill(rrect(linha.x, linha.y, linha.w, linha.h, 5.0), &solid(ui.theme.accent));
                }
                let cor = if ativa { Color::WHITE } else { tinta() };
                let t = ui.fonts.ellipsize(&format!("{}   {}", titulo, mostrar(url)), 12.0, false, linha.w - 16.0);
                ui.fonts.draw(ui.c, &t, linha.x + 8.0, linha.y + 16.0, 12.0, false, cor);
                if h.clicked {
                    escolhida = Some(url.clone());
                }
            }
            if let Some(u) = escolhida {
                self.carregar(&u);
                self.barra_foco = false;
                self.sugestoes.clear();
                self.sugestao_sel = None;
                self.sugestao_area = None;
            }
        }
        // O mouse vai sempre para a página (passar sobre ela, a mãozinha nos links); o
        // teclado só quando nenhum campo da Jangada está em foco (ver `entrada`).
        self.entrada(ui, area);
        self.menu_contexto(ui, area);
    }
}

/// `jangada --icones PASTA`: o ícone da Jangada em PNG nos tamanhos do tema hicolor (para
/// o pacote: menus e lançadores de fora do Glove usam esses arquivos).
fn gerar_icones(dir: &std::path::Path) -> std::process::ExitCode {
    let Some(fonts) = Fonts::load_system() else { return 1.into() };
    for px in [16u32, 22, 24, 32, 48, 64, 128, 256] {
        let Some(mut c) = Canvas::new(px as f32, px as f32, 1.0) else { continue };
        glove_ui::paint::icons::draw_bare(&mut c, &fonts, glove_ui::paint::icons::Icon::Jangada, 0.0, 0.0, px as f32, None);
        let d = dir.join(format!("{px}x{px}/apps"));
        let _ = std::fs::create_dir_all(&d);
        let ok = tiny_skia::IntSize::from_wh(c.px_width(), c.px_height())
            .and_then(|s| tiny_skia::Pixmap::from_vec(c.data().to_vec(), s))
            .and_then(|p| p.encode_png().ok())
            .is_some_and(|png| std::fs::write(d.join("jangada.png"), png).is_ok());
        if !ok {
            return 1.into();
        }
    }
    0.into()
}

fn main() -> std::process::ExitCode {
    if let (Some(a), Some(dir)) = (std::env::args().nth(1), std::env::args().nth(2)) {
        if a == "--icones" {
            return gerar_icones(std::path::Path::new(&dir));
        }
    }
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let args = cef::args::Args::new();
    let Some(cmd) = args.as_cmd_line() else { return 1.into() };
    let is_browser = cmd.has_switch(Some(&cef::CefString::from("type"))) != 1;
    let mut app = motor::app();
    let ret = cef::execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if !is_browser {
        // Processo da página, da rede etc.: o motor já fez o trabalho dele.
        return (ret.max(0) as u8).into();
    }
    let aj = Ajustes::carregar();
    aj.aplicar();
    let favoritos = ajustes::favoritos();
    let dados = pasta_dados();
    // A página inicial da Jangada (refeita a cada abertura: tema, idioma, buscador, favoritos).
    let escuro = glove_config::Config::load().theme.contains("escuro");
    let inicio_url = match Fonts::load_system() {
        Some(f) => format!("file://{}", inicio::pagina_inicial(&dados, &f, escuro, aj.busca_url(), &favoritos, &aj.opcoes_inicio()).display()),
        None => "https://duckduckgo.com/".to_string(),
    };
    // Endereços pedidos (o resto da linha de comando que não é opção).
    let urls: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).map(|a| endereco(&a, &inicio_url, aj.busca_url())).collect();
    // Já aberta: manda os endereços para ela e sai.
    // A janela privada é outra janela: não manda os endereços para a normal e não a substitui.
    let privada = std::env::args().any(|a| a == "--privada");
    ajustes::PRIVADA.store(privada, std::sync::atomic::Ordering::Relaxed);
    let sock = socket_path();
    if !privada {
        if let Ok(mut c) = UnixStream::connect(&sock) {
            let text = if urls.is_empty() { "\n".to_string() } else { urls.join("\n") };
            let _ = c.write_all(text.as_bytes());
            return 0.into();
        }
        let _ = std::fs::remove_file(&sock);
    }
    // A pasta temporária da janela privada (na memória, apagada ao fechar).
    let tmp_privada = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(format!("jangada-privada-{}", std::process::id()));
    if privada {
        // As pastas de janelas privadas que caíram (sem fechar direito) saem agora.
        if let Some(base) = tmp_privada.parent() {
            for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
                let nome = e.file_name().to_string_lossy().to_string();
                if let Some(pid) = nome.strip_prefix("jangada-privada-").and_then(|n| n.parse::<u32>().ok()) {
                    if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                        let _ = std::fs::remove_dir_all(e.path());
                    }
                }
            }
        }
        let _ = std::fs::create_dir_all(&tmp_privada);
    }
    let servidor = if privada {
        None
    } else {
        UnixListener::bind(&sock).ok().inspect(|l| {
            let _ = l.set_nonblocking(true);
        })
    };

    let perfil = dados.join("Perfil");
    let idioma = if glove_config::idioma().starts_with("en") { "en-US" } else { "pt-BR" };
    // Os sites veem a Jangada como um Chrome comum: com um nome a mais no fim, o Google
    // recusa iniciar a sessão ("este navegador pode não ser seguro") e alguns sites
    // tratam a página como de um navegador desconhecido.
    let produto = format!("Chrome/{}.0.0.0", cef::sys::CHROME_VERSION_MAJOR);
    let settings = cef::Settings {
        windowless_rendering_enabled: 1,
        external_message_pump: 1,
        no_sandbox: (std::env::var_os("JANGADA_SEM_SANDBOX").is_some()) as i32,
        // Privada: a pasta do motor fica no tmpfs (XDG_RUNTIME_DIR, na memória) e é apagada ao
        // fechar; o perfil não tem caminho nenhum (cookies e senhas só na memória).
        root_cache_path: if privada { tmp_privada.to_string_lossy().as_ref().into() } else { dados.to_string_lossy().as_ref().into() },
        cache_path: if privada { "".into() } else { perfil.to_string_lossy().as_ref().into() },
        log_file: if privada { tmp_privada.join("motor.log").to_string_lossy().as_ref().into() } else { "".into() },
        persist_session_cookies: 1,
        locale: idioma.into(),
        accept_language_list: if idioma == "pt-BR" { "pt-BR,pt,en-US,en".into() } else { "en-US,en".into() },
        user_agent_product: produto.as_str().into(),
        log_severity: cef::LogSeverity::ERROR,
        background_color: 0xffff_ffff,
        ..Default::default()
    };
    if cef::initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
        eprintln!("jangada: o motor do Chromium não iniciou");
        return 1.into();
    }
    // Os escudos sobem numa linha à parte (o motor guardado abre na hora).
    escudos::iniciar();
    // As abas da última vez: reabertas já (Preferências) ou guardadas para o menu Histórico.
    let fundo_imagem = aj.inicio_fundo.strip_prefix("imagem:").unwrap_or("").to_string();
    let sessao = ajustes::sessao();
    let aj_reabrir = aj.reabrir_abas && urls.is_empty();
    let mut nav = Navegador {
        abas: Vec::new(),
        atual: 0,
        barra: String::new(),
        barra_foco: false,
        focar_barra: false,
        area: Rect::new(0.0, BARRA_H, 1100.0, 700.0),
        escala: 1.0,
        dentro: false,
        apertado: false,
        aviso: None,
        servidor,
        ativo: Instant::now(),
        ctx_dados: None,
        inicio: inicio_url,
        dados,
        escuro,
        sobre: false,
        teclas_da_barra: false,
        aj,
        favoritos,
        sugestoes: Vec::new(),
        sugestao_sel: None,
        sugestao_area: None,
        recentes_todos: Vec::new(),
        recentes: Vec::new(),
        achar: None,
        focar_achar: false,
        prefs: false,
        secao: Secao::Geral,
        inicio_editado: String::new(),
        pergunta: 0,
        reabrir: false,
        hist_lido: Instant::now() - Duration::from_secs(10),
        fechar_janela: false,
        fechadas: Vec::new(),
        sessao_anterior: if aj_reabrir { Vec::new() } else { sessao.clone() },
        sessao_gravada: Vec::new(),
        assinatura: 0,
        imprimir: None,
        hist_abrir: false,
        hist_busca: String::new(),
        hist_lista: Vec::new(),
        hist_sel: None,
        hist_tabela: TableState::default(),
        down_abrir: false,
        down_sel: None,
        down_tabela: TableState::default(),
        mudou: true,
        google_lido: Instant::now() - Duration::from_secs(10),
        fundo_imagem,
        senhas_lista: None,
        senhas_sel: None,
        senhas_tabela: TableState::default(),
        senha_vista: None,
        senhas_erro: None,
        imp_abrir: false,
        imp_fontes: Vec::new(),
        imp_escolha: 0,
        imp_partes: importar::Partes { favoritos: true, historico: true, sessoes: true, senhas: true },
        imp_job: None,
        imp_msg: None,
    };
    if !urls.is_empty() {
        for u in &urls {
            nav.abrir(u);
        }
    } else if aj_reabrir && !sessao.is_empty() {
        for u in &sessao {
            nav.abrir(u);
        }
    } else {
        nav.nova_aba();
    }
    // A janela principal sem a barra de título do Glove: os botões ficam na barra da
    // Jangada (como no Safari), e sobra mais espaço para a página.
    glove_ui::use_own_frame_main();
    let opts = WindowOptions { title: "Jangada".into(), app_id: "glove.Navegador".into(), size: (1100, 760), fixed: false, min_size: Some((560, 380)) };
    let r = run(opts, nav);
    // Fecha o motor com calma (as abas já pediram para fechar).
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(300) {
        cef::do_message_loop_work();
        std::thread::sleep(Duration::from_millis(10));
    }
    cef::shutdown();
    if !privada {
        let _ = std::fs::remove_file(&sock);
    } else {
        let _ = std::fs::remove_dir_all(&tmp_privada);
    }
    if r.is_err() { 1.into() } else { 0.into() }
}
