//! O motor: o Chromium (CEF) sem janela própria. Cada aba é um navegador que desenha a
//! página numa imagem (`on_paint`); o app copia essa imagem para a janela do glove-ui e
//! manda para o motor o mouse e o teclado. Tudo roda na linha principal: o CEF trabalha
//! quando o app chama `cef::do_message_loop_work()` (a cada tique do glove-ui).

use std::cell::{Cell, RefCell};
use std::rc::Rc as StdRc;

use cef::*;

/// O que uma aba sabe da página (escrito pelo motor, lido pelo app).
pub struct Pagina {
    /// Área da página na janela, em pontos, e a escala da tela.
    pub w: i32,
    pub h: i32,
    pub scale: f32,
    /// A imagem da página (RGBA pré-multiplicado), já do tamanho da janela, e a versão
    /// dela (muda a cada imagem nova: a janela só a recopia quando muda).
    pub tela: Option<glove_ui::paint::draw::Canvas>,
    pub versao: u64,
    /// A versão de cada faixa de `FAIXA` linhas de pixel (só as que mudaram vão para a
    /// tela de novo: um cursor piscando não recopia a página inteira).
    pub faixas: Vec<u64>,
    /// Lista suspensa da página (<select>), desenhada por cima: onde e a imagem.
    pub popup: Option<cef::Rect>,
    pub popup_frame: Vec<u8>,
    pub pw: i32,
    pub ph: i32,
    /// Chegou imagem nova desde o último quadro.
    pub dirty: bool,
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub back: bool,
    pub forward: bool,
    pub browser: Option<cef::Browser>,
    /// Menu de contexto pedido pela página (botão direito).
    pub ctx: Option<Contexto>,
    /// Diálogo do JavaScript esperando resposta (alert/confirm).
    pub dialog: Option<(bool, String, JsdialogCallback)>,
    /// Texto da dica (title de um link ou botão).
    pub tooltip: String,
    /// Endereço do link sob o mouse (mostrado embaixo, como nos navegadores).
    pub status: String,
    /// O cursor que a página pede (mãozinha nos links, barra nos textos).
    pub cursor: glove_ui::Cursor,
    /// O ícone do site (RGBA pré-multiplicado, largura, altura) e de onde veio.
    pub favicon: Option<(Vec<u8>, i32, i32)>,
    pub favicon_url: String,
    /// O ícone mudou (o app refaz a imagem dele).
    pub favicon_novo: bool,
    /// Um site não abriu: (endereço, código do erro) para o app mostrar a página de erro
    /// da Jangada; e o endereço que falhou enquanto ela está à vista.
    pub erro_pedido: Option<(String, String)>,
    pub erro_url: Option<String>,
    /// Buscar na página: (o atual, quantos achou).
    pub achados: (i32, i32),
    /// Endereço pedido antes de o motor terminar de criar a aba (carrega quando ficar pronta).
    pub pendente: Option<String>,
}

impl Pagina {
    pub fn new(scale: f32) -> Self {
        Self { w: 800, h: 600, scale, tela: None, versao: 0, faixas: Vec::new(), popup: None, popup_frame: Vec::new(), pw: 0, ph: 0, dirty: true, title: String::new(), url: String::new(), loading: true, back: false, forward: false, browser: None, ctx: None, dialog: None, tooltip: String::new(), status: String::new(), cursor: glove_ui::Cursor::Arrow, favicon: None, favicon_url: String::new(), favicon_novo: false, erro_pedido: None, erro_url: None, achados: (0, 0), pendente: None }
    }
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

pub type Shared = StdRc<RefCell<Pagina>>;

thread_local! {
    /// Endereços que pediram aba nova (target=_blank, window.open) e se ela abre por
    /// trás (botão do meio, Ctrl + clique).
    pub static NOVAS_ABAS: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
    /// Quando o motor quer trabalhar de novo (ms; o CEF avisa).
    pub static BOMBA: Cell<i64> = const { Cell::new(0) };
    /// Avisos para a barra de baixo (downloads).
    pub static AVISOS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Os downloads desta vez (a janela Downloads), do mais velho para o mais novo.
    pub static DOWNLOADS: RefCell<Vec<Download>> = const { RefCell::new(Vec::new()) };
    /// A página virou PDF para imprimir: o arquivo (o app abre a folha Imprimir).
    pub static IMPRIMIR: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
    /// Mudou algo nos downloads (o app redesenha).
    pub static DOWNLOADS_MUDOU: Cell<bool> = const { Cell::new(false) };
    /// Há uma sessão do Google aberta (os cookies da conta existem)? É o resultado da
    /// conferência anterior (`conferir_google`): a da vez vai para GOOGLE_NOVO e só passa
    /// a valer na seguinte, quando já terminou (senão o estado piscava).
    pub static GOOGLE: Cell<bool> = const { Cell::new(false) };
    static GOOGLE_NOVO: Cell<bool> = const { Cell::new(false) };
    static GOOGLE_CONFERIDO: Cell<bool> = const { Cell::new(false) };
}

wrap_cookie_visitor! {
    pub struct Biscoitos;

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&Cookie>, _count: ::std::os::raw::c_int, _total: ::std::os::raw::c_int, _delete_cookie: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            if let Some(c) = cookie {
                let nome = c.name.to_string();
                if matches!(nome.as_str(), "SID" | "__Secure-1PSID" | "SAPISID" | "__Secure-3PSID") {
                    GOOGLE_NOVO.with(|g| g.set(true));
                    return 0;
                }
            }
            1
        }
    }
}

/// Confere (sem bloquear) se há uma conta do Google com sessão aberta: os cookies de
/// sessão do Google existem. A resposta chega em `GOOGLE` logo depois.
pub fn conferir_google() {
    let Some(m) = cookie_manager_get_global_manager(None) else { return };
    // A conferência anterior já terminou: o resultado dela passa a valer.
    if GOOGLE_CONFERIDO.with(|c| c.get()) {
        GOOGLE.with(|g| g.set(GOOGLE_NOVO.with(|n| n.get())));
    }
    GOOGLE_CONFERIDO.with(|c| c.set(true));
    GOOGLE_NOVO.with(|g| g.set(false));
    m.visit_url_cookies(Some(&"https://accounts.google.com/".into()), 1, Some(&mut Biscoitos::new()));
}

/// Um download: o nome, onde fica, quanto chegou e o estado.
pub struct Download {
    pub id: u32,
    pub nome: String,
    pub caminho: std::path::PathBuf,
    pub recebido: i64,
    pub total: i64,
    pub velocidade: i64,
    pub pronto: bool,
    pub cancelado: bool,
    /// Para cancelar enquanto baixa.
    pub cb: Option<DownloadItemCallback>,
}

fn s(v: Option<&CefString>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

/// BGRA (do motor) para RGBA (do glove-paint), os dois pré-multiplicados.
/// Altura das faixas da imagem da página (em pixels).
pub const FAIXA: i32 = 64;

fn bgra_to_rgba(src: &[u8], dst: &mut Vec<u8>) {
    dst.resize(src.len(), 0);
    bgra_to_rgba_em(src, dst);
}

fn bgra_to_rgba_em(src: &[u8], dst: &mut [u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
        d[3] = s[3];
    }
}

// ---------------------------------------------------------------------------
// App (opções do motor) e o processo do navegador
// ---------------------------------------------------------------------------

wrap_app! {
    pub struct MotorApp;

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            // Só no processo principal (os outros herdam o que precisam).
            if process_type.is_some_and(|t| !t.to_string().is_empty()) {
                return;
            }
            let Some(cl) = command_line else { return };
            let sw = |cl: &mut CommandLine, n: &str| cl.append_switch(Some(&n.into()));
            let val = |cl: &mut CommandLine, n: &str, v: &str| cl.append_switch_with_value(Some(&n.into()), Some(&v.into()));
            // Leve para máquinas fracas: sem o processo da placa de vídeo (a página é
            // desenhada pelo processador e copiada para a janela); poucos processos de
            // página; sem o cache de voltar (páginas inteiras guardadas na memória).
            let aj = crate::ajustes::Ajustes::carregar();
            if aj.escuro_nas_paginas {
                val(cl, "enable-features", "WebContentsForceDark");
            }
            if std::env::var_os("JANGADA_GPU").is_none() && !aj.placa_de_video {
                sw(cl, "disable-gpu");
                sw(cl, "disable-gpu-compositing");
            }
            val(cl, "renderer-process-limit", "4");
            sw(cl, "process-per-site");
            val(cl, "disable-features", "BackForwardCache,Translate,MediaRouter,OptimizationHints,OptimizationGuideModelDownloading,InterestFeedContentSuggestions");
            // Chaves de acesso (passkeys): sem janela própria o motor não mostra o pedido do
            // sistema e a página fica esperando para sempre (o login do Facebook travava).
            // Sem elas, os sites oferecem a senha e o código.
            val(cl, "disable-blink-features", "WebAuth");
            // O endereço da rede local não vaza pelo WebRTC (como no Brave).
            val(cl, "force-webrtc-ip-handling-policy", "default_public_interface_only");
            sw(cl, "disable-background-networking");
            sw(cl, "disable-component-update");
            sw(cl, "no-first-run");
            sw(cl, "no-default-browser-check");
            sw(cl, "disable-default-apps");
            // Teste (JANGADA_TESTE_MIDIA=1): uma câmera e um microfone falsos, para conferir
            // os pedidos de permissão sem hardware.
            if std::env::var_os("JANGADA_TESTE_MIDIA").is_some() {
                sw(cl, "use-fake-device-for-media-stream");
            }
            // Sem X11 à vista: o motor não precisa de tela nenhuma.
            if std::env::var_os("DISPLAY").is_none() {
                val(cl, "ozone-platform", "headless");
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(Processo::new())
        }
    }
}

/// O motor já subiu (as preferências do perfil só podem mudar depois disso).
static INICIADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Barrar (ou não) os cookies de terceiros, como o Brave: a preferência do perfil.
/// Grava cookies importados de outro navegador (as sessões abertas nos sites).
pub fn gravar_cookies(v: &[crate::importar::Biscoito]) -> usize {
    let Some(m) = cookie_manager_get_global_manager(None) else { return 0 };
    let mut n = 0;
    for b in v {
        let host = b.host.trim_start_matches('.');
        let url = format!("{}://{}{}", if b.seguro { "https" } else { "http" }, host, if b.caminho.is_empty() { "/" } else { &b.caminho });
        let c = Cookie {
            name: b.nome.as_str().into(),
            value: b.valor.as_str().into(),
            domain: b.host.as_str().into(),
            path: b.caminho.as_str().into(),
            secure: b.seguro as i32,
            httponly: b.http_only as i32,
            has_expires: (b.expira > 0) as i32,
            expires: Basetime { val: b.expira },
            same_site: match b.same_site {
                0 => CookieSameSite::NO_RESTRICTION,
                1 => CookieSameSite::LAX_MODE,
                2 => CookieSameSite::STRICT_MODE,
                _ => CookieSameSite::UNSPECIFIED,
            },
            ..Default::default()
        };
        if m.set_cookie(Some(&url.as_str().into()), Some(&c), None) != 0 {
            n += 1;
        }
    }
    n
}

pub fn aplicar_cookies(barrar: bool) {
    if !INICIADO.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let (Some(ctx), Some(mut v)) = (request_context_get_global_context(), value_create()) else { return };
    v.set_int(if barrar { 1 } else { 0 });
    let mut erro = CefString::default();
    ctx.set_preference(Some(&"profile.cookie_controls_mode".into()), Some(&mut v), Some(&mut erro));
}

wrap_browser_process_handler! {
    pub struct Processo;

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            INICIADO.store(true, std::sync::atomic::Ordering::Relaxed);
            aplicar_cookies(crate::ajustes::COOKIES_TERCEIROS.load(std::sync::atomic::Ordering::Relaxed));
        }

        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            BOMBA.with(|b| b.set(delay_ms.max(0)));
        }
    }
}

pub fn app() -> cef::App {
    MotorApp::new()
}

// ---------------------------------------------------------------------------
// Uma aba: os handlers do cliente, todos com a mesma página
// ---------------------------------------------------------------------------

wrap_render_handler! {
    pub struct Desenho {
        p: Shared,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut cef::Rect>) {
            if let (Some(r), Ok(p)) = (rect, self.p.try_borrow()) {
                r.x = 0;
                r.y = 0;
                r.width = p.w.max(1);
                r.height = p.h.max(1);
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            let (Some(info), Ok(p)) = (info, self.p.try_borrow()) else { return 0 };
            info.device_scale_factor = p.scale;
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            if let Ok(mut p) = self.p.try_borrow_mut() {
                if show == 0 {
                    p.popup = None;
                    p.popup_frame.clear();
                }
                p.dirty = true;
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&cef::Rect>) {
            if let (Some(r), Ok(mut p)) = (rect, self.p.try_borrow_mut()) {
                p.popup = Some(r.clone());
            }
        }

        fn on_paint(&self, _browser: Option<&mut Browser>, type_: PaintElementType, dirty_rects: Option<&[cef::Rect]>, buffer: *const u8, width: ::std::os::raw::c_int, height: ::std::os::raw::c_int) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let Ok(mut p) = self.p.try_borrow_mut() else { return };
            // Segurança: o motor garante o buffer (largura × altura × 4) durante a chamada.
            let src = unsafe { std::slice::from_raw_parts(buffer, (width * height * 4) as usize) };
            if type_ == PaintElementType::POPUP {
                let mut f = std::mem::take(&mut p.popup_frame);
                bgra_to_rgba(src, &mut f);
                p.popup_frame = f;
                p.pw = width;
                p.ph = height;
            } else {
                // Direto na imagem da aba, só nas áreas que mudaram (o motor diz quais):
                // passar o mouse sobre um link não copia a página inteira de novo.
                let escala = p.scale.max(0.5);
                let ok = p.tela.as_ref().is_some_and(|c| c.px_width() == width as u32 && c.px_height() == height as u32);
                if !ok {
                    p.tela = glove_ui::paint::draw::Canvas::new(width as f32 / escala, height as f32 / escala, escala).filter(|c| c.px_width() == width as u32 && c.px_height() == height as u32);
                }
                let areas: Vec<(i32, i32, i32, i32)> = match dirty_rects {
                    Some(r) if ok && !r.is_empty() => r.iter().map(|r| (r.x, r.y, r.width, r.height)).collect(),
                    _ => vec![(0, 0, width, height)],
                };
                let versao = p.versao.wrapping_add(1);
                let nf = (height as usize).div_ceil(FAIXA as usize);
                if !ok || p.faixas.len() != nf {
                    p.faixas = vec![versao; nf];
                }
                for &(_, y, _, h) in &areas {
                    let (a, b) = ((y.max(0) / FAIXA) as usize, (((y + h).min(height) + FAIXA - 1) / FAIXA) as usize);
                    for f in a..b.min(nf) {
                        p.faixas[f] = versao;
                    }
                }
                let Some(c) = p.tela.as_mut() else { return };
                let dst = c.pixels_mut();
                for (x, y, w, h) in areas {
                    let (x0, y0) = (x.clamp(0, width), y.clamp(0, height));
                    let (x1, y1) = ((x + w).clamp(0, width), (y + h).clamp(0, height));
                    if x0 >= x1 {
                        continue;
                    }
                    for yy in y0..y1 {
                        let a = ((yy * width + x0) * 4) as usize;
                        let b = ((yy * width + x1) * 4) as usize;
                        bgra_to_rgba_em(&src[a..b], &mut dst[a..b]);
                    }
                }
                p.versao = p.versao.wrapping_add(1);
            }
            p.dirty = true;
        }
    }
}

wrap_display_handler! {
    pub struct Exibicao {
        p: Shared,
    }

    impl DisplayHandler {
        /// As mensagens dos scripts da Jangada na página (as classes e ids para os filtros
        /// genéricos dos escudos) chegam pelo console; não aparecem no log.
        fn on_console_message(&self, browser: Option<&mut Browser>, _level: LogSeverity, message: Option<&CefString>, _source: Option<&CefString>, _line: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            let m = s(message);
            // O chrome.storage das extensões: a página pede pelo console e a Jangada responde.
            if let Some(resto) = m.strip_prefix(crate::extensoes::MARCA) {
                if let Ok(pedido) = serde_json::from_str::<serde_json::Value>(resto) {
                    let id = pedido["ext"].as_str().unwrap_or("").to_string();
                    let op = pedido["op"].as_str().unwrap_or("").to_string();
                    let k = pedido["k"].clone();
                    let resposta = crate::extensoes::armazenar(&id, &op, &pedido["d"]);
                    if let Some(f) = browser.and_then(|b| b.main_frame()) {
                        let js = format!("window.__jangadaResposta&&window.__jangadaResposta({k},{});", resposta);
                        f.execute_java_script(Some(&js.as_str().into()), None, 0);
                    }
                }
                return 1;
            }
            if let Some(pedido) = crate::senhas::ler(&m) {
                if let Some(f) = browser.and_then(|b| b.main_frame()) {
                    let origem = crate::importar::origem(&CefString::from(&f.url()).to_string());
                    match pedido {
                        crate::senhas::Pedido::Preencher => {
                            if let Some(js) = crate::senhas::script_preencher(&origem) {
                                f.execute_java_script(Some(&js.as_str().into()), None, 0);
                            }
                        }
                        crate::senhas::Pedido::Guardar(u, senha) => crate::senhas::enviado(&origem, &u, &senha),
                    }
                }
                return 1;
            }
            let Some(resto) = m.strip_prefix(crate::escudos::MARCA) else { return 0 };
            if let Some(b) = browser {
                if let (Some(js), Some(f)) = (crate::escudos::cosmetico_generico(b.identifier(), resto), b.main_frame()) {
                    f.execute_java_script(Some(&js.as_str().into()), None, 0);
                }
            }
            1
        }

        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|f| f.is_main() != 0) {
                if let Ok(mut p) = self.p.try_borrow_mut() {
                    let u = s(url);
                    // A página de erro da Jangada fica com o endereço que falhou.
                    if u.starts_with("data:text/html") && p.erro_url.is_some() {
                        p.url = p.erro_url.clone().unwrap_or_default();
                    } else {
                        p.erro_url = None;
                        p.url = u;
                    }
                    p.dirty = true;
                }
            }
        }

        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.title = s(title);
                p.dirty = true;
                // O histórico guarda a página com o título dela.
                if p.erro_url.is_none() {
                    crate::ajustes::visitar(&p.title, &p.url);
                }
            }
        }

        fn on_tooltip(&self, _browser: Option<&mut Browser>, text: Option<&mut CefString>) -> ::std::os::raw::c_int {
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.tooltip = text.map(|t| t.to_string()).unwrap_or_default();
                p.dirty = true;
            }
            1
        }

        fn on_cursor_change(&self, _browser: Option<&mut Browser>, _cursor: ::std::os::raw::c_ulong, type_: CursorType, _custom_cursor_info: Option<&CursorInfo>) -> ::std::os::raw::c_int {
            let c = if type_ == CursorType::HAND {
                glove_ui::Cursor::Hand
            } else if type_ == CursorType::IBEAM {
                glove_ui::Cursor::Text
            } else {
                glove_ui::Cursor::Arrow
            };
            if let Ok(mut p) = self.p.try_borrow_mut() {
                if p.cursor != c {
                    p.cursor = c;
                    p.dirty = true;
                }
            }
            1
        }

        /// O site disse onde está o ícone dele: baixa (no tamanho da aba).
        fn on_favicon_urlchange(&self, browser: Option<&mut Browser>, icon_urls: Option<&mut CefStringList>) {
            let Some(url) = icon_urls.and_then(primeira) else { return };
            let scale = {
                let Ok(mut p) = self.p.try_borrow_mut() else { return };
                if p.favicon_url == url {
                    return;
                }
                p.favicon_url = url.clone();
                p.scale
            };
            if let Some(h) = browser.and_then(|b| b.host()) {
                let mut cb = Favicon::new(self.p.clone(), url.clone());
                h.download_image(Some(&url.as_str().into()), 1, (16.0 * scale).ceil() as u32, 0, Some(&mut cb));
            }
        }

        fn on_status_message(&self, _browser: Option<&mut Browser>, value: Option<&CefString>) {
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.status = s(value);
                p.dirty = true;
            }
        }
    }
}

/// O primeiro endereço da lista (os ícones vêm do melhor para o pior).
fn primeira(list: &mut CefStringList) -> Option<String> {
    let raw: *mut cef::sys::_cef_string_list_t = list.into();
    if raw.is_null() {
        return None;
    }
    // Segurança: a lista é do motor e vale durante a chamada; o valor copiado é liberado aqui.
    unsafe {
        if cef::sys::cef_string_list_size(raw) == 0 {
            return None;
        }
        let mut v: cef::sys::cef_string_t = std::mem::zeroed();
        if cef::sys::cef_string_list_value(raw, 0, &mut v) == 0 {
            return None;
        }
        let s = CefString::from(std::ptr::from_ref(&v)).to_string();
        cef::sys::cef_string_utf16_clear(&mut v);
        Some(s).filter(|s| !s.is_empty())
    }
}

wrap_download_image_callback! {
    pub struct Favicon {
        p: Shared,
        url: String,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(&self, _image_url: Option<&CefString>, _http_status_code: ::std::os::raw::c_int, image: Option<&mut Image>) {
            let Some(img) = image else { return };
            let Ok(mut p) = self.p.try_borrow_mut() else { return };
            // Outro ícone pedido depois deste: fica o mais novo.
            if p.favicon_url != self.url {
                return;
            }
            let (mut w, mut h) = (0, 0);
            let Some(bin) = img.as_bitmap(p.scale, ColorType::RGBA_8888, AlphaType::PREMULTIPLIED, Some(&mut w), Some(&mut h)) else { return };
            let mut buf = vec![0u8; bin.size()];
            bin.data(Some(&mut buf), 0);
            if w > 0 && h > 0 && buf.len() == (w * h * 4) as usize {
                p.favicon = Some((buf, w, h));
                p.favicon_novo = true;
                p.dirty = true;
            }
        }
    }
}

wrap_load_handler! {
    pub struct Carga {
        p: Shared,
    }

    impl LoadHandler {
        fn on_load_error(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, error_code: Errorcode, error_text: Option<&CefString>, failed_url: Option<&CefString>) {
            // Parar o carregamento (ou trocar de página no meio) não é erro.
            if error_code == Errorcode::ABORTED || !frame.is_some_and(|f| f.is_main() != 0) {
                return;
            }
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.erro_pedido = Some((s(failed_url), s(error_text)));
                p.dirty = true;
            }
        }

        /// A página carregou: o estilo dos escudos (os anúncios do site escondidos).
        fn on_load_end(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, _http_status_code: ::std::os::raw::c_int) {
            let (Some(b), Some(f)) = (browser, frame) else { return };
            if f.is_main() == 0 {
                return;
            }
            let url = CefString::from(&f.url()).to_string();
            if let Some(js) = crate::escudos::script_da_pagina(b.identifier(), &url) {
                f.execute_java_script(Some(&js.as_str().into()), Some(&url.as_str().into()), 0);
            }
            // As senhas: preencher as guardadas e oferecer guardar as novas.
            if let Some(js) = crate::senhas::script_da_pagina(&url) {
                f.execute_java_script(Some(&js.as_str().into()), Some(&url.as_str().into()), 0);
            }
            // As extensões da Jangada que batem com a página (scripts e CSS de conteúdo).
            if let Some(js) = crate::extensoes::script_para(&url) {
                f.execute_java_script(Some(&js.as_str().into()), Some(&url.as_str().into()), 0);
            }
        }

        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: ::std::os::raw::c_int, can_go_back: ::std::os::raw::c_int, can_go_forward: ::std::os::raw::c_int) {
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.loading = is_loading != 0;
                p.back = can_go_back != 0;
                p.forward = can_go_forward != 0;
                p.dirty = true;
            }
        }
    }
}

wrap_life_span_handler! {
    pub struct Vida {
        p: Shared,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let (Some(b), Ok(mut p)) = (browser, self.p.try_borrow_mut()) {
                p.browser = Some(b.clone());
            }
        }

        /// Janela nova pedida pela página: vira uma aba.
        fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: ::std::os::raw::c_int, target_url: Option<&CefString>, _target_frame_name: Option<&CefString>, _target_disposition: WindowOpenDisposition, _user_gesture: ::std::os::raw::c_int, _popup_features: Option<&PopupFeatures>, _window_info: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            let url = s(target_url);
            // Janelas que a página abre sozinha (sem um clique) ficam bloqueadas, se ligado.
            let sozinha = _user_gesture == 0 && crate::ajustes::BLOQUEAR_JANELAS.load(std::sync::atomic::Ordering::Relaxed);
            if !url.is_empty() && !sozinha {
                NOVAS_ABAS.with(|n| n.borrow_mut().push((url, _target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB)));
            } else if sozinha {
                AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Janela que a página abriu sozinha foi bloqueada").to_string()));
            }
            1
        }
    }
}

wrap_context_menu_handler! {
    pub struct MenuPagina {
        p: Shared,
    }

    impl ContextMenuHandler {
        /// O menu do motor fica vazio: o Glove mostra o dele (solto, por cima de tudo).
        fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) {
            if let Some(m) = model {
                m.clear();
            }
            let Some(params) = params else { return };
            let st = |u: CefStringUserfree| CefString::from(&u).to_string();
            let c = Contexto {
                x: params.xcoord(),
                y: params.ycoord(),
                link: st(params.link_url()),
                selection: st(params.selection_text()),
                editable: params.is_editable() != 0,
                image: if params.has_image_contents() != 0 { st(params.source_url()) } else { String::new() },
            };
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.ctx = Some(c);
                p.dirty = true;
            }
        }
    }
}

wrap_download_handler! {
    pub struct Baixar {
        p: Shared,
    }

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _request_method: Option<&CefString>) -> ::std::os::raw::c_int {
            1
        }

        /// Vai direto para a pasta Downloads (sem perguntar), com um nome livre.
        fn on_before_download(&self, _browser: Option<&mut Browser>, _download_item: Option<&mut DownloadItem>, suggested_name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> ::std::os::raw::c_int {
            let Some(cb) = callback else { return 0 };
            let name = s(suggested_name);
            let name = if name.is_empty() { "download".to_string() } else { name.replace('/', "_") };
            let path = livre(&crate::pasta_downloads().join(&name));
            AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Baixando {}…", name)));
            cb.cont(Some(&path.to_string_lossy().as_ref().into()), 0);
            1
        }

        fn on_download_updated(&self, _browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, callback: Option<&mut DownloadItemCallback>) {
            let Some(d) = download_item else { return };
            let caminho = std::path::PathBuf::from(CefString::from(&d.full_path()).to_string());
            DOWNLOADS.with(|l| {
                let mut l = l.borrow_mut();
                let id = d.id();
                let i = match l.iter().position(|x| x.id == id) {
                    Some(i) => i,
                    None => {
                        l.push(Download { id, nome: String::new(), caminho: std::path::PathBuf::new(), recebido: 0, total: 0, velocidade: 0, pronto: false, cancelado: false, cb: None });
                        l.len() - 1
                    }
                };
                let x = &mut l[i];
                if !caminho.as_os_str().is_empty() {
                    x.nome = caminho.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    x.caminho = caminho.clone();
                }
                if x.nome.is_empty() {
                    x.nome = CefString::from(&d.suggested_file_name()).to_string();
                }
                x.recebido = d.received_bytes();
                x.total = d.total_bytes();
                x.velocidade = d.current_speed();
                x.pronto = d.is_complete() != 0;
                x.cancelado = d.is_canceled() != 0 || d.is_interrupted() != 0;
                x.cb = if x.pronto || x.cancelado { None } else { callback.map(|c| c.clone()) };
            });
            DOWNLOADS_MUDOU.with(|m| m.set(true));
            if d.is_complete() != 0 {
                let path = CefString::from(&d.full_path()).to_string();
                let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path);
                AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Baixado em Downloads: {}", name)));
                if let Ok(mut p) = self.p.try_borrow_mut() {
                    p.dirty = true;
                }
            }
        }
    }
}

wrap_jsdialog_handler! {
    pub struct Dialogo {
        p: Shared,
    }

    impl JsdialogHandler {
        /// alert() e confirm() viram alertas do Glove (em janela própria); prompt() é recusado.
        fn on_jsdialog(&self, _browser: Option<&mut Browser>, _origin_url: Option<&CefString>, dialog_type: JsdialogType, message_text: Option<&CefString>, _default_prompt_text: Option<&CefString>, callback: Option<&mut JsdialogCallback>, suppress_message: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            let Some(cb) = callback else { return 0 };
            if dialog_type == JsdialogType::PROMPT {
                if let Some(sup) = suppress_message {
                    *sup = 1;
                }
                return 0;
            }
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.dialog = Some((dialog_type == JsdialogType::CONFIRM, s(message_text), cb.clone()));
                p.dirty = true;
                return 1;
            }
            0
        }

        fn on_before_unload_dialog(&self, _browser: Option<&mut Browser>, _message_text: Option<&CefString>, _is_reload: ::std::os::raw::c_int, callback: Option<&mut JsdialogCallback>) -> ::std::os::raw::c_int {
            // Sair da página sempre pode (como fechar a aba).
            if let Some(cb) = callback {
                cb.cont(1, None);
                return 1;
            }
            0
        }
    }
}

wrap_find_handler! {
    pub struct Achar {
        p: Shared,
    }

    impl FindHandler {
        fn on_find_result(&self, _browser: Option<&mut Browser>, _identifier: ::std::os::raw::c_int, count: ::std::os::raw::c_int, _selection_rect: Option<&cef::Rect>, active_match_ordinal: ::std::os::raw::c_int, _final_update: ::std::os::raw::c_int) {
            if let Ok(mut p) = self.p.try_borrow_mut() {
                p.achados = (active_match_ordinal, count);
                p.dirty = true;
            }
        }
    }
}

// O bloqueador e o "Não Rastrear" (rodam fora da linha principal: nada de `Shared` aqui).
wrap_resource_request_handler! {
    pub struct Filtro;

    impl ResourceRequestHandler {
        fn on_before_resource_load(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            let Some(r) = request else { return ReturnValue::CONTINUE };
            // Os escudos (anúncios e rastreadores), por aba.
            let aba = browser.map(|b| b.identifier()).unwrap_or(0);
            let url = CefString::from(&r.url()).to_string();
            if crate::escudos::barrar(aba, &url, tipo_de_recurso(r.resource_type())) {
                return ReturnValue::CANCEL;
            }
            if crate::ajustes::NAO_RASTREAR.load(std::sync::atomic::Ordering::Relaxed) {
                r.set_header_by_name(Some(&"DNT".into()), Some(&"1".into()), 1);
                r.set_header_by_name(Some(&"Sec-GPC".into()), Some(&"1".into()), 1);
            }
            ReturnValue::CONTINUE
        }
    }
}

/// O tipo do pedido no formato do motor de bloqueio.
fn tipo_de_recurso(t: ResourceType) -> &'static str {
    match t {
        ResourceType::MAIN_FRAME => "document",
        ResourceType::SUB_FRAME => "subdocument",
        ResourceType::STYLESHEET => "stylesheet",
        ResourceType::SCRIPT => "script",
        ResourceType::IMAGE | ResourceType::FAVICON => "image",
        ResourceType::FONT_RESOURCE => "font",
        ResourceType::MEDIA => "media",
        ResourceType::XHR => "xmlhttprequest",
        ResourceType::PING | ResourceType::CSP_REPORT => "ping",
        ResourceType::OBJECT | ResourceType::PLUGIN_RESOURCE => "object",
        _ => "other",
    }
}

wrap_request_handler! {
    pub struct Pedidos;

    impl RequestHandler {
        /// Botão do meio ou Ctrl + clique num link: aba nova (por trás, como nos outros
        /// navegadores), não na mesma.
        fn on_open_urlfrom_tab(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, target_url: Option<&CefString>, target_disposition: WindowOpenDisposition, _user_gesture: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            let url = s(target_url);
            let fundo = target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB;
            let nova = fundo || target_disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB || target_disposition == WindowOpenDisposition::NEW_WINDOW;
            if nova && !url.is_empty() {
                NOVAS_ABAS.with(|n| n.borrow_mut().push((url, fundo)));
                return 1;
            }
            0
        }

        fn resource_request_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _is_navigation: ::std::os::raw::c_int, _is_download: ::std::os::raw::c_int, _request_initiator: Option<&CefString>, _disable_default_handling: Option<&mut ::std::os::raw::c_int>) -> Option<ResourceRequestHandler> {
            Some(Filtro::new())
        }
    }
}

wrap_pdf_print_callback! {
    pub struct Pdf {
        imprimir: bool,
    }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(&self, path: Option<&CefString>, ok: ::std::os::raw::c_int) {
            let nome = s(path);
            if self.imprimir {
                if ok != 0 {
                    IMPRIMIR.with(|i| *i.borrow_mut() = Some(std::path::PathBuf::from(&nome)));
                } else {
                    AVISOS.with(|a| a.borrow_mut().push(glove_config::tr!("Não foi possível preparar a impressão.").to_string()));
                }
                return;
            }
            let nome = std::path::Path::new(&nome).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(nome);
            AVISOS.with(|a| a.borrow_mut().push(if ok != 0 { glove_config::tr!("PDF salvo em Downloads: {}", nome) } else { glove_config::tr!("Não foi possível salvar o PDF.").to_string() }));
        }
    }
}

/// Salva a página como PDF na pasta Downloads.
pub fn salvar_pdf(host: &BrowserHost, titulo: &str) {
    let nome: String = titulo.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let nome = if nome.trim().is_empty() { "pagina".to_string() } else { nome.trim().chars().take(80).collect() };
    let path = livre(&crate::pasta_downloads().join(format!("{nome}.pdf")));
    let settings = PdfPrintSettings { print_background: 1, ..Default::default() };
    host.print_to_pdf(Some(&path.to_string_lossy().as_ref().into()), Some(&settings), Some(&mut Pdf::new(false)));
}

/// Imprimir: a página vira um PDF (A4, com os fundos) e a folha Imprimir do Glove o
/// manda para a impressora (CUPS).
pub fn imprimir(host: &BrowserHost, pasta: &std::path::Path) {
    let path = pasta.join("imprimir.pdf");
    let _ = std::fs::remove_file(&path);
    let settings = PdfPrintSettings { print_background: 1, ..Default::default() };
    host.print_to_pdf(Some(&path.to_string_lossy().as_ref().into()), Some(&settings), Some(&mut Pdf::new(true)));
}

/// Um pedido de permissão de um site, esperando a escolha da pessoa.
pub enum Permissao {
    /// Câmera e microfone (getUserMedia): o pedido do motor e os bits pedidos.
    Midia(MediaAccessCallback, u32),
    /// Localização e notificações (o aviso do motor): o pedido e os bits.
    Aviso(PermissionPromptCallback),
}

thread_local! {
    /// Os pedidos de permissão à espera da janela de perguntas (um por vez).
    pub static PERMISSOES: RefCell<Vec<(String, Vec<&'static str>, Permissao)>> = const { RefCell::new(Vec::new()) };
}

/// Os nomes que a pessoa lê para cada permissão (a ordem é a da pergunta).
/// As chaves de cada permissão (guardadas em permissoes.tsv): camera, microfone,
/// localizacao, notificacoes.
fn nomes_midia(bits: u32) -> Vec<&'static str> {
    let mut v = Vec::new();
    if bits & (1 << 0) != 0 {
        v.push("microfone");
    }
    if bits & (1 << 1) != 0 {
        v.push("camera");
    }
    // Captura de tela e de áudio do computador: a Jangada não concede.
    v
}

fn nomes_aviso(bits: u32) -> Vec<&'static str> {
    let mut v = Vec::new();
    if bits & (1 << 8) != 0 {
        v.push("localizacao");
    }
    if bits & (1 << 15) != 0 {
        v.push("notificacoes");
    }
    v
}

/// O nome de uma permissão numa frase: com o artigo ("a câmera", "o microfone").
pub fn rotulo_permissao(chave: &str) -> &'static str {
    match chave {
        "camera" => "a câmera",
        "microfone" => "o microfone",
        "localizacao" => "a localização",
        _ => "as notificações",
    }
}

wrap_permission_handler! {
    pub struct Permissoes {
        p: Shared,
    }

    impl PermissionHandler {
        /// Câmera e microfone: o que foi guardado para o site, ou a pergunta na janela.
        fn on_request_media_access_permission(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut MediaAccessCallback>) -> ::std::os::raw::c_int {
            let Some(cb) = callback else { return 0 };
            let origem = crate::importar::origem(&s(requesting_origin));
            let nomes = nomes_midia(requested_permissions);
            if nomes.is_empty() {
                cb.cancel();
                return 1;
            }
            // Os dois precisam estar liberados (ou não ter sido decidido).
            let liberado = |k: &str| crate::ajustes::permissao(&origem, k);
            let pedidos: Vec<&str> = nomes.clone();
            if pedidos.iter().all(|k| liberado(k) == Some(true)) {
                cb.cont(requested_permissions & 0x3);
                return 1;
            }
            if pedidos.iter().any(|k| liberado(k) == Some(false)) {
                cb.cancel();
                return 1;
            }
            PERMISSOES.with(|p| p.borrow_mut().push((origem, nomes, Permissao::Midia(cb.clone(), requested_permissions & 0x3))));
            1
        }

        /// Localização e notificações: o aviso do motor (a pergunta é a da Jangada).
        fn on_show_permission_prompt(&self, _browser: Option<&mut Browser>, _prompt_id: u64, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut PermissionPromptCallback>) -> ::std::os::raw::c_int {
            let Some(cb) = callback else { return 0 };
            let origem = crate::importar::origem(&s(requesting_origin));
            let nomes = nomes_aviso(requested_permissions);
            if nomes.is_empty() {
                cb.cont(PermissionRequestResult::DENY);
                return 1;
            }
            let chave = nomes[0];
            match crate::ajustes::permissao(&origem, chave) {
                Some(true) => cb.cont(PermissionRequestResult::ACCEPT),
                Some(false) => cb.cont(PermissionRequestResult::DENY),
                None => PERMISSOES.with(|p| p.borrow_mut().push((origem, nomes, Permissao::Aviso(cb.clone())))),
            }
            1
        }
    }
}

wrap_client! {
    pub struct Cliente {
        desenho: RenderHandler,
        exibicao: DisplayHandler,
        carga: LoadHandler,
        vida: LifeSpanHandler,
        menu: ContextMenuHandler,
        baixar: DownloadHandler,
        dialogo: JsdialogHandler,
        achar: FindHandler,
        pedidos: RequestHandler,
        permissoes: PermissionHandler,
    }

    impl Client {
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(self.permissoes.clone())
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(self.achar.clone())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.pedidos.clone())
        }
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.desenho.clone())
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.exibicao.clone())
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.carga.clone())
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.vida.clone())
        }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.menu.clone())
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(self.baixar.clone())
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(self.dialogo.clone())
        }
    }
}

/// Um nome que ainda não existe ("arquivo (2).zip").
fn livre(path: &std::path::Path) -> std::path::PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..1000).map(|n| path.with_file_name(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or_else(|| path.to_path_buf())
}

/// Abre uma aba nova no motor (o navegador chega depois, em `on_after_created`).
pub fn nova_aba(p: &Shared, url: &str) -> bool {
    let window_info = WindowInfo { windowless_rendering_enabled: 1, runtime_style: RuntimeStyle::ALLOY, ..Default::default() };
    let settings = BrowserSettings { windowless_frame_rate: 60, background_color: 0xffff_ffff, ..Default::default() };
    let mut client = Cliente::new(Desenho::new(p.clone()), Exibicao::new(p.clone()), Carga::new(p.clone()), Vida::new(p.clone()), MenuPagina::new(p.clone()), Baixar::new(p.clone()), Dialogo::new(p.clone()), Achar::new(p.clone()), Pedidos::new(), Permissoes::new(p.clone()));
    browser_host_create_browser(Some(&window_info), Some(&mut client), Some(&url.into()), Some(&settings), None, None) == 1
}
