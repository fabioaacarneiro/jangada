//! O motor: o WebKitGTK 6.0, o da distribuição (as correções de segurança chegam pelo
//! gerenciador de pacotes). Cada aba é um `WebView` posto na janela do glove-ui, embaixo
//! do desenho da Jangada (`Ui::native_host`): a página desenha direto pela placa de vídeo
//! e recebe o mouse e o teclado sem passar pela Jangada.
//!
//! Os sinais do motor escrevem na `Pagina` da aba e pedem um quadro à janela
//! (`glove_ui::redraw_soon`); a Jangada lê tudo no quadro seguinte. Os scripts da Jangada
//! (senhas, extensões) rodam num mundo isolado ("jangada"): a página não os vê e não
//! consegue falar com a Jangada se passando por eles.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use glove_ui::gtk::{self, gdk, gio, glib, prelude::*};
use webkit6::prelude::*;
use webkit6::{
    CookieAcceptPolicy, CookiePersistentStorage, Download, FindOptions, HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision, NetworkSession, PermissionRequest, PolicyDecisionType, PrintOperation, ResponsePolicyDecision, ScriptDialog, ScriptDialogType, Settings, UserContentFilter, UserContentInjectedFrames, UserContentManager, UserStyleLevel, UserStyleSheet, WebContext, WebView,
};

/// O mundo isolado dos scripts da Jangada (e o nome do canal deles para cá).
pub const MUNDO: &str = "jangada";

/// O que uma aba sabe da página (escrito pelos sinais do motor, lido pela janela).
pub struct Pagina {
    pub view: WebView,
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub progresso: f64,
    pub back: bool,
    pub forward: bool,
    /// Menu de contexto pedido pela página (botão direito).
    pub ctx: Option<Contexto>,
    /// Diálogo do JavaScript esperando resposta: (o tipo, a mensagem, o diálogo).
    pub dialog: Option<(Dialogo, String, ScriptDialog)>,
    /// Endereço do link sob o mouse (mostrado embaixo, como nos navegadores).
    pub status: String,
    /// O ícone do site (RGBA pré-multiplicado, largura, altura) e se mudou.
    pub favicon: Option<(Vec<u8>, i32, i32)>,
    pub favicon_novo: bool,
    /// Um site não abriu: (endereço, motivo) para a Jangada mostrar a página de erro dela.
    pub erro_pedido: Option<(String, String)>,
    /// A página de erro da Jangada vai entrar (a próxima carga é ela) e está à vista (não
    /// entra no histórico).
    pub erro_alt: bool,
    pub erro_mostrando: bool,
    /// Buscar na página: (o atual, quantos achou).
    pub achados: (i32, i32),
    /// A página está em tela cheia (um vídeo): as barras somem.
    pub tela_cheia: bool,
    /// A aba mostra o código-fonte de outra (não entra na sessão nem no histórico).
    pub fonte: bool,
    /// Onde o mouse está na página (o menu de contexto abre ali).
    pub mouse: (f32, f32),
    /// A aba ainda não carregou o endereço dela: carrega quando for escolhida (as abas da
    /// última vez, reabertas por trás).
    pub adiada: bool,
    /// A página pediu para fechar a própria aba (window.close()).
    pub fechar: bool,
    /// A aba divide o processo de página com outra: ela abriu uma janela (window.open,
    /// target=_blank) ou foi aberta assim. Ao fechar, o processo dela não pode ser morto.
    pub parentes: Rc<Cell<bool>>,
    /// O que já foi notado mudar (a janela redesenha quando muda).
    pub versao: u64,
}

impl Pagina {
    fn mudou(&mut self) {
        self.versao = self.versao.wrapping_add(1);
        glove_ui::redraw_soon();
    }
}

/// O tipo de um diálogo do JavaScript à espera da pessoa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialogo {
    /// alert(): só OK.
    Alerta,
    /// confirm(): Cancelar ou OK.
    Confirma,
    /// prompt(): um texto (o que a página sugere vem aqui), Cancelar ou OK.
    Pergunta(String),
}

/// O que o botão direito pegou na página.
#[derive(Clone, Debug, Default)]
pub struct Contexto {
    pub x: i32,
    pub y: i32,
    pub link: String,
    pub selection: String,
    pub editable: bool,
    pub image: String,
}

pub type Shared = Rc<RefCell<Pagina>>;

/// Um download: o nome, onde fica, quanto chegou e o estado.
pub struct Download_ {
    pub id: u32,
    pub nome: String,
    pub caminho: PathBuf,
    pub recebido: i64,
    pub total: i64,
    pub velocidade: i64,
    pub pronto: bool,
    pub cancelado: bool,
    /// Parou por erro (sem espaço, sem permissão na pasta, a rede caiu), não por escolha.
    pub falhou: bool,
    /// Para cancelar enquanto baixa.
    pub dl: Option<Download>,
    /// Para calcular a velocidade: (quando, quanto).
    marca: (std::time::Instant, i64),
}

pub use Download_ as Baixado;

/// Um pedido de permissão de um site, esperando a escolha da pessoa.
pub struct Permissao(pub PermissionRequest);

thread_local! {
    /// O motor desta janela: a sessão (cookies, cache), os ajustes e os filtros dos escudos.
    static MOTOR: RefCell<Option<Motor>> = const { RefCell::new(None) };
    /// Abas que a página pediu (target=_blank, window.open, botão do meio): (o endereço,
    /// por trás?, a aba já criada pelo motor, se ele a criou).
    pub static NOVAS_ABAS: RefCell<Vec<(String, bool, Option<WebView>)>> = const { RefCell::new(Vec::new()) };
    /// Avisos para a barra de baixo (downloads).
    pub static AVISOS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Os downloads desta vez (a janela Downloads), do mais velho para o mais novo.
    pub static DOWNLOADS: RefCell<Vec<Baixado>> = const { RefCell::new(Vec::new()) };
    /// A página virou PDF para imprimir: o arquivo (a Jangada abre a folha Imprimir).
    pub static IMPRIMIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Mudou algo nos downloads (a janela redesenha).
    pub static DOWNLOADS_MUDOU: Cell<bool> = const { Cell::new(false) };
    /// Há uma sessão do Google aberta (os cookies da conta existem)?
    pub static GOOGLE: Cell<bool> = const { Cell::new(false) };
    /// Os pedidos de permissão à espera da pergunta (um por vez): (origem, chaves, pedido).
    pub static PERMISSOES: RefCell<Vec<(String, Vec<&'static str>, Permissao)>> = const { RefCell::new(Vec::new()) };
    /// As abas abertas (para achar a `Pagina` de uma mensagem dos scripts).
    static ABAS: RefCell<Vec<std::rc::Weak<RefCell<Pagina>>>> = const { RefCell::new(Vec::new()) };
    static PROXIMO_DOWNLOAD: Cell<u32> = const { Cell::new(1) };
}

struct Motor {
    sessao: NetworkSession,
    ajustes: Settings,
    /// Os filtros dos escudos (o formato do Safari, compilados pelo próprio WebKit).
    filtros: Vec<UserContentFilter>,
    /// O estilo do modo escuro nas páginas.
    escuro: Option<UserStyleSheet>,
}

fn com<R>(f: impl FnOnce(&Motor) -> R) -> Option<R> {
    MOTOR.with(|m| m.borrow().as_ref().map(f))
}

/// Sobe o motor: a sessão (na memória, na janela privada), os ajustes e os escudos.
pub fn iniciar(privada: bool, dados: &Path, aj: &crate::ajustes::Ajustes) {
    let sessao = if privada {
        NetworkSession::new_ephemeral()
    } else {
        let d = dados.join("WebKit");
        let c = glib::user_cache_dir().join("jangada/WebKit");
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::create_dir_all(&c);
        let s = NetworkSession::new(Some(&d.to_string_lossy()), Some(&c.to_string_lossy()));
        if let Some(cm) = s.cookie_manager() {
            cm.set_persistent_storage(&d.join("cookies.sqlite").to_string_lossy(), CookiePersistentStorage::Sqlite);
        }
        s
    };
    if let Some(dm) = sessao.website_data_manager() {
        dm.set_favicons_enabled(true);
    }
    sessao.connect_download_started(|_, d| novo_download(d));
    let ctx = WebContext::default();
    if let Some(ctx) = &ctx {
        ctx.set_cache_model(webkit6::CacheModel::WebBrowser);
        ctx.set_spell_checking_enabled(true);
        ctx.set_spell_checking_languages(&["pt_BR", "en_US"]);
    }
    let ajustes = Settings::new();
    let gpu = std::env::var_os("JANGADA_SEM_GPU").is_none() && aj.placa_de_video;
    ajustes.set_hardware_acceleration_policy(if gpu { HardwareAccelerationPolicy::Always } else { HardwareAccelerationPolicy::Never });
    ajustes.set_enable_developer_extras(aj.ferramentas_dev || std::env::var_os("WEBKIT_INSPECTOR_HTTP_SERVER").is_some());
    ajustes.set_enable_smooth_scrolling(true);
    // Dois dedos para os lados no touchpad: voltar e avançar, como no Safari.
    ajustes.set_enable_back_forward_navigation_gestures(true);
    ajustes.set_javascript_can_open_windows_automatically(false);
    ajustes.set_enable_media_stream(true);
    ajustes.set_enable_webrtc(true);
    ajustes.set_enable_mediasource(true);
    ajustes.set_enable_encrypted_media(true);
    ajustes.set_media_playback_requires_user_gesture(false);
    ajustes.set_enable_site_specific_quirks(true);
    // Os sites veem o Safari (o motor é o mesmo): mandam a versão completa deles, e os que
    // conferem a versão do navegador (WhatsApp Web, bancos) aceitam. O WebKit ainda corrige
    // por site o que algum site exigir (as "quirks" ligadas acima).
    ajustes.set_user_agent(Some(AGENTE));
    let escuro = aj.escuro_nas_paginas.then(|| UserStyleSheet::new(ESCURO, UserContentInjectedFrames::AllFrames, UserStyleLevel::User, &[], &[]));
    MOTOR.with(|m| *m.borrow_mut() = Some(Motor { sessao, ajustes, filtros: Vec::new(), escuro }));
    aplicar_cookies(crate::ajustes::COOKIES_TERCEIROS.load(std::sync::atomic::Ordering::Relaxed));
}

/// Como a Jangada se apresenta aos sites: o Safari no Linux, na versão de agora.
pub const AGENTE: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.1 Safari/605.1.15";

/// O modo escuro nas páginas: a página invertida (as imagens e vídeos voltam ao normal).
const ESCURO: &str = "html{filter:invert(0.92) hue-rotate(180deg)!important;background:#fff!important}\
img,video,picture,canvas,iframe,embed,object,svg image,[style*=\"background-image\"]{filter:invert(1) hue-rotate(180deg)!important}";

/// Barrar (ou não) os cookies de terceiros, e a prevenção de rastreamento do WebKit (a do
/// Safari: os rastreadores entre sites perdem os cookies).
pub fn aplicar_cookies(barrar: bool) {
    com(|m| {
        if let Some(cm) = m.sessao.cookie_manager() {
            cm.set_accept_policy(if barrar { CookieAcceptPolicy::NoThirdParty } else { CookieAcceptPolicy::Always });
        }
        m.sessao.set_itp_enabled(barrar);
    });
}

/// Os filtros dos escudos ficaram prontos (compilados pelo WebKit): valem nas abas.
pub fn usar_filtros(f: Vec<UserContentFilter>) {
    MOTOR.with(|m| {
        if let Some(m) = m.borrow_mut().as_mut() {
            m.filtros = f;
        }
    });
    for p in abas_abertas() {
        let (view, url) = {
            let p = p.borrow();
            (p.view.clone(), p.url.clone())
        };
        filtros_para(&view, &url);
    }
}

/// Liga ou desliga os escudos na aba, conforme o site.
fn filtros_para(view: &WebView, url: &str) {
    let Some(ucm) = view.user_content_manager() else { return };
    let vale = url.starts_with("http") && crate::escudos::valem(&crate::escudos::site(url));
    ucm.remove_all_filters();
    if vale {
        com(|m| {
            for f in &m.filtros {
                ucm.add_filter(f);
            }
        });
    }
}

fn abas_abertas() -> Vec<Shared> {
    ABAS.with(|a| {
        let mut a = a.borrow_mut();
        a.retain(|w| w.strong_count() > 0);
        a.iter().filter_map(|w| w.upgrade()).collect()
    })
}

/// O que vai em toda página de uma aba: o canal dos scripts da Jangada, o modo escuro e o
/// pedido para não rastrear.
fn conteudo(m: &Motor) -> UserContentManager {
    let ucm = UserContentManager::new();
    ucm.register_script_message_handler(MUNDO, Some(MUNDO));
    if let Some(e) = &m.escuro {
        ucm.add_style_sheet(e);
    }
    // Não Rastrear e GPC: o motor não deixa mudar os cabeçalhos dos pedidos; os sites leem
    // o pedido pela página (navigator.globalPrivacyControl e navigator.doNotTrack), que é o
    // que o GPC define para os scripts.
    if crate::ajustes::NAO_RASTREAR.load(std::sync::atomic::Ordering::Relaxed) {
        ucm.add_script(&webkit6::UserScript::new(NAO_RASTREAR, UserContentInjectedFrames::AllFrames, webkit6::UserScriptInjectionTime::Start, &[], &[]));
    }
    // O documento ficou pronto (antes das imagens e do resto): é a hora dos scripts da
    // Jangada (senhas, extensões). Esperar a página acabar de carregar demorava nas páginas
    // pesadas, e nas que nunca acabam eles nem entravam.
    // (E o aviso de que a página prendeu o mouse de fato: o pedido pode ser aceito e o
    // sistema não prender.)
    let texto = |t: &str| serde_json::to_string(t).unwrap_or_default();
    let pronto = format!(
        "window.webkit.messageHandlers.{MUNDO}.postMessage({});document.addEventListener('pointerlockchange',function(){{if(document.pointerLockElement)window.webkit.messageHandlers.{MUNDO}.postMessage({})}});",
        texto(PRONTO),
        texto(MOUSE_PRESO)
    );
    ucm.add_script(&webkit6::UserScript::for_world(&pronto, UserContentInjectedFrames::TopFrame, webkit6::UserScriptInjectionTime::End, MUNDO, &[], &[]));
    ucm
}

/// O aviso de "documento pronto" (vem do mundo isolado: a página não consegue mandá-lo).
const PRONTO: &str = "\u{1}JANGADA:pronto";
/// O aviso de que a página prendeu o mouse (pointer lock).
const MOUSE_PRESO: &str = "\u{1}JANGADA:mouse";

const NAO_RASTREAR: &str = "(function(){try{Object.defineProperty(Navigator.prototype,'globalPrivacyControl',{get:function(){return true},configurable:true});Object.defineProperty(Navigator.prototype,'doNotTrack',{get:function(){return '1'},configurable:true})}catch(e){}})();";

/// Uma aba nova. `relacionada`: a aba que pediu (window.open: a nova fica ligada a ela).
pub fn nova_aba(url: &str, relacionada: Option<&WebView>) -> Option<Shared> {
    let view = com(|m| {
        let ucm = conteudo(m);
        let mut b = WebView::builder().network_session(&m.sessao).user_content_manager(&ucm).settings(&m.ajustes);
        if let Some(r) = relacionada {
            b = b.related_view(r);
        }
        b.build()
    })?;
    Some(preparar(view, url))
}

/// Uma aba que só carrega o endereço quando for escolhida (`carregar_adiada`): reabrir
/// vinte abas da última vez não põe vinte páginas para carregar de uma vez.
pub fn nova_aba_adiada(url: &str, titulo: &str) -> Option<Shared> {
    let p = nova_aba("", None)?;
    {
        let mut p = p.borrow_mut();
        p.url = url.to_string();
        p.title = titulo.to_string();
        p.loading = false;
        p.adiada = true;
    }
    Some(p)
}

/// A aba adiada foi escolhida: carrega agora.
pub fn carregar_adiada(view: &WebView, url: &str) {
    filtros_para(view, url);
    view.load_uri(url);
}

/// Uma aba que o motor já criou (window.open): só liga os sinais.
pub fn aba_criada(view: WebView, url: &str) -> Shared {
    let p = preparar(view, url);
    p.borrow().parentes.set(true);
    p
}

fn preparar(view: WebView, url: &str) -> Shared {
    let p: Shared = Rc::new(RefCell::new(Pagina {
        view: view.clone(),
        title: String::new(),
        url: url.to_string(),
        loading: true,
        progresso: 0.0,
        back: false,
        forward: false,
        ctx: None,
        dialog: None,
        status: String::new(),
        favicon: None,
        favicon_novo: false,
        erro_pedido: None,
        erro_alt: false,
        erro_mostrando: false,
        achados: (0, 0),
        tela_cheia: false,
        fonte: false,
        mouse: (0.0, 0.0),
        adiada: false,
        fechar: false,
        parentes: Rc::new(Cell::new(false)),
        versao: 0,
    }));
    ABAS.with(|a| a.borrow_mut().push(Rc::downgrade(&p)));
    view.set_hexpand(true);
    view.set_vexpand(true);
    ligar(&p, &view);
    if !url.is_empty() {
        filtros_para(&view, url);
        view.load_uri(url);
    }
    p
}

/// Os sinais do motor para a aba.
fn ligar(p: &Shared, view: &WebView) {
    let fraco = Rc::downgrade(p);
    // Mexe na página (se ela ainda existe e não está ocupada) e pede um quadro.
    let com_pagina = {
        let fraco = fraco.clone();
        move |f: &dyn Fn(&mut Pagina)| {
            if let Some(p) = fraco.upgrade() {
                if let Ok(mut p) = p.try_borrow_mut() {
                    f(&mut p);
                    p.mudou();
                }
            }
        }
    };
    {
        let c = com_pagina.clone();
        view.connect_title_notify(move |v| {
            let t = v.title().map(|t| t.to_string()).unwrap_or_default();
            c(&|p| {
                p.title = t.clone();
                if !p.fonte && p.erro_pedido.is_none() && !p.erro_mostrando {
                    crate::ajustes::visitar(&p.title, &p.url);
                }
            });
        });
    }
    {
        let c = com_pagina.clone();
        view.connect_uri_notify(move |v| {
            let u = v.uri().map(|u| u.to_string()).unwrap_or_default();
            let (b, f) = (v.can_go_back(), v.can_go_forward());
            c(&|p| {
                if !p.fonte {
                    p.url = u.clone();
                }
                p.back = b;
                p.forward = f;
            });
        });
    }
    {
        let c = com_pagina.clone();
        view.connect_is_loading_notify(move |v| {
            let l = v.is_loading();
            let (b, f) = (v.can_go_back(), v.can_go_forward());
            c(&|p| {
                p.loading = l;
                p.back = b;
                p.forward = f;
            });
        });
    }
    {
        let c = com_pagina.clone();
        view.connect_estimated_load_progress_notify(move |v| {
            let e = v.estimated_load_progress();
            c(&|p| p.progresso = e);
        });
    }
    // O ícone do site.
    {
        let c = com_pagina.clone();
        view.connect_favicon_notify(move |v| {
            let Some(t) = v.favicon() else { return };
            let (w, h) = (t.width(), t.height());
            if w <= 0 || h <= 0 || w > 512 || h > 512 {
                return;
            }
            let mut buf = vec![0u8; (w * h * 4) as usize];
            t.download(&mut buf, (w * 4) as usize);
            // O GTK entrega BGRA pré-multiplicado (na ordem da memória): vira RGBA.
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            c(&|p| {
                p.favicon = Some((buf.clone(), w, h));
                p.favicon_novo = true;
            });
        });
    }
    // O link sob o mouse.
    {
        let c = com_pagina.clone();
        view.connect_mouse_target_changed(move |_, hit, _| {
            let s = if hit.context_is_link() { hit.link_uri().map(|u| u.to_string()).unwrap_or_default() } else { String::new() };
            c(&|p| {
                if p.status != s {
                    p.status = s.clone();
                }
            });
        });
    }
    // Onde o mouse está (o menu de contexto abre ali).
    {
        let m = gtk::EventControllerMotion::new();
        // Antes do motor (ele fica com os eventos dele).
        m.set_propagation_phase(gtk::PropagationPhase::Capture);
        let fraco = fraco.clone();
        let anota = move |x: f64, y: f64| {
            if let Some(p) = fraco.upgrade() {
                if let Ok(mut p) = p.try_borrow_mut() {
                    p.mouse = (x as f32, y as f32);
                }
            }
        };
        // Ao entrar também: com o mouse parado sobre uma aba que acabou de aparecer (ou
        // vindo das barras num pulo só) não há "mexeu", e o menu abria no canto da página.
        let anota2 = anota.clone();
        m.connect_enter(move |_, x, y| anota2(x, y));
        m.connect_motion(move |_, x, y| anota(x, y));
        view.add_controller(m);
    }
    // O menu de contexto: o da Jangada (o do motor não aparece).
    {
        let fraco = fraco.clone();
        view.connect_context_menu(move |v, _, hit| {
            let Some(p) = fraco.upgrade() else { return true };
            let (x, y) = p.borrow().mouse;
            let mut c = Contexto {
                x: x.round() as i32,
                y: y.round() as i32,
                link: if hit.context_is_link() { hit.link_uri().map(|u| u.to_string()).unwrap_or_default() } else { String::new() },
                selection: String::new(),
                editable: hit.context_is_editable(),
                image: if hit.context_is_image() { hit.image_uri().map(|u| u.to_string()).unwrap_or_default() } else { String::new() },
            };
            if hit.context_is_selection() {
                // O texto escolhido vem do script (o motor só diz que há seleção).
                let fraco = Rc::downgrade(&p);
                v.evaluate_javascript("String(window.getSelection())", Some(MUNDO), None, None::<&gio::Cancellable>, move |r| {
                    c.selection = r.ok().map(|v| v.to_str().to_string()).unwrap_or_default();
                    if let Some(p) = fraco.upgrade() {
                        if let Ok(mut p) = p.try_borrow_mut() {
                            p.ctx = Some(c.clone());
                            p.mudou();
                        }
                    }
                });
            } else if let Ok(mut p) = p.try_borrow_mut() {
                p.ctx = Some(c);
                p.mudou();
            }
            true
        });
    }
    // alert() e confirm(): alertas do Glove (janela própria); prompt() é recusado.
    {
        let fraco = fraco.clone();
        view.connect_script_dialog(move |_, d| {
            let t = d.dialog_type();
            // Este é respondido na hora. Nada de `close()` aqui dentro: o motor solta o
            // diálogo sozinho quando o sinal volta, e fechá-lo antes derrubava a Jangada
            // inteira (qualquer página com aviso de sair).
            if t == ScriptDialogType::BeforeUnloadConfirm {
                // Sair da página sempre pode (como fechar a aba).
                d.confirm_set_confirmed(true);
                return true;
            }
            // Os outros esperam a pessoa (o diálogo fica guardado: a página espera).
            let tipo = match t {
                ScriptDialogType::Confirm => Dialogo::Confirma,
                ScriptDialogType::Prompt => Dialogo::Pergunta(d.prompt_get_default_text().map(|t| t.to_string()).unwrap_or_default()),
                _ => Dialogo::Alerta,
            };
            let Some(p) = fraco.upgrade() else { return false };
            let Ok(mut p) = p.try_borrow_mut() else { return false };
            p.dialog = Some((tipo, d.message().map(|m| m.to_string()).unwrap_or_default(), d.clone()));
            p.mudou();
            true
        });
    }
    // Permissões: o que foi guardado para o site, ou a pergunta da Jangada.
    {
        let v = view.clone();
        view.connect_permission_request(move |_, req| pedido_de_permissao(&v, req));
    }
    // window.close(): a página pediu para fechar a própria aba (o motor só deixa as
    // que a página abriu, como as janelinhas de login).
    {
        let c = com_pagina.clone();
        view.connect_close(move |_| c(&|p| p.fechar = true));
    }
    // Janela nova pedida pela página (window.open, target=_blank): vira uma aba.
    {
        let parentes = p.borrow().parentes.clone();
        view.connect_create(move |v, acao| {
            let url = acao.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
            let sozinha = !acao.is_user_gesture() && crate::ajustes::BLOQUEAR_JANELAS.load(std::sync::atomic::Ordering::Relaxed);
            if sozinha {
                AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Janela que a página abriu sozinha foi bloqueada").to_string()));
                glove_ui::redraw_soon();
                return None;
            }
            let nova = com(|m| {
                let ucm = conteudo(m);
                WebView::builder().related_view(v).user_content_manager(&ucm).settings(&m.ajustes).build()
            })?;
            // A nova divide o processo com esta.
            parentes.set(true);
            let fundo = acao.mouse_button() == 2;
            NOVAS_ABAS.with(|n| n.borrow_mut().push((url, fundo, Some(nova.clone()))));
            glove_ui::redraw_soon();
            Some(nova.upcast())
        });
    }
    // Botão do meio ou Ctrl + clique num link: aba nova por trás.
    {
        view.connect_decide_policy(move |_, d, tipo| {
            // A resposta chegou e o motor não sabe mostrar o tipo (um .zip, um .deb, um
            // application/octet-stream): é um download. Sem isto o motor só baixava o que
            // viesse marcado como anexo, e o resto era ignorado sem aviso.
            if tipo == PolicyDecisionType::Response {
                if d.downcast_ref::<ResponsePolicyDecision>().is_some_and(|r| !r.is_mime_type_supported()) {
                    d.download();
                    return true;
                }
                return false;
            }
            if tipo != PolicyDecisionType::NavigationAction && tipo != PolicyDecisionType::NewWindowAction {
                return false;
            }
            let Some(n) = d.downcast_ref::<NavigationPolicyDecision>() else { return false };
            let Some(acao) = n.navigation_action() else { return false };
            let url = acao.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
            let ctrl = gdk::ModifierType::from_bits_truncate(acao.modifiers()).contains(gdk::ModifierType::CONTROL_MASK);
            // O motor manda o botão do meio e o Ctrl + clique como "janela nova".
            if (acao.mouse_button() == 2 || ctrl) && acao.navigation_type() == webkit6::NavigationType::LinkClicked && !url.is_empty() {
                d.ignore();
                NOVAS_ABAS.with(|n| n.borrow_mut().push((url, true, None)));
                glove_ui::redraw_soon();
                return true;
            }
            false
        });
    }
    // A página começou a carregar: os escudos do site e o estado da aba. (Os scripts da
    // Jangada entram quando o documento fica pronto: veja `mensagem`.)
    {
        let c = com_pagina.clone();
        view.connect_load_changed(move |v, ev| {
            if ev == LoadEvent::Started {
                // Os escudos do site (ligados ou não nele) antes de a página pedir o resto.
                filtros_para(v, &v.uri().map(|u| u.to_string()).unwrap_or_default());
                c(&|p| {
                    p.erro_mostrando = std::mem::take(&mut p.erro_alt);
                    p.erro_pedido = None;
                    p.status.clear();
                });
            }
        });
    }
    // Um site não abriu: a página de erro da Jangada no lugar da do motor.
    {
        let c = com_pagina.clone();
        view.connect_load_failed(move |_, _, uri, err| {
            // Parar o carregamento (ou trocar de página no meio) não é erro; nem o
            // download que o motor assumiu.
            if err.matches(webkit6::NetworkError::Cancelled) || err.matches(webkit6::PolicyError::FrameLoadInterruptedByPolicyChange) {
                return false;
            }
            let (u, m) = (uri.to_string(), err.message().to_string());
            c(&|p| p.erro_pedido = Some((u.clone(), m.clone())));
            true
        });
    }
    // Tela cheia (um vídeo): as barras da Jangada somem.
    {
        let c = com_pagina.clone();
        view.connect_enter_fullscreen(move |_| {
            c(&|p| p.tela_cheia = true);
            false
        });
    }
    {
        let c = com_pagina.clone();
        view.connect_leave_fullscreen(move |_| {
            c(&|p| p.tela_cheia = false);
            false
        });
    }
    // Notificações dos sites (já permitidas): no avisador do sistema.
    view.connect_show_notification(|_, n| {
        let titulo = n.title().map(|t| t.to_string()).unwrap_or_default();
        let corpo = n.body().map(|t| t.to_string()).unwrap_or_default();
        std::thread::spawn(move || notificar(&titulo, &corpo));
        true
    });
    // O processo da página caiu (memória, falha): recarrega.
    {
        let c = com_pagina.clone();
        view.connect_web_process_terminated(move |v, _| {
            let u = v.uri().map(|u| u.to_string()).unwrap_or_default();
            c(&|p| p.erro_pedido = Some((u.clone(), glove_config::tr!("A página parou de responder.").to_string())));
        });
    }
    // Buscar na página.
    if let Some(fc) = view.find_controller() {
        let c = com_pagina.clone();
        fc.connect_counted_matches(move |_, n| c(&|p| p.achados.1 = n as i32));
        let c = com_pagina.clone();
        fc.connect_found_text(move |_, n| {
            c(&|p| {
                p.achados.1 = n as i32;
                if p.achados.0 == 0 {
                    p.achados.0 = 1;
                }
            })
        });
        let c = com_pagina.clone();
        fc.connect_failed_to_find_text(move |_| c(&|p| p.achados = (0, 0)));
    }
    // As mensagens dos scripts da Jangada (o mundo isolado).
    if let Some(ucm) = view.user_content_manager() {
        let v = view.clone();
        ucm.connect_script_message_received(Some(MUNDO), move |_, valor| mensagem(&v, &valor.to_str()));
    }
}

/// Uma mensagem de um script da Jangada na página (senhas, extensões).
fn mensagem(view: &WebView, m: &str) {
    let url = view.uri().map(|u| u.to_string()).unwrap_or_default();
    if m == PRONTO {
        // Um por vez: um erro de escrita no script de uma extensão não leva os outros.
        for js in crate::senhas::script_da_pagina(&url).into_iter().chain(crate::extensoes::scripts_para(&url)) {
            view.evaluate_javascript(&js, Some(MUNDO), Some(&url), None::<&gio::Cancellable>, |_| {});
        }
        return;
    }
    if m == MOUSE_PRESO {
        AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("A página prendeu o mouse: Esc solta.").to_string()));
        glove_ui::redraw_soon();
        return;
    }
    if let Some(resto) = m.strip_prefix(crate::extensoes::MARCA) {
        if let Ok(pedido) = serde_json::from_str::<serde_json::Value>(resto) {
            let id = pedido["ext"].as_str().unwrap_or("").to_string();
            let op = pedido["op"].as_str().unwrap_or("").to_string();
            let k = pedido["k"].clone();
            let resposta = crate::extensoes::armazenar(&id, &op, &pedido["d"]);
            let js = format!("window.__jangadaResposta&&window.__jangadaResposta({k},{resposta});");
            view.evaluate_javascript(&js, Some(MUNDO), None, None::<&gio::Cancellable>, |_| {});
        }
        return;
    }
    if let Some(pedido) = crate::senhas::ler(m) {
        let origem = crate::importar::origem(&url);
        // O chaveiro responde por outra linha de execução: se ele demora (trancado, à
        // espera da senha da pessoa), a Jangada não congela.
        match pedido {
            crate::senhas::Pedido::Preencher => {
                let v = view.clone();
                glib::spawn_future_local(async move {
                    let o = origem.clone();
                    let Ok(Some(js)) = gio::spawn_blocking(move || crate::senhas::script_preencher(&o)).await else { return };
                    // A página pode ter ido para outro site enquanto o chaveiro respondia.
                    let agora = crate::importar::origem(&v.uri().map(|u| u.to_string()).unwrap_or_default());
                    if agora == origem {
                        v.evaluate_javascript(&js, Some(MUNDO), None, None::<&gio::Cancellable>, |_| {});
                    }
                });
            }
            crate::senhas::Pedido::Guardar(u, s) => {
                glib::spawn_future_local(async move {
                    let (o, u2, s2) = (origem.clone(), u.clone(), s.clone());
                    if let Ok(true) = gio::spawn_blocking(move || crate::senhas::e_nova(&o, &u2, &s2)).await {
                        crate::senhas::oferecer(&origem, &u, &s);
                        glove_ui::redraw_soon();
                    }
                });
            }
        }
    }
}

/// As chaves das permissões (guardadas em permissoes.tsv): camera, microfone,
/// localizacao, notificacoes, transferencia.
fn pedido_de_permissao(view: &WebView, req: &PermissionRequest) -> bool {
    let origem = crate::importar::origem(&view.uri().map(|u| u.to_string()).unwrap_or_default());
    let mut nomes: Vec<&'static str> = Vec::new();
    if let Some(m) = req.downcast_ref::<webkit6::UserMediaPermissionRequest>() {
        if webkit6::functions::user_media_permission_is_for_audio_device(m) {
            nomes.push("microfone");
        }
        if webkit6::functions::user_media_permission_is_for_video_device(m) {
            nomes.push("camera");
        }
        // Captura de tela: a Jangada não concede.
        if webkit6::functions::user_media_permission_is_for_display_device(m) {
            req.deny();
            return true;
        }
    } else if req.is::<webkit6::GeolocationPermissionRequest>() {
        nomes.push("localizacao");
    } else if req.is::<webkit6::NotificationPermissionRequest>() {
        nomes.push("notificacoes");
    } else if req.type_().name() == "WebKitClipboardPermissionRequest" {
        // Ler o que foi copiado (o botão "colar" de um site): a pessoa decide. (Pelo nome
        // do tipo: ele só existe no WebKitGTK 2.42 em diante, e a Jangada roda nos de antes.)
        nomes.push("transferencia");
    } else if req.is::<webkit6::PointerLockPermissionRequest>() {
        // Prender o mouse (jogos, visualizadores 3D): pode, e o Esc solta. (O aviso sai
        // quando o mouse fica preso de verdade: veja `mensagem`.)
        req.allow();
        return true;
    } else {
        // O resto (armazenamento entre sites, mídia protegida etc.): o padrão do motor.
        return false;
    }
    if nomes.is_empty() {
        req.deny();
        return true;
    }
    let liberado = |k: &str| crate::ajustes::permissao(&origem, k);
    if nomes.iter().all(|k| liberado(k) == Some(true)) {
        req.allow();
    } else if nomes.iter().any(|k| liberado(k) == Some(false)) {
        req.deny();
    } else {
        PERMISSOES.with(|p| p.borrow_mut().push((origem, nomes, Permissao(req.clone()))));
        glove_ui::redraw_soon();
    }
    true
}

/// O nome de uma permissão numa frase: com o artigo ("a câmera", "o microfone").
pub fn rotulo_permissao(chave: &str) -> &'static str {
    match chave {
        "camera" => "a câmera",
        "microfone" => "o microfone",
        "localizacao" => "a localização",
        "transferencia" => "a área de transferência",
        _ => "as notificações",
    }
}

/// Uma notificação de um site, no avisador do sistema (org.freedesktop.Notifications).
fn notificar(titulo: &str, corpo: &str) {
    let Ok(c) = zbus::blocking::Connection::session() else { return };
    let dicas: std::collections::HashMap<&str, zbus::zvariant::Value> = std::collections::HashMap::new();
    let _ = c.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("Jangada", 0u32, "jangada", titulo, corpo, Vec::<&str>::new(), dicas, -1i32),
    );
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

fn novo_download(d: &Download) {
    let id = PROXIMO_DOWNLOAD.with(|n| n.replace(n.get() + 1));
    DOWNLOADS.with(|l| l.borrow_mut().push(Baixado { id, nome: String::new(), caminho: PathBuf::new(), recebido: 0, total: 0, velocidade: 0, pronto: false, cancelado: false, falhou: false, dl: Some(d.clone()), marca: (std::time::Instant::now(), 0) }));
    // Vai direto para a pasta Downloads (sem perguntar), com um nome livre.
    d.connect_decide_destination(move |d, sugerido| {
        let nome = if sugerido.is_empty() { "download".to_string() } else { sugerido.replace('/', "_") };
        // A pasta Downloads pode não existir ainda (conta nova): sem ela, o motor
        // desistia do download na hora.
        let pasta = crate::pasta_downloads();
        let _ = std::fs::create_dir_all(&pasta);
        let path = livre(&pasta.join(&nome));
        d.set_destination(&path.to_string_lossy());
        let n = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(nome);
        atualizar(id, |x| {
            x.nome = n.clone();
            x.caminho = path.clone();
        });
        AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Baixando {}…", n)));
        true
    });
    d.connect_received_data(move |d, _| {
        let recebido = d.received_data_length() as i64;
        let total = d.response().map(|r| r.content_length() as i64).unwrap_or(0);
        atualizar(id, |x| {
            x.recebido = recebido;
            x.total = total;
            let dt = x.marca.0.elapsed().as_secs_f64();
            if dt >= 0.5 {
                x.velocidade = ((recebido - x.marca.1) as f64 / dt) as i64;
                x.marca = (std::time::Instant::now(), recebido);
            }
        });
    });
    d.connect_finished(move |d| {
        if std::env::var_os("JANGADA_DEBUG").is_some() {
            eprintln!("download pronto: {:?}", d.destination());
        }
        let mut nome = String::new();
        atualizar(id, |x| {
            if !x.cancelado {
                x.pronto = true;
                x.recebido = x.recebido.max(x.total);
                nome = x.nome.clone();
            }
            x.dl = None;
        });
        if !nome.is_empty() {
            AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Baixado em Downloads: {}", nome)));
        }
    });
    d.connect_failed(move |_, _| {
        // Cancelar também chega aqui: só é falha se ninguém cancelou.
        let mut falhou = None;
        atualizar(id, |x| {
            if !x.cancelado {
                x.falhou = true;
                falhou = Some(x.nome.clone());
            }
            x.cancelado = true;
            x.dl = None;
        });
        if let Some(nome) = falhou {
            AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Não foi possível baixar {}", if nome.is_empty() { "download".to_string() } else { nome })));
        }
    });
}

fn atualizar(id: u32, f: impl FnOnce(&mut Baixado)) {
    DOWNLOADS.with(|l| {
        if let Some(x) = l.borrow_mut().iter_mut().find(|x| x.id == id) {
            f(x);
        }
    });
    DOWNLOADS_MUDOU.with(|m| m.set(true));
    glove_ui::redraw_soon();
}

/// Cancela um download em andamento.
pub fn cancelar_download(id: u32) {
    let dl = DOWNLOADS.with(|l| l.borrow_mut().iter_mut().find(|x| x.id == id).and_then(|x| {
        x.cancelado = true;
        x.dl.take()
    }));
    if let Some(d) = dl {
        d.cancel();
    }
    DOWNLOADS_MUDOU.with(|m| m.set(true));
}

/// Um nome que ainda não existe ("arquivo (2).zip").
fn livre(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..1000).map(|n| path.with_file_name(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or_else(|| path.to_path_buf())
}

// ---------------------------------------------------------------------------
// PDF e impressão
// ---------------------------------------------------------------------------

/// A impressora "Imprimir para Arquivo" do GTK (o nome dela vem traduzido).
fn para_arquivo(view: &WebView, destino: &Path, imprimir: bool) {
    let op = PrintOperation::new(view);
    let s = gtk::PrintSettings::new();
    let nome = glib::dgettext(Some("gtk40"), "Print to File");
    s.set_printer(&nome);
    s.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
    s.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(&glib::filename_to_uri(destino, None).map(|u| u.to_string()).unwrap_or_default()));
    s.set_paper_size(&gtk::PaperSize::new(Some(gtk::PAPER_NAME_A4)));
    s.set_bool("print-backgrounds", true);
    op.set_print_settings(&s);
    let pagina = gtk::PageSetup::new();
    pagina.set_paper_size(&gtk::PaperSize::new(Some(gtk::PAPER_NAME_A4)));
    op.set_page_setup(&pagina);
    let caminho = destino.to_path_buf();
    op.connect_finished(move |_| {
        if imprimir {
            IMPRIMIR.with(|i| *i.borrow_mut() = Some(caminho.clone()));
        } else {
            let nome = caminho.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("PDF salvo em Downloads: {}", nome)));
        }
        glove_ui::redraw_soon();
    });
    op.connect_failed(move |_, _| {
        AVISOS.with(|a| a.borrow_mut().push(if imprimir { glove_config::tr!("Não foi possível preparar a impressão.").to_string() } else { glove_config::tr!("Não foi possível salvar o PDF.").to_string() }));
        glove_ui::redraw_soon();
    });
    op.print();
}

/// Salva a página como PDF na pasta Downloads.
pub fn salvar_pdf(view: &WebView, titulo: &str) {
    let nome: String = titulo.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let nome = if nome.trim().is_empty() { "pagina".to_string() } else { nome.trim().chars().take(80).collect() };
    let path = livre(&crate::pasta_downloads().join(format!("{nome}.pdf")));
    para_arquivo(view, &path, false);
}

/// Imprimir: a página vira um PDF (A4, com os fundos) e a folha Imprimir do Glove o
/// manda para a impressora (CUPS).
pub fn imprimir(view: &WebView, pasta: &Path) {
    let path = pasta.join("imprimir.pdf");
    let _ = std::fs::remove_file(&path);
    para_arquivo(view, &path, true);
}

// ---------------------------------------------------------------------------
// Cookies, dados e Google
// ---------------------------------------------------------------------------

/// Grava cookies importados de outro navegador (as sessões abertas nos sites).
pub fn gravar_cookies(v: &[crate::importar::Biscoito]) -> usize {
    let Some(cm) = com(|m| m.sessao.cookie_manager()).flatten() else { return 0 };
    let mut n = 0;
    for b in v {
        // Microssegundos desde 1601 → segundos a partir de agora (0: da sessão).
        let agora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let expira = if b.expira > 0 { b.expira / 1_000_000 - 11_644_473_600 } else { 0 };
        let idade = if b.expira > 0 { (expira - agora).max(0) as i32 } else { -1 };
        let mut c = webkit6::soup::Cookie::new(&b.nome, &b.valor, &b.host, if b.caminho.is_empty() { "/" } else { &b.caminho }, idade);
        c.set_secure(b.seguro);
        c.set_http_only(b.http_only);
        c.set_same_site_policy(match b.same_site {
            0 => webkit6::soup::SameSitePolicy::None,
            2 => webkit6::soup::SameSitePolicy::Strict,
            _ => webkit6::soup::SameSitePolicy::Lax,
        });
        cm.add_cookie(&c, None::<&gio::Cancellable>, |_| {});
        n += 1;
    }
    n
}

/// Confere (sem bloquear) se há uma conta do Google com sessão aberta: os cookies de
/// sessão do Google existem. A resposta chega em `GOOGLE` logo depois.
pub fn conferir_google() {
    let Some(cm) = com(|m| m.sessao.cookie_manager()).flatten() else { return };
    cm.cookies("https://accounts.google.com/", None::<&gio::Cancellable>, |r| {
        let tem = r.is_ok_and(|mut v| v.iter_mut().any(|c| matches!(c.name().as_deref(), Some("SID" | "__Secure-1PSID" | "SAPISID" | "__Secure-3PSID"))));
        GOOGLE.with(|g| g.set(tem));
        glove_ui::redraw_soon();
    });
}

/// Apaga os cookies e os dados dos sites (Preferências ▸ Privacidade).
pub fn apagar_dados() {
    let Some(dm) = com(|m| m.sessao.website_data_manager()).flatten() else { return };
    dm.clear(webkit6::WebsiteDataTypes::ALL, glib::TimeSpan::from_seconds(0), None::<&gio::Cancellable>, |_| {});
}

/// Fecha a página de uma aba que já saiu da janela. O processo de página de uma aba
/// comum é só dela e é encerrado na hora (a memória volta já; sem isso o motor o deixa
/// guardado por um bom tempo). `divide_o_processo`: a aba abriu ou foi aberta por outra
/// (window.open, login em janelinha) e as duas usam o mesmo processo: matá-lo derrubava a
/// outra, então aqui a página só é fechada, e o motor solta o processo quando ninguém
/// mais o usa.
pub fn fechar_pagina(view: &WebView, divide_o_processo: bool) {
    if !divide_o_processo {
        view.terminate_web_process();
    }
    // Segurança: a aba saiu da janela e ninguém a usa depois daqui (os sinais dela são
    // desligados aqui mesmo; a Jangada só solta a referência em seguida).
    unsafe { view.run_dispose() };
}

/// Liga ou desliga o inspetor do motor (as Ferramentas do Desenvolvedor).
pub fn ferramentas_dev(ligadas: bool) {
    com(|m| m.ajustes.set_enable_developer_extras(ligadas));
}

/// As Ferramentas do Desenvolvedor da aba (o inspetor do WebKit, numa janela ao lado).
pub fn inspecionar(view: &WebView) {
    if let Some(i) = view.inspector() {
        i.show();
    }
}

/// Buscar na página (texto vazio: para de buscar).
pub fn buscar(view: &WebView, texto: &str, voltar: bool) {
    let Some(fc) = view.find_controller() else { return };
    if texto.is_empty() {
        fc.search_finish();
        return;
    }
    let op = FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND;
    if fc.search_text().as_deref() == Some(texto) {
        if voltar { fc.search_previous() } else { fc.search_next() }
    } else {
        fc.search(texto, op.bits(), 10_000);
        fc.count_matches(texto, op.bits(), 10_000);
    }
}

/// O código-fonte da página numa aba nova (o texto da página, sem estilo).
pub fn codigo_fonte(view: &WebView, destino: Shared) {
    let Some(r) = view.main_resource() else { return };
    let url = view.uri().map(|u| u.to_string()).unwrap_or_default();
    r.data(None::<&gio::Cancellable>, move |d| {
        let texto = d.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let esc = texto.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let titulo = glove_config::tr!("Código-fonte de {}", url).replace('<', "&lt;");
        let html = format!("<!doctype html><meta charset=utf-8><title>{titulo}</title><style>body{{margin:0;background:#fff;color:#1d1d1f}}pre{{margin:0;padding:12px 16px;font:12px/1.5 monospace;white-space:pre-wrap;word-break:break-all}}@media(prefers-color-scheme:dark){{body{{background:#1e1e1e;color:#ddd}}}}</style><pre>{esc}</pre>");
        // O empréstimo acaba antes de falar com o motor: ele avisa na hora (carregando,
        // endereço), e com a página ainda emprestada esses avisos se perdiam.
        let v = {
            let mut p = destino.borrow_mut();
            p.fonte = true;
            p.url = format!("view-source:{url}");
            p.mudou();
            p.view.clone()
        };
        v.load_html(&html, None);
    });
}
