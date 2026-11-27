//! Jangada: o navegador do Glove OS. O motor é o WebKitGTK do sistema (veja `motor`); a
//! janela, as abas, a barra de endereço, as Preferências e as páginas da própria Jangada
//! (início, erro) são do Glove, no visual clássico ou no moderno. Feita para pesar pouco:
//! a página é desenhada pela placa de vídeo e, parada, a Jangada não gasta processador.
//!
//!   jangada [ENDEREÇO…]   abre os endereços (numa janela que já esteja aberta, se houver)

mod ajustes;
mod arquivos;
mod chaveiro;
mod escudos;
mod extensoes;
mod importar;
mod inicio;
mod motor;
mod ponte;
mod senhas;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ajustes::Ajustes;
use glove_ui::gtk::prelude::*;
use webkit6::prelude::*;
use webkit6::WebView;
use glove_config::tr;
use glove_ui::{
    paint::{draw::*, icons::Glyph, tiny_skia, Color, Fonts},
    run, App, Column, Cursor, Menu, MenuItem, Rect, TableRow, TableState, TableStyle, Ui, WindowOptions, SEGMENT_H, SEGMENT_W,
};

use glove_ui::gtk::glib;
use motor::Shared;

pub const VERSAO: &str = env!("CARGO_PKG_VERSION");

const BARRA_H: f32 = 40.0;
/// Altura do campo de endereço.
const CAMPO_H: f32 = 24.0;
const ABAS_H: f32 = 32.0;
const FAVS_H: f32 = 26.0;
const ACHAR_H: f32 = 32.0;
const STATUS_H: f32 = 20.0;
/// O grafite da janela privada (o alto e o baixo da barra).
fn cor_privada() -> (Color, Color) {
    (rgba(62, 58, 80, 255), rgba(46, 43, 60, 255))
}
/// A janela das Preferências.
const JANELA_PREFS: u32 = 1;
/// Onde se inicia a sessão do Google (volta para a página da conta).
const GOOGLE_LOGIN: &str = "https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fmyaccount.google.com%2F";
const JANELA_HIST: u32 = 2;
const JANELA_DOWN: u32 = 3;
const JANELA_IMPORTAR: u32 = 4;
/// A janela de um prompt() de uma página (a pergunta com o campo do texto).
const JANELA_PERGUNTA: u32 = 5;

/// Zoom das páginas (porcentagem) e o nível do motor (cada nível = ×1,2).
const ZOOMS: [u32; 7] = [67, 80, 90, 100, 110, 125, 150];
fn nivel(pct: u32) -> f64 {
    (pct as f64 / 100.0).ln() / 1.2f64.ln()
}

/// O fator de zoom do motor para um nível (cada nível = ×1,2).
fn fator(nivel: f64) -> f64 {
    1.2f64.powf(nivel)
}

/// Opens the folder of a downloaded file with the file already selected: the Files of
/// Glove (`--show`); elsewhere, the file manager by the standard request (FileManager1
/// ShowItems); last, just the folder.
fn mostrar_na_pasta(arquivo: &std::path::Path) {
    use std::process::{Command, Stdio};
    let pasta = arquivo.parent().map(|p| p.to_path_buf()).unwrap_or_else(pasta_downloads);
    if !arquivo.exists() {
        let _ = Command::new("glove-files").arg(&pasta).spawn().or_else(|_| Command::new("xdg-open").arg(&pasta).spawn());
        return;
    }
    if Command::new("glove-files").arg("--show").arg(arquivo).spawn().is_ok() {
        return;
    }
    let uri = webkit6::gio::File::for_path(arquivo).uri().to_string();
    let arquivo = arquivo.to_path_buf();
    std::thread::spawn(move || {
        let ok = Command::new("gdbus")
            .args(["call", "--session", "--dest", "org.freedesktop.FileManager1", "--object-path", "/org/freedesktop/FileManager1", "--method", "org.freedesktop.FileManager1.ShowItems"])
            .arg(format!("['{}']", uri.replace('\'', "%27")))
            .arg("")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = Command::new("xdg-open").arg(arquivo.parent().unwrap_or(&arquivo)).spawn();
        }
    });
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

    /// Todo texto com tr! tem a tradução em locales/en.po.
    #[test]
    fn todos_os_textos_traduzidos() {
        let raiz = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let po = std::fs::read_to_string(raiz.join("locales/en.po")).unwrap();
        let ids: std::collections::HashSet<&str> = po.lines().filter_map(|l| l.strip_prefix("msgid \"")?.strip_suffix('"')).collect();
        let mut faltam = Vec::new();
        for e in std::fs::read_dir(raiz.join("src")).unwrap().flatten() {
            let texto = std::fs::read_to_string(e.path()).unwrap_or_default();
            for pedaco in texto.split("tr!(").skip(1) {
                let Some(resto) = pedaco.trim_start().strip_prefix('"') else { continue };
                // O literal até a aspa que não está escapada.
                let mut fim = None;
                let b = resto.as_bytes();
                let mut i = 0;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            fim = Some(i);
                            break;
                        }
                        _ => i += 1,
                    }
                }
                let lit = &resto[..fim.unwrap()];
                // (O próprio padrão deste teste não é um texto: atravessa linhas.)
                if !lit.contains('\n') && !ids.contains(lit) {
                    faltam.push(format!("{}: {lit}", e.file_name().to_string_lossy()));
                }
            }
        }
        assert!(faltam.is_empty(), "sem tradução em locales/en.po:\n{}", faltam.join("\n"));
    }

    #[test]
    fn faixa_de_abas_que_rola() {
        let (mut primeira, mut seguida) = (0, (0, 0));
        // Cabem todas: nada rola.
        assert_eq!(janela_de_abas(5, 4, 1100.0, 208.0, &mut primeira, &mut seguida), (0, 5));
        // 20 abas de 80 em 1100 pontos: cabem 12 (com as setas), e a faixa vai até a escolhida.
        assert_eq!(janela_de_abas(20, 0, 1100.0, 80.0, &mut primeira, &mut seguida), (0, 12));
        assert_eq!(janela_de_abas(20, 19, 1100.0, 80.0, &mut primeira, &mut seguida), (8, 12));
        // Rolou com a roda para o começo: a faixa fica onde a pessoa pôs.
        primeira = 0;
        assert_eq!(janela_de_abas(20, 19, 1100.0, 80.0, &mut primeira, &mut seguida), (0, 12));
        // Escolheu outra fora da vista: a faixa a acompanha.
        assert_eq!(janela_de_abas(20, 15, 1100.0, 80.0, &mut primeira, &mut seguida), (4, 12));
        // Fecharam abas: a primeira não passa do fim.
        primeira = 8;
        assert_eq!(janela_de_abas(14, 3, 1100.0, 80.0, &mut primeira, &mut seguida), (2, 12));
    }

    #[test]
    fn janela_cabe_na_tela() {
        assert_eq!(caber(None, None), (1100, 760));
        assert_eq!(caber(Some(1920), Some(1080)), (1100, 760));
        // Um notebook de 1366 × 768: a altura sobra para a barra de menu e o Dock.
        assert_eq!(caber(Some(1366), Some(768)), (1100, 658));
        // 1280 × 800 em 150%: 853 × 533 pontos.
        assert_eq!(caber(Some(853), Some(533)), (829, 423));
        assert_eq!(caber(Some(400), Some(300)), (560, 380));
    }

    #[test]
    fn sugestao_pela_altura() {
        assert_eq!(sugestao_na_altura(2.0, 3), None);
        assert_eq!(sugestao_na_altura(10.0, 3), Some(0));
        assert_eq!(sugestao_na_altura(4.0 + 26.0 + 5.0, 3), Some(1));
        assert_eq!(sugestao_na_altura(4.0 + 3.0 * 26.0 + 1.0, 3), None);
    }

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
    /// O ícone do site, em 16 pontos.
    icone: Option<Canvas>,
}

/// Um prompt() de uma página à espera do texto (numa janela própria).
struct Pergunta {
    pagina: std::rc::Weak<std::cell::RefCell<motor::Pagina>>,
    dialogo: webkit6::ScriptDialog,
    site: String,
    mensagem: String,
    texto: String,
    focar: bool,
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
    /// O texto da barra em edição (com a seleção e a rolagem) e se já foi mexido desde
    /// que ela pegou o foco (as sugestões só aparecem depois de digitar).
    barra_buf: glove_ui::TextBuffer,
    barra_mexida: bool,
    barra_foco: bool,
    focar_barra: bool,
    /// Área da página na janela (do quadro anterior).
    area: Rect,
    escala: f32,
    /// A página deve pegar o teclado no próximo quadro (depois de navegar, trocar de aba).
    focar_pagina: bool,
    /// O botão desceu sobre a lista de sugestões: até ele subir, a página não recebe o mouse.
    na_sugestao: bool,
    aviso: Option<(String, Instant)>,
    servidor: Option<UnixListener>,
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
    /// The typed text the suggestions were made for (they are made again when it changes).
    sugestoes_de: Option<String>,
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
    /// The history file as it was when last read (it is read again only when it changes).
    hist_marca: ajustes::HistoryMark,
    /// O botão vermelho da barra (a janela não tem a barra de título do Glove).
    fechar_janela: bool,
    /// As abas fechadas (para reabrir: Ctrl+Shift+T), da mais velha para a mais nova.
    fechadas: Vec<String>,
    /// As abas da última vez que a Jangada fechou (Histórico ▸ Reabrir as Abas da Última
    /// Sessão) e o que já foi gravado desta vez.
    sessao_anterior: Vec<(String, String)>,
    sessao_gravada: Vec<(String, String)>,
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
    /// Extensões: a origem digitada, a preparação em andamento (uma pasta, um zip ou um
    /// repositório), a extensão esperando a confirmação das permissões e a mensagem.
    ext_fonte: String,
    ext_rx: Option<std::sync::mpsc::Receiver<Result<extensoes::Extensao, String>>>,
    ext_confirmar: Option<extensoes::Extensao>,
    ext_msg: Option<String>,
    /// A faixa de abas quando não cabem todas: a primeira à vista, a (escolhida, quantas)
    /// que a faixa já acompanhou e o resto da roda do mouse.
    aba_primeira: usize,
    aba_seguida: (usize, usize),
    aba_rolagem: f32,
    /// O prompt() de uma página à espera da resposta.
    pergunta_js: Option<Pergunta>,
    /// O lugar das páginas na janela (embaixo do desenho da Jangada).
    host: Option<glove_ui::NativeHost>,
    /// A aba que está na janela agora (as outras ficam escondidas).
    a_vista: Option<WebView>,
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
    /// Extensões na barra: o lugar do botão de cada uma (a janelinha abre embaixo dele), o
    /// do botão das extensões (a peça), os ícones já desenhados e (versão da lista,
    /// escala) com que foram feitos.
    ext_botoes: Vec<(String, Rect)>,
    ext_menu_rect: Option<Rect>,
    /// The little window of the puzzle piece: the extensions installed, to open and to pin
    /// to the bar.
    ext_painel: bool,
    /// The extensions pinned to the bar (read once; changed by the pin).
    ext_fixadas: Option<Vec<String>>,
    ext_icones: std::collections::HashMap<String, Option<Canvas>>,
    ext_icones_de: (u64, u32),
    /// A janelinha da extensão: o botão em que ela se ancora e onde ela está agora.
    popup_ancora: Rect,
    popup_rect: Option<Rect>,
    /// A rolagem das Preferências (na aba Extensões) e a altura do conteúdo.
    prefs_rolagem: f32,
    prefs_altura: f32,
}

impl Navegador {
    fn aba(&self) -> Option<&Aba> {
        self.abas.get(self.atual)
    }

    fn view(&self) -> Option<WebView> {
        self.aba().map(|a| a.p.borrow().view.clone())
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

    /// Ferramentas do desenvolvedor: o inspetor do motor (o do Safari), numa janela ao
    /// lado, ligado à página da aba atual.
    fn ferramentas(&mut self) {
        motor::ferramentas_dev(true);
        // Com o teclado na janelinha de uma extensão, o inspetor é o dela.
        if let Some(v) = self.popup_com_foco() {
            motor::inspecionar(&v);
        } else if let Some(v) = self.view() {
            motor::inspecionar(&v);
        }
    }

    /// Abre uma aba; `fundo`: por trás (a escolhida continua a mesma).
    fn abrir_em(&mut self, url: &str, fundo: bool) {
        if let Some(p) = motor::nova_aba(url, None) {
            self.pôr_aba(p, fundo);
        }
    }

    /// Uma aba por trás que só carrega o endereço quando for escolhida (até lá, com o
    /// título que ela tinha).
    fn abrir_adiada(&mut self, url: &str, titulo: &str) {
        if let Some(p) = motor::nova_aba_adiada(url, titulo) {
            self.pôr_aba(p, true);
        }
    }

    /// A resposta a um prompt() (None: cancelado). A janela da pergunta fecha em seguida.
    fn responder_pergunta(&mut self, texto: Option<&str>) {
        let Some(q) = self.pergunta_js.take() else { return };
        // A aba ainda existe? (Fechada, o diálogo já foi junto com ela.)
        if let Some(p) = q.pagina.upgrade() {
            if let Some(t) = texto {
                q.dialogo.prompt_set_text(t);
            }
            q.dialogo.close();
            p.borrow_mut().dialog = None;
        }
        self.mudou = true;
        glove_ui::redraw_soon();
    }

    /// A janela de um prompt(): a mensagem da página, o campo do texto, Cancelar e OK.
    fn janela_pergunta(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.c.w, ui.c.h);
        ui.c.fill_rect(0.0, 0.0, w, h, &solid(ui.theme.body_a));
        let Some(q) = self.pergunta_js.as_mut() else {
            ui.close = true;
            return;
        };
        let titulo = tr!("A página {} pergunta:", q.site);
        // Mensagens enormes ficam no começo (a janela tem tamanho fixo).
        let msg: String = if q.mensagem.chars().count() > 220 { q.mensagem.chars().take(220).chain("…".chars()).collect() } else { q.mensagem.clone() };
        let foco = std::mem::take(&mut q.focar);
        let mut texto = std::mem::take(&mut q.texto);
        let mut resposta: Option<bool> = None;
        if ui.keys().iter().any(|k| k.key == glove_ui::ui::Key::Escape) {
            resposta = Some(false);
        }
        ui.area(Rect::new(20.0, 16.0, w - 40.0, h - 76.0), |ui| {
            ui.label_ex(&titulo, 12.0, true, tinta());
            ui.space(4.0);
            ui.paragraph(&msg);
        });
        let (_, enter) = ui.text_field_in(Rect::new(20.0, h - 84.0, w - 40.0, 22.0), &mut texto, foco);
        if enter {
            resposta = Some(true);
        }
        ui.area(Rect::new(w - 200.0, h - 46.0, 180.0, 30.0), |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr!("Cancelar")) {
                    resposta = Some(false);
                }
                if ui.default_button(tr!("OK")) {
                    resposta = Some(true);
                }
            });
        });
        match resposta {
            Some(ok) => {
                self.responder_pergunta(ok.then_some(texto.as_str()));
                ui.close = true;
            }
            None => {
                if let Some(q) = self.pergunta_js.as_mut() {
                    q.texto = texto;
                }
            }
        }
    }

    /// Uma aba nova (já com a página) entra na faixa.
    fn pôr_aba(&mut self, p: Shared, fundo: bool) {
        if self.aj.zoom != 0.0 {
            p.borrow().view.set_zoom_level(fator(self.aj.zoom));
        }
        self.abas.push(Aba { p, icone: None });
        if !fundo || self.abas.len() == 1 {
            self.mudar(self.abas.len() - 1);
        }
        self.mudou = true;
    }

    /// Aba nova na página inicial, com o cursor na barra de endereço.
    fn nova_aba(&mut self) {
        let i = self.pagina_inicial();
        self.abrir(&i);
        self.focar_barra = true;
    }

    fn mudar(&mut self, i: usize) {
        // Outra aba à frente: a janelinha da extensão fecha (como no Chrome).
        if i != self.atual || self.abas.len() <= 1 {
            self.fechar_popup();
        }
        self.atual = i.min(self.abas.len().saturating_sub(1));
        self.mudou = true;
        self.barra_foco = false;
        // A página da aba vai para a janela no próximo quadro (e fica com o teclado).
        self.focar_pagina = true;
    }

    fn fechar(&mut self, i: usize) {
        if i >= self.abas.len() {
            return;
        }
        let era_a_escolhida = i == self.atual;
        let a = self.abas.remove(i);
        let url = a.p.borrow().url.clone();
        if !url.is_empty() && !url.starts_with("data:") && !self.eh_inicio(&url) {
            self.fechadas.push(url);
            if self.fechadas.len() > 30 {
                self.fechadas.remove(0);
            }
        }
        let v = a.p.borrow().view.clone();
        if let Some(h) = &self.host {
            h.remove(&v);
        }
        if self.a_vista.as_ref() == Some(&v) {
            self.a_vista = None;
        }
        // Uma aba que nunca carregou não tem processo para encerrar.
        let divide = a.p.borrow().parentes.get() || a.p.borrow().adiada;
        // Um prompt() desta aba à espera vai junto, antes de a página fechar (o diálogo
        // não pode sobrar depois dela).
        if self.pergunta_js.as_ref().is_some_and(|q| q.pagina.upgrade().is_some_and(|p| std::rc::Rc::ptr_eq(&p, &a.p))) {
            self.pergunta_js = None;
            glove_ui::redraw_soon();
        }
        drop(a);
        motor::fechar_pagina(&v, divide);
        if self.abas.is_empty() {
            self.nova_aba();
        } else if era_a_escolhida {
            self.mudar(self.atual.min(self.abas.len() - 1));
        } else {
            // Fechou outra aba: a escolhida continua a mesma (se a fechada vinha antes
            // dela, a posição da escolhida anda uma para trás).
            if i < self.atual {
                self.atual -= 1;
            }
            self.mudou = true;
        }
    }

    fn carregar(&mut self, url: &str) {
        let url = endereco(url, &self.inicio, self.aj.busca_url());
        let v = self.aba().map(|a| {
            let mut p = a.p.borrow_mut();
            p.fonte = false;
            p.url = url.clone();
            p.view.clone()
        });
        // Fora do empréstimo: o motor avisa na hora que começou a carregar (ou que o
        // endereço não presta), e com a página ainda emprestada esses avisos se perdiam:
        // a barra não mostrava o carregamento nem a página de erro.
        if let Some(v) = v {
            motor::permitir_navegacao(&url);
            v.load_uri(&url);
        }
        self.focar_pagina = true;
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
        let b = self.view();
        let editar = |c: &str| {
            if let Some(v) = &b {
                v.execute_editing_command(c);
            }
        };
        match combo {
            "Ctrl+T" => self.nova_aba(),
            "Ctrl+U" => {
                // O código-fonte da página numa aba nova.
                let u = self.url_atual();
                if let (Some(v), false) = (&b, u.is_empty() || u.starts_with("data:")) {
                    if let Some(p) = motor::nova_aba("", None) {
                        motor::codigo_fonte(v, p.clone());
                        self.pôr_aba(p, false);
                    }
                }
            }
            "Ctrl+Shift+I" => self.ferramentas(),
            "Ctrl+Shift+N" => {
                // Outra janela (um processo), com o armazenamento só em memória.
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--private").spawn();
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
                if let Some(v) = &b {
                    motor::imprimir(v, &self.dados);
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
                    b.reload_bypass_cache();
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
            "Escape" if self.achar.is_some() => {
                self.achar = None;
                if let Some(v) = &b {
                    motor::buscar(v, "", false);
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
                if let Some(v) = &b {
                    v.set_zoom_level(fator(self.aj.zoom));
                }
            }
            "Ctrl+C" if pagina => editar("Copy"),
            "Ctrl+X" if pagina => editar("Cut"),
            "Ctrl+V" if pagina => {
                if let Some(v) = &b {
                    arquivos::colar_pelo_menu(v);
                }
            }
            "Ctrl+A" if pagina => editar("SelectAll"),
            "Ctrl+Z" if pagina => editar("Undo"),
            "Ctrl+Shift+Z" | "Ctrl+Y" if pagina => editar("Redo"),
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
        // A primeira carrega já; as outras, quando forem escolhidas.
        let mut primeira = true;
        for (t, u) in lista.iter().filter(|(_, u)| !abertas.contains(u)) {
            if std::mem::take(&mut primeira) {
                self.abrir(u);
            } else {
                self.abrir_adiada(u, t);
            }
        }
        if so_inicio && self.abas.len() > 1 {
            self.fechar(0);
            self.fechadas.pop();
        }
    }

    /// As abas abertas agora (para a próxima vez), gravadas quando mudam.
    fn gravar_sessao(&mut self) {
        // Com o título: a aba reaberta já aparece com o nome dela, antes de carregar.
        let abas: Vec<(String, String)> = self
            .abas
            .iter()
            .map(|a| {
                let p = a.p.borrow();
                (p.title.clone(), p.url.clone())
            })
            .filter(|(_, u)| !u.is_empty() && !u.starts_with("data:") && !u.starts_with("devtools:") && !u.starts_with("view-source:") && !self.eh_inicio(u))
            .collect();
        if abas != self.sessao_gravada {
            ajustes::gravar_sessao(&abas);
            self.sessao_gravada = abas;
        }
    }

    /// O zoom em passos (os do Chrome: 1,2 vezes a cada dois passos de 0,5).
    fn zoom(&self, d: f64) {
        if let Some(v) = self.view() {
            let nivel = v.zoom_level().max(0.01).ln() / 1.2f64.ln();
            v.set_zoom_level(fator((nivel + d).clamp(-4.0, 6.0)));
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

    /// Uma pergunta de permissão numa faixa no alto da página (sem janela à parte): o
    /// texto e Negar, Só desta vez, Permitir. Devolve o botão escolhido (0, 1 ou 2).
    fn faixa_de_pergunta(ui: &mut Ui, area: Rect, titulo: &str) -> Option<usize> {
        let r = Rect::new(area.x, area.y, area.w, 34.0);
        let fundo = if ui.theme.dark { rgba(58, 58, 62, 255) } else { rgba(250, 244, 206, 255) };
        ui.c.fill_rect(r.x, r.y, r.w, r.h, &solid(fundo));
        ui.c.hline(r.x, r.y + r.h - 1.0, r.w, rgba(170, 170, 176, 255));
        let t = ui.fonts.ellipsize(titulo, 12.0, true, (r.w - 350.0).max(60.0));
        ui.fonts.draw(ui.c, &t, r.x + 12.0, r.y + 21.0, 12.0, true, tinta());
        let mut escolha = None;
        ui.area(Rect::new(r.x + r.w - 330.0, r.y + 5.0, 322.0, 24.0), |ui| {
            ui.horizontal(|ui| {
                for (i, b) in [tr!("Negar"), tr!("Só desta vez"), tr!("Permitir")].into_iter().enumerate() {
                    if ui.small_button(b) {
                        escolha = Some(i);
                    }
                }
            });
        });
        escolha
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
        let privada = ajustes::privada();
        if privada {
            // A janela privada se vê de longe: a barra e as abas em grafite, em qualquer tema.
            ui.c.fill_rect(0.0, 0.0, w, BARRA_H, &vgrad(0.0, BARRA_H, &[(0.0, cor_privada().0), (1.0, cor_privada().1)]));
        } else if modern {
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
            let r = Rect::new(dir - 28.0, by, 28.0, 26.0);
            let h = ui.hit(r);
            if h.hover && web {
                ui.c.fill(rrect(r.x, r.y, r.w, r.h, 6.0), &solid(rgba(0, 0, 0, if h.down { 34 } else { 18 })));
            }
            let ativo = web && escudos::valem(&site_);
            let cor = if ativo { ui.theme.accent } else { cor_glifo(ui, 0.35) };
            sem_troca(ui.c, |c| escudo(c, r.x + r.w / 2.0, r.y + r.h / 2.0, cor, ativo));
            if h.clicked && web {
                let mut itens = vec![
                    MenuItem::new("esc-site", &tr!("Escudos Ligados em {}", site_), "").checked(escudos::valem(&site_)).enabled(self.aj.bloqueador),
                    MenuItem::new("esc-info", tr!("O motor barra os anúncios e rastreadores antes de carregarem"), "").enabled(false),
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
        // As extensões: a peça (o menu de todas) e o botão de cada uma, até onde couber
        // sem apertar o endereço.
        let dir = self.botoes_extensoes(ui, dir, by, x0 + 104.0 + 220.0);
        // O endereço no meio (no máximo 640 pontos, centrado como no Safari).
        let livre = (x0 + 104.0, dir - 6.0);
        let fw = (livre.1 - livre.0).clamp(120.0, 640.0);
        let fx = ((w - fw) / 2.0).max(livre.0).min(livre.1 - fw);
        let fr = Rect::new(fx, ((BARRA_H - CAMPO_H) / 2.0).round(), fw, CAMPO_H);
        // O selo "Privada", ao lado do endereço (quando cabe).
        if privada {
            let rotulo = tr!("Privada");
            let lw = ui.fonts.measure(rotulo, 11.0, true) + 18.0;
            // À direita do campo (é onde sobra espaço); senão, à esquerda.
            let direita = fr.x + fr.w + 8.0;
            let bx = if direita + lw <= dir - 4.0 { direita } else { fr.x - lw - 8.0 };
            if bx > x0 + 96.0 {
                let by = fr.y + (CAMPO_H - 18.0) / 2.0;
                ui.c.fill(rrect(bx, by, lw, 18.0, 9.0), &solid(rgba(124, 96, 220, 255)));
                ui.fonts.draw(ui.c, rotulo, bx + 9.0, by + 13.0, 11.0, true, Color::WHITE);
            }
        }
        let autofocus = std::mem::take(&mut self.focar_barra);
        // Clique numa sugestão: vale já no apertar. O campo perde o foco nesse mesmo quadro
        // (o botão desceu fora dele) e a lista some; esperar o soltar perdia o clique.
        if self.barra_foco && ui.mouse_pressed() {
            let (px, py) = ui.pointer();
            let escolhida = self
                .sugestao_area
                .filter(|r| r.contains((px, py)))
                .and_then(|r| sugestao_na_altura(py - r.y, self.sugestoes.len()))
                .and_then(|i| self.sugestoes.get(i))
                .map(|(_, u)| u.clone());
            if let Some(u) = escolhida {
                self.na_sugestao = true;
                self.carregar(&u);
                self.barra_foco = false;
                self.sugestoes.clear();
                self.sugestoes_de = None;
                self.sugestao_sel = None;
            }
        }
        if autofocus {
            // O endereço inteiro, todo escolhido: digitar troca tudo; clicar de novo põe o
            // cursor no lugar para mudar só um pedaço. Esc devolve.
            let inteiro = if self.eh_inicio(&url) || url.starts_with("data:") { String::new() } else { url.clone() };
            self.barra_buf = glove_ui::TextBuffer::new(inteiro);
            self.barra_mexida = false;
        }
        if self.barra_foco || autofocus {
            let (mexeu, enter) = ui.line_edit(fr, &mut self.barra_buf, autofocus);
            self.barra_mexida |= mexeu;
            self.barra = self.barra_buf.text().to_string();
            self.barra_foco = ui.has_focus();
            if self.barra.is_empty() {
                let dica = if ajustes::privada() { tr!("Janela privada: nada fica guardado").to_string() } else { tr!("Buscar ou digitar um endereço").to_string() };
                ui.fonts.draw(ui.c, &dica, fr.x + 8.0, fr.y + CAMPO_H / 2.0 + 4.5, 12.5, false, rgba(150, 150, 156, 255));
            }
            if self.barra_foco && ui.keys().iter().any(|k| k.key == glove_ui::ui::Key::Escape) {
                self.barra_foco = false;
                self.focar_pagina = true;
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
            } else if self.barra_foco && !digitado.is_empty() && self.barra_mexida {
                // Only when the text changed: going through the whole history on every
                // frame made typing here drag.
                if self.sugestoes_de.as_deref() != Some(digitado.as_str()) {
                    let new = sugestoes_para(&digitado, &self.favoritos, &self.recentes_todos);
                    if new != self.sugestoes {
                        self.sugestao_sel = None;
                    }
                    self.sugestoes = new;
                    self.sugestoes_de = Some(digitado.clone());
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
                self.sugestoes_de = None;
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
                self.sugestoes_de = None;
                self.sugestao_sel = None;
            }
        } else {
            // Fora de foco: uma pílula com o site no meio (e o cadeado), como no Safari.
            let hit = ui.hit(fr);
            let fill = if privada { rgba(255, 255, 255, 30) } else if ui.theme.dark { rgba(58, 58, 62, 255) } else if modern { rgba(0, 0, 0, 14) } else { Color::WHITE };
            ui.c.fill(rrect(fr.x, fr.y, fr.w, fr.h, 7.0), &solid(fill));
            if !modern && !privada {
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
            let cor_texto = match (privada, vazio) {
                (true, true) => rgba(176, 172, 196, 255),
                (true, false) => rgba(238, 236, 246, 255),
                (false, true) => rgba(150, 150, 156, 255),
                (false, false) => tinta(),
            };
            ui.fonts.draw(ui.c, &t, tx, fr.y + CAMPO_H / 2.0 + 4.5, 13.0, false, cor_texto);
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
                if let Some(b) = self.view() {
                    if loading { b.stop_loading() } else { b.reload() }
                }
            }
        }
        // Carregando: uma faixa azul fina embaixo do campo, do tamanho do que já chegou,
        // como no Safari.
        if loading {
            let prog = self.aba().map(|a| a.p.borrow().progresso).unwrap_or(0.0).clamp(0.05, 1.0) as f32;
            ui.c.fill_rect(fr.x + 4.0, fr.y + fr.h - 2.0, (fr.w - 8.0) * prog, 2.0, &solid(ui.theme.accent));
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
        let privada = ajustes::privada();
        let escuro = ui.theme.dark || privada;
        let modern = ui.theme.modern;
        // Faixa < aba de fundo < aba escolhida (do mais escuro para o mais claro).
        let (fundo, apagada, acesa, escolhida) = if privada {
            (rgba(34, 32, 44, 255), rgba(48, 46, 60, 255), rgba(58, 56, 72, 255), rgba(76, 72, 96, 255))
        } else if escuro {
            (rgba(30, 30, 33, 255), rgba(42, 42, 46, 255), rgba(52, 52, 57, 255), rgba(70, 70, 75, 255))
        } else if modern {
            (rgba(218, 218, 222, 255), rgba(228, 228, 232, 255), rgba(236, 236, 240, 255), Color::WHITE)
        } else {
            (rgba(200, 200, 205, 255), rgba(214, 214, 219, 255), rgba(224, 224, 228, 255), rgba(246, 246, 248, 255))
        };
        ui.c.fill_rect(0.0, y, w, ABAS_H, &solid(fundo));
        ui.c.hline(0.0, y, w, if escuro { rgba(0, 0, 0, 120) } else { rgba(0, 0, 0, 30) });
        ui.c.hline(0.0, y + ABAS_H - 1.0, w, if escuro { rgba(0, 0, 0, 160) } else { rgba(150, 150, 156, 255) });
        let total = self.abas.len();
        let n = total.max(1) as f32;
        let (x0, gap, mais, seta) = (6.0, 4.0, 28.0, 22.0);
        let tw = ((w - x0 * 2.0 - mais - gap * n) / n).clamp(110.0, 260.0);
        // Quando não cabem todas (cada uma tem no mínimo 110 pontos: menos que isso o título
        // não se lê), a faixa rola: uma seta
        // em cada ponta, a roda do mouse, e ela acompanha a aba escolhida. Antes, as que não
        // cabiam ficavam fora do alcance do mouse.
        let (primeira, cabem) = janela_de_abas(total, self.atual, w, tw, &mut self.aba_primeira, &mut self.aba_seguida);
        let rola = cabem < total;
        if rola {
            self.aba_rolagem += ui.scroll_in(Rect::new(0.0, y, w, ABAS_H));
            while self.aba_rolagem >= 15.0 {
                self.aba_rolagem -= 15.0;
                self.aba_primeira = (self.aba_primeira + 1).min(total - cabem);
            }
            while self.aba_rolagem <= -15.0 {
                self.aba_rolagem += 15.0;
                self.aba_primeira = self.aba_primeira.saturating_sub(1);
            }
            if self.aba_primeira != primeira {
                ui.redraw = true;
            }
        } else {
            self.aba_rolagem = 0.0;
        }
        let x_abas = if rola { x0 + seta + 2.0 } else { x0 };
        let mut acao: Option<(usize, bool)> = None;
        let mut fim = x_abas;
        for i in primeira..(primeira + cabem).min(total) {
            let r = Rect::new(x_abas + (i - primeira) as f32 * (tw + gap), y + 4.0, tw, ABAS_H - 8.0);
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
            let cor = if privada { if sel { rgba(242, 240, 250, 255) } else { rgba(176, 172, 196, 255) } } else if sel { tinta() } else { rgba(90, 90, 96, 255) };
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
        // As setas da faixa que rola (apagadas na ponta).
        if rola {
            if Self::botao(ui, Rect::new(x0, y + 4.0, seta, ABAS_H - 8.0), Glyph::Back, primeira > 0) {
                self.aba_primeira = primeira - 1;
                ui.redraw = true;
            }
            if Self::botao(ui, Rect::new(fim, y + 4.0, seta, ABAS_H - 8.0), Glyph::Forward, primeira + cabem < total) {
                self.aba_primeira = primeira + 1;
                ui.redraw = true;
            }
            fim += seta + 2.0;
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
        if let Some(v) = self.view() {
            let anda = |a: &Aba, d: i32| {
                let mut p = a.p.borrow_mut();
                let n = p.achados.1.max(1);
                p.achados.0 = ((p.achados.0 - 1 + d).rem_euclid(n)) + 1;
            };
            if mudou {
                if let Some(a) = self.aba() {
                    a.p.borrow_mut().achados = (0, 0);
                }
                motor::buscar(&v, &texto, false);
            } else if enter || anterior.is_some() {
                let frente = anterior != Some(0) && !ui.mods().shift;
                if !texto.is_empty() {
                    motor::buscar(&v, &texto, !frente);
                    if let Some(a) = self.aba() {
                        anda(a, if frente { 1 } else { -1 });
                    }
                }
            }
            if fechar || esc {
                motor::buscar(&v, "", false);
                self.focar_pagina = true;
            }
        }
        self.achar = if fechar || esc { None } else { Some(texto) };
        ACHAR_H
    }

    /// Os ícones novos dos sites (refeitos em 16 pontos).
    fn atualizar_tela(&mut self) {
        let escala = self.escala;
        for a in self.abas.iter_mut() {
            let mut p = a.p.borrow_mut();
            if std::mem::take(&mut p.favicon_novo) {
                a.icone = p.favicon.as_ref().and_then(|(d, w, h)| icone_16(d, *w, *h, escala));
            }
        }
    }

    /// O que o motor deixou para a janela: abas novas, avisos, a impressão pronta.
    fn processar(&mut self) {
        let novas = motor::NOVAS_ABAS.with(|n| std::mem::take(&mut *n.borrow_mut()));
        for (u, fundo, view) in novas {
            match view {
                Some(v) => {
                    let p = motor::aba_criada(v, &u);
                    self.pôr_aba(p, fundo);
                }
                None => self.abrir_em(&u, fundo),
            }
        }
        self.pedidos_das_extensoes();
        // window.close(): a página pediu para fechar a própria aba.
        while let Some(i) = self.abas.iter().position(|a| a.p.borrow().fechar) {
            self.fechar(i);
        }
        if let Some(a) = motor::AVISOS.with(|a| a.borrow_mut().pop()) {
            self.aviso = Some((a, Instant::now()));
        }
        // A página virou PDF: a folha Imprimir.
        if let Some(pdf) = motor::IMPRIMIR.with(|i| i.borrow_mut().take()) {
            let nome = self.titulo_atual();
            self.imprimir = Some(glove_ui::print::PrintSheet::new(if nome.is_empty() { "Jangada" } else { &nome }, glove_ui::print::PrintContent::Pdf(pdf)));
        }
    }

    /// A página da aba atual na janela (no lugar `area`); as outras abas ficam escondidas
    /// (o motor para de desenhá-las).
    fn pôr_pagina(&mut self, area: Rect) {
        let Some(host) = self.host.clone() else { return };
        let atual = self.view();
        for a in &self.abas {
            let v = a.p.borrow().view.clone();
            let escolhida = Some(&v) == atual.as_ref();
            // As outras abas saem da tela (o motor para de desenhá-las), mas ficam com o
            // lugar marcado: a página de uma aba aberta por trás carrega já no tamanho
            // certo. Escondidas de vez, carregavam com tamanho zero, e alguns sites
            // montavam o layout errado. (Antes de entrar na janela, para não piscar.)
            if v.is_child_visible() != escolhida {
                v.set_child_visible(escolhida);
            }
            // O tamanho das escondidas só muda quando forem escolhidas (redimensionar a
            // janela não refaz o layout de todas as abas).
            if v.parent().is_none() || escolhida {
                host.place(&v, area.x, area.y, area.w, area.h);
            }
        }
        if self.a_vista != atual {
            self.a_vista = atual.clone();
            self.focar_pagina = true;
        }
        // A ordem das abas e a escolhida, para as extensões (tabs.query).
        let ids: Vec<u32> = self.abas.iter().map(|a| a.p.borrow().id).collect();
        let ativa = self.aba().map(|a| a.p.borrow().id).unwrap_or(0);
        motor::definir_ordem(ids, ativa);
        // A aba adiada foi escolhida: carrega agora.
        let adiada = self.aba().and_then(|a| {
            let mut p = a.p.borrow_mut();
            std::mem::take(&mut p.adiada).then(|| {
                p.loading = true;
                (p.view.clone(), p.url.clone())
            })
        });
        if let Some((v, url)) = adiada {
            motor::carregar_adiada(&v, &url);
        }
    }

    /// As teclas que chegaram ao kit (o teclado nas barras da Jangada, ou um item da
    /// barra de menu): os atalhos da Jangada. Com o teclado na página, os atalhos chegam
    /// por `native_key` (antes de a página recebê-los).
    fn teclas(&mut self, ui: &mut Ui) {
        // Num campo da Jangada (endereço, busca), copiar e colar são do campo.
        let pagina = !ui.has_focus();
        for k in ui.keys().to_vec() {
            self.atalho(&k.combo, pagina);
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
            itens.push(MenuItem::new("ctx-img-copiar", tr!("Copiar Imagem"), ""));
            itens.push(MenuItem::new("ctx-img-copiar-link", tr!("Copiar Endereço da Imagem"), ""));
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
        if self.aj.ferramentas_dev {
            itens.push(MenuItem::Separator);
            itens.push(MenuItem::new("ctx-inspecionar", tr!("Inspecionar Elemento"), ""));
        }
        self.ctx_dados = Some(c.clone());
        ui.context_menu((area.x + c.x as f32, area.y + c.y as f32), &itens);
    }

    fn acao_contexto(&mut self, id: &str) {
        let Some(c) = self.ctx_dados.clone() else { return };
        let b = self.view();
        let editar = |cmd: &str| {
            if let Some(v) = &b {
                v.execute_editing_command(cmd);
            }
        };
        match id {
            "ctx-link-aba" => self.abrir(&c.link),
            "ctx-img-aba" => self.abrir(&c.image),
            "ctx-img-copiar" => {
                if let Some(v) = &b {
                    motor::copiar_imagem(v, &c.image);
                }
            }
            "ctx-img-copiar-link" => copiar_texto(&c.image),
            "ctx-link-copiar" => copiar_texto(&c.link),
            "ctx-link-favorito" => {
                if !self.eh_favorito(&c.link) {
                    self.favoritos.push((site(&c.link), c.link.clone()));
                    ajustes::gravar_favoritos(&self.favoritos);
                    self.aviso = Some((tr!("Adicionada aos Favoritos.").into(), Instant::now()));
                }
            }
            "ctx-recortar" => editar("Cut"),
            "ctx-copiar" => editar("Copy"),
            "ctx-colar" => {
                if let Some(v) = self.view() {
                    arquivos::colar_pelo_menu(&v);
                }
            }
            "ctx-inspecionar" => self.ferramentas(),
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
            "ctx-pdf" => self.acao_menu("pdf"),
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
        } else if let Some(ext) = id.strip_prefix("xacao:") {
            let ancora = self.ancora_de(ext);
            self.clicar_extensao(ext, ancora);
        } else if let Some(ext) = id.strip_prefix("xopcoes:") {
            if let Some(e) = extensoes::ativa(ext) {
                abrir_opcoes(&e);
            }
        } else {
            match id {
                "sobre" => self.sobre = true,
                "reabrir-sessao" => self.reabrir_sessao(),
                "nova-privada" => {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).arg("--private").spawn();
                    }
                }
                "importar" => self.imp_abrir = true,
                "codigo" => {
                    self.atalho("Ctrl+U", false);
                }
                "dev" => self.ferramentas(),
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
                    if let Some(v) = self.view() {
                        motor::salvar_pdf(&v, &self.titulo_atual());
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
            // No lugar do endereço que falhou (a barra continua com ele; Tentar de Novo
            // recarrega).
            let html = inicio::html_erro(fonts, escuro, &url, &codigo);
            let v = a.p.borrow().view.clone();
            a.p.borrow_mut().erro_alt = true;
            v.load_alternate_html(&html, &url, None);
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
                } else if d.falhou {
                    tr!("Falhou").to_string()
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
            mostrar_na_pasta(&caminho);
        }
        if cancelar {
            if let Some(id) = idx.and_then(|i| motor::DOWNLOADS.with(|l| l.borrow().get(i).map(|d| d.id))) {
                motor::cancelar_download(id);
            }
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
            self.prefs_rolagem = 0.0;
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
                    for a in &self.abas {
                        a.p.borrow().view.set_zoom_level(fator(self.aj.zoom));
                    }
                }
                ui.space(12.0);
                ui.label_ex(tr!("Página inicial: fundo"), 12.0, true, tinta());
                // Cor pronta, ou a imagem pelo caminho dela (a página embute a imagem).
                let cores = [("", tr!("Padrão")), ("color:#eceef3", tr!("Cinza claro")), ("color:#dbe7f7", tr!("Azul claro")), ("color:#2b2b31", tr!("Grafite"))];
                let imagem = self.aj.inicio_fundo.starts_with("image:");
                let mut k = cores.iter().position(|c| c.0 == self.aj.inicio_fundo).unwrap_or(if imagem { 4 } else { 0 });
                let mut nomes_fundo: Vec<String> = cores.iter().map(|c| c.1.to_string()).collect();
                nomes_fundo.push(tr!("Imagem…").into());
                if ui.popup(&mut k, &nomes_fundo, 160.0) {
                    if k < cores.len() {
                        self.aj.inicio_fundo = cores[k].0.to_string();
                    } else if !imagem {
                        self.aj.inicio_fundo = format!("image:{}", self.fundo_imagem);
                    }
                }
                if k == 4 {
                    ui.space(4.0);
                    if ui.text_field(&mut self.fundo_imagem, 360.0) {
                        self.aj.inicio_fundo = format!("image:{}", self.fundo_imagem.trim());
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
                ui.space(10.0);
                ui.label_ex(tr!("Permissões dos sites"), 12.0, true, tinta());
                ui.space(4.0);
                ui.paragraph(&tr!("Câmera, microfone, localização e notificações que você permitiu ou negou para cada site. A Jangada pergunta antes de cada um e lembra a resposta."));
                ui.horizontal(|ui| {
                    ui.small(&tr!("{} sites com permissões guardadas.", ajustes::quantas_permissoes()));
                    if ui.small_button(tr!("Esquecer as Permissões")) {
                        ajustes::esquecer_permissoes();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(tr!("Limpar o Histórico…")) {
                        self.pergunta = 1;
                    }
                    if ui.button(tr!("Apagar Cookies e Dados dos Sites…")) {
                        self.pergunta = 2;
                    }
                });
            }
            // Tudo das extensões num lugar só: instalar, as instaladas e as que vêm com a
            // Jangada. Rola quando não cabe (a janela tem tamanho fixo).
            Secao::Extensoes => {
                let (x, y) = ui.cursor();
                let area = Rect::new(x, y, ui.available_width() + 20.0, ui.c.h - y - 8.0);
                let (mut rolagem, altura) = (self.prefs_rolagem, self.prefs_altura);
                self.prefs_altura = ui.scroll_view(area, &mut rolagem, altura, |ui| {
                    // Uma folga entre o texto e a barra de rolagem.
                    let largura = ui.available_width() - 14.0;
                    ui.column(largura, |ui| {
                        ui.label_ex(tr!("Instalar uma extensão"), 12.0, true, tinta());
                        ui.space(4.0);
                        ui.paragraph(tr!("Uma pasta, um arquivo .zip ou um repositório do GitHub (https://github.com/usuario/repositorio). Antes de instalar, você vê o que ela pede."));
                        ui.space(4.0);
                        ui.horizontal(|ui| {
                            ui.text_field(&mut self.ext_fonte, 330.0);
                            let ocupado = self.ext_rx.is_some();
                            ui.set_active(!ocupado && !self.ext_fonte.trim().is_empty());
                            if ui.default_button(tr!("Verificar")) {
                                let origem = self.ext_fonte.trim().to_string();
                                let (tx, rx) = std::sync::mpsc::channel();
                                std::thread::spawn(move || {
                                    let _ = tx.send(extensoes::preparar(&origem));
                                });
                                self.ext_rx = Some(rx);
                                self.ext_msg = Some(tr!("Verificando…").into());
                            }
                            ui.set_active(true);
                        });
                        if let Some(m) = self.ext_msg.clone() {
                            ui.small(&m);
                        }
                        ui.space(12.0);
                        ui.label_ex(tr!("Instaladas"), 12.0, true, tinta());
                        ui.space(4.0);
                        let lista = extensoes::listar();
                        if lista.is_empty() {
                            ui.paragraph(tr!("Nenhuma extensão instalada."));
                        }
                        for e in lista {
                            ui.push_id(&e.id, |ui| {
                                ui.horizontal(|ui| {
                                    let mut ativa = e.ativa;
                                    if ui.checkbox(&mut ativa, &format!("{} {}", e.nome, e.versao)) {
                                        extensoes::alternar(&e.id); motor::extensoes_mudaram();
                                    }
                                    if e.ativa && e.opcoes.is_some() && ui.small_button(tr!("Opções…")) {
                                        abrir_opcoes(&e);
                                    }
                                    if ui.small_button(tr!("Remover")) {
                                        extensoes::remover(&e.id); motor::extensoes_mudaram();
                                        self.ext_msg = Some(tr!("Extensão removida.").into());
                                    }
                                });
                                if !e.descricao.is_empty() {
                                    ui.small(&e.descricao);
                                }
                            });
                        }
                        ui.space(18.0);
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
                        ui.space(8.0);
                    });
                });
                self.prefs_rolagem = rolagem;
            }
            Secao::Avancado => {
                ui.checkbox(&mut self.aj.endereco_completo, tr!("Mostrar o endereço completo na barra"));
                ui.checkbox(&mut self.aj.placa_de_video, tr!("Usar a placa de vídeo (vale ao reabrir a Jangada)"));
                ui.paragraph(tr!("Sem ela a Jangada gasta menos memória; com ela, vídeos e jogos ficam mais leves para o processador."));
                ui.space(6.0);
                ui.checkbox(&mut self.aj.ferramentas_dev, tr!("Mostrar \"Inspecionar Elemento\" no menu das páginas"));
                ui.paragraph(tr!("Visualizar ▸ Ferramentas do Desenvolvedor (Ctrl+Shift+I) abre o inspetor da aba numa janela ao lado. Nada fica aberto para outros programas."));
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
        // A preparação de uma extensão terminou: mostra o que ela pede e espera a escolha.
        if let Some(rx) = &self.ext_rx {
            match rx.try_recv() {
                Ok(Ok(e)) => {
                    self.ext_confirmar = Some(e);
                    self.ext_rx = None;
                    self.ext_msg = None;
                }
                Ok(Err(e)) => {
                    self.ext_msg = Some(e);
                    self.ext_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.wake_after(Duration::from_millis(200)),
                Err(_) => self.ext_rx = None,
            }
        }
        if let Some(e) = self.ext_confirmar.clone() {
            let pede = if e.permissoes.is_empty() { tr!("Não pede acesso especial a nada.").to_string() } else { tr!("Pede acesso a: {}.", e.permissoes.join(", ")) };
            let texto = format!("{} {} — {}\n{}", e.nome, e.versao, e.descricao, pede);
            match ui.alert(glove_ui::AlertKind::Note, &tr!("Instalar esta extensão?"), &texto, &[tr!("Cancelar"), tr!("Instalar")]) {
                Some(1) => {
                    self.ext_msg = Some(match extensoes::instalar(&e) {
                        Ok(i) => tr!("Instalada: {}.", i.nome).to_string(),
                        Err(err) => err,
                    });
                    motor::extensoes_mudaram();
                    self.ext_confirmar = None;
                    self.ext_fonte.clear();
                }
                Some(_) => self.ext_confirmar = None,
                None => {}
            }
        }
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
                    motor::apagar_dados();
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
            if self.aj.ferramentas_dev != antes.ferramentas_dev {
                motor::ferramentas_dev(self.aj.ferramentas_dev);
            }
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
    let (forte, fraco) = if ui.theme.dark || ajustes::privada() { (228.0, 70.0) } else { (64.0, 200.0) };
    let v = (fraco + (forte - fraco) * forca.clamp(0.0, 1.0)) as u8;
    rgba(v, v, v.saturating_add(4), 255)
}

fn fr_sugestao_w(w: f32) -> f32 {
    (w - 240.0).clamp(120.0, 640.0)
}

/// Qual sugestão está na altura `y` (contada do topo da lista): 4 de margem e 26 por linha.
fn sugestao_na_altura(y: f32, n: usize) -> Option<usize> {
    let i = ((y - 4.0) / 26.0).floor();
    (i >= 0.0 && (i as usize) < n).then_some(i as usize)
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
    // Each entry is lowercased once and goes to one of the two lists.
    let mut ordem: Vec<&(String, String)> = Vec::new();
    let mut resto: Vec<&(String, String)> = Vec::new();
    for item in favoritos.iter().chain(historico.iter()) {
        let (t, u) = (item.0.to_lowercase(), item.1.to_lowercase());
        let sem_esquema = u.split_once("://").map_or(u.as_str(), |(_, r)| r).trim_start_matches("www.");
        if sem_esquema.starts_with(&busca) || t.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&busca)) {
            ordem.push(item);
        } else if t.contains(&busca) || u.contains(&busca) {
            resto.push(item);
        }
    }
    ordem.extend(resto);
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

/// As extensões na barra e a janelinha delas.
impl Navegador {
    /// A peça (o menu das extensões) e o botão de cada extensão com `action`, da direita
    /// para a esquerda, até `limite`. Devolve onde a barra continua.
    fn botoes_extensoes(&mut self, ui: &mut Ui, mut dir: f32, by: f32, limite: f32) -> f32 {
        self.ext_botoes.clear();
        self.ext_menu_rect = None;
        let lista: Vec<extensoes::Extensao> = extensoes::listar().into_iter().filter(|e| e.ativa).collect();
        if lista.is_empty() {
            return dir;
        }
        let chave = (extensoes::versao(), (ui.c.scale * 100.0) as u32);
        if self.ext_icones_de != chave {
            self.ext_icones.clear();
            self.ext_icones_de = chave;
        }
        // A peça: todas as extensões, as opções delas e o gerenciador.
        let r = Rect::new(dir - 28.0, by, 28.0, 26.0);
        let h = ui.hit(r);
        if h.hover || self.ext_painel {
            ui.c.fill(rrect(r.x, r.y, r.w, r.h, 6.0), &solid(rgba(0, 0, 0, if h.down || self.ext_painel { 34 } else { 18 })));
        }
        let cor = cor_glifo(ui, 1.0);
        sem_troca(ui.c, |c| peca(c, r.x + r.w / 2.0, r.y + r.h / 2.0, cor));
        if !self.ext_painel {
            ui.tooltip(r, tr!("Extensões"));
        }
        if h.clicked {
            self.ext_painel = !self.ext_painel;
            if self.ext_painel {
                self.fechar_popup();
            }
        }
        self.ext_menu_rect = Some(r);
        dir -= 32.0;
        // Os botões das fixadas na barra: na ordem da lista, da esquerda para a direita.
        let fixadas = self.ext_fixadas.get_or_insert_with(extensoes::fixadas).clone();
        let com_acao: Vec<&extensoes::Extensao> = lista.iter().filter(|e| e.acao.is_some() && fixadas.contains(&e.id)).collect();
        let cabem = (((dir - limite) / 30.0).floor().max(0.0) as usize).min(6);
        for e in com_acao.iter().take(cabem).rev() {
            let r = Rect::new(dir - 28.0, by, 28.0, 26.0);
            self.botao_extensao(ui, e, r);
            self.ext_botoes.push((e.id.clone(), r));
            dir -= 30.0;
        }
        dir
    }

    /// The icon of an extension (16 points), made once.
    fn icone_ext(&mut self, ui: &mut Ui, e: &extensoes::Extensao, ix: f32, iy: f32) {
        if !self.ext_icones.contains_key(&e.id) {
            let px = (16.0 * ui.c.scale).ceil() as u32;
            let ic = icone_de_arquivo(e.icone(px), ui.c.scale);
            self.ext_icones.insert(e.id.clone(), ic);
        }
        let antes = std::mem::replace(&mut ui.c.dark_remap, false);
        match self.ext_icones.get(&e.id) {
            Some(Some(ic)) => ui.c.draw_canvas(ic, ix, iy),
            // Sem ícone: a inicial do nome num quadradinho.
            _ => {
                ui.c.fill(rrect(ix, iy, 16.0, 16.0, 4.0), &solid(ui.theme.accent));
                let l: String = e.nome.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                let lw = ui.fonts.measure(&l, 10.0, true);
                ui.fonts.draw(ui.c, &l, ix + (16.0 - lw) / 2.0, iy + 12.0, 10.0, true, Color::WHITE);
            }
        }
        ui.c.dark_remap = antes;
    }

    /// The little window of the puzzle piece, under it (over the page): each extension
    /// installed with its icon and name (a click opens it, as its button does), the options
    /// and the pin that puts its button on the bar; and Manage Extensions.
    fn painel_extensoes(&mut self, ui: &mut Ui) {
        let Some(peca_r) = self.ext_menu_rect.filter(|_| self.ext_painel) else {
            self.ext_painel = false;
            return;
        };
        let lista: Vec<extensoes::Extensao> = extensoes::listar().into_iter().filter(|e| e.ativa).collect();
        let fixadas = self.ext_fixadas.get_or_insert_with(extensoes::fixadas).clone();
        const W: f32 = 300.0;
        const LINHA: f32 = 34.0;
        let n = lista.len().max(1) as f32;
        let h = 34.0 + n * LINHA + 10.0 + 30.0 + 6.0;
        let x = (peca_r.x + peca_r.w - W).max(8.0);
        let r = Rect::new(x, peca_r.y + peca_r.h + 4.0, W, h);
        let escuro = ui.theme.dark;
        ui.c.fill(rrect(r.x + 1.0, r.y + 3.0, r.w, r.h, 10.0), &solid(rgba(0, 0, 0, 40)));
        ui.c.fill(rrect(r.x, r.y, r.w, r.h, 10.0), &solid(if escuro { rgba(44, 44, 48, 252) } else { rgba(252, 252, 254, 252) }));
        ui.c.stroke(rrect(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 10.0), &solid(rgba(150, 150, 158, 200)), 1.0);
        ui.fonts.draw(ui.c, tr!("Extensões"), r.x + 14.0, r.y + 22.0, 13.0, true, tinta());
        let mut y = r.y + 34.0;
        let mut fechar = false;
        if lista.is_empty() {
            ui.fonts.draw(ui.c, tr!("Nenhuma extensão ligada"), r.x + 14.0, y + 21.0, 12.0, false, rgba(140, 140, 146, 255));
            y += LINHA;
        }
        for e in &lista {
            let linha = Rect::new(r.x + 6.0, y, r.w - 12.0, LINHA - 2.0);
            let pino = Rect::new(r.x + r.w - 40.0, y + 4.0, 26.0, 26.0);
            let opcoes = Rect::new(r.x + r.w - 68.0, y + 4.0, 26.0, 26.0);
            let tem_opcoes = e.opcoes.is_some();
            // The name opens it (only the part out of the small buttons).
            let nome_r = Rect::new(linha.x, linha.y, if tem_opcoes { opcoes.x } else { pino.x } - linha.x - 2.0, linha.h);
            let hn = ui.hit(nome_r);
            if hn.hover {
                ui.c.fill(rrect(linha.x, linha.y, linha.w, linha.h, 6.0), &solid(rgba(0, 0, 0, if hn.down { 30 } else { 16 })));
            }
            self.icone_ext(ui, e, r.x + 16.0, y + 8.0);
            let nome = ui.fonts.ellipsize(&e.nome, 13.0, false, nome_r.w - 42.0);
            ui.fonts.draw(ui.c, &nome, r.x + 42.0, y + 21.0, 13.0, false, if e.acao.is_some() { tinta() } else { rgba(140, 140, 146, 255) });
            if hn.clicked && e.acao.is_some() {
                self.clicar_extensao(&e.id, peca_r);
                fechar = true;
            }
            if tem_opcoes {
                let ho = ui.hit(opcoes);
                if ho.hover {
                    ui.c.fill(rrect(opcoes.x, opcoes.y, opcoes.w, opcoes.h, 6.0), &solid(rgba(0, 0, 0, if ho.down { 34 } else { 18 })));
                }
                let cor = cor_glifo(ui, 0.85);
                sem_troca(ui.c, |c| engrenagem(c, opcoes.x + 13.0, opcoes.y + 13.0, cor));
                ui.tooltip(opcoes, &tr!("Opções de {}…", e.nome));
                if ho.clicked {
                    abrir_opcoes(e);
                    fechar = true;
                }
            }
            if e.acao.is_some() {
                let fixada = fixadas.contains(&e.id);
                let hp = ui.hit(pino);
                if hp.hover {
                    ui.c.fill(rrect(pino.x, pino.y, pino.w, pino.h, 6.0), &solid(rgba(0, 0, 0, if hp.down { 34 } else { 18 })));
                }
                let cor = if fixada { ui.theme.accent } else { cor_glifo(ui, 0.7) };
                sem_troca(ui.c, |c| alfinete(c, pino.x + 13.0, pino.y + 13.0, cor, fixada));
                ui.tooltip(pino, if fixada { tr!("Tirar da barra") } else { tr!("Fixar na barra") });
                if hp.clicked {
                    self.ext_fixadas = Some(extensoes::fixar(&e.id, !fixada));
                    self.mudou = true;
                }
            }
            y += LINHA;
        }
        y += 4.0;
        ui.c.hline(r.x + 10.0, y, r.w - 20.0, rgba(150, 150, 158, 120));
        y += 4.0;
        let gerenciar = Rect::new(r.x + 6.0, y, r.w - 12.0, 28.0);
        let hg = ui.hit(gerenciar);
        if hg.hover {
            ui.c.fill(rrect(gerenciar.x, gerenciar.y, gerenciar.w, gerenciar.h, 6.0), &solid(rgba(0, 0, 0, if hg.down { 30 } else { 16 })));
        }
        ui.fonts.draw(ui.c, tr!("Gerenciar Extensões…"), r.x + 14.0, y + 19.0, 13.0, false, tinta());
        if hg.clicked {
            self.acao_menu("ext-gerenciar");
            fechar = true;
        }
        // A click out of it (and out of the piece) or Esc closes it.
        let (px, py) = ui.pointer();
        let fora = !r.contains((px, py)) && !peca_r.contains((px, py));
        if (ui.mouse_pressed() && fora) || ui.keys().iter().any(|k| k.key == glove_ui::ui::Key::Escape) {
            fechar = true;
        }
        if fechar {
            self.ext_painel = false;
            self.mudou = true;
        }
    }

    /// O botão de uma extensão: o ícone, o selo (`setBadgeText`) e a dica.
    fn botao_extensao(&mut self, ui: &mut Ui, e: &extensoes::Extensao, r: Rect) {
        let est = ponte::estado(&e.id);
        let aberta = ponte::POPUP.with(|p| p.borrow().as_ref().is_some_and(|p| p.ext == e.id));
        let h = ui.hit(r);
        if (h.hover && !est.desligado) || aberta {
            ui.c.fill(rrect(r.x, r.y, r.w, r.h, 6.0), &solid(rgba(0, 0, 0, if h.down || aberta { 34 } else { 18 })));
        }
        let (ix, iy) = (r.x + (r.w - 16.0) / 2.0, r.y + (r.h - 16.0) / 2.0);
        self.icone_ext(ui, e, ix, iy);
        let antes = std::mem::replace(&mut ui.c.dark_remap, false);
        if est.desligado {
            let fundo = if ui.theme.dark { rgba(40, 40, 44, 150) } else { rgba(236, 236, 236, 160) };
            ui.c.fill(rrect(ix - 1.0, iy - 1.0, 18.0, 18.0, 4.0), &solid(fundo));
        }
        if !est.selo.is_empty() {
            let tw = ui.fonts.measure(&est.selo, 9.0, true);
            let (bw, bh) = ((tw + 6.0).max(13.0), 12.0);
            let bx = (r.x + r.w - bw).max(r.x);
            let byy = r.y + r.h - bh;
            let fundo = est.cor.as_deref().and_then(cor_css).unwrap_or(rgba(217, 48, 37, 255));
            let letra = est.cor_texto.as_deref().and_then(cor_css).unwrap_or(Color::WHITE);
            ui.c.fill(rrect(bx, byy, bw, bh, 6.0), &solid(fundo));
            ui.fonts.draw(ui.c, &est.selo, bx + (bw - tw) / 2.0, byy + 9.0, 9.0, true, letra);
        }
        ui.c.dark_remap = antes;
        // A dica fica de fora com a janelinha aberta (ela cairia por cima).
        if !aberta {
            ui.tooltip(r, &est.titulo.clone().unwrap_or_else(|| e.titulo_da_acao()));
        }
        if h.clicked && !est.desligado {
            self.clicar_extensao(&e.id, r);
        }
    }

    /// Onde ancorar a janelinha de uma extensão: no botão dela, ou na peça.
    fn ancora_de(&self, id: &str) -> Rect {
        self.ext_botoes.iter().find(|(i, _)| i == id).map(|(_, r)| *r).or(self.ext_menu_rect).unwrap_or(Rect::new(self.area.x + self.area.w - 40.0, 7.0, 28.0, 26.0))
    }

    /// O clique no botão de uma extensão (ou no nome dela no menu): abre a janelinha (o
    /// segundo clique a fecha) ou avisa os scripts dela na aba (`action.onClicked`).
    fn clicar_extensao(&mut self, id: &str, ancora: Rect) {
        let Some(e) = extensoes::ativa(id) else { return };
        let mesma = ponte::POPUP.with(|p| p.borrow().as_ref().is_some_and(|p| p.ext == id));
        self.fechar_popup();
        if mesma {
            return;
        }
        // Com "activeTab", ser chamada nesta aba é poder vê-la (como no Chrome).
        if e.permissoes.iter().any(|p| p == "activeTab") {
            if let Some(a) = self.aba() {
                ponte::conceder_aba(id, a.p.borrow().id);
            }
        }
        match ponte::popup_de(&e) {
            Some(c) => self.abrir_popup(&e, &c, ancora),
            None => ponte::clique_na_acao(&e),
        }
    }

    fn abrir_popup(&mut self, e: &extensoes::Extensao, caminho: &str, ancora: Rect) {
        self.fechar_popup();
        let Some(view) = motor::janelinha(e, caminho) else { return };
        self.popup_ancora = ancora;
        ponte::POPUP.with(|p| *p.borrow_mut() = Some(ponte::Popup { ext: e.id.clone(), view, tamanho: None, fechar: false, focada: false, desde: Instant::now() }));
        self.mudou = true;
        glove_ui::redraw_soon();
    }

    fn fechar_popup(&mut self) {
        let Some(p) = ponte::POPUP.with(|p| p.borrow_mut().take()) else { return };
        if let Some(h) = &self.host {
            h.remove(&p.view);
        }
        // O processo da janelinha é só dela.
        p.view.terminate_web_process();
        self.popup_rect = None;
        self.focar_pagina = true;
        self.mudou = true;
        glove_ui::redraw_soon();
    }

    /// A janelinha, se o teclado estiver nela.
    fn popup_com_foco(&self) -> Option<WebView> {
        ponte::POPUP.with(|p| p.borrow().as_ref().filter(|p| p.view.has_focus()).map(|p| p.view.clone()))
    }

    /// A janelinha da extensão: embaixo do botão dela, no tamanho que a página pediu (até
    /// 800 × 600), com moldura e sombra. Fecha ao clicar fora (nas barras ou na página), com
    /// Esc, com window.close(), ao trocar de aba ou se a extensão for desligada.
    fn janelinha(&mut self, ui: &mut Ui) {
        let Some((ext, fechar, tamanho, desde, focada, view)) = ponte::POPUP.with(|p| p.borrow().as_ref().map(|p| (p.ext.clone(), p.fechar, p.tamanho, p.desde, p.focada, p.view.clone()))) else {
            self.popup_rect = None;
            motor::CLIQUE_NA_PAGINA.set(false);
            return;
        };
        let na_pagina = motor::CLIQUE_NA_PAGINA.replace(false);
        let (px, py) = ui.pointer();
        let fora = ui.mouse_pressed() && !self.popup_rect.is_some_and(|r| r.contains((px, py))) && !self.popup_ancora.contains((px, py));
        if fechar || na_pagina || fora || extensoes::ativa(&ext).is_none() {
            self.fechar_popup();
            return;
        }
        let Some(host) = self.host.clone() else { return };
        // O botão pode ter mudado de lugar (a janela mudou de tamanho).
        if let Some((_, r)) = self.ext_botoes.iter().find(|(i, _)| *i == ext) {
            self.popup_ancora = *r;
        }
        let (w, h) = (ui.c.w, ui.c.h);
        // Até a página dizer o tamanho dela, fica fora da janela (medindo-se); se não disser
        // logo, um tamanho padrão.
        let medida = tamanho.or_else(|| (desde.elapsed() > Duration::from_millis(700)).then_some((360.0, 240.0)));
        let Some((pw, ph)) = medida else {
            host.place(&view, w + 50.0, 0.0, 400.0, 300.0);
            ui.wake_after(Duration::from_millis(60));
            return;
        };
        let a = self.popup_ancora;
        let y = (a.y + a.h + 6.0).round();
        let pw = pw.min(w - 16.0).max(25.0).round();
        let ph = ph.min(h - y - 8.0).max(25.0).round();
        let x = (a.x + a.w - pw).min(w - pw - 8.0).max(8.0).round();
        // A sombra, a borda e o furo onde a página da extensão aparece.
        for i in 1..=6 {
            let f = i as f32;
            ui.c.fill(rrect(x - f, y - f + 2.0, pw + 2.0 * f, ph + 2.0 * f, 6.0 + f), &solid(rgba(0, 0, 0, 7)));
        }
        let borda = if ui.theme.dark { rgba(90, 90, 96, 255) } else { rgba(170, 170, 176, 255) };
        ui.c.fill(rrect(x - 1.0, y - 1.0, pw + 2.0, ph + 2.0, 5.0), &solid(borda));
        ui.c.clear_rect(x, y, pw, ph);
        host.place(&view, x, y, pw, ph);
        self.popup_rect = Some(Rect::new(x, y, pw, ph));
        if !focada {
            view.grab_focus();
            ponte::POPUP.with(|p| {
                if let Some(p) = p.borrow_mut().as_mut() {
                    p.focada = true;
                }
            });
        }
    }

    /// O que as extensões pediram à janela: abrir, mudar e fechar abas, abrir a janelinha.
    fn pedidos_das_extensoes(&mut self) {
        let pedidos = ponte::PEDIDOS.with(|p| std::mem::take(&mut *p.borrow_mut()));
        for p in pedidos {
            match p {
                ponte::Pedido::AbrirAba { url, ativa, volta } => {
                    let url = if url.is_empty() { self.pagina_inicial() } else { url };
                    // Uma página de extensão já aberta (as opções): vai para a aba dela.
                    let ja = url.starts_with(extensoes::ESQUEMA).then(|| self.abas.iter().position(|a| a.p.borrow().url == url)).flatten();
                    let id = match ja {
                        Some(i) => {
                            self.mudar(i);
                            Some(self.abas[i].p.borrow().id)
                        }
                        None => {
                            let antes = self.abas.len();
                            self.abrir_em(&url, !ativa);
                            (self.abas.len() > antes).then(|| self.abas[self.abas.len() - 1].p.borrow().id)
                        }
                    };
                    ponte::responder_aba(volta, id);
                }
                ponte::Pedido::AtualizarAba { aba, url, ativa, volta } => {
                    let Some(i) = self.abas.iter().position(|a| a.p.borrow().id == aba) else {
                        ponte::responder_vazio(volta, Err(format!("No tab with id: {aba}.")));
                        continue;
                    };
                    if let Some(u) = url {
                        let v = {
                            let mut p = self.abas[i].p.borrow_mut();
                            p.url = u.clone();
                            p.view.clone()
                        };
                        motor::permitir_navegacao(&u);
                        v.load_uri(&u);
                    }
                    if ativa {
                        self.mudar(i);
                    }
                    ponte::responder_aba(volta, Some(aba));
                }
                ponte::Pedido::FecharAbas { abas, volta } => {
                    for id in abas {
                        if let Some(i) = self.abas.iter().position(|a| a.p.borrow().id == id) {
                            self.fechar(i);
                        }
                    }
                    ponte::responder_vazio(volta, Ok(()));
                }
                ponte::Pedido::AbrirPopup { ext, volta } => {
                    if let Some(e) = extensoes::ativa(&ext) {
                        if let Some(c) = ponte::popup_de(&e) {
                            let ancora = self.ancora_de(&ext);
                            self.abrir_popup(&e, &c, ancora);
                        }
                    }
                    ponte::responder_vazio(volta, Ok(()));
                }
            }
        }
    }
}

/// Os itens das extensões ligadas nos menus: o botão de cada uma e as opções.
fn itens_das_extensoes() -> Vec<MenuItem> {
    let mut v = Vec::new();
    for e in extensoes::listar().into_iter().filter(|e| e.ativa) {
        v.push(MenuItem::new(&format!("xacao:{}", e.id), &e.nome, "").enabled(e.acao.is_some()));
        if e.opcoes.is_some() {
            v.push(MenuItem::new(&format!("xopcoes:{}", e.id), &tr!("Opções de {}…", e.nome), ""));
        }
    }
    if v.is_empty() {
        v.push(MenuItem::new("ext-nenhuma", tr!("Nenhuma extensão ligada"), "").enabled(false));
    }
    v.push(MenuItem::Separator);
    v
}

/// Abre a página de opções de uma extensão numa aba (ou vai para a aba dela, se já
/// estiver aberta).
fn abrir_opcoes(e: &extensoes::Extensao) {
    if let Some(o) = &e.opcoes {
        ponte::PEDIDOS.with(|p| p.borrow_mut().push(ponte::Pedido::AbrirAba { url: e.url(o), ativa: true, volta: None }));
        glove_ui::redraw_soon();
    }
}

/// O ícone de uma extensão (PNG, JPEG, SVG...) em 16 pontos.
fn icone_de_arquivo(caminho: Option<PathBuf>, escala: f32) -> Option<Canvas> {
    let t = glove_ui::gtk::gdk::Texture::from_filename(caminho?).ok()?;
    let (w, h) = (t.width(), t.height());
    if w <= 0 || h <= 0 || w > 2048 || h > 2048 {
        return None;
    }
    let mut buf = vec![0u8; (w * h * 4) as usize];
    t.download(&mut buf, (w * 4) as usize);
    // O GTK entrega BGRA pré-multiplicado (na ordem da memória): vira RGBA.
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    icone_16(&buf, w, h, escala)
}

/// Uma cor do CSS (#rgb, #rrggbb, #rrggbbaa, rgb(), rgba() e alguns nomes), para o selo.
fn cor_css(t: &str) -> Option<Color> {
    let t = t.trim().to_ascii_lowercase();
    if let Some(h) = t.strip_prefix('#') {
        let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok().map(|x| if n == 1 { x * 17 } else { x });
        return match h.len() {
            3 => Some(rgba(v(0, 1)?, v(1, 1)?, v(2, 1)?, 255)),
            6 => Some(rgba(v(0, 2)?, v(2, 2)?, v(4, 2)?, 255)),
            8 => Some(rgba(v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?)),
            _ => None,
        };
    }
    if let Some(dentro) = t.strip_prefix("rgba(").or_else(|| t.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
        let n: Vec<f32> = dentro.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if n.len() >= 3 {
            let a = n.get(3).map(|a| if *a <= 1.0 { a * 255.0 } else { *a }).unwrap_or(255.0);
            return Some(rgba(n[0] as u8, n[1] as u8, n[2] as u8, a.clamp(0.0, 255.0) as u8));
        }
        return None;
    }
    Some(match t.as_str() {
        "red" => rgba(217, 48, 37, 255),
        "green" => rgba(30, 142, 62, 255),
        "blue" => rgba(26, 115, 232, 255),
        "orange" => rgba(242, 153, 0, 255),
        "yellow" => rgba(249, 202, 36, 255),
        "purple" => rgba(132, 48, 206, 255),
        "gray" | "grey" => rgba(95, 99, 104, 255),
        "black" => rgba(0, 0, 0, 255),
        "white" => rgba(255, 255, 255, 255),
        _ => return None,
    })
}

/// A pin (pinned: filled), to put an extension on the bar.
fn alfinete(c: &mut Canvas, cx: f32, cy: f32, cor: Color, cheio: bool) {
    // The head, the plate under it and the needle.
    let cabeca = rrect(cx - 3.5, cy - 7.0, 7.0, 6.0, 1.5);
    if cheio {
        c.fill(cabeca, &solid(cor));
    } else {
        c.stroke(cabeca, &solid(cor), 1.2);
    }
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(cx - 5.5, cy - 0.5);
    pb.line_to(cx + 5.5, cy - 0.5);
    pb.move_to(cx, cy - 0.5);
    pb.line_to(cx, cy + 7.0);
    c.stroke_round(pb.finish(), &solid(cor), 1.4);
}

/// A small gear (the options of an extension).
fn engrenagem(c: &mut Canvas, cx: f32, cy: f32, cor: Color) {
    let mut pb = tiny_skia::PathBuilder::new();
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        pb.move_to(cx + 4.5 * a.cos(), cy + 4.5 * a.sin());
        pb.line_to(cx + 7.0 * a.cos(), cy + 7.0 * a.sin());
    }
    c.stroke_round(pb.finish(), &solid(cor), 2.0);
    c.stroke(circle(cx, cy, 4.2), &solid(cor), 1.6);
}

/// A peça de quebra-cabeça (o botão das extensões).
fn peca(c: &mut Canvas, cx: f32, cy: f32, cor: Color) {
    let (x0, y0, x1, y1) = (cx - 6.0, cy - 4.0, cx + 4.0, cy + 6.0);
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(cx - 3.0, y0);
    pb.cubic_to(cx - 3.5, y0 - 4.0, cx + 1.5, y0 - 4.0, cx + 1.0, y0);
    pb.line_to(x1, y0);
    pb.line_to(x1, cy - 1.0);
    pb.cubic_to(x1 + 4.0, cy - 1.5, x1 + 4.0, cy + 3.5, x1, cy + 3.0);
    pb.line_to(x1, y1);
    pb.line_to(x0, y1);
    pb.close();
    c.stroke(pb.finish(), &solid(cor), 1.3);
}

impl App for Navegador {
    /// O segundo `jangada` manda os endereços pelo socket: a Jangada acorda quando chega
    /// alguém (nada fica rodando de fundo esperando).
    fn watch_fd(&self) -> Option<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        self.servidor.as_ref().map(|l| l.as_raw_fd())
    }

    fn fd_ready_on(&mut self, _fd: std::os::fd::RawFd) -> bool {
        self.ler_servidor();
        self.mudou = true;
        true
    }

    /// Com o teclado na página: os atalhos da Jangada vêm antes (copiar, colar e o resto
    /// ficam com a página).
    fn native_key(&mut self, combo: &str) -> bool {
        let da_jangada = matches!(
            combo,
            "Ctrl+T" | "Ctrl+W" | "Ctrl+Tab" | "Ctrl+Shift+Tab" | "Ctrl+R" | "F5" | "Ctrl+Shift+R" | "Ctrl+," | "Ctrl+comma" | "Ctrl+P" | "Ctrl+Shift+T" | "Ctrl+H" | "Ctrl+J" | "Ctrl+Shift+N" | "Ctrl+U" | "Ctrl+Shift+I" | "Ctrl+L" | "F6" | "Ctrl+F" | "Ctrl+D" | "Ctrl+Shift+B" | "Alt+Left" | "Alt+Right" | "Alt+Home" | "Ctrl+=" | "Ctrl++" | "Ctrl+Shift+=" | "Ctrl+plus" | "Ctrl+equal" | "Ctrl+-" | "Ctrl+minus" | "Ctrl+0"
        ) || (combo == "Escape" && self.achar.is_some());
        // A janelinha de uma extensão com o teclado: Esc a fecha; colar vai para ela.
        let popup = self.popup_com_foco();
        if combo == "Escape" && popup.is_some() {
            self.fechar_popup();
            return true;
        }
        // Colar: uma imagem ou arquivos copiados chegam à página como arquivos (o motor
        // sozinho não os entrega; o Telegram, o WhatsApp etc. não recebiam nada).
        if combo == "Ctrl+V" {
            return popup.or_else(|| self.view()).is_some_and(|v| arquivos::colar(&v));
        }
        if da_jangada {
            self.atalho(combo, false);
        }
        da_jangada
    }

    fn close_window(&mut self, id: u32) -> bool {
        if id == JANELA_PERGUNTA {
            // Fechar a janela da pergunta é cancelar.
            self.responder_pergunta(None);
        }
        if id != 0 {
            return true;
        }
        self.close_requested()
    }

    fn close_requested(&mut self) -> bool {
        // As abas da última vez (para "reabrir as abas") e as visitas ainda à espera.
        self.gravar_sessao();
        ajustes::gravar_visitas();
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
                MenuItem::new("dev", tr!("Ferramentas do Desenvolvedor"), "Ctrl+Shift+I"),
            ]),
            Menu::new(tr!("Histórico"), m_hist),
            Menu::new(tr!("Favoritos"), m_favs),
            Menu::new(tr!("Extensões"), vec![
                MenuItem::new("ext-escudos", tr!("Escudos: Anúncios e Rastreadores"), "").checked(self.aj.bloqueador),
                MenuItem::new("ext-cookies", tr!("Barrar Cookies de Terceiros"), "").checked(self.aj.cookies_terceiros),
                MenuItem::new("ext-escuro", tr!("Modo Escuro nas Páginas"), "").checked(self.aj.escuro_nas_paginas),
                MenuItem::Separator,
            ]
            .into_iter()
            .chain(itens_das_extensoes())
            .chain([MenuItem::new("ext-gerenciar", tr!("Gerenciar Extensões…"), "")])
            .collect()),
        ]
    }

    fn ui(&mut self, ui: &mut Ui) {
        match ui.window_id() {
            JANELA_PREFS => return self.preferencias(ui),
            JANELA_HIST => return self.janela_historico(ui),
            JANELA_DOWN => return self.janela_downloads(ui),
            JANELA_IMPORTAR => return self.janela_importar(ui),
            JANELA_PERGUNTA => return self.janela_pergunta(ui),
            0 => {}
            // Outra janela (do kit): o navegador só se desenha na principal.
            _ => return,
        }
        self.escala = ui.c.scale;
        self.escuro = ui.theme.dark;
        self.teclas_da_barra = false;
        if self.host.is_none() {
            self.host = ui.native_host();
        }
        self.processar();
        // O teclado foi para a página (um clique nela): o campo de endereço larga.
        if ui.native_focus() {
            self.barra_foco = false;
        }
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
            ui.open_window(JANELA_PREFS, WindowOptions { title: tr!("Preferências da Jangada").into(), app_id: "glove.Browser".into(), size: (600, 420), fixed: true, min_size: None });
        }
        if std::mem::take(&mut self.hist_abrir) {
            self.hist_lista = ajustes::historico();
            self.hist_busca.clear();
            self.hist_sel = None;
            ui.open_window(JANELA_HIST, WindowOptions { title: tr!("Histórico").into(), app_id: "glove.Browser".into(), size: (720, 480), fixed: false, min_size: Some((420, 260)) });
        }
        if std::mem::take(&mut self.imp_abrir) {
            self.imp_fontes = importar::fontes();
            self.imp_escolha = 0;
            self.imp_msg = None;
            ui.open_window(JANELA_IMPORTAR, WindowOptions { title: tr!("Importar de Outro Navegador").into(), app_id: "glove.Browser".into(), size: (460, 330), fixed: true, min_size: None });
        }
        if std::mem::take(&mut self.down_abrir) {
            ui.open_window(JANELA_DOWN, WindowOptions { title: tr!("Downloads").into(), app_id: "glove.Browser".into(), size: (560, 340), fixed: false, min_size: Some((380, 200)) });
        }
        // O botão vermelho da barra.
        if std::mem::take(&mut self.fechar_janela) && self.close_requested() {
            ui.close = true;
            return;
        }
        self.pagina_de_erro(ui.fonts);
        // O histórico do menu (os últimos 12), relido a cada poucos segundos.
        if self.hist_lido.elapsed() > Duration::from_secs(3) {
            if let Some(todos) = ajustes::history_if_changed(&mut self.hist_marca) {
                self.recentes = todos.iter().take(12).cloned().collect();
                self.recentes_todos = todos;
            }
            self.hist_lido = Instant::now();
            // As abas abertas ficam gravadas (se a Jangada cair, a sessão não se perde).
            self.gravar_sessao();
        }
        let (w, h) = (ui.c.w, ui.c.h);
        // Com a folha Imprimir aberta, as barras não respondem (a folha é da janela).
        ui.set_enabled(self.imprimir.is_none());
        self.atualizar_tela();
        // Um vídeo em tela cheia: só a página.
        let cheia = self.aba().is_some_and(|a| a.p.borrow().tela_cheia);
        let mut top = 0.0;
        if !cheia {
            self.barra(ui);
            top = BARRA_H;
            top += self.faixa_abas(ui, top);
            if top == BARRA_H {
                ui.c.hline(0.0, BARRA_H - 1.0, w, rgba(170, 170, 176, 255));
            }
            top += self.barra_favoritos(ui, top);
            top += self.barra_achar(ui, top);
        }
        // Sem o puxador do kit no canto (ele ficava por cima da página): a janela se
        // redimensiona pelas bordas. Depois dos botões da janela (o zoom continua ligado).
        ui.hide_resize_grip();
        let aviso = self.aviso.as_ref().map(|(_, t)| t.elapsed()).filter(|d| *d < Duration::from_secs(5));
        if let Some(d) = aviso {
            // O aviso some sozinho.
            ui.wake_after(Duration::from_secs(5) - d + Duration::from_millis(20));
        }
        let status = self.aviso.as_ref().filter(|_| aviso.is_some()).map(|(m, _)| m.clone()).or_else(|| {
            self.aba().map(|a| a.p.borrow().status.clone()).filter(|s| !s.is_empty())
        });
        let area = Rect::new(0.0, top, w, (h - top).max(1.0));
        // A página: o widget do motor, embaixo do desenho da Jangada (aqui o desenho fica
        // transparente: aparece a página, que recebe o mouse e o teclado direto).
        self.area = area;
        ui.c.clear_rect(area.x, area.y, area.w, area.h);
        self.pôr_pagina(area);
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
        // prompt(): a pergunta numa janela própria, com o campo do texto (uma de cada vez).
        if let (Some((motor::Dialogo::Pergunta(sugerido), msg, cb)), None, Some(a)) = (&dialog, &self.pergunta_js, self.aba()) {
            self.pergunta_js = Some(Pergunta { pagina: std::rc::Rc::downgrade(&a.p), dialogo: cb.clone(), site: site(&self.url_atual()), mensagem: msg.clone(), texto: sugerido.clone(), focar: true });
            ui.open_window(JANELA_PERGUNTA, WindowOptions { title: tr!("Pergunta da página").into(), app_id: "glove.Browser".into(), size: (440, 190), fixed: true, min_size: None });
        }
        // (alert e confirm vão pelo alerta do Glove, mais abaixo.)
        let dialog = dialog.filter(|d| !matches!(d.0, motor::Dialogo::Pergunta(_)));
        let guardar = senhas::PARA_GUARDAR.with(|p| p.borrow().first().cloned());
        // Permissões pedidas pelo site (câmera, microfone, localização, notificações): uma
        // pergunta de cada vez, numa janela de alerta própria. "Permitir" e "Negar" ficam
        // guardados para o site; "Só desta vez" não.
        let pedido = motor::PERMISSOES.with(|p| p.borrow().first().map(|(o, n, _)| (o.clone(), n.clone())));
        if let Some((origem, nomes)) = pedido {
            let site_ = escudos::site(&origem);
            let quer = nomes.iter().map(|n| tr(motor::rotulo_permissao(n)).to_string()).collect::<Vec<_>>().join(" e ");
            let titulo = tr!("{} quer usar {}?", site_, quer);
            let texto = tr!("Você decide agora. \"Permitir\" e \"Negar\" valem para este site da próxima vez; \"Só desta vez\" não fica guardado.");
            // A área de transferência é perguntada numa faixa dentro da janela: quem só queria
            // colar responde ali mesmo, sem uma janela de alerta tirando o teclado da página.
            let escolha = if nomes == ["clipboard"] {
                let i = Self::faixa_de_pergunta(ui, area, &titulo);
                self.focar_pagina |= i.is_some();
                i
            } else {
                ui.alert(glove_ui::AlertKind::Note, &titulo, &texto, &[tr!("Negar"), tr!("Só desta vez"), tr!("Permitir")])
            };
            if let Some(i) = escolha {
                let (_, _, pedido) = motor::PERMISSOES.with(|p| p.borrow_mut().remove(0));
                let permitir = i != 0;
                if i != 1 {
                    for n in &nomes {
                        ajustes::gravar_permissao(&origem, n, permitir);
                    }
                }
                if permitir {
                    pedido.0.allow();
                } else {
                    pedido.0.deny();
                }
            }
        } else if let (Some((origem, usuario, senha)), None) = (guardar, &dialog) {
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
        } else if let Some((tipo, msg, cb)) = dialog {
            let confirm = tipo == motor::Dialogo::Confirma;
            let botoes: Vec<&str> = if confirm { vec![tr!("Cancelar"), tr!("OK")] } else { vec![tr!("OK")] };
            let url = site(&self.url_atual());
            if let Some(i) = ui.alert(glove_ui::AlertKind::Note, &tr!("A página {} diz:", url), &msg, &botoes) {
                if confirm {
                    cb.confirm_set_confirmed(i == 1);
                }
                cb.close();
                if let Some(a) = self.aba() {
                    a.p.borrow_mut().dialog = None;
                }
            }
        } else if self.sobre {
            let motor_v = format!("{}.{}.{}", webkit6::functions::major_version(), webkit6::functions::minor_version(), webkit6::functions::micro_version());
            let texto = tr!("O navegador do Glove OS. Motor: WebKitGTK {}, o do sistema (as correções de segurança chegam pelas atualizações da distribuição). Leve: a placa de vídeo desenha as páginas e nada fica rodando de fundo.", motor_v);
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
            }
        }
        // A janelinha da peça (as extensões) e a de uma extensão, por cima da página.
        self.painel_extensoes(ui);
        self.janelinha(ui);
        // Os atalhos que chegaram ao kit; o menu de contexto que a página pediu.
        self.teclas(ui);
        self.menu_contexto(ui, area);
        // A página fica com o teclado (depois de navegar, trocar de aba, fechar a busca).
        if std::mem::take(&mut self.focar_pagina) && !self.barra_foco && !ui.has_focus() {
            if let Some(v) = self.view() {
                v.grab_focus();
            }
        }
    }
}

/// `jangada --install ORIGEM [--yes]`: mostra o que a extensão pede e instala (pergunta antes,
/// a não ser com --yes). ORIGEM: uma pasta, um .zip ou um repositório do GitHub.
fn instalar_pela_linha(resto: &[String]) -> std::process::ExitCode {
    let Some(origem) = resto.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: jangada --install FOLDER | FILE.zip | https://github.com/USER/REPOSITORY [--yes]");
        return 2.into();
    };
    let sim = resto.iter().any(|a| a == "--yes");
    let prep = match extensoes::preparar(origem) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("jangada: {e}");
            return 1.into();
        }
    };
    println!("{} {} — {}", prep.nome, prep.versao, prep.descricao);
    if prep.permissoes.is_empty() {
        println!("Não pede permissões especiais.");
    } else {
        println!("Pede acesso a:");
        for p in &prep.permissoes {
            println!("  - {p}");
        }
    }
    if !sim {
        print!("Instalar? [s/N] ");
        let _ = std::io::stdout().flush();
        let mut r = String::new();
        let _ = std::io::stdin().read_line(&mut r);
        if !matches!(r.trim().to_lowercase().as_str(), "s" | "sim" | "y" | "yes") {
            println!("Não instalada.");
            return 0.into();
        }
    }
    match extensoes::instalar(&prep) {
        Ok(i) => {
            println!("Instalada: {} ({})", i.nome, i.id);
            0.into()
        }
        Err(e) => {
            eprintln!("jangada: {e}");
            1.into()
        }
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

const AJUDA: &str = "Usage: jangada [OPTIONS] [URL...]

Opens the URLs in tabs (in the window that is already open, if there is one).

Options:
  --private               open a private window (nothing is kept)
  --debug [PORT]          serve the engine inspector on http://127.0.0.1:PORT (default 9222)
  --install SOURCE [--yes]
                          install an extension from a folder, a .zip or a GitHub link
                          (--yes: do not ask for confirmation)
  --extensions            list the installed extensions
  --remove ID             remove an installed extension
  --version               print the version
  --help                  print this help
";

/// A porta pedida com `--debug [PORTA]` (padrão 9222), ou None sem a opção.
fn porta_depuracao() -> Option<u16> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--debug")?;
    Some(args.get(i + 1).and_then(|p| p.parse::<u16>().ok()).filter(|p| *p != 0).unwrap_or(9222))
}

/// Os endereços da linha de comando: sem as opções e sem o número de `--debug`.
fn enderecos_da_linha() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = Vec::new();
    let mut pular = false;
    for (i, a) in args.iter().enumerate() {
        if pular {
            pular = false;
            continue;
        }
        if a == "--debug" {
            pular = args.get(i + 1).is_some_and(|p| p.parse::<u16>().is_ok());
            continue;
        }
        if !a.starts_with("--") {
            out.push(a.clone());
        }
    }
    out
}

/// As abas que aparecem na faixa: (a primeira à vista, quantas cabem). Cabendo todas,
/// (0, total). Senão, a faixa guarda um lugar para as setas e anda só o que for preciso
/// para a escolhida aparecer, e só quando a escolhida (ou o número de abas) muda: rolar
/// com a roda ou com as setas não é desfeito no quadro seguinte.
fn janela_de_abas(total: usize, atual: usize, w: f32, tw: f32, primeira: &mut usize, seguida: &mut (usize, usize)) -> (usize, usize) {
    let (x0, gap, mais, seta) = (6.0, 4.0, 28.0, 22.0);
    if total == 0 || x0 * 2.0 + mais + total as f32 * (tw + gap) <= w + 0.5 {
        *primeira = 0;
        *seguida = (atual, total);
        return (0, total);
    }
    let util = w - x0 * 2.0 - mais - (seta + 2.0) * 2.0;
    let cabem = (((util + gap) / (tw + gap)).floor() as usize).clamp(1, total);
    if *seguida != (atual, total) {
        *seguida = (atual, total);
        if atual < *primeira {
            *primeira = atual;
        } else if atual >= *primeira + cabem {
            *primeira = atual + 1 - cabem;
        }
    }
    *primeira = (*primeira).min(total - cabem);
    (*primeira, cabem)
}

/// O tamanho da janela ao abrir: 1100 × 760, ou o que couber na menor tela ligada (num
/// notebook de 1366 × 768, ou com a tela em 150%, a janela abria maior que a tela).
fn tamanho_inicial() -> (u32, u32) {
    use glove_ui::gtk::gdk;
    let telas: Vec<(i32, i32)> = gdk::Display::default()
        .map(|d| d.monitors())
        .map(|m| (0..m.n_items()).filter_map(|i| m.item(i).and_downcast::<gdk::Monitor>()).map(|m| (m.geometry().width(), m.geometry().height())).collect())
        .unwrap_or_default();
    caber(telas.iter().map(|t| t.0).min(), telas.iter().map(|t| t.1).min())
}

/// 1100 × 760 dentro da tela dada (em pontos), com folga para a barra de menu e o Dock;
/// nunca menor que o mínimo da janela.
fn caber(largura: Option<i32>, altura: Option<i32>) -> (u32, u32) {
    let w = largura.map_or(1100, |l| 1100.min(l - 24)).max(560);
    let h = altura.map_or(760, |a| 760.min(a - 110)).max(380);
    (w as u32, h as u32)
}

/// Where the engine travels with the Jangada (Slackware: the system has no WebKitGTK).
const MOTOR_LIB: &str = "/usr/lib/jangada/motor/lib";

fn main() -> std::process::ExitCode {
    // The processes of the engine (the page, the network, the GPU) are other programs: with
    // the engine of the package they must find its libraries too (the package also writes
    // the path in them; this covers a package made before that). Still one thread only:
    // changing the environment here is safe.
    if std::path::Path::new(MOTOR_LIB).is_dir() {
        let atual = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        if !atual.split(':').any(|p| p == MOTOR_LIB) {
            let novo = if atual.is_empty() { MOTOR_LIB.to_string() } else { format!("{MOTOR_LIB}:{atual}") };
            unsafe { std::env::set_var("LD_LIBRARY_PATH", novo) };
        }
    }
    // The page is painted by the processor (several threads) and the video card only puts
    // the pieces together on the screen. Painting on the video card (the default of the
    // engine since 2.46) left white areas after scrolling for a while and short freezes
    // (new content waits for the card to prepare its drawings) on Intel cards. The page
    // processes inherit this. JANGADA_GPU_PAINT=1 goes back to painting on the card.
    if std::env::var_os("JANGADA_GPU_PAINT").is_none() && std::env::var_os("WEBKIT_SKIA_ENABLE_CPU_RENDERING").is_none() {
        unsafe { std::env::set_var("WEBKIT_SKIA_ENABLE_CPU_RENDERING", "1") };
    }
    if let (Some(a), Some(dir)) = (std::env::args().nth(1), std::env::args().nth(2)) {
        if a == "--icons" {
            return gerar_icones(std::path::Path::new(&dir));
        }
    }
    // What an older Jangada stored under Portuguese names moves to the names of today.
    ajustes::migrate_old_names();
    // As extensões pela linha de comando (sem abrir o navegador).
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{AJUDA}");
            return 0.into();
        }
        Some("--version") => {
            println!("Jangada {VERSAO}");
            return 0.into();
        }
        Some("--install") => return instalar_pela_linha(&args[2..]),
        Some("--extensions") => {
            for e in extensoes::listar() {
                println!("{}\t{} {}\t{}", e.id, e.nome, e.versao, if e.ativa { "ativa" } else { "desligada" });
            }
            return 0.into();
        }
        Some("--remove") => {
            let id = args.get(2).cloned().unwrap_or_default();
            extensoes::remover(&id); motor::extensoes_mudaram();
            println!("{id} removida.");
            return 0.into();
        }
        _ => {}
    }
    // A porta de depuração (`--debug [PORTA]`): o inspetor do motor fica aberto em
    // http://127.0.0.1:PORTA para outro navegador (só neste computador).
    let pedida = porta_depuracao();
    if let Some(p) = pedida {
        // Segurança: ainda não há outras linhas de execução (o motor não subiu).
        unsafe { std::env::set_var("WEBKIT_INSPECTOR_HTTP_SERVER", format!("127.0.0.1:{p}")) };
        eprintln!("jangada: depuração em http://127.0.0.1:{p} (só neste computador)");
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
    let urls: Vec<String> = enderecos_da_linha().into_iter().map(|a| endereco(&a, &inicio_url, aj.busca_url())).collect();
    // Já aberta: manda os endereços para ela e sai.
    // A janela privada é outra janela: não manda os endereços para a normal e não a substitui.
    let privada = std::env::args().any(|a| a == "--private");
    ajustes::PRIVADA.store(privada, std::sync::atomic::Ordering::Relaxed);
    let sock = socket_path();
    if !privada {
        if let Ok(mut c) = UnixStream::connect(&sock) {
            if pedida.is_some() {
                eprintln!("jangada: já está aberta; a depuração só vale ao abrir a Jangada (feche-a e rode --debug de novo)");
            }
            let text = if urls.is_empty() { "\n".to_string() } else { urls.join("\n") };
            let _ = c.write_all(text.as_bytes());
            return 0.into();
        }
        let _ = std::fs::remove_file(&sock);
    }
    let servidor = if privada {
        None
    } else {
        UnixListener::bind(&sock).ok().inspect(|l| {
            let _ = l.set_nonblocking(true);
        })
    };
    // O GTK antes do motor (as abas são widgets dele).
    glib::set_prgname(Some("glove.Browser"));
    if glove_ui::gtk::init().is_err() {
        eprintln!("jangada: sem tela (o GTK não abriu)");
        return 1.into();
    }
    // Privada: a sessão do motor fica só na memória (cookies, cache, senhas digitadas).
    motor::iniciar(privada, &dados, &aj);
    // Os escudos (os filtros já compilados abrem na hora).
    escudos::iniciar();
    // As abas da última vez: reabertas já (Preferências) ou guardadas para o menu Histórico.
    let fundo_imagem = aj.inicio_fundo.strip_prefix("image:").unwrap_or("").to_string();
    let sessao = ajustes::sessao();
    let aj_reabrir = aj.reabrir_abas && urls.is_empty();
    let mut nav = Navegador {
        abas: Vec::new(),
        atual: 0,
        barra: String::new(),
        barra_buf: glove_ui::TextBuffer::default(),
        barra_mexida: false,
        barra_foco: false,
        focar_barra: false,
        area: Rect::new(0.0, BARRA_H, 1100.0, 700.0),
        escala: 1.0,
        na_sugestao: false,
        aviso: None,
        servidor,
        ctx_dados: None,
        inicio: inicio_url,
        dados,
        escuro,
        sobre: false,
        teclas_da_barra: false,
        aj,
        favoritos,
        sugestoes: Vec::new(),
        sugestoes_de: None,
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
        hist_marca: None,
        fechar_janela: false,
        fechadas: Vec::new(),
        sessao_anterior: if aj_reabrir { Vec::new() } else { sessao.clone() },
        sessao_gravada: Vec::new(),
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
        ext_fonte: String::new(),
        ext_rx: None,
        ext_confirmar: None,
        ext_msg: None,
        aba_primeira: 0,
        aba_seguida: (0, 0),
        aba_rolagem: 0.0,
        pergunta_js: None,
        host: None,
        a_vista: None,
        focar_pagina: false,
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
        ext_botoes: Vec::new(),
        ext_menu_rect: None,
        ext_painel: false,
        ext_fixadas: None,
        ext_icones: std::collections::HashMap::new(),
        ext_icones_de: (0, 0),
        popup_ancora: Rect::new(0.0, 0.0, 0.0, 0.0),
        popup_rect: None,
        prefs_rolagem: 0.0,
        prefs_altura: 0.0,
    };
    if !urls.is_empty() {
        for u in &urls {
            nav.abrir(u);
        }
    } else if aj_reabrir && !sessao.is_empty() {
        // Só a última (a que fica à vista) carrega já; as outras, quando forem escolhidas.
        for (i, (t, u)) in sessao.iter().enumerate() {
            if i + 1 == sessao.len() {
                nav.abrir(u);
            } else {
                nav.abrir_adiada(u, t);
            }
        }
    } else {
        nav.nova_aba();
    }
    // A janela principal sem a barra de título do Glove: os botões ficam na barra da
    // Jangada (como no Safari), e sobra mais espaço para a página.
    glove_ui::use_own_frame_main();
    let opts = WindowOptions { title: "Jangada".into(), app_id: "glove.Browser".into(), size: tamanho_inicial(), fixed: false, min_size: Some((560, 380)) };
    let r = run(opts, nav);
    if !privada {
        let _ = std::fs::remove_file(&sock);
    }
    if r.is_err() { 1.into() } else { 0.into() }
}
