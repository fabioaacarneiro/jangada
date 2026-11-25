//! Arquivos para a página: colar uma imagem (uma captura de tela, uma figura copiada) ou
//! arquivos copiados, e soltar arquivos arrastados de fora (da mesa, do Arquivos).
//!
//! O WebKitGTK não entrega isso à página: no `paste` e no `drop` ela só vê
//! `text/uri-list` (vazio) e nenhum arquivo, e então o Telegram, o WhatsApp, o Gmail etc.
//! não recebem nada. Aqui a Jangada lê a área de transferência (ou os arquivos soltos),
//! monta os `File` e manda à página o mesmo evento que os outros navegadores mandam.

use std::path::{Path, PathBuf};

use glove_ui::gtk::{self, gdk, gio, glib, prelude::*};
use webkit6::WebView;
use webkit6::prelude::*;

/// Acima disso (somando tudo) não vai: os bytes atravessam como texto até a página.
const LIMITE: u64 = 512 * 1024 * 1024;

/// Os tipos de imagem que a área de transferência pode trazer, na ordem de preferência.
const IMAGENS: [&str; 5] = ["image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp"];

/// Um arquivo pronto para a página: nome, tipo e conteúdo.
struct Arquivo {
    nome: String,
    tipo: String,
    dados: Vec<u8>,
}

/// O tipo de um arquivo pela extensão (a página usa para decidir: foto, vídeo, documento).
fn tipo_de(p: &Path) -> &'static str {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "avif" => "image/avif",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        _ => "application/octet-stream",
    }
}

/// Lê os arquivos (só os comuns; pastas ficam de fora), até o limite.
fn ler(caminhos: &[PathBuf]) -> Vec<Arquivo> {
    let mut total = 0u64;
    let mut v = Vec::new();
    for p in caminhos {
        let Ok(m) = std::fs::metadata(p) else { continue };
        if !m.is_file() || total + m.len() > LIMITE {
            continue;
        }
        if let Ok(dados) = std::fs::read(p) {
            total += dados.len() as u64;
            let nome = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "arquivo".into());
            v.push(Arquivo { nome, tipo: tipo_de(p).into(), dados });
        }
    }
    v
}

/// Os arquivos do próprio computador numa lista de arquivos do GTK (os de outros lugares
/// ficam de fora, e com eles os endereços: soltar um link não é soltar um arquivo).
fn locais(l: &gdk::FileList) -> (Vec<PathBuf>, Vec<String>) {
    let mut caminhos = Vec::new();
    let mut enderecos = Vec::new();
    for f in l.files() {
        match f.path() {
            Some(p) => caminhos.push(p),
            None => enderecos.push(f.uri().to_string()),
        }
    }
    (caminhos, enderecos)
}

/// O script que monta os arquivos e manda o evento. `modo`: "paste", "dragenter",
/// "dragover", "dragleave" ou "drop". Devolve se a página tratou (preventDefault).
fn script(modo: &str, arquivos: &[Arquivo], nomes: &[(String, String)], x: f64, y: f64) -> String {
    let lista: Vec<serde_json::Value> = if arquivos.is_empty() {
        // Arrastando: os arquivos ainda não foram lidos; a página vê os nomes e os tipos.
        nomes.iter().map(|(n, t)| serde_json::json!({"n": n, "t": t, "d": ""})).collect()
    } else {
        arquivos.iter().map(|a| serde_json::json!({"n": a.nome, "t": a.tipo, "d": glib::base64_encode(&a.dados).as_str()})).collect()
    };
    let lista = serde_json::Value::Array(lista).to_string();
    format!(
        r#"(function(modo, fs, x, y) {{
  const dt = new DataTransfer();
  for (const f of fs) {{
    const s = atob(f.d), b = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) b[i] = s.charCodeAt(i);
    dt.items.add(new File([b], f.n, {{ type: f.t, lastModified: Date.now() }}));
  }}
  // O elemento: o do foco (colar) ou o que está sob o mouse (arrastar), entrando nas
  // sombras e nos quadros do mesmo site.
  let doc = document, alvo = null;
  for (let i = 0; i < 8; i++) {{
    let e = modo === "paste" ? doc.activeElement : doc.elementFromPoint(x, y);
    while (e && e.shadowRoot) {{
      const d = modo === "paste" ? e.shadowRoot.activeElement : e.shadowRoot.elementFromPoint(x, y);
      if (!d || d === e) break;
      e = d;
    }}
    alvo = e || doc.body || doc.documentElement;
    if (alvo && alvo.tagName === "IFRAME") {{
      try {{
        const r = alvo.getBoundingClientRect(), d = alvo.contentDocument;
        if (d) {{ x -= r.left; y -= r.top; doc = d; continue; }}
      }} catch (_) {{}}
    }}
    break;
  }}
  if (!alvo) return false;
  let ev;
  if (modo === "paste") {{
    try {{ ev = new ClipboardEvent("paste", {{ clipboardData: dt, bubbles: true, cancelable: true, composed: true }}); }} catch (_) {{}}
    if (!ev) ev = new Event("paste", {{ bubbles: true, cancelable: true, composed: true }});
    if (ev.clipboardData !== dt) Object.defineProperty(ev, "clipboardData", {{ value: dt }});
  }} else {{
    dt.dropEffect = "copy";
    const init = {{ dataTransfer: dt, clientX: x, clientY: y, screenX: x, screenY: y, bubbles: true, cancelable: modo !== "dragleave", composed: true }};
    try {{ ev = new DragEvent(modo, init); }} catch (_) {{}}
    if (!ev) ev = new MouseEvent(modo, init);
    if (ev.dataTransfer !== dt) Object.defineProperty(ev, "dataTransfer", {{ value: dt }});
  }}
  alvo.dispatchEvent(ev);
  return ev.defaultPrevented;
}})({modo:?}, {lista}, {x}, {y})"#
    )
}

/// Manda o evento à página (no mundo dela: os ouvintes dela é que recebem).
fn entregar(view: &WebView, js: &str, depois: impl FnOnce(bool) + 'static) {
    view.evaluate_javascript(js, None, None, None::<&gio::Cancellable>, move |r| {
        depois(r.map(|v| v.to_boolean()).unwrap_or(false));
    });
}

/// Colar na página. Se a área de transferência tem uma imagem ou arquivos, a página os
/// recebe como arquivos (como nos outros navegadores); se ela não quiser (um campo de
/// texto comum), o motor cola do jeito dele. Devolve `false` (sem fazer nada) quando só
/// há texto: aí o motor cola sozinho, com a tecla chegando à página como sempre.
pub fn colar(view: &WebView) -> bool {
    let cb = view.clipboard();
    let formatos = cb.formats();
    let imagem = IMAGENS.iter().copied().find(|m| formatos.contain_mime_type(m));
    let tem_arquivos = formatos.contain_mime_type("text/uri-list") || formatos.contain_mime_type("x-special/gnome-copied-files") || formatos.contains_type(gdk::FileList::static_type());
    if imagem.is_none() && !tem_arquivos {
        return false;
    }
    let view = view.clone();
    glib::MainContext::default().spawn_local(async move {
        let mut arquivos = Vec::new();
        // Arquivos copiados (no Arquivos, na mesa): os próprios arquivos. Uma captura de
        // tela copiada vem como os dois: o arquivo dela basta.
        if tem_arquivos {
            if let Ok(v) = cb.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await {
                if let Ok(l) = v.get::<gdk::FileList>() {
                    let (caminhos, _) = locais(&l);
                    arquivos = gio::spawn_blocking(move || ler(&caminhos)).await.unwrap_or_default();
                }
            }
        }
        // Uma imagem copiada (de outro app, de um site): os bytes dela.
        if arquivos.is_empty() {
            if let Some(m) = imagem {
                if let Ok((s, tipo)) = cb.read_future(&[m], glib::Priority::DEFAULT).await {
                    let mut dados = Vec::new();
                    loop {
                        match s.read_bytes_future(1 << 20, glib::Priority::DEFAULT).await {
                            Ok(b) if !b.is_empty() => dados.extend_from_slice(&b),
                            _ => break,
                        }
                    }
                    if !dados.is_empty() {
                        let ext = tipo.rsplit('/').next().unwrap_or("png").replace("jpeg", "jpg");
                        arquivos.push(Arquivo { nome: format!("{}.{ext}", glove_config::tr!("imagem")), tipo: tipo.to_string(), dados });
                    }
                }
            }
        }
        if arquivos.is_empty() {
            view.execute_editing_command("Paste");
            return;
        }
        let v = view.clone();
        entregar(&view, &script("paste", &arquivos, &[], 0.0, 0.0), move |tratou| {
            if !tratou {
                v.execute_editing_command("Paste");
            }
        });
    });
    true
}

/// Colar pelo menu (Editar ▸ Colar, o menu de contexto): com arquivos, como acima; só
/// com texto, o motor.
pub fn colar_pelo_menu(view: &WebView) {
    if !colar(view) {
        view.execute_editing_command("Paste");
    }
}

/// Soltar arquivos de fora na página (da mesa, do Arquivos, de outro app). Arrastar dentro
/// da própria Jangada (um link, um texto, uma imagem da página) continua com o motor.
pub fn ligar_soltar(view: &WebView) {
    let alvo = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    // Antes do motor: ele recebe o arrastar, mas não passa os arquivos para a página.
    alvo.set_propagation_phase(gtk::PropagationPhase::Capture);
    alvo.set_preload(true);
    alvo.connect_accept(|_, drop| drop.drag().is_none() && drop.formats().union_deserialize_types().contains_type(gdk::FileList::static_type()));
    // Os nomes do que vem (lidos assim que o GTK entrega a lista).
    let nomes = |t: &gtk::DropTarget| -> Vec<(String, String)> {
        let Some(l) = t.value().and_then(|v| v.get::<gdk::FileList>().ok()) else { return Vec::new() };
        locais(&l).0.iter().map(|p| (p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), tipo_de(p).to_string())).collect()
    };
    let escala = |v: &WebView| {
        let z = v.zoom_level();
        if z > 0.0 { z } else { 1.0 }
    };
    {
        let v = view.clone();
        alvo.connect_enter(move |t, x, y| {
            let z = escala(&v);
            entregar(&v, &script("dragenter", &[], &nomes(t), x / z, y / z), |_| {});
            gdk::DragAction::COPY
        });
    }
    {
        let v = view.clone();
        alvo.connect_motion(move |t, x, y| {
            let z = escala(&v);
            entregar(&v, &script("dragover", &[], &nomes(t), x / z, y / z), |_| {});
            gdk::DragAction::COPY
        });
    }
    {
        let v = view.clone();
        alvo.connect_leave(move |_| entregar(&v, &script("dragleave", &[], &[], -1.0, -1.0), |_| {}));
    }
    {
        let v = view.clone();
        alvo.connect_drop(move |_, valor, x, y| {
            let Ok(l) = valor.get::<gdk::FileList>() else { return false };
            let (caminhos, enderecos) = locais(&l);
            if caminhos.is_empty() {
                // Um link solto de outro app: abre, como nos outros navegadores.
                if let Some(u) = enderecos.iter().find(|u| u.starts_with("http://") || u.starts_with("https://")) {
                    v.load_uri(u);
                    return true;
                }
                return false;
            }
            let z = escala(&v);
            let v = v.clone();
            glib::MainContext::default().spawn_local(async move {
                let arquivos = gio::spawn_blocking(move || ler(&caminhos)).await.unwrap_or_default();
                if !arquivos.is_empty() {
                    entregar(&v, &script("drop", &arquivos, &[], x / z, y / z), |_| {});
                }
            });
            true
        });
    }
    view.add_controller(alvo);
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn tipos_pela_extensao() {
        assert_eq!(tipo_de(Path::new("/x/Captura.PNG")), "image/png");
        assert_eq!(tipo_de(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(tipo_de(Path::new("v.mp4")), "video/mp4");
        assert_eq!(tipo_de(Path::new("sem-extensao")), "application/octet-stream");
    }

    #[test]
    fn le_so_arquivos_comuns() {
        let d = std::env::temp_dir().join(format!("jangada-arquivos-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a.png");
        std::fs::write(&f, b"\x89PNG").unwrap();
        let v = ler(&[f.clone(), d.clone(), d.join("nao-existe")]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].nome, "a.png");
        assert_eq!(v[0].tipo, "image/png");
        assert_eq!(v[0].dados, b"\x89PNG");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn script_leva_os_arquivos() {
        let a = vec![Arquivo { nome: "x\".png".into(), tipo: "image/png".into(), dados: vec![1, 2, 3] }];
        let s = script("paste", &a, &[], 0.0, 0.0);
        assert!(s.contains(r#""n":"x\".png""#));
        assert!(s.contains(r#""d":"AQID""#));
        assert!(s.ends_with(r#"0, 0)"#));
    }
}
