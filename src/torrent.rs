//! Torrents na Jangada. Quem baixa é um ajudante, a própria Jangada com
//! `--torrent-daemon`, num processo à parte (com o motor de BitTorrent librqbit): ele
//! continua baixando e compartilhando com a Jangada fechada e sai sozinho quando não
//! sobra nada para fazer. A Jangada fala com ele por um soquete local
//! (`$XDG_RUNTIME_DIR/jangada-torrent.sock`): um pedido em JSON por conexão, uma
//! resposta em JSON.
//!
//! Os arquivos vão para Downloads/Torrents. Terminado o download, o torrent continua
//! compartilhando até enviar o mesmo tanto que baixou (proporção 1:1) e então para,
//! como "Concluído".

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Um torrent como a janela mostra.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub id: usize,
    pub nome: String,
    pub total: u64,
    pub feito: u64,
    pub enviado: u64,
    pub descida: f64,
    pub subida: f64,
    pub pares: u32,
    /// Segundos para terminar (se dá para saber).
    pub falta: Option<u64>,
    pub estado: Estado,
    pub erro: Option<String>,
    /// Onde ficam os arquivos.
    pub caminho: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Estado {
    #[default]
    Preparando,
    Baixando,
    Compartilhando,
    Pausado,
    Concluido,
    Erro,
}

/// Proporção de compartilhamento: para de enviar depois de enviar o mesmo tanto que baixou.
pub const PROPORCAO: f64 = 1.0;

fn soquete() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("jangada-torrent.sock")
}

/// Onde os torrents são baixados: Downloads/Torrents.
pub fn pasta_torrents() -> PathBuf {
    crate::pasta_downloads().join("Torrents")
}

fn pasta_estado() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("jangada/torrents")
}

// ---------------------------------------------------------------------------
// O lado da Jangada (cliente)
// ---------------------------------------------------------------------------

/// Manda um pedido e espera a resposta (curta: o ajudante está na mesma máquina).
fn pedir(p: &Value) -> Option<Value> {
    let mut s = UnixStream::connect(soquete()).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    writeln!(s, "{p}").ok()?;
    let mut linha = String::new();
    BufReader::new(s).read_line(&mut linha).ok()?;
    serde_json::from_str(&linha).ok()
}

/// O ajudante está rodando?
pub fn rodando() -> bool {
    pedir(&json!({"op": "ping"})).is_some()
}

/// Liga o ajudante (se já não estiver) e espera ele responder.
fn garantir() -> bool {
    if rodando() {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    use std::os::unix::process::CommandExt;
    let filho = std::process::Command::new(exe)
        .arg("--torrent-daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    if let Ok(mut c) = filho {
        std::thread::spawn(move || c.wait());
    }
    (0..50).any(|_| {
        std::thread::sleep(Duration::from_millis(100));
        rodando()
    })
}

/// Pede algo ao ajudante numa linha à parte (a janela não espera). `depois` roda com a
/// resposta (ou None), ainda fora da janela; a lista é relida em seguida.
pub fn mandar(p: Value) {
    std::thread::spawn(move || {
        if garantir() {
            let r = pedir(&p);
            if let Some(e) = r.as_ref().and_then(|r| r.get("erro")).and_then(|e| e.as_str()) {
                ERRO.lock().unwrap().replace(e.to_string());
            }
        } else {
            ERRO.lock().unwrap().replace(glove_config::tr!("O ajudante de torrents não abriu.").to_string());
        }
        atualizar_agora();
    });
}

pub fn adicionar_magnet(url: &str) {
    mandar(json!({"op": "add", "url": url}));
}

pub fn adicionar_arquivo(p: &Path) {
    mandar(json!({"op": "add_file", "path": p.to_string_lossy()}));
}

pub fn pausar(id: usize) {
    mandar(json!({"op": "pause", "id": id}));
}

pub fn continuar(id: usize) {
    mandar(json!({"op": "resume", "id": id}));
}

pub fn parar_de_compartilhar(id: usize) {
    mandar(json!({"op": "stop_sharing", "id": id}));
}

pub fn remover(id: usize, apagar_arquivos: bool) {
    mandar(json!({"op": "remove", "id": id, "files": apagar_arquivos}));
}

/// A última lista lida e o último erro (para a janela).
static LISTA: Mutex<Vec<Info>> = Mutex::new(Vec::new());
static ERRO: Mutex<Option<String>> = Mutex::new(None);
static LENDO: Mutex<bool> = Mutex::new(false);

pub fn lista() -> Vec<Info> {
    LISTA.lock().unwrap().clone()
}

pub fn tomar_erro() -> Option<String> {
    ERRO.lock().unwrap().take()
}

/// Relê a lista do ajudante numa linha à parte (sem ligar o ajudante se ele não está:
/// sem ele, não há torrents ativos). A janela pede de segundo em segundo.
pub fn atualizar() {
    let mut l = LENDO.lock().unwrap();
    if *l {
        return;
    }
    *l = true;
    std::thread::spawn(atualizar_agora);
}

fn atualizar_agora() {
    let nova = pedir(&json!({"op": "list"})).map(|r| ler_lista(&r)).unwrap_or_default();
    let mudou = {
        let mut l = LISTA.lock().unwrap();
        let m = *l != nova;
        *l = nova;
        m
    };
    *LENDO.lock().unwrap() = false;
    if mudou {
        acordar();
    }
}

/// Pede um quadro novo à janela. Esta linha não é a da janela: o pedido vai pelo laço
/// principal (chamar o kit daqui derruba a Jangada).
fn acordar() {
    glove_ui::gtk::glib::MainContext::default().invoke(glove_ui::redraw_soon);
}

fn ler_lista(r: &Value) -> Vec<Info> {
    r["torrents"].as_array().map(|v| v.iter().map(ler_info).collect()).unwrap_or_default()
}

fn ler_info(t: &Value) -> Info {
    let n = |k: &str| t[k].as_u64().unwrap_or(0);
    let estado = match t["state"].as_str().unwrap_or("") {
        "downloading" => Estado::Baixando,
        "seeding" => Estado::Compartilhando,
        "paused" => Estado::Pausado,
        "done" => Estado::Concluido,
        "error" => Estado::Erro,
        _ => Estado::Preparando,
    };
    Info {
        id: n("id") as usize,
        nome: t["name"].as_str().unwrap_or("").to_string(),
        total: n("total"),
        feito: n("done"),
        enviado: n("uploaded"),
        descida: t["down"].as_f64().unwrap_or(0.0),
        subida: t["up"].as_f64().unwrap_or(0.0),
        pares: n("peers") as u32,
        falta: t["eta"].as_u64(),
        estado,
        erro: t["error"].as_str().map(str::to_string),
        caminho: PathBuf::from(t["path"].as_str().unwrap_or("")),
    }
}

/// Mesmo que a janela não esteja aberta: com a Jangada abrindo, se o ajudante tem
/// torrents guardados de antes, ele volta a rodar (continua de onde parou).
pub fn retomar_se_preciso() {
    let tem = std::fs::read_dir(pasta_estado()).map(|d| d.flatten().next().is_some()).unwrap_or(false);
    if tem {
        std::thread::spawn(|| {
            garantir();
            atualizar_agora();
        });
    }
}

// ---------------------------------------------------------------------------
// O ajudante (`jangada --torrent-daemon`)
// ---------------------------------------------------------------------------

pub fn ajudante() -> std::process::ExitCode {
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("jangada-torrent: {e}");
            return 1.into();
        }
    };
    match rt.block_on(daemon::rodar()) {
        Ok(()) => 0.into(),
        Err(e) => {
            eprintln!("jangada-torrent: {e:#}");
            1.into()
        }
    }
}

mod daemon {
    use super::*;
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions, SessionPersistenceConfig};
    use std::collections::HashSet;
    use std::time::Instant;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    /// Sem nada baixando nem compartilhando e sem ninguém perguntando por este tempo, o
    /// ajudante sai (volta quando a Jangada precisar).
    const OCIOSO: Duration = Duration::from_secs(10 * 60);

    struct Estado {
        session: Arc<Session>,
        /// Os que pararam de compartilhar (pela proporção ou pela pessoa).
        parados: HashSet<String>,
        ultimo_pedido: Instant,
    }

    fn arquivo_parados() -> PathBuf {
        pasta_estado().join("parados.json")
    }

    fn ler_parados() -> HashSet<String> {
        std::fs::read_to_string(arquivo_parados()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn gravar_parados(p: &HashSet<String>) {
        let _ = std::fs::write(arquivo_parados(), serde_json::to_string(p).unwrap_or_default());
    }

    pub async fn rodar() -> anyhow::Result<()> {
        let sock = soquete();
        // Já há um ajudante: este sai.
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return Ok(());
        }
        let _ = std::fs::remove_file(&sock);
        let saida = pasta_torrents();
        std::fs::create_dir_all(&saida)?;
        let estado_dir = pasta_estado();
        std::fs::create_dir_all(&estado_dir)?;
        let opts = SessionOptions { fastresume: true, persistence: Some(SessionPersistenceConfig::Json { folder: Some(estado_dir.clone()) }), ..Default::default() };
        let session = Session::new_with_opts(saida, opts).await?;
        let listener = tokio::net::UnixListener::bind(&sock)?;
        let estado = Arc::new(tokio::sync::Mutex::new(Estado { session, parados: ler_parados(), ultimo_pedido: Instant::now() }));
        let mut tique = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                c = listener.accept() => {
                    if let Ok((s, _)) = c {
                        let estado = estado.clone();
                        tokio::spawn(async move {
                            let _ = atender(s, estado).await;
                        });
                    }
                }
                _ = tique.tick() => {
                    let mut e = estado.lock().await;
                    if cuidar(&mut e).await {
                        break;
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&sock);
        Ok(())
    }

    /// De tempos em tempos: para de compartilhar quem já enviou o bastante, e diz se o
    /// ajudante pode sair (nada ativo e ninguém perguntando há um tempo).
    async fn cuidar(e: &mut Estado) -> bool {
        let (parar, ativo) = e.session.with_torrents(|it| {
            let mut parar = Vec::new();
            let mut ativo = false;
            for (_, t) in it {
                let s = t.stats();
                let hash = t.info_hash().as_string();
                let pausado = matches!(s.state, librqbit::TorrentStatsState::Paused);
                if !pausado && !matches!(s.state, librqbit::TorrentStatsState::Error) {
                    ativo = true;
                }
                if s.finished && !pausado && s.total_bytes > 0 && s.uploaded_bytes as f64 >= s.total_bytes as f64 * PROPORCAO {
                    parar.push((t.clone(), hash));
                }
            }
            (parar, ativo)
        });
        for (t, hash) in parar {
            let _ = e.session.pause(&t).await;
            e.parados.insert(hash);
        }
        if !e.parados.is_empty() {
            gravar_parados(&e.parados);
        }
        !ativo && e.ultimo_pedido.elapsed() > OCIOSO
    }

    async fn atender(s: tokio::net::UnixStream, estado: Arc<tokio::sync::Mutex<Estado>>) -> anyhow::Result<()> {
        let (r, mut w) = s.into_split();
        let mut linha = String::new();
        tokio::io::BufReader::new(r).read_line(&mut linha).await?;
        let pedido: Value = serde_json::from_str(&linha).unwrap_or(Value::Null);
        let mut e = estado.lock().await;
        e.ultimo_pedido = Instant::now();
        let resposta = match responder(&mut e, &pedido).await {
            Ok(v) => v,
            Err(err) => json!({"erro": format!("{err:#}")}),
        };
        drop(e);
        w.write_all(format!("{resposta}\n").as_bytes()).await?;
        Ok(())
    }

    fn achar(e: &Estado, id: usize) -> anyhow::Result<Arc<librqbit::ManagedTorrent>> {
        e.session.get(id.into()).ok_or_else(|| anyhow::anyhow!("torrent {id} não existe"))
    }

    async fn responder(e: &mut Estado, p: &Value) -> anyhow::Result<Value> {
        let id = p["id"].as_u64().unwrap_or(u64::MAX) as usize;
        match p["op"].as_str().unwrap_or("") {
            "ping" => Ok(json!({"ok": true})),
            "list" => Ok(json!({"torrents": listar(e)})),
            "add" | "add_file" => {
                let add = if p["op"] == "add" {
                    AddTorrent::from_url(p["url"].as_str().unwrap_or("").to_string())
                } else {
                    let bytes = std::fs::read(p["path"].as_str().unwrap_or(""))?;
                    AddTorrent::from_bytes(bytes)
                };
                let opts = AddTorrentOptions { overwrite: true, ..Default::default() };
                e.session.add_torrent(add, Some(opts)).await?;
                Ok(json!({"ok": true}))
            }
            "pause" => {
                let t = achar(e, id)?;
                e.session.pause(&t).await?;
                Ok(json!({"ok": true}))
            }
            "resume" => {
                let t = achar(e, id)?;
                // Continuar um concluído volta a compartilhar.
                if e.parados.remove(&t.info_hash().as_string()) {
                    gravar_parados(&e.parados);
                }
                e.session.unpause(&t).await?;
                Ok(json!({"ok": true}))
            }
            "stop_sharing" => {
                let t = achar(e, id)?;
                e.session.pause(&t).await?;
                e.parados.insert(t.info_hash().as_string());
                gravar_parados(&e.parados);
                Ok(json!({"ok": true}))
            }
            "remove" => {
                let t = achar(e, id)?;
                let hash = t.info_hash().as_string();
                e.session.delete(id.into(), p["files"].as_bool().unwrap_or(false)).await?;
                if e.parados.remove(&hash) {
                    gravar_parados(&e.parados);
                }
                Ok(json!({"ok": true}))
            }
            outro => Err(anyhow::anyhow!("pedido desconhecido: {outro}")),
        }
    }

    fn listar(e: &Estado) -> Vec<Value> {
        let saida = pasta_torrents();
        e.session.with_torrents(|it| {
            it.map(|(id, t)| {
                let s = t.stats();
                let hash = t.info_hash().as_string();
                let pausado = matches!(s.state, librqbit::TorrentStatsState::Paused);
                let estado = match s.state {
                    librqbit::TorrentStatsState::Error => "error",
                    librqbit::TorrentStatsState::Initializing { .. } => "preparing",
                    _ if pausado && (s.finished && e.parados.contains(&hash)) => "done",
                    _ if pausado => "paused",
                    _ if s.finished => "seeding",
                    _ => "downloading",
                };
                let nome = t.name().unwrap_or_else(|| hash.clone());
                let (down, up, pares) = match &s.live {
                    Some(l) => (l.download_speed.as_bytes() as f64, l.upload_speed.as_bytes() as f64, l.snapshot.peer_stats.live as u64),
                    None => (0.0, 0.0, 0),
                };
                let eta = (down > 1.0 && !s.finished).then(|| (s.total_bytes.saturating_sub(s.progress_bytes) as f64 / down) as u64);
                json!({
                    "id": id,
                    "name": nome,
                    "total": s.total_bytes,
                    "done": s.progress_bytes,
                    "uploaded": s.uploaded_bytes,
                    "down": down,
                    "up": up,
                    "eta": eta,
                    "peers": pares,
                    "state": estado,
                    "error": s.error,
                    "path": saida.join(&nome).to_string_lossy(),
                })
            })
            .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_a_lista_do_ajudante() {
        let r = json!({"torrents": [{"id": 3, "name": "debian.iso", "total": 1000, "done": 500, "uploaded": 10, "down": 2048.0, "up": 0.0, "eta": 30, "peers": 7, "state": "downloading", "error": null, "path": "/tmp/debian.iso"}]});
        let l = ler_lista(&r);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, 3);
        assert_eq!(l[0].estado, Estado::Baixando);
        assert_eq!(l[0].falta, Some(30));
        assert_eq!(l[0].pares, 7);
        assert_eq!(ler_info(&json!({"state": "done"})).estado, Estado::Concluido);
    }
}
