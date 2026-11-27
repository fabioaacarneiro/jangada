//! O motor: o WebKitGTK 6.0, o da distribuição (as correções de segurança chegam pelo
//! gerenciador de pacotes). Cada aba é um `WebView` posto na janela do glove-ui, embaixo
//! do desenho da Jangada (`Ui::native_host`): a página desenha direto pela placa de vídeo
//! e recebe o mouse e o teclado sem passar pela Jangada.
//!
//! Os sinais do motor escrevem na `Pagina` da aba e pedem um quadro à janela
//! (`glove_ui::redraw_soon`); a Jangada lê tudo no quadro seguinte. Os scripts da Jangada
//! (senhas) rodam num mundo isolado ("jangada"), e os de cada extensão num mundo só dela:
//! a página não os vê e não consegue falar com a Jangada se passando por eles.
//!
//! As páginas das extensões (a janelinha do botão, as opções) vêm do esquema
//! `jangada-extension://`, servido aqui direto da pasta da extensão.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use glove_ui::gtk::{self, gdk, gio, glib, prelude::*};
use webkit6::prelude::*;
use webkit6::{
    CookieAcceptPolicy, CookiePersistentStorage, Download, FindOptions, HardwareAccelerationPolicy, LoadEvent, NavigationPolicyDecision, NetworkSession, PermissionRequest, PolicyDecisionType, PrintOperation, ResponsePolicyDecision, ScriptDialog, ScriptDialogType, Settings, URISchemeRequest, URISchemeResponse, UserContentFilter, UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime, UserStyleLevel, UserStyleSheet, WebContext, WebView,
};

use crate::extensoes::{self, Extensao, CANAL_PAGINAS, ESQUEMA};
use crate::ponte;

/// O mundo isolado dos scripts da Jangada (e o nome do canal deles para cá).
pub const MUNDO: &str = "jangada";

/// O que uma aba sabe da página (escrito pelos sinais do motor, lido pela janela).
pub struct Pagina {
    /// O número da aba (o `tabs.Tab.id` das extensões): não muda enquanto ela existir.
    pub id: u32,
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
    /// As extensões com scripts de conteúdo na página de agora.
    pub extensoes: Vec<String>,
    /// As extensões cuja API já está no conteúdo da aba (para as páginas delas, como as
    /// opções), e se o canal das páginas de extensão já existe nela.
    pub apis_ext: Vec<String>,
    pub canal_ext: bool,
    /// O CORS livre na aba vale para as páginas desta extensão (só enquanto a aba mostrar
    /// uma página dela).
    pub cors_ext: Option<String>,
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
    /// O endereço do arquivo (para baixar de novo).
    pub uri: String,
    /// De onde veio (o site).
    pub origem: String,
    /// Quando começou (segundos desde 1970).
    pub quando: i64,
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
    /// Imagens pedidas por "Copiar Imagem": baixadas para um arquivo temporário e postas
    /// na área de transferência (não entram na lista de downloads).
    static COPIAR_IMAGEM: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Os scripts `document_start` das extensões postos em cada aba (para tirar e pôr de
    /// novo quando as extensões mudam).
    static INICIO: RefCell<Vec<(glib::WeakRef<WebView>, Vec<webkit6::UserScript>)>> = const { RefCell::new(Vec::new()) };
    /// A página virou PDF para imprimir: o arquivo (a Jangada abre a folha Imprimir).
    pub static IMPRIMIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Mudou algo nos downloads (a janela redesenha).
    pub static DOWNLOADS_MUDOU: Cell<bool> = const { Cell::new(false) };
    /// Um link magnet ou um .torrent foi para o ajudante: a janela abre a aba Torrents.
    pub static ABRIR_TORRENTS: Cell<bool> = const { Cell::new(false) };
    /// Há uma sessão do Google aberta (os cookies da conta existem)?
    pub static GOOGLE: Cell<bool> = const { Cell::new(false) };
    /// Os pedidos de permissão à espera da pergunta (um por vez): (origem, chaves, pedido).
    pub static PERMISSOES: RefCell<Vec<(String, Vec<&'static str>, Permissao)>> = const { RefCell::new(Vec::new()) };
    /// As abas abertas (para achar a `Pagina` de uma mensagem dos scripts).
    static ABAS: RefCell<Vec<std::rc::Weak<RefCell<Pagina>>>> = const { RefCell::new(Vec::new()) };
    static PROXIMO_DOWNLOAD: Cell<u32> = const { Cell::new(1) };
    static PROXIMA_ABA: Cell<u32> = const { Cell::new(1) };
    /// As abas na ordem da faixa e a escolhida (a janela atualiza; as extensões consultam).
    static ORDEM: RefCell<(Vec<u32>, u32)> = const { RefCell::new((Vec::new(), 0)) };
    /// A pessoa clicou numa página (a janelinha de extensão aberta fecha).
    pub static CLIQUE_NA_PAGINA: Cell<bool> = const { Cell::new(false) };
    /// Endereços de páginas de extensão que a própria Jangada mandou abrir (a barra, as
    /// abas da última vez, as extensões): só esses entram numa aba que mostra outro site.
    static PERMITIDAS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// As abas na ordem da faixa e a escolhida.
pub fn ordem() -> (Vec<u32>, u32) {
    ORDEM.with(|o| o.borrow().clone())
}

/// A janela diz a ordem das abas e a escolhida.
pub fn definir_ordem(ids: Vec<u32>, ativa: u32) {
    ORDEM.with(|o| {
        let mut o = o.borrow_mut();
        if o.0 != ids || o.1 != ativa {
            *o = (ids, ativa);
        }
    });
}

/// A Jangada vai abrir este endereço (de uma extensão): pode entrar mesmo vindo de outro
/// site.
pub fn permitir_navegacao(url: &str) {
    if url.starts_with(ESQUEMA) {
        PERMITIDAS.with(|p| {
            let mut p = p.borrow_mut();
            p.push(url.to_string());
            if p.len() > 16 {
                p.remove(0);
            }
        });
    }
}

/// Uma página pode ir para este endereço de extensão? Se ela já é da mesma extensão, se
/// a Jangada mandou, se é ir e voltar no histórico, ou se o arquivo é acessível pela web.
fn pode_ir_para_extensao(atual: &str, alvo: &str, historico: bool) -> bool {
    let Some((id, caminho)) = extensoes::partes_da_url(alvo) else { return true };
    let Some(e) = extensoes::ativa(&id) else { return false };
    if extensoes::partes_da_url(atual).is_some_and(|(a, _)| a == id) || historico {
        return true;
    }
    let mandou = PERMITIDAS.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|u| u == alvo).map(|i| p.remove(i)).is_some()
    });
    mandou || e.acessivel(&caminho)
}

/// Serve um arquivo de extensão (jangada-extension://ID/caminho). Uma página da própria
/// extensão pede o que quiser da pasta dela; uma página da web, só o que a extensão
/// declarou acessível (`web_accessible_resources`).
fn servir_extensao(req: &URISchemeRequest) {
    let falhar = |req: &URISchemeRequest, msg: &str| {
        let mut e = glib::Error::new(gio::IOErrorEnum::NotFound, msg);
        req.finish_error(&mut e);
    };
    let url = req.uri().map(|u| u.to_string()).unwrap_or_default();
    let Some((id, caminho)) = extensoes::partes_da_url(&url) else { return falhar(req, "endereço inválido") };
    let Some(e) = extensoes::ativa(&id) else { return falhar(req, "extensão não instalada ou desligada") };
    let topo = req.web_view().and_then(|v| v.uri()).map(|u| u.to_string()).unwrap_or_default();
    let da_mesma = extensoes::partes_da_url(&topo).is_some_and(|(t, _)| t == id);
    if !da_mesma && !e.acessivel(&caminho) {
        return falhar(req, "arquivo não acessível pela web");
    }
    let Some(arq) = extensoes::arquivo(&e.pasta, &caminho) else { return falhar(req, "arquivo não encontrado") };
    match std::fs::read(&arq) {
        Ok(dados) => {
            let n = dados.len() as i64;
            let fluxo = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(dados));
            let r = URISchemeResponse::new(&fluxo, n);
            r.set_content_type(extensoes::tipo_do_arquivo(&arq));
            r.set_status(200, Some("OK"));
            req.finish_with_response(&r);
        }
        Err(_) => falhar(req, "arquivo não encontrado"),
    }
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
    carregar_downloads();
    let ctx = WebContext::default();
    if let Some(ctx) = &ctx {
        // As páginas e os arquivos das extensões: um esquema seguro (as APIs que pedem
        // contexto seguro funcionam) e com CORS (fetch() de arquivos da própria extensão).
        if let Some(sm) = ctx.security_manager() {
            sm.register_uri_scheme_as_secure(ESQUEMA);
            sm.register_uri_scheme_as_cors_enabled(ESQUEMA);
        }
        ctx.register_uri_scheme(ESQUEMA, servir_extensao);
        ctx.set_cache_model(webkit6::CacheModel::WebBrowser);
        ctx.set_spell_checking_enabled(true);
        ctx.set_spell_checking_languages(&["pt_BR", "en_US"]);
    }
    let ajustes = Settings::new();
    let gpu = std::env::var_os("JANGADA_NO_GPU").is_none() && aj.placa_de_video;
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
    recursos_do_motor(&ajustes);
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

/// Barrar (ou não) os cookies de terceiros. A prevenção de rastreamento do WebKit (a do
/// Safari) fica desligada: ela apagava os dados dos sites que ela classifica sozinha, até de
/// um usado todo dia (o WhatsApp Web perdia a sessão, e o celular continuava mostrando o
/// aparelho conectado). Os cookies de terceiros barrados já cortam o rastreamento entre sites.
pub fn aplicar_cookies(barrar: bool) {
    com(|m| {
        if let Some(cm) = m.sessao.cookie_manager() {
            cm.set_accept_policy(if barrar { CookieAcceptPolicy::NoThirdParty } else { CookieAcceptPolicy::Always });
        }
        m.sessao.set_itp_enabled(false);
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

pub fn abas_abertas() -> Vec<Shared> {
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
    ucm.add_script(&webkit6::UserScript::for_world(&COR_DE_FUNDO.replace("{MUNDO}", MUNDO), UserContentInjectedFrames::TopFrame, webkit6::UserScriptInjectionTime::End, MUNDO, &[], &[]));
    ucm
}

/// O que a página ainda não pintou (rolando rápido) mostra o fundo do motor: com a cor da
/// página, como no Safari, e não um clarão branco no meio de um site escuro. A cor é a do
/// html, a do body ou, nos sites que pintam o fundo num elemento por cima de tudo (o
/// YouTube), a do primeiro elemento com fundo no meio da tela. Refeita quando o site troca
/// de tema (classe ou atributo do html e do body).
const COR_DE_FUNDO: &str = r#"(function(){var u='';function g(x){if(!x||!x.nodeType)return null;var v=getComputedStyle(x).backgroundColor;if(!v||v==='transparent'||/rgba\([^)]*,\s*0\)$/.test(v))return null;return v}
function c(){var d=document,v=g(d.documentElement)||g(d.body);if(!v){var e=d.elementFromPoint(innerWidth/2,innerHeight-8);while(e&&!v){v=g(e);e=e.parentElement}}v=v||'rgb(255, 255, 255)';if(v!==u){u=v;window.webkit.messageHandlers.{MUNDO}.postMessage('jangada-fundo:'+v)}}
c();var t=0,o=new MutationObserver(function(){clearTimeout(t);t=setTimeout(c,150)});o.observe(document.documentElement,{attributes:true});if(document.body)o.observe(document.body,{attributes:true});addEventListener('load',function(){setTimeout(c,300)});setTimeout(c,1500);setTimeout(c,5000)})();"#;

/// O aviso de "documento pronto" (vem do mundo isolado: a página não consegue mandá-lo).
const PRONTO: &str = "\u{1}JANGADA:pronto";
/// O aviso de que a página prendeu o mouse (pointer lock).
const MOUSE_PRESO: &str = "\u{1}JANGADA:mouse";

const NAO_RASTREAR: &str = "(function(){try{Object.defineProperty(Navigator.prototype,'globalPrivacyControl',{get:function(){return true},configurable:true});Object.defineProperty(Navigator.prototype,'doNotTrack',{get:function(){return '1'},configurable:true})}catch(e){}})();";

/// Uma aba nova. `relacionada`: a aba que pediu (window.open: a nova fica ligada a ela).
/// Recursos internos do motor (os "feature flags" do WebKit). `JANGADA_FEATURES` liga ou
/// desliga para testar ("Nome=0,Outro=1"); `JANGADA_LIST_FEATURES` mostra todos.
fn recursos_do_motor(ajustes: &Settings) {
    let Some(todos) = Settings::all_features() else { return };
    // JANGADA_SYNC_SCROLL=1: the page scrolls together with its drawing (never shows what
    // was not painted yet, but each step waits for the page).
    let sync = std::env::var_os("JANGADA_SYNC_SCROLL").is_some();
    let mut pedidos: Vec<(String, bool)> = if sync { vec![("AsyncFrameScrolling".into(), false), ("ThreadedScrolling".into(), false)] } else { Vec::new() };
    // What was already painted stays (scrolling back up showed only the page color while
    // the engine painted it all again).
    pedidos.push(("AggressiveTileRetention".into(), true));
    pedidos.extend(
        std::env::var("JANGADA_FEATURES")
            .unwrap_or_default()
            .split(',')
            .filter_map(|p| p.split_once('=').map(|(n, v)| (n.trim().to_string(), v.trim() != "0"))),
    );
    let listar = std::env::var_os("JANGADA_LIST_FEATURES").is_some();
    for i in 0..todos.length() {
        let Some(f) = todos.get(i) else { continue };
        let id = f.identifier().map(|s| s.to_string()).unwrap_or_default();
        // The last one asked wins (the user's over the defaults).
        if let Some((_, v)) = pedidos.iter().rev().find(|(n, _)| *n == id) {
            ajustes.set_feature_enabled(&f, *v);
        }
        if listar {
            eprintln!("{id}={}", ajustes.is_feature_enabled(&f) as u8);
        }
    }
}

pub fn nova_aba(url: &str, relacionada: Option<&WebView>) -> Option<Shared> {
    let view = com(|m| {
        let ucm = conteudo(m);
        let mut b = WebView::builder().network_session(&m.sessao).user_content_manager(&ucm).settings(&m.ajustes);
        if let Some(r) = relacionada {
            b = b.related_view(r);
        }
        b.build()
    })?;
    extensoes_no_inicio(&view);
    Some(preparar(view, url))
}

/// Os scripts das extensões que pedem o começo do documento (`run_at: document_start`,
/// como os bloqueadores de anúncio): entram em toda página da aba antes dos scripts dela,
/// cada um no mundo da extensão e só nas páginas que casam com a regra. Os que a aba já
/// tinha saem antes (a lista de extensões pode ter mudado).
pub fn extensoes_no_inicio(view: &WebView) {
    let Some(ucm) = view.user_content_manager() else { return };
    let velhos = INICIO.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(w, _)| w.upgrade().is_some());
        l.iter().position(|(w, _)| w.upgrade().as_ref() == Some(view)).map(|i| l.remove(i).1).unwrap_or_default()
    });
    for s in &velhos {
        ucm.remove_script(s);
    }
    let mut novos = Vec::new();
    for (ext, mundo, codigo) in extensoes::scripts_no_inicio() {
        if ucm.register_script_message_handler(&mundo, Some(&mundo)) {
            let (v, e) = (view.clone(), ext.clone());
            ucm.connect_script_message_received(Some(&mundo), move |_, valor| {
                ponte::pedido(ponte::Origem::Conteudo { view: v.clone(), ext: e.clone() }, &valor.to_str());
            });
        }
        let s = webkit6::UserScript::for_world(&codigo, UserContentInjectedFrames::TopFrame, webkit6::UserScriptInjectionTime::Start, &mundo, &[], &[]);
        ucm.add_script(&s);
        novos.push(s);
    }
    if !novos.is_empty() {
        INICIO.with(|l| l.borrow_mut().push((view.downgrade(), novos)));
    }
}

/// As extensões mudaram (instalada, ligada, desligada, removida): os scripts do começo
/// do documento de todas as abas abertas são refeitos (valem a partir da próxima página).
pub fn extensoes_mudaram() {
    for p in abas_abertas() {
        let v = p.borrow().view.clone();
        extensoes_no_inicio(&v);
    }
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
    permitir_navegacao(url);
    view.load_uri(url);
}

/// Uma aba que o motor já criou (window.open): só liga os sinais.
pub fn aba_criada(view: WebView, url: &str) -> Shared {
    let p = preparar(view, url);
    p.borrow().parentes.set(true);
    p
}

fn preparar(view: WebView, url: &str) -> Shared {
    let id = PROXIMA_ABA.with(|n| {
        let i = n.get();
        n.set(i + 1);
        i
    });
    let p: Shared = Rc::new(RefCell::new(Pagina {
        id,
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
        extensoes: Vec::new(),
        apis_ext: Vec::new(),
        canal_ext: false,
        cors_ext: None,
    }));
    ABAS.with(|a| a.borrow_mut().push(Rc::downgrade(&p)));
    view.set_hexpand(true);
    view.set_vexpand(true);
    ligar(&p, &view);
    if !url.is_empty() {
        filtros_para(&view, url);
        permitir_navegacao(url);
        view.load_uri(url);
    }
    p
}

/// Os sinais do motor para a aba.
fn ligar(p: &Shared, view: &WebView) {
    // Arquivos soltos de fora chegam à página como arquivos.
    crate::arquivos::ligar_soltar(view);
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
    // O link sob o mouse. O motor avisa a cada elemento que passa sob o mouse (rolando, sem
    // parar): a Jangada só redesenha quando o link mudou e depois de o mouse parar nele um
    // instante, como o Safari (redesenhar a cada aviso tirava a fluidez da rolagem).
    {
        let fraco = fraco.clone();
        let vez = Rc::new(Cell::new(0u64));
        view.connect_mouse_target_changed(move |_, hit, _| {
            let s = if hit.context_is_link() { hit.link_uri().map(|u| u.to_string()).unwrap_or_default() } else { String::new() };
            let n = vez.get().wrapping_add(1);
            vez.set(n);
            let (fraco, vez) = (fraco.clone(), vez.clone());
            // Saindo do link, some logo; um link novo aparece se o mouse ficar nele.
            let espera = if s.is_empty() { 0 } else { 120 };
            glib::timeout_add_local_once(std::time::Duration::from_millis(espera), move || {
                if vez.get() != n {
                    return;
                }
                if let Some(p) = fraco.upgrade() {
                    if let Ok(mut p) = p.try_borrow_mut() {
                        if p.status != s {
                            p.status = s;
                            p.mudou();
                        }
                    }
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
    // Um clique na página: a janelinha de extensão aberta fecha (como clicar fora dela).
    // Só olha: o clique segue para a página.
    {
        let l = gtk::EventControllerLegacy::new();
        l.set_propagation_phase(gtk::PropagationPhase::Capture);
        l.connect_event(|_, ev| {
            if ev.event_type() == gdk::EventType::ButtonPress && ponte::POPUP.with(|p| p.borrow().is_some()) {
                CLIQUE_NA_PAGINA.set(true);
                glove_ui::redraw_soon();
            }
            glib::Propagation::Proceed
        });
        view.add_controller(l);
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
            // Uma página de extensão abrindo outra página dela numa aba nova.
            let atual = v.uri().map(|u| u.to_string()).unwrap_or_default();
            if extensoes::partes_da_url(&url).is_some_and(|(id, _)| extensoes::partes_da_url(&atual).is_some_and(|(a, _)| a == id)) {
                permitir_navegacao(&url);
            }
            let nova = com(|m| {
                let ucm = conteudo(m);
                WebView::builder().related_view(v).user_content_manager(&ucm).settings(&m.ajustes).build()
            })?;
            extensoes_no_inicio(&nova);
            // A nova divide o processo com esta.
            parentes.set(true);
            let fundo = acao.mouse_button() == 2;
            NOVAS_ABAS.with(|n| n.borrow_mut().push((url, fundo, Some(nova.clone()))));
            glove_ui::redraw_soon();
            Some(nova.upcast())
        });
    }
    // Botão do meio ou Ctrl + clique num link: aba nova por trás. E a guarda das páginas
    // de extensão: um site não abre uma página de extensão (só os arquivos que ela
    // declarou acessíveis), e a aba que vai mostrar uma ganha a API dela.
    {
        let fraco = fraco.clone();
        view.connect_decide_policy(move |v, d, tipo| {
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
            // Um link magnet: o torrent vai para o ajudante (a página fica onde está).
            if url.starts_with("magnet:") {
                d.ignore();
                crate::torrent::adicionar_magnet(&url);
                ABRIR_TORRENTS.with(|a| a.set(true));
                glove_ui::redraw_soon();
                return true;
            }
            if url.starts_with(ESQUEMA) {
                let atual = v.uri().map(|u| u.to_string()).unwrap_or_default();
                let historico = matches!(acao.navigation_type(), webkit6::NavigationType::BackForward | webkit6::NavigationType::Reload);
                if !pode_ir_para_extensao(&atual, &url, historico) {
                    d.ignore();
                    return true;
                }
                if tipo == PolicyDecisionType::NavigationAction {
                    if let (Some(p), Some((id, _))) = (fraco.upgrade(), extensoes::partes_da_url(&url)) {
                        if let (Some(e), Ok(mut p)) = (extensoes::ativa(&id), p.try_borrow_mut()) {
                            preparar_pagina_de_extensao(v, &mut p, &e);
                        }
                    }
                }
            }
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
                let u = v.uri().map(|u| u.to_string()).unwrap_or_default();
                // Os escudos do site (ligados ou não nele) antes de a página pedir o resto.
                filtros_para(v, &u);
                // Saiu da página da extensão: o CORS livre dela não vale para outro site.
                let ext = extensoes::partes_da_url(&u).map(|(id, _)| id);
                c(&|p| {
                    p.extensoes.clear();
                    if p.cors_ext.is_some() && p.cors_ext != ext {
                        p.cors_ext = None;
                        p.view.set_cors_allowlist(&[]);
                    }
                    // E o canal das páginas de extensão sai: o site não o vê.
                    if p.canal_ext && ext.is_none() {
                        if let Some(ucm) = p.view.user_content_manager() {
                            ucm.unregister_script_message_handler(CANAL_PAGINAS, None);
                        }
                        p.canal_ext = false;
                    }
                });
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
    // As mensagens dos scripts da Jangada (o mundo isolado) e as das páginas de extensão
    // abertas na aba (o canal delas só passa a existir quando a aba abre uma).
    if let Some(ucm) = view.user_content_manager() {
        let v = view.clone();
        ucm.connect_script_message_received(Some(MUNDO), move |_, valor| mensagem(&v, &valor.to_str()));
        let v = view.clone();
        ucm.connect_script_message_received(Some(CANAL_PAGINAS), move |_, valor| pedido_de_pagina(&v, &valor.to_str()));
    }
}

/// Um pedido de uma página de extensão (numa aba ou na janelinha): a extensão é a do
/// endereço da página, nunca a que a mensagem diz.
fn pedido_de_pagina(view: &WebView, texto: &str) {
    let url = view.uri().map(|u| u.to_string()).unwrap_or_default();
    if let Some((id, _)) = extensoes::partes_da_url(&url) {
        ponte::pedido(ponte::Origem::Pagina { view: view.clone(), ext: id }, texto);
    }
}

/// A aba vai mostrar uma página de extensão: o canal das páginas, a API dela (no começo de
/// cada documento dela) e o CORS livre para os sites que ela pede.
fn preparar_pagina_de_extensao(view: &WebView, p: &mut Pagina, e: &Extensao) {
    let Some(ucm) = view.user_content_manager() else { return };
    if !p.canal_ext {
        ucm.register_script_message_handler(CANAL_PAGINAS, None);
        p.canal_ext = true;
    }
    if !p.apis_ext.contains(&e.id) {
        ucm.add_script(&UserScript::new(&extensoes::api_das_paginas(e), UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, &[], &[]));
        p.apis_ext.push(e.id.clone());
    }
    let hosts = e.hosts_do_motor();
    let hosts: Vec<&str> = hosts.iter().map(String::as_str).collect();
    view.set_cors_allowlist(&hosts);
    p.cors_ext = Some(e.id.clone());
}

/// A janelinha de uma extensão (a página do botão dela): fora das abas, com a API da
/// extensão, o script que a mede e o CORS livre para os sites que ela pede.
pub fn janelinha(e: &Extensao, caminho: &str) -> Option<WebView> {
    let view = com(|m| {
        let ucm = UserContentManager::new();
        ucm.register_script_message_handler(CANAL_PAGINAS, None);
        ucm.add_script(&UserScript::new(&extensoes::api_das_paginas(e), UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, &[], &[]));
        ucm.add_script(&UserScript::new(ponte::MEDIR_POPUP, UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, &[], &[]));
        WebView::builder().network_session(&m.sessao).user_content_manager(&ucm).settings(&m.ajustes).build()
    })?;
    let hosts = e.hosts_do_motor();
    let hosts: Vec<&str> = hosts.iter().map(String::as_str).collect();
    view.set_cors_allowlist(&hosts);
    // Uma referência fraca: a janelinha abre e fecha muitas vezes, e uma forte (a página
    // segurando o canal que segura a página) a deixaria na memória para sempre.
    if let Some(ucm) = view.user_content_manager() {
        let v = view.downgrade();
        ucm.connect_script_message_received(Some(CANAL_PAGINAS), move |_, valor| {
            if let Some(v) = v.upgrade() {
                pedido_de_pagina(&v, &valor.to_str());
            }
        });
    }
    // Ir para outra página da extensão (ou para um site) dentro da janelinha pode; uma
    // página de outra extensão, só se acessível.
    view.connect_decide_policy(|v, d, tipo| {
        if tipo != PolicyDecisionType::NavigationAction {
            return false;
        }
        let Some(n) = d.downcast_ref::<NavigationPolicyDecision>() else { return false };
        let Some(acao) = n.navigation_action() else { return false };
        let url = acao.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
        let atual = v.uri().map(|u| u.to_string()).unwrap_or_default();
        if url.starts_with(ESQUEMA) && !pode_ir_para_extensao(&atual, &url, false) {
            d.ignore();
            return true;
        }
        false
    });
    // window.open e target=_blank: uma aba (a janelinha fecha, como no Chrome).
    view.connect_create(|v, acao| {
        let url = acao.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
        if !url.is_empty() {
            let atual = v.uri().map(|u| u.to_string()).unwrap_or_default();
            if extensoes::partes_da_url(&url).is_some_and(|(id, _)| extensoes::partes_da_url(&atual).is_some_and(|(a, _)| a == id)) {
                permitir_navegacao(&url);
            }
            NOVAS_ABAS.with(|n| n.borrow_mut().push((url, false, None)));
            ponte::fechar_popup_pedido();
        }
        None
    });
    view.connect_close(|_| ponte::fechar_popup_pedido());
    // Esc fecha a janelinha (como no Chrome).
    {
        let k = gtk::EventControllerKey::new();
        k.connect_key_pressed(|_, tecla, _, _| {
            if tecla == gdk::Key::Escape {
                ponte::fechar_popup_pedido();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(k);
    }
    let url = e.url(caminho);
    permitir_navegacao(&url);
    view.load_uri(&url);
    Some(view)
}

/// Uma mensagem de um script da Jangada na página (senhas, extensões).
fn mensagem(view: &WebView, m: &str) {
    // A cor de fundo da página (ver COR_DE_FUNDO).
    if let Some(cor) = m.strip_prefix("jangada-fundo:") {
        if let Ok(mut c) = gdk::RGBA::parse(cor) {
            c.set_alpha(1.0);
            view.set_background_color(&c);
        }
        return;
    }
    let url = view.uri().map(|u| u.to_string()).unwrap_or_default();
    if m == PRONTO {
        // Um por vez: um erro de escrita no script de uma extensão não leva os outros.
        if let Some(js) = crate::senhas::script_da_pagina(&url) {
            view.evaluate_javascript(&js, Some(MUNDO), Some(&url), None::<&gio::Cancellable>, |_| {});
        }
        scripts_das_extensoes(view, &url);
        return;
    }
    if m == MOUSE_PRESO {
        AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("A página prendeu o mouse: Esc solta.").to_string()));
        glove_ui::redraw_soon();
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

/// Os scripts de conteúdo das extensões na página: cada extensão no mundo dela, com o
/// canal dela para a Jangada (criado na primeira vez que ela entra numa página da aba).
fn scripts_das_extensoes(view: &WebView, url: &str) {
    let todas = extensoes::scripts_para(url);
    if todas.is_empty() {
        return;
    }
    let Some(ucm) = view.user_content_manager() else { return };
    let pagina = abas_abertas().into_iter().find(|p| &p.borrow().view == view);
    for (ext, mundo, partes) in todas {
        if ucm.register_script_message_handler(&mundo, Some(&mundo)) {
            let (v, e) = (view.clone(), ext.clone());
            ucm.connect_script_message_received(Some(&mundo), move |_, valor| {
                ponte::pedido(ponte::Origem::Conteudo { view: v.clone(), ext: e.clone() }, &valor.to_str());
            });
        }
        if let Some(p) = &pagina {
            let mut p = p.borrow_mut();
            if !p.extensoes.contains(&ext) {
                p.extensoes.push(ext.clone());
            }
        }
        // (Vazia: só regras `document_start`, que já entraram no começo do documento.)
        for js in partes {
            view.evaluate_javascript(&js, Some(&mundo), Some(url), None::<&gio::Cancellable>, |_| {});
        }
    }
}

/// As chaves das permissões (guardadas em permissoes.tsv): camera, microfone,
/// location, notifications, clipboard.
fn pedido_de_permissao(view: &WebView, req: &PermissionRequest) -> bool {
    let origem = crate::importar::origem(&view.uri().map(|u| u.to_string()).unwrap_or_default());
    let mut nomes: Vec<&'static str> = Vec::new();
    if let Some(m) = req.downcast_ref::<webkit6::UserMediaPermissionRequest>() {
        if webkit6::functions::user_media_permission_is_for_audio_device(m) {
            nomes.push("microphone");
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
        nomes.push("location");
    } else if req.is::<webkit6::NotificationPermissionRequest>() {
        nomes.push("notifications");
    } else if req.type_().name() == "WebKitClipboardPermissionRequest" {
        // Ler o que foi copiado (o botão "colar" de um site): a pessoa decide. (Pelo nome
        // do tipo: ele só existe no WebKitGTK 2.42 em diante, e a Jangada roda nos de antes.)
        nomes.push("clipboard");
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
        "microphone" => "o microfone",
        "location" => "a localização",
        "clipboard" => "a área de transferência",
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

/// "Copiar Imagem": o motor baixa a imagem (com os cookies do site, como a página a vê)
/// e ela vai para a área de transferência como PNG.
pub fn copiar_imagem(view: &WebView, uri: &str) {
    let Some(s) = view.network_session() else { return };
    COPIAR_IMAGEM.with(|l| l.borrow_mut().push(uri.to_string()));
    s.download_uri(uri);
}

fn copiar_download(d: &Download) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("jangada-imagem-{}-{n}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    d.set_allow_overwrite(true);
    let dest = tmp.clone();
    d.connect_decide_destination(move |d, _| {
        d.set_destination(&dest.to_string_lossy());
        true
    });
    let dest = tmp.clone();
    d.connect_finished(move |d| {
        let mime = d.response().and_then(|r| r.mime_type()).map(|m| m.to_string()).unwrap_or_default();
        // Qualquer formato que o GTK abre (JPEG, WebP, GIF…) vira PNG, que todo app cola.
        let png = gdk::Texture::from_filename(&dest).ok().map(|t| t.save_to_png_bytes().to_vec());
        let ok = match png {
            Some(b) => por_na_area(&b, "image/png"),
            None => std::fs::read(&dest).ok().is_some_and(|b| mime.starts_with("image/") && por_na_area(&b, &mime)),
        };
        let _ = std::fs::remove_file(&dest);
        AVISOS.with(|a| a.borrow_mut().push(if ok { glove_config::tr!("Imagem copiada").to_string() } else { glove_config::tr!("Não foi possível copiar a imagem").to_string() }));
        glove_ui::redraw_soon();
    });
    d.connect_failed(move |_, _| {
        let _ = std::fs::remove_file(&tmp);
        AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Não foi possível copiar a imagem").to_string()));
        glove_ui::redraw_soon();
    });
}

/// Põe os bytes na área de transferência, com o tipo dado.
fn por_na_area(dados: &[u8], tipo: &str) -> bool {
    use std::io::Write;
    let Ok(mut c) = std::process::Command::new("wl-copy").args(["--type", tipo]).stdin(std::process::Stdio::piped()).spawn() else { return false };
    if let Some(mut i) = c.stdin.take() {
        if i.write_all(dados).is_err() {
            return false;
        }
    }
    c.wait().is_ok_and(|s| s.success())
}

fn novo_download(d: &Download) {
    let uri = d.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
    if !uri.is_empty() && COPIAR_IMAGEM.with(|l| {
        let mut l = l.borrow_mut();
        l.iter().position(|u| *u == uri).map(|i| l.remove(i)).is_some()
    }) {
        copiar_download(d);
        return;
    }
    let id = PROXIMO_DOWNLOAD.with(|n| n.replace(n.get() + 1));
    // De onde veio: o site da página que pediu (ou o do próprio arquivo).
    let pagina = d.web_view().and_then(|v| v.uri()).map(|u| u.to_string()).unwrap_or_default();
    let origem = host_de(if pagina.starts_with("http") { &pagina } else { &uri });
    DOWNLOADS.with(|l| l.borrow_mut().push(Baixado { id, nome: String::new(), caminho: PathBuf::new(), recebido: 0, total: 0, velocidade: 0, pronto: false, cancelado: false, falhou: false, dl: Some(d.clone()), marca: (std::time::Instant::now(), 0), uri: uri.clone(), origem, quando: agora() }));
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
        let mut caminho = PathBuf::new();
        atualizar(id, |x| {
            if !x.cancelado {
                x.pronto = true;
                x.recebido = x.recebido.max(x.total);
                nome = x.nome.clone();
                caminho = x.caminho.clone();
            }
            x.dl = None;
        });
        // Um arquivo .torrent: vai direto para o ajudante (e a aba Torrents abre).
        let e_torrent = caminho.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent")) || d.response().and_then(|r| r.mime_type()).is_some_and(|m| m == "application/x-bittorrent");
        if e_torrent && caminho.is_file() {
            crate::torrent::adicionar_arquivo(&caminho);
            ABRIR_TORRENTS.with(|a| a.set(true));
        }
        if !nome.is_empty() {
            AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Baixado em Downloads: {}", nome)));
        }
        gravar_downloads();
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
        gravar_downloads();
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
    gravar_downloads();
}

/// Baixa um endereço (o "+ Adicionar URL" da janela Downloads), como um clique num link.
pub fn baixar_url(url: &str) -> bool {
    let url = url.trim();
    let url = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    if glib::Uri::parse(&url, glib::UriFlags::NONE).is_err() {
        return false;
    }
    com(|m| {
        m.sessao.download_uri(&url);
    });
    true
}

/// Baixa de novo um download que falhou ou foi cancelado (ele sai da lista; o novo entra).
pub fn tentar_de_novo(id: u32) {
    let uri = DOWNLOADS.with(|l| {
        let mut l = l.borrow_mut();
        let i = l.iter().position(|x| x.id == id)?;
        let uri = l[i].uri.clone();
        (!uri.is_empty()).then(|| l.remove(i).uri)
    });
    if let Some(u) = uri {
        baixar_url(&u);
    }
    DOWNLOADS_MUDOU.with(|m| m.set(true));
    gravar_downloads();
}

/// Tira da lista (o arquivo continua na pasta). Um em andamento é cancelado antes.
pub fn remover_da_lista(id: u32) {
    let dl = DOWNLOADS.with(|l| {
        let mut l = l.borrow_mut();
        let i = l.iter().position(|x| x.id == id)?;
        l.remove(i).dl
    });
    if let Some(d) = dl {
        d.cancel();
    }
    DOWNLOADS_MUDOU.with(|m| m.set(true));
    gravar_downloads();
}

/// Tira da lista os que já terminaram (prontos, cancelados, com falha).
pub fn limpar_concluidos() {
    DOWNLOADS.with(|l| l.borrow_mut().retain(|d| !d.pronto && !d.cancelado));
    DOWNLOADS_MUDOU.with(|m| m.set(true));
    gravar_downloads();
}

fn agora() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// O site de um endereço ("www.exemplo.com" → "exemplo.com").
pub fn host_de(url: &str) -> String {
    let h = glib::Uri::parse(url, glib::UriFlags::NONE).ok().and_then(|u| u.host()).map(|h| h.to_string()).unwrap_or_default();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

/// A lista guardada: os downloads que já terminaram (os mais recentes, até 300).
const LISTA_DOWNLOADS: &str = "downloads.tsv";

fn gravar_downloads() {
    if crate::ajustes::privada() {
        return;
    }
    let linhas: Vec<String> = DOWNLOADS.with(|l| {
        let l = l.borrow();
        let feitos: Vec<&Baixado> = l.iter().filter(|d| d.dl.is_none() && !d.nome.is_empty()).collect();
        let tira = feitos.len().saturating_sub(300);
        feitos[tira..]
            .iter()
            .map(|d| {
                let estado = if d.pronto { "ok" } else if d.falhou { "falhou" } else { "cancelado" };
                let limpo = |t: &str| t.replace(['\t', '\n'], " ");
                format!("{}\t{}\t{}\t{}\t{}\t{}\t{}", d.quando, estado, d.total.max(d.recebido), limpo(&d.nome), limpo(&d.caminho.to_string_lossy()), limpo(&d.uri), limpo(&d.origem))
            })
            .collect()
    });
    let _ = std::fs::write(crate::ajustes::pasta().join(LISTA_DOWNLOADS), linhas.join("\n") + "\n");
}

/// Lê a lista guardada (ao abrir a Jangada): os downloads de antes voltam à janela.
pub fn carregar_downloads() {
    if crate::ajustes::privada() {
        return;
    }
    let texto = std::fs::read_to_string(crate::ajustes::pasta().join(LISTA_DOWNLOADS)).unwrap_or_default();
    let antes: Vec<Baixado> = texto
        .lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 7 {
                return None;
            }
            let id = PROXIMO_DOWNLOAD.with(|n| n.replace(n.get() + 1));
            let total = c[2].parse().unwrap_or(0);
            Some(Baixado { id, nome: c[3].into(), caminho: PathBuf::from(c[4]), recebido: total, total, velocidade: 0, pronto: c[1] == "ok", cancelado: c[1] != "ok", falhou: c[1] == "falhou", dl: None, marca: (std::time::Instant::now(), 0), uri: c[5].into(), origem: c[6].into(), quando: c[0].parse().unwrap_or(0) })
        })
        .collect();
    DOWNLOADS.with(|l| {
        let mut l = l.borrow_mut();
        let novos = std::mem::take(&mut *l);
        *l = antes;
        l.extend(novos);
    });
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
