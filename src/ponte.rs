//! A ponte entre as extensões e a Jangada: os pedidos que os scripts de conteúdo e as
//! páginas das extensões mandam (dados, mensagens, abas, o botão da barra), as respostas,
//! os avisos de mudança nos dados e a janelinha aberta.
//!
//! Quem pede é sempre conhecido pela Jangada, nunca pelo que vem na mensagem: um script de
//! conteúdo fala pelo canal do mundo da própria extensão; uma página da extensão, pelo
//! canal das páginas, e a extensão é a do endereço dela (jangada-extension://ID/...).
//!
//! O que só a janela pode fazer (abrir, trocar e fechar abas, abrir a janelinha) fica em
//! `PEDIDOS`, e a janela atende no quadro seguinte.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use glove_ui::gtk::{gio, glib, prelude::*};
use serde_json::{json, Value};
use webkit6::prelude::*;
use webkit6::WebView;

use crate::extensoes::{self, Extensao};
use crate::motor;

/// A resposta de quando ninguém recebe uma mensagem (a mesma do Chrome).
const SEM_RECEPTOR: &str = "Could not establish connection. Receiving end does not exist.";

/// De onde veio um pedido.
#[derive(Clone)]
pub enum Origem {
    /// Um script de conteúdo numa aba (no mundo isolado da extensão).
    Conteudo { view: WebView, ext: String },
    /// Uma página da extensão (a janelinha, as opções numa aba), no mundo principal.
    Pagina { view: WebView, ext: String },
}

impl Origem {
    fn view(&self) -> &WebView {
        match self {
            Origem::Conteudo { view, .. } | Origem::Pagina { view, .. } => view,
        }
    }

    fn ext(&self) -> &str {
        match self {
            Origem::Conteudo { ext, .. } | Origem::Pagina { ext, .. } => ext,
        }
    }

    fn mundo(&self) -> Option<String> {
        match self {
            Origem::Conteudo { ext, .. } => Some(extensoes::mundo_de(ext)),
            Origem::Pagina { .. } => None,
        }
    }
}

/// Para onde vai uma resposta: a página que pediu, o mundo e o número do pedido.
#[derive(Clone)]
pub struct Volta {
    view: glib::WeakRef<WebView>,
    mundo: Option<String>,
    k: u64,
}

impl Volta {
    fn de(o: &Origem, k: u64) -> Self {
        Volta { view: o.view().downgrade(), mundo: o.mundo(), k }
    }

    /// Responde ao pedido (se a página ainda existe e esperava resposta).
    pub fn responder(&self, r: Result<Value, String>) {
        if self.k == 0 {
            return;
        }
        let Some(v) = self.view.upgrade() else { return };
        let corpo = match r {
            Ok(v) => json!({ "v": v }),
            Err(e) => json!({ "e": e }),
        };
        let js = format!("window.__jangadaResposta&&window.__jangadaResposta({},{});", self.k, extensoes::js_valor(&corpo));
        v.evaluate_javascript(&js, self.mundo.as_deref(), None, None::<&gio::Cancellable>, |_| {});
    }
}

/// O estado do botão de uma extensão, como os scripts deixaram (volta ao do manifesto
/// quando a Jangada reabre, como no Chrome).
#[derive(Clone, Debug, Default)]
pub struct EstadoAcao {
    pub selo: String,
    pub cor: Option<String>,
    pub cor_texto: Option<String>,
    pub titulo: Option<String>,
    /// A página da janelinha trocada por `setPopup` ("" = sem janelinha).
    pub popup: Option<String>,
    pub desligado: bool,
}

/// A janelinha aberta: a extensão, a página e o tamanho que ela mediu de si mesma.
pub struct Popup {
    pub ext: String,
    pub view: WebView,
    pub tamanho: Option<(f32, f32)>,
    /// Pediu para fechar (window.close(), Esc, clique fora).
    pub fechar: bool,
    /// Já ficou com o teclado (uma vez, quando aparece).
    pub focada: bool,
    pub desde: Instant,
}

/// O que só a janela faz, pedido pelas extensões.
pub enum Pedido {
    AbrirAba { url: String, ativa: bool, volta: Option<Volta> },
    AtualizarAba { aba: u32, url: Option<String>, ativa: bool, volta: Option<Volta> },
    FecharAbas { abas: Vec<u32>, volta: Option<Volta> },
    AbrirPopup { ext: String, volta: Option<Volta> },
}

struct Espera {
    volta: Volta,
    faltam: usize,
    ouvinte: bool,
}

thread_local! {
    pub static ACOES: RefCell<HashMap<String, EstadoAcao>> = RefCell::new(HashMap::new());
    pub static POPUP: RefCell<Option<Popup>> = const { RefCell::new(None) };
    pub static PEDIDOS: RefCell<Vec<Pedido>> = const { RefCell::new(Vec::new()) };
    /// A aba que cada extensão com "activeTab" pode ver por ter sido chamada nela (o
    /// clique no botão dela), como no Chrome.
    static ABA_CONCEDIDA: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
    static ESPERAS: RefCell<HashMap<u64, Espera>> = RefCell::new(HashMap::new());
    static PROXIMA: Cell<u64> = const { Cell::new(1) };
}

fn pedir_a_janela(p: Pedido) {
    PEDIDOS.with(|v| v.borrow_mut().push(p));
    glove_ui::redraw_soon();
}

/// O estado do botão de uma extensão.
pub fn estado(id: &str) -> EstadoAcao {
    ACOES.with(|a| a.borrow().get(id).cloned().unwrap_or_default())
}

/// A página da janelinha de uma extensão agora (a do `setPopup`, ou a do manifesto).
pub fn popup_de(e: &Extensao) -> Option<String> {
    match estado(&e.id).popup {
        Some(p) if p.is_empty() => None,
        Some(p) => Some(p),
        None => e.acao.as_ref().and_then(|a| a.popup.clone()),
    }
}

/// A extensão com "activeTab" foi chamada pela pessoa nesta aba: pode vê-la.
pub fn conceder_aba(ext: &str, aba: u32) {
    ABA_CONCEDIDA.with(|c| {
        c.borrow_mut().insert(ext.to_string(), aba);
    });
}

/// A janelinha aberta é desta página?
pub fn eh_popup(view: &WebView) -> bool {
    POPUP.with(|p| p.borrow().as_ref().is_some_and(|p| &p.view == view))
}

// ---------------------------------------------------------------------------
// As abas, como as extensões as veem
// ---------------------------------------------------------------------------

/// As abas na ordem da faixa (as que a janela ainda não pôs na faixa vêm no fim).
fn abas() -> Vec<motor::Shared> {
    let (ordem, _) = motor::ordem();
    let mut todas = motor::abas_abertas();
    todas.sort_by_key(|p| ordem.iter().position(|i| *i == p.borrow().id).unwrap_or(usize::MAX));
    todas
}

fn aba_por_id(id: u32) -> Option<motor::Shared> {
    motor::abas_abertas().into_iter().find(|p| p.borrow().id == id)
}

fn aba_da_view(v: &WebView) -> Option<motor::Shared> {
    motor::abas_abertas().into_iter().find(|p| &p.borrow().view == v)
}

/// A extensão pode ver o endereço e o título desta aba? Com a permissão "tabs", com a
/// permissão do site, se a página for dela, ou com "activeTab" na aba em que foi chamada.
fn pode_ver(e: &Extensao, id_aba: u32, url: &str) -> bool {
    e.permissoes.iter().any(|p| p == "tabs")
        || e.hosts.iter().any(|h| extensoes::casa(h, url))
        || e.regras.iter().any(|r| r.matches.iter().any(|m| extensoes::casa(m, url)))
        || extensoes::partes_da_url(url).is_some_and(|(id, _)| id == e.id)
        || (e.permissoes.iter().any(|p| p == "activeTab") && ABA_CONCEDIDA.with(|c| c.borrow().get(&e.id) == Some(&id_aba)))
}

/// Uma aba no formato do `tabs.Tab` do Chrome.
fn info(e: &Extensao, p: &motor::Shared, indice: usize, ativa: u32) -> Value {
    let p = p.borrow();
    let mut o = json!({
        "id": p.id,
        "index": indice,
        "windowId": 1,
        "active": p.id == ativa,
        "highlighted": p.id == ativa,
        "selected": p.id == ativa,
        "pinned": false,
        "incognito": crate::ajustes::privada(),
        "status": if p.loading { "loading" } else { "complete" },
        "discarded": p.adiada,
        "autoDiscardable": true,
        "audible": false,
        "mutedInfo": { "muted": false },
        "groupId": -1,
    });
    if pode_ver(e, p.id, &p.url) {
        o["url"] = json!(p.url);
        o["title"] = json!(p.title);
    }
    o
}

fn info_por_id(e: &Extensao, id: u32) -> Option<Value> {
    let (_, ativa) = motor::ordem();
    abas().iter().enumerate().find(|(_, p)| p.borrow().id == id).map(|(i, p)| info(e, p, i, ativa))
}

/// `tabs.query`: as abas que batem com o pedido (active, url, title, status, index,
/// windowId; currentWindow e lastFocusedWindow valem sempre, a Jangada tem uma janela).
fn consultar(e: &Extensao, q: &Value) -> Vec<Value> {
    let (_, ativa) = motor::ordem();
    let padroes = |v: &Value| -> Vec<String> {
        match v {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            _ => vec![],
        }
    };
    let urls = padroes(&q["url"]);
    let titulo = q["title"].as_str().map(str::to_string);
    let mut r = Vec::new();
    for (i, p) in abas().iter().enumerate() {
        let t = info(e, p, i, ativa);
        if let Some(a) = q["active"].as_bool() {
            if t["active"].as_bool() != Some(a) {
                continue;
            }
        }
        if let Some(w) = q["windowId"].as_i64() {
            // -2: a janela atual.
            if w != 1 && w != -2 {
                continue;
            }
        }
        if let Some(s) = q["status"].as_str() {
            if t["status"] != s {
                continue;
            }
        }
        if let Some(n) = q["index"].as_u64() {
            if n as usize != i {
                continue;
            }
        }
        if !urls.is_empty() {
            // Sem poder ver o endereço, a aba não entra numa busca por endereço.
            let Some(u) = t["url"].as_str() else { continue };
            if !urls.iter().any(|pd| extensoes::casa(pd, u)) {
                continue;
            }
        }
        if let Some(tp) = &titulo {
            let Some(tt) = t["title"].as_str() else { continue };
            if !extensoes::glob(tp.as_bytes(), tt.as_bytes()) {
                continue;
            }
        }
        r.push(t);
    }
    r
}

/// Quem mandou uma mensagem, no formato do `MessageSender` do Chrome.
fn remetente(o: &Origem, e: &Extensao) -> Value {
    let url = o.view().uri().map(|u| u.to_string()).unwrap_or_default();
    let mut r = json!({ "id": e.id, "url": url, "origin": crate::importar::origem(&url) });
    if let Origem::Conteudo { .. } = o {
        r["frameId"] = json!(0);
        r["documentLifecycle"] = json!("active");
    }
    if let Some(p) = aba_da_view(o.view()) {
        let id = p.borrow().id;
        if let Some(t) = info_por_id(e, id) {
            r["tab"] = t;
        }
    }
    r
}

// ---------------------------------------------------------------------------
// Os pedidos
// ---------------------------------------------------------------------------

/// Um pedido de uma extensão: {op, k, d}. `k` é o número da resposta (0: sem resposta).
pub fn pedido(origem: Origem, texto: &str) {
    let Ok(p) = serde_json::from_str::<Value>(texto) else { return };
    let op = p["op"].as_str().unwrap_or("");
    let k = p["k"].as_u64().unwrap_or(0);
    let d = &p["d"];
    // Uma extensão desligada (ou removida) com a página ainda aberta não pede mais nada.
    let Some(e) = extensoes::ativa(origem.ext()) else { return };
    let volta = Volta::de(&origem, k);
    match op {
        "storage.get" | "storage.set" | "storage.remove" | "storage.clear" | "storage.bytes" => {
            let area = if d["area"] == "sync" { "sync" } else { "local" };
            let (r, mudancas) = extensoes::armazenar_e_mudancas(&e.id, &op["storage.".len()..], &d["c"]);
            volta.responder(Ok(r));
            if !mudancas.is_empty() {
                avisar_mudancas(&e.id, area, &Value::Object(mudancas));
            }
        }
        "runtime.sendMessage" => {
            let alvos = paginas_da_extensao(&e.id).into_iter().filter(|v| v != origem.view()).map(|v| (v, None)).collect();
            entregar(volta, alvos, d.clone(), remetente(&origem, &e));
        }
        "tabs.sendMessage" => {
            let alvos = d["tab"].as_u64().and_then(|t| aba_por_id(t as u32)).filter(|p| p.borrow().extensoes.contains(&e.id)).map(|p| p.borrow().view.clone());
            entregar(volta, alvos.into_iter().map(|v| (v, Some(extensoes::mundo_de(&e.id)))).collect(), d["msg"].clone(), remetente(&origem, &e));
        }
        "msg.resposta" => resposta_de_mensagem(d),
        "tabs.query" => volta.responder(Ok(Value::Array(consultar(&e, d)))),
        "tabs.get" => {
            let r = d.as_u64().and_then(|t| info_por_id(&e, t as u32)).ok_or_else(|| format!("No tab with id: {d}."));
            volta.responder(r);
        }
        "tabs.getCurrent" => {
            let id = aba_da_view(origem.view()).map(|p| p.borrow().id);
            volta.responder(Ok(id.and_then(|i| info_por_id(&e, i)).unwrap_or(Value::Null)));
        }
        "tabs.create" => {
            let url = d["url"].as_str().unwrap_or("").to_string();
            if !url_permitida(&e, &url) {
                volta.responder(Err(format!("Cannot navigate to {url}")));
                return;
            }
            pedir_a_janela(Pedido::AbrirAba { url, ativa: d["active"].as_bool() != Some(false), volta: Some(volta) });
        }
        "tabs.update" => {
            let aba = d["tab"].as_u64().map(|t| t as u32).unwrap_or_else(|| motor::ordem().1);
            let url = d["p"]["url"].as_str().map(str::to_string);
            if let Some(u) = &url {
                if !url_permitida(&e, u) {
                    volta.responder(Err(format!("Cannot navigate to {u}")));
                    return;
                }
            }
            if aba_por_id(aba).is_none() {
                volta.responder(Err(format!("No tab with id: {aba}.")));
                return;
            }
            pedir_a_janela(Pedido::AtualizarAba { aba, url, ativa: d["p"]["active"].as_bool() == Some(true), volta: Some(volta) });
        }
        "tabs.reload" => {
            let aba = d.as_u64().map(|t| t as u32).unwrap_or_else(|| motor::ordem().1);
            match aba_por_id(aba) {
                Some(p) => {
                    let v = p.borrow().view.clone();
                    v.reload();
                    volta.responder(Ok(Value::Null));
                }
                None => volta.responder(Err(format!("No tab with id: {aba}."))),
            }
        }
        "tabs.remove" => {
            let abas: Vec<u32> = d.as_array().into_iter().flatten().filter_map(|t| t.as_u64().map(|t| t as u32)).collect();
            pedir_a_janela(Pedido::FecharAbas { abas, volta: Some(volta) });
        }
        "runtime.openOptionsPage" => match &e.opcoes {
            Some(o) => pedir_a_janela(Pedido::AbrirAba { url: e.url(o), ativa: true, volta: Some(volta) }),
            None => volta.responder(Err("Could not create an options page.".into())),
        },
        "action.set" => {
            ACOES.with(|a| {
                let mut a = a.borrow_mut();
                let s = a.entry(e.id.clone()).or_default();
                if let Some(t) = d["selo"].as_str() {
                    s.selo = t.chars().take(4).collect();
                }
                if let Some(c) = d["cor"].as_str() {
                    s.cor = Some(c.to_string()).filter(|c| !c.is_empty());
                }
                if let Some(c) = d["corTexto"].as_str() {
                    s.cor_texto = Some(c.to_string()).filter(|c| !c.is_empty());
                }
                if let Some(t) = d["titulo"].as_str() {
                    s.titulo = Some(t.to_string()).filter(|t| !t.is_empty());
                }
                if let Some(p) = d["popup"].as_str() {
                    s.popup = Some(p.trim_start_matches('/').to_string());
                }
                if let Some(l) = d["ligado"].as_bool() {
                    s.desligado = !l;
                }
            });
            glove_ui::redraw_soon();
            volta.responder(Ok(Value::Null));
        }
        "action.get" => {
            let s = estado(&e.id);
            let r = match d.as_str().unwrap_or("") {
                "selo" => json!(s.selo),
                "cor" => json!(s.cor.unwrap_or_default()),
                "titulo" => json!(s.titulo.unwrap_or_else(|| e.titulo_da_acao())),
                "popup" => json!(popup_de(&e).map(|p| e.url(&p)).unwrap_or_default()),
                _ => Value::Null,
            };
            volta.responder(Ok(r));
        }
        "action.openPopup" => {
            if popup_de(&e).is_some() {
                pedir_a_janela(Pedido::AbrirPopup { ext: e.id.clone(), volta: Some(volta) });
            } else {
                volta.responder(Err("Extension does not have a popup on the active tab.".into()));
            }
        }
        "janela.tamanho" => {
            // Só a janelinha mede a si mesma para a Jangada.
            let (w, h) = (d["w"].as_f64().unwrap_or(0.0) as f32, d["h"].as_f64().unwrap_or(0.0) as f32);
            POPUP.with(|p| {
                if let Some(p) = p.borrow_mut().as_mut().filter(|p| &p.view == origem.view()) {
                    p.tamanho = Some((w.clamp(25.0, 800.0), h.clamp(25.0, 600.0)));
                }
            });
            glove_ui::redraw_soon();
        }
        "janela.fechar" => {
            if eh_popup(origem.view()) {
                fechar_popup_pedido();
            } else if let Some(p) = aba_da_view(origem.view()) {
                let id = p.borrow().id;
                pedir_a_janela(Pedido::FecharAbas { abas: vec![id], volta: None });
            }
        }
        _ => volta.responder(Err(format!("{op}: a Jangada ainda não tem essa função"))),
    }
}

/// A extensão pode abrir este endereço numa aba? Os sites (http e https), as páginas dela,
/// e as páginas de outra só se forem acessíveis pela web.
fn url_permitida(e: &Extensao, url: &str) -> bool {
    if url.is_empty() || url.starts_with("http://") || url.starts_with("https://") {
        return true;
    }
    match extensoes::partes_da_url(url) {
        Some((id, _)) if id == e.id => true,
        Some((id, caminho)) => extensoes::ativa(&id).is_some_and(|o| o.acessivel(&caminho)),
        None => false,
    }
}

/// Pede para a janelinha fechar (a janela a tira no quadro seguinte).
pub fn fechar_popup_pedido() {
    POPUP.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            p.fechar = true;
        }
    });
    glove_ui::redraw_soon();
}

// ---------------------------------------------------------------------------
// Mensagens e avisos
// ---------------------------------------------------------------------------

/// As páginas abertas de uma extensão: a janelinha e as abas com páginas dela.
fn paginas_da_extensao(id: &str) -> Vec<WebView> {
    let mut v: Vec<WebView> = POPUP.with(|p| p.borrow().as_ref().filter(|p| p.ext == id).map(|p| p.view.clone())).into_iter().collect();
    for p in motor::abas_abertas() {
        let p = p.borrow();
        if extensoes::partes_da_url(&p.url).is_some_and(|(e, _)| e == id) {
            v.push(p.view.clone());
        }
    }
    v
}

fn proxima() -> u64 {
    PROXIMA.with(|p| {
        let n = p.get();
        p.set(n + 1);
        n
    })
}

/// Entrega uma mensagem a cada alvo (página, mundo); a primeira resposta volta para quem
/// mandou. Sem ouvinte em nenhum, quem mandou recebe o erro do Chrome.
fn entregar(volta: Volta, alvos: Vec<(WebView, Option<String>)>, msg: Value, remetente: Value) {
    if alvos.is_empty() {
        volta.responder(Err(SEM_RECEPTOR.into()));
        return;
    }
    let rid = proxima();
    ESPERAS.with(|e| e.borrow_mut().insert(rid, Espera { volta, faltam: alvos.len(), ouvinte: false }));
    let js = format!(
        "(window.__jangadaEntrega?(window.__jangadaEntrega({rid},{},{}),1):0)",
        extensoes::js_valor(&msg),
        extensoes::js_valor(&remetente)
    );
    for (v, mundo) in alvos {
        v.evaluate_javascript(&js, mundo.as_deref(), None, None::<&gio::Cancellable>, move |r| {
            // A página ainda não montou a API (ou fechou): conta como sem ouvinte.
            let entregue = r.ok().is_some_and(|v| v.to_str() == "1");
            if !entregue {
                resposta_de_mensagem(&json!({ "rid": rid, "ouvinte": false }));
            }
        });
    }
}

/// A resposta de um alvo: {rid, ouvinte, tem, valor}.
fn resposta_de_mensagem(d: &Value) {
    let Some(rid) = d["rid"].as_u64() else { return };
    let pronta = ESPERAS.with(|e| {
        let mut e = e.borrow_mut();
        let es = e.get_mut(&rid)?;
        if d["tem"] == true {
            let es = e.remove(&rid)?;
            return Some((es.volta, Ok(d["valor"].clone())));
        }
        es.faltam = es.faltam.saturating_sub(1);
        es.ouvinte |= d["ouvinte"] == true;
        if es.faltam > 0 {
            return None;
        }
        let es = e.remove(&rid)?;
        Some((es.volta, if es.ouvinte { Ok(Value::Null) } else { Err(SEM_RECEPTOR.to_string()) }))
    });
    if let Some((volta, r)) = pronta {
        volta.responder(r);
    }
}

/// Os dados de uma extensão mudaram: avisa os scripts dela em todas as abas e as páginas
/// dela (o `storage.onChanged`).
fn avisar_mudancas(id: &str, area: &str, mudancas: &Value) {
    let js = format!("window.__jangadaMudou&&window.__jangadaMudou({},{});", extensoes::js_valor(mudancas), extensoes::js_str(area));
    let mundo = extensoes::mundo_de(id);
    for p in motor::abas_abertas() {
        let p = p.borrow();
        if p.extensoes.iter().any(|e| e == id) {
            p.view.evaluate_javascript(&js, Some(&mundo), None, None::<&gio::Cancellable>, |_| {});
        }
    }
    for v in paginas_da_extensao(id) {
        v.evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |_| {});
    }
}

/// O clique no botão de uma extensão sem janelinha: o `action.onClicked` dos scripts dela
/// na aba ativa.
pub fn clique_na_acao(e: &Extensao) {
    let (_, ativa) = motor::ordem();
    let Some(p) = aba_por_id(ativa) else { return };
    if !p.borrow().extensoes.contains(&e.id) {
        return;
    }
    let Some(t) = info_por_id(e, ativa) else { return };
    let v = p.borrow().view.clone();
    let js = format!("window.__jangadaClique&&window.__jangadaClique({});", extensoes::js_valor(&t));
    v.evaluate_javascript(&js, Some(&extensoes::mundo_de(&e.id)), None, None::<&gio::Cancellable>, |_| {});
}

/// A aba que a janela abriu (ou mudou) a pedido de uma extensão: a resposta com ela.
pub fn responder_aba(volta: Option<Volta>, id_aba: Option<u32>) {
    let Some(volta) = volta else { return };
    let Some(view) = volta.view.upgrade() else { return };
    // A extensão de quem pediu: pelo mundo (script de conteúdo) ou pelo endereço (página).
    let ext = match &volta.mundo {
        Some(m) => m.strip_prefix("jangada-x-").map(str::to_string),
        None => view.uri().and_then(|u| extensoes::partes_da_url(&u)).map(|(id, _)| id),
    };
    let r = match (ext.and_then(|id| extensoes::ativa(&id)), id_aba) {
        (Some(e), Some(i)) => info_por_id(&e, i).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    volta.responder(Ok(r));
}

/// Responde um pedido da janela que não devolve nada.
pub fn responder_vazio(volta: Option<Volta>, r: Result<(), String>) {
    if let Some(v) = volta {
        v.responder(r.map(|_| Value::Null));
    }
}

// ---------------------------------------------------------------------------
// A medida da janelinha
// ---------------------------------------------------------------------------

/// O script que mede a página da janelinha e manda o tamanho para a Jangada (como no
/// Chrome: a largura que o conteúdo pede, até 800 × 600). Mede de novo quando o conteúdo
/// muda.
pub const MEDIR_POPUP: &str = r#"(function () {
  if (location.protocol !== "jangada-extension:") return;
  var ultimo = "", pendente = false;
  function medir() {
    var h = document.documentElement, b = document.body;
    if (!h || !window.webkit || !webkit.messageHandlers.jangadaExt) return;
    var antes = h.style.width;
    h.style.width = "max-content";
    var w = Math.ceil(h.getBoundingClientRect().width);
    w = Math.min(800, Math.max(25, w));
    h.style.width = w + "px";
    var alt = Math.ceil(Math.max(h.getBoundingClientRect().height, b ? b.scrollHeight : 0));
    h.style.width = antes;
    var s = w + "x" + alt;
    if (s === ultimo) return;
    ultimo = s;
    webkit.messageHandlers.jangadaExt.postMessage(JSON.stringify({ op: "janela.tamanho", k: 0, d: { w: w, h: alt } }));
  }
  function agendar(regs) {
    // As mudanças que a própria medida faz no estilo do <html> não contam.
    if (regs && regs.every && regs.every(function (r) { return r.type === "attributes" && r.target === document.documentElement; })) return;
    if (pendente) return;
    pendente = true;
    setTimeout(function () { pendente = false; medir(); }, 16);
  }
  function comecar() {
    medir();
    new MutationObserver(agendar).observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
    if (window.ResizeObserver && document.body) new ResizeObserver(function () { agendar(); }).observe(document.body);
    [100, 400, 1000].forEach(function (t) { setTimeout(medir, t); });
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", comecar);
  else comecar();
  window.addEventListener("load", medir);
})();"#;
