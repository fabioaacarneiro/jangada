//! As extensões da Jangada: um formato próprio e pequeno, com os nomes de campo do Chrome
//! (Manifest V3). Uma pasta com `manifest.json` e:
//!
//! - scripts de conteúdo (JavaScript e CSS) que entram nas páginas que batem com `matches`;
//! - um botão na barra (`action`), que abre a janelinha da extensão (`default_popup`, uma
//!   página HTML da própria extensão: botões, menus, um aplicativo inteiro) ou avisa os
//!   scripts de conteúdo do clique (`action.onClicked`);
//! - uma página de opções (`options_page` ou `options_ui.page`), aberta numa aba;
//! - textos traduzidos (`_locales`, `__MSG_nome__` e `i18n.getMessage`).
//!
//! As páginas da extensão (a janelinha, as opções) são servidas pelo esquema
//! `jangada-extension://IDENTIFICADOR/caminho` e recebem a API inteira (`jangada`, que
//! também atende por `chrome` e por `browser`): storage (com o aviso de mudanças valendo
//! entre todas as abas e páginas), runtime (mensagens, getURL, getManifest, opções), tabs
//! (query, get, sendMessage, create, update, reload, remove), action (o selo no botão, o
//! título, a janelinha) e i18n. Os scripts de conteúdo recebem a mesma API, menos tabs.
//!
//! Isto NÃO roda as extensões da loja do Chrome: não há script de fundo (service worker),
//! nem chrome.webRequest, scripting, contextMenus, downloads e as outras APIs. O guia de quem
//! escreve uma extensão está em docs/EXTENSOES.md.
//!
//! Instala-se de uma pasta, de um `.zip` ou de um repositório do GitHub (pela linha de
//! comando, `jangada --install`, ou em Preferências ▸ Instaladas). Antes de instalar, a
//! Jangada mostra o que a extensão pede.
//!
//! Os scripts de conteúdo rodam assim que o documento da página fica pronto, no quadro
//! principal, num mundo isolado só da extensão ("jangada-x-IDENTIFICADOR"): veem o documento,
//! mas não as variáveis da página nem as de outra extensão, e nenhuma das duas os vê. O canal
//! de cada mundo para a Jangada tem o nome da extensão: uma não fala pela outra.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Map, Value};

/// O esquema das páginas e arquivos das extensões: jangada-extension://IDENTIFICADOR/caminho.
pub const ESQUEMA: &str = "jangada-extension";

/// O canal (e o mundo) das páginas da extensão: o mundo principal da página, que é dela.
pub const CANAL_PAGINAS: &str = "jangadaExt";

/// Uma extensão instalada (ou preparada para instalar).
#[derive(Clone, Debug)]
pub struct Extensao {
    pub id: String,
    pub nome: String,
    pub versao: String,
    pub descricao: String,
    /// A pasta da extensão (instalada, ou o lugar onde foi preparada).
    pub pasta: PathBuf,
    /// Os padrões de página e os arquivos de cada regra de conteúdo.
    pub regras: Vec<Regra>,
    /// As permissões que ela pede (permissions e host_permissions).
    pub permissoes: Vec<String>,
    pub ativa: bool,
    /// O botão na barra (`action`), se ela tiver um.
    pub acao: Option<Acao>,
    /// Os ícones da extensão (`icons`): (tamanho, caminho na pasta).
    pub icones: Vec<(u32, String)>,
    /// A página de opções (caminho na pasta).
    pub opcoes: Option<String>,
    /// Os arquivos que as páginas da web podem carregar (`web_accessible_resources`).
    pub acessiveis: Vec<String>,
    /// Os sites que as páginas da extensão podem pedir sem CORS (`host_permissions`).
    pub hosts: Vec<String>,
    /// O manifesto, com os `__MSG_nome__` traduzidos (para `runtime.getManifest()`).
    pub manifesto: Value,
    /// As mensagens do `_locales` na língua da pessoa (para `i18n.getMessage`), com os
    /// nomes em minúsculas.
    pub mensagens: Map<String, Value>,
}

/// O botão de uma extensão na barra.
#[derive(Clone, Debug, Default)]
pub struct Acao {
    /// O texto da dica (`default_title`; sem ele, o nome da extensão).
    pub titulo: String,
    /// A página da janelinha (caminho na pasta), se houver.
    pub popup: Option<String>,
    /// Os ícones do botão (`default_icon`): (tamanho, caminho na pasta).
    pub icones: Vec<(u32, String)>,
}

#[derive(Clone, Debug)]
pub struct Regra {
    pub matches: Vec<String>,
    pub js: Vec<String>,
    pub css: Vec<String>,
    /// `"run_at": "document_start"`: entra no começo do documento, antes dos scripts da
    /// página (os bloqueadores de anúncio precisam disso). As outras entram com a página
    /// pronta, como o `document_idle`, o padrão do Chrome.
    pub inicio: bool,
}

impl Extensao {
    /// O endereço de um arquivo da extensão (jangada-extension://ID/caminho).
    pub fn url(&self, caminho: &str) -> String {
        url_de(&self.id, caminho)
    }

    /// O ícone mais adequado para `px` pixels: o menor que não fique borrado (>= px), ou o
    /// maior que houver. Os do botão vêm antes dos da extensão.
    pub fn icone(&self, px: u32) -> Option<PathBuf> {
        let lista = self.acao.as_ref().map(|a| a.icones.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| self.icones.clone());
        let mut ok: Vec<&(u32, String)> = lista.iter().filter(|(_, c)| arquivo(&self.pasta, c).is_some()).collect();
        ok.sort_by_key(|(t, _)| *t);
        let escolha = ok.iter().find(|(t, _)| *t >= px).or(ok.last())?;
        arquivo(&self.pasta, &escolha.1)
    }

    /// O texto do botão na barra: o `default_title`, ou o nome.
    pub fn titulo_da_acao(&self) -> String {
        self.acao.as_ref().map(|a| a.titulo.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| self.nome.clone())
    }

    /// Um arquivo da extensão pode ser carregado por uma página da web (está em
    /// `web_accessible_resources`)?
    pub fn acessivel(&self, caminho: &str) -> bool {
        let c = caminho.trim_start_matches('/');
        self.acessiveis.iter().any(|p| glob(p.trim_start_matches('/').as_bytes(), c.as_bytes()))
    }

    /// Os padrões de `host_permissions` no formato do motor (o CORS fica de fora para eles
    /// nas páginas da extensão, como no Chrome).
    pub fn hosts_do_motor(&self) -> Vec<String> {
        let mut v = Vec::new();
        for h in &self.hosts {
            if h == "<all_urls>" || h == "*://*/*" {
                v.push("http://*/*".to_string());
                v.push("https://*/*".to_string());
            } else if let Some(resto) = h.strip_prefix("*://") {
                v.push(format!("http://{resto}"));
                v.push(format!("https://{resto}"));
            } else if h.starts_with("http://") || h.starts_with("https://") {
                v.push(h.clone());
            }
        }
        v
    }
}

fn pasta_extensoes() -> PathBuf {
    // Nos testes, uma pasta à parte: nunca a das extensões da pessoa.
    #[cfg(test)]
    let p = std::env::temp_dir().join(format!("jangada-extensoes-de-teste-{}", std::process::id()));
    #[cfg(not(test))]
    let p = {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("jangada/extensions")
    };
    let _ = std::fs::create_dir_all(&p);
    p
}

/// O nome de pasta de uma extensão nova: o nome dela em letras simples ("Olá, Jangada" →
/// "ola-jangada") e um pedaço do manifesto (duas de nomes parecidos não se misturam).
fn id_para(nome: &str, manifesto: &str) -> String {
    let mut slug = String::new();
    for c in nome.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        if c != '-' || !(slug.is_empty() || slug.ends_with('-')) {
            slug.push(c);
        }
    }
    let slug = slug.trim_matches('-');
    let mut h = DefaultHasher::new();
    manifesto.hash(&mut h);
    format!("{}-{:08x}", if slug.is_empty() { "extensao" } else { slug }, h.finish() as u32)
}

fn ativas_arquivo() -> PathBuf {
    pasta_extensoes().join("disabled.txt")
}

fn desativadas() -> Vec<String> {
    std::fs::read_to_string(ativas_arquivo()).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

// ---------------------------------------------------------------------------
// O manifesto
// ---------------------------------------------------------------------------

/// A língua da pessoa no formato do `_locales` ("pt_BR"), pela mesma ordem do glove-ui.
fn lingua_do_ambiente() -> String {
    for v in ["GLOVE_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(s) = std::env::var(v) {
            let base = s.split(['.', '@']).next().unwrap_or("").replace('-', "_");
            if !base.is_empty() && base != "C" && base != "POSIX" {
                return base;
            }
        }
    }
    "en".into()
}

/// A língua para `i18n.getUILanguage()` ("pt-BR").
pub fn lingua() -> String {
    lingua_do_ambiente().replace('_', "-")
}

/// As mensagens do `_locales`: as da língua padrão da extensão, cobertas pelas da língua
/// geral da pessoa ("pt") e depois pelas da variante ("pt_BR"). Os nomes ficam em
/// minúsculas (no Chrome, `getMessage` não liga para maiúsculas).
fn ler_mensagens(pasta: &Path, padrao: Option<&str>, lingua: &str) -> Map<String, Value> {
    let mut m = Map::new();
    let geral = lingua.split('_').next().unwrap_or("").to_string();
    let mut ordem: Vec<String> = Vec::new();
    if let Some(p) = padrao {
        ordem.push(p.to_string());
    }
    ordem.push(geral);
    ordem.push(lingua.to_string());
    ordem.dedup();
    for l in ordem.iter().filter(|l| !l.is_empty() && !l.contains(['/', '\\', '.'])) {
        let arq = pasta.join("_locales").join(l).join("messages.json");
        let Some(Value::Object(o)) = std::fs::read_to_string(&arq).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()) else {
            continue;
        };
        for (k, v) in o {
            let mut v = v;
            // Os nomes dos marcadores também ficam em minúsculas.
            if let Some(Value::Object(ph)) = v.get_mut("placeholders") {
                let novos: Map<String, Value> = ph.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect();
                *ph = novos;
            }
            m.insert(k.to_lowercase(), v);
        }
    }
    m
}

/// Troca os `__MSG_nome__` de um texto do manifesto pela mensagem traduzida.
fn traduzir(texto: &str, msgs: &Map<String, Value>) -> String {
    let mut out = String::new();
    let mut resto = texto;
    while let Some(i) = resto.find("__MSG_") {
        let depois = &resto[i + 6..];
        let Some(f) = depois.find("__") else { break };
        let nome = &depois[..f];
        out.push_str(&resto[..i]);
        out.push_str(msgs.get(&nome.to_lowercase()).and_then(|v| v["message"].as_str()).unwrap_or(""));
        resto = &depois[f + 2..];
    }
    out.push_str(resto);
    out
}

/// Troca os `__MSG_nome__` em todos os textos de um valor do manifesto.
fn traduzir_valor(v: &mut Value, msgs: &Map<String, Value>) {
    match v {
        Value::String(s) if s.contains("__MSG_") => *s = traduzir(s, msgs),
        Value::Array(a) => a.iter_mut().for_each(|x| traduzir_valor(x, msgs)),
        Value::Object(o) => o.values_mut().for_each(|x| traduzir_valor(x, msgs)),
        _ => {}
    }
}

/// Os ícones de um campo: um texto (um ícone só) ou um objeto {"16": "a.png", ...}.
fn ler_icones(v: &Value) -> Vec<(u32, String)> {
    match v {
        Value::String(s) => vec![(16, s.clone())],
        Value::Object(o) => o.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string()))).collect(),
        _ => vec![],
    }
}

/// Lê o manifesto de uma pasta de extensão e devolve a extensão (sem instalar).
pub fn ler(pasta: &Path) -> Result<Extensao, String> {
    let texto = std::fs::read_to_string(pasta.join("manifest.json")).map_err(|_| glove_config::tr!("A pasta não tem um manifest.json.").to_string())?;
    // O Chrome aceita o BOM no começo do arquivo.
    let mut m: Value = serde_json::from_str(texto.trim_start_matches('\u{feff}')).map_err(|e| glove_config::tr!("O manifest.json não é um JSON válido: {}", e))?;
    let versao_manifesto = m["manifest_version"].as_u64().unwrap_or(0);
    if versao_manifesto != 3 {
        return Err(glove_config::tr!("A Jangada aceita extensões do Manifest V3 (o manifest_version é {}).", versao_manifesto));
    }
    let mensagens = ler_mensagens(pasta, m["default_locale"].as_str(), &lingua_do_ambiente());
    traduzir_valor(&mut m, &mensagens);
    let nome = m["name"].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| glove_config::tr!("O manifesto não tem o nome."))?.to_string();
    let versao = m["version"].as_str().unwrap_or("0").to_string();
    let descricao = m["description"].as_str().unwrap_or("").to_string();
    let existe = |f: &str| -> Result<(), String> {
        if arquivo(pasta, f).is_none() {
            return Err(glove_config::tr!("O arquivo {} do manifesto não existe.", f));
        }
        Ok(())
    };
    let mut regras = Vec::new();
    for r in m["content_scripts"].as_array().cloned().unwrap_or_default() {
        let lista = |k: &str| -> Vec<String> { r[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let js = lista("js");
        let css = lista("css");
        for f in js.iter().chain(css.iter()) {
            existe(f)?;
        }
        let inicio = r["run_at"].as_str() == Some("document_start");
        regras.push(Regra { matches: lista("matches"), js, css, inicio });
    }
    // O botão na barra (o `browser_action` do Manifest V2 também vale, para quem traz um
    // manifesto antigo com só a versão trocada).
    let a = if m["action"].is_object() { &m["action"] } else { &m["browser_action"] };
    let acao = if a.is_object() {
        let popup = a["default_popup"].as_str().filter(|p| !p.trim().is_empty()).map(str::to_string);
        if let Some(p) = &popup {
            existe(p)?;
        }
        Some(Acao { titulo: a["default_title"].as_str().unwrap_or("").to_string(), popup, icones: ler_icones(&a["default_icon"]) })
    } else {
        None
    };
    let opcoes = m["options_ui"]["page"].as_str().or(m["options_page"].as_str()).filter(|p| !p.trim().is_empty()).map(str::to_string);
    if let Some(p) = &opcoes {
        existe(p)?;
    }
    let mut acessiveis = Vec::new();
    for w in m["web_accessible_resources"].as_array().cloned().unwrap_or_default() {
        match w {
            // O formato do V3: [{"resources": [...], "matches": [...]}].
            Value::Object(o) => acessiveis.extend(o.get("resources").and_then(|r| r.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string))),
            // O do V2: uma lista de caminhos.
            Value::String(s) => acessiveis.push(s),
            _ => {}
        }
    }
    let hosts: Vec<String> = m["host_permissions"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
    let mut permissoes: Vec<String> = Vec::new();
    for k in ["permissions", "host_permissions"] {
        for p in m[k].as_array().cloned().unwrap_or_default() {
            if let Some(s) = p.as_str() {
                permissoes.push(s.to_string());
            }
        }
    }
    for r in &regras {
        for mt in &r.matches {
            if !permissoes.contains(mt) {
                permissoes.push(mt.clone());
            }
        }
    }
    let id = id_para(&nome, &texto);
    Ok(Extensao {
        id: id.clone(),
        nome,
        versao,
        descricao,
        pasta: pasta.to_path_buf(),
        regras,
        permissoes,
        ativa: !desativadas().contains(&id),
        acao,
        icones: ler_icones(&m["icons"]),
        opcoes,
        acessiveis,
        hosts,
        manifesto: m,
        mensagens,
    })
}

// ---------------------------------------------------------------------------
// As instaladas
// ---------------------------------------------------------------------------

/// A lista das instaladas fica guardada: a barra a consulta a cada quadro. Ela é relida
/// quando a pasta das extensões ou a lista das desligadas mudam (instalar, remover, ligar,
/// desligar, mesmo por outro `jangada --install`).
struct Cache {
    marca: (Option<std::time::SystemTime>, Option<std::time::SystemTime>),
    lista: Vec<Extensao>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
static VERSAO: AtomicU64 = AtomicU64::new(0);

fn marca_da_pasta() -> (Option<std::time::SystemTime>, Option<std::time::SystemTime>) {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    (m(&pasta_extensoes()), m(&ativas_arquivo()))
}

/// Esquece a lista guardada (depois de mudar as extensões).
fn invalidar() {
    if let Ok(mut c) = CACHE.lock() {
        *c = None;
    }
}

/// Muda toda vez que a lista das instaladas é relida (quem guarda algo dela, como os
/// ícones da barra, sabe quando refazer).
pub fn versao() -> u64 {
    let _ = listar();
    VERSAO.load(Ordering::Relaxed)
}

/// As extensões instaladas. A identidade de uma instalada é o nome da pasta dela (os
/// dados guardados ficam ali): não muda quando o manifesto muda.
pub fn listar() -> Vec<Extensao> {
    let marca = marca_da_pasta();
    if let Ok(c) = CACHE.lock() {
        if let Some(c) = c.as_ref().filter(|c| c.marca == marca) {
            return c.lista.clone();
        }
    }
    let desligadas = desativadas();
    let mut v: Vec<Extensao> = std::fs::read_dir(pasta_extensoes())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("manifest.json").is_file())
        .filter_map(|e| {
            let mut x = ler(&e.path()).ok()?;
            x.id = e.file_name().to_string_lossy().into_owned();
            x.ativa = !desligadas.contains(&x.id);
            Some(x)
        })
        .collect();
    v.sort_by(|a, b| a.nome.to_lowercase().cmp(&b.nome.to_lowercase()));
    if let Ok(mut c) = CACHE.lock() {
        *c = Some(Cache { marca, lista: v.clone() });
    }
    VERSAO.fetch_add(1, Ordering::Relaxed);
    v
}

/// Uma extensão instalada e ligada, pelo identificador.
pub fn ativa(id: &str) -> Option<Extensao> {
    listar().into_iter().find(|e| e.id == id && e.ativa)
}

/// Copia uma extensão preparada para a pasta de instalação. Uma instalada com o mesmo
/// nome é a versão anterior: a nova entra no lugar dela (mesma pasta, mesmos dados
/// guardados em storage.json), mesmo que o manifesto tenha mudado. Antes, mudar o
/// manifesto e instalar de novo deixava as duas instaladas, e o script rodava duas vezes.
pub fn instalar(e: &Extensao) -> Result<Extensao, String> {
    let id = listar().into_iter().find(|i| i.nome == e.nome).map(|i| i.id).unwrap_or_else(|| e.id.clone());
    let destino = pasta_extensoes().join(&id);
    let guardado = destino.join("storage.json");
    let dados = std::fs::read(&guardado).ok();
    let _ = std::fs::remove_dir_all(&destino);
    copiar(&e.pasta, &destino).map_err(|x| x.to_string())?;
    if let Some(d) = dados {
        let _ = std::fs::write(destino.join("storage.json"), d);
    }
    // Instalar é querer usar: uma que estava desligada volta ligada.
    if desativadas().contains(&id) {
        alternar(&id);
    }
    invalidar();
    let mut inst = ler(&destino)?;
    inst.id = id;
    inst.ativa = true;
    Ok(inst)
}

/// Copia a pasta da extensão. O que começa com ponto fica de fora (o `.git` de um clone,
/// o `.github`, os arquivos escondidos de editores): não é da extensão.
fn copiar(de: &Path, para: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(para)?;
    for e in std::fs::read_dir(de)? {
        let e = e?;
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let p = para.join(e.file_name());
        if e.file_type()?.is_dir() {
            copiar(&e.path(), &p)?;
        } else {
            std::fs::copy(e.path(), &p)?;
        }
    }
    Ok(())
}

pub fn remover(id: &str) {
    let _ = std::fs::remove_dir_all(pasta_extensoes().join(id));
    let mut v = desativadas();
    v.retain(|x| x != id);
    let _ = std::fs::write(ativas_arquivo(), v.join("\n") + "\n");
    invalidar();
}

pub fn alternar(id: &str) {
    let mut v = desativadas();
    if let Some(i) = v.iter().position(|x| x == id) {
        v.remove(i);
    } else {
        v.push(id.to_string());
    }
    let _ = std::fs::write(ativas_arquivo(), v.join("\n") + "\n");
    invalidar();
}

/// Prepara uma extensão a partir de uma pasta, de um .zip ou de um link do GitHub: devolve
/// a extensão lida numa pasta temporária (ainda não instalada).
pub fn preparar(origem: &str) -> Result<Extensao, String> {
    let origem = origem.trim();
    let tmp = std::env::temp_dir().join(format!("jangada-ext-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let raiz = if let Some(repo) = repositorio_github(origem) {
        let base = baixar_github(&repo, &tmp)?;
        match subpasta_github(origem) {
            Some(sub) => base.join(sub),
            None => base,
        }
    } else if origem.to_lowercase().ends_with(".zip") {
        let arq = Path::new(origem);
        descompactar(arq, &tmp.join("zip"))?;
        tmp.join("zip")
    } else {
        let p = PathBuf::from(origem.strip_prefix("file://").unwrap_or(origem));
        if !p.is_dir() {
            return Err(glove_config::tr!("Não achei a pasta {}.", origem));
        }
        p
    };
    // O manifest.json pode estar numa subpasta (o repositório baixado tem uma pasta raiz).
    let pasta = achar_manifesto(&raiz).ok_or_else(|| glove_config::tr!("Não achei o manifest.json em {}.", origem))?;
    ler(&pasta)
}

/// "https://github.com/usuario/repositorio" (ou "usuario/repositorio") → ("usuario", "repositorio").
fn repositorio_github(origem: &str) -> Option<(String, String, Option<String>)> {
    let resto = origem.strip_prefix("https://github.com/").or_else(|| origem.strip_prefix("http://github.com/")).or_else(|| {
        let s = origem.strip_prefix("github:").unwrap_or(origem);
        let partes: Vec<&str> = s.split('/').collect();
        // "usuario/repositorio" só é o GitHub se não for uma pasta que existe aqui.
        (partes.len() == 2 && !origem.starts_with('/') && !origem.starts_with('~') && !origem.starts_with('.') && !Path::new(origem).exists()).then_some(s)
    })?;
    let mut partes = resto.trim_end_matches('/').split('/');
    let usuario = partes.next()?.to_string();
    let repo = partes.next()?.trim_end_matches(".git").to_string();
    let valido = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !valido(&usuario) || !valido(&repo) {
        return None;
    }
    // /tree/RAMO/PASTA: o ramo é o primeiro pedaço (a pasta vem de subpasta_github).
    let ramo = if partes.next() == Some("tree") { partes.next().map(str::to_string) } else { None };
    Some((usuario, repo, ramo))
}

/// O caminho dentro do repositório (depois de /tree/RAMO/), se o link tiver um.
fn subpasta_github(origem: &str) -> Option<String> {
    let s = origem.strip_prefix("https://github.com/").or_else(|| origem.strip_prefix("http://github.com/"))?;
    let partes: Vec<&str> = s.trim_end_matches('/').split('/').collect();
    (partes.get(2) == Some(&"tree") && partes.len() > 4).then(|| partes[4..].join("/"))
}

/// Baixa o repositório num .zip. Sem ramo no link, vale o ramo padrão do repositório (o
/// HEAD: main, master ou outro); "main" e "master" ficam de reserva. Os erros do curl não
/// aparecem (um 404 de um ramo que não existe não é erro de quem instala).
fn baixar_github(repo: &(String, String, Option<String>), dir: &Path) -> Result<PathBuf, String> {
    let (usuario, nome, ramo) = repo;
    let refs: Vec<String> = match ramo {
        Some(r) => vec![format!("refs/heads/{r}")],
        None => vec!["HEAD".into(), "refs/heads/main".into(), "refs/heads/master".into()],
    };
    for r in refs {
        let url = format!("https://github.com/{usuario}/{nome}/archive/{r}.zip");
        let arq = dir.join("repo.zip");
        let ok = std::process::Command::new("curl").args(["-fsL", "--max-time", "120", "-o"]).arg(&arq).arg(&url).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
            || std::process::Command::new("wget").args(["-q", "-T", "120", "-O"]).arg(&arq).arg(&url).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if ok {
            let saida = dir.join("repo");
            descompactar(&arq, &saida)?;
            return Ok(saida);
        }
    }
    Err(glove_config::tr!("Não consegui baixar o repositório {}/{} do GitHub.", usuario, nome))
}

fn descompactar(arq: &Path, destino: &Path) -> Result<(), String> {
    let f = std::fs::File::open(arq).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| glove_config::tr!("O arquivo não é um .zip válido: {}", e))?;
    std::fs::create_dir_all(destino).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        // Nunca sai da pasta de destino (caminhos com "..").
        let Some(rel) = e.enclosed_name() else { continue };
        let saida = destino.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&saida).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = saida.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&saida).map_err(|e| e.to_string())?;
            std::io::copy(&mut e, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// A pasta que tem o manifest.json: a própria, ou a mais rasa das subpastas que o tiver
/// (até 3 níveis; o repositório baixado tem uma pasta raiz com o nome dele).
fn achar_manifesto(raiz: &Path) -> Option<PathBuf> {
    let mut fila = vec![(raiz.to_path_buf(), 0u32)];
    while !fila.is_empty() {
        let (dir, nivel) = fila.remove(0);
        if dir.join("manifest.json").is_file() {
            return Some(dir);
        }
        if nivel >= 3 {
            continue;
        }
        let mut subs: Vec<PathBuf> = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        fila.extend(subs.into_iter().map(|p| (p, nivel + 1)));
    }
    None
}

// ---------------------------------------------------------------------------
// Os arquivos da extensão (o esquema jangada-extension://)
// ---------------------------------------------------------------------------

/// O endereço de um arquivo de uma extensão.
pub fn url_de(id: &str, caminho: &str) -> String {
    format!("{ESQUEMA}://{id}/{}", caminho.trim_start_matches('/'))
}

/// (identificador, caminho) de um endereço jangada-extension://ID/caminho?busca#parte.
pub fn partes_da_url(url: &str) -> Option<(String, String)> {
    let resto = url.strip_prefix(ESQUEMA)?.strip_prefix("://")?;
    let (id, caminho) = resto.split_once('/').unwrap_or((resto, ""));
    let caminho = caminho.split(['?', '#']).next().unwrap_or("");
    (!id.is_empty()).then(|| (id.to_ascii_lowercase(), caminho.to_string()))
}

/// %XX → o byte (os caminhos chegam codificados do motor).
fn decodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        // Pelos bytes: fatiar o texto no meio de um caractere (um "%é") derrubaria tudo.
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// O arquivo de um caminho da extensão, só se ele estiver dentro da pasta dela (nada de
/// "..", caminhos absolutos ou atalhos que levem para fora).
pub fn arquivo(pasta: &Path, caminho: &str) -> Option<PathBuf> {
    let c = decodificar(caminho);
    let c = c.trim_start_matches('/');
    if c.is_empty() {
        return None;
    }
    let mut p = pasta.to_path_buf();
    for parte in c.split('/') {
        if parte.is_empty() || parte == "." {
            continue;
        }
        if parte == ".." || parte.contains('\\') || parte.contains('\0') {
            return None;
        }
        p.push(parte);
    }
    let real = p.canonicalize().ok()?;
    let base = pasta.canonicalize().ok()?;
    (real.starts_with(&base) && real.is_file()).then_some(real)
}

/// O tipo de um arquivo, pela extensão do nome (o que o motor precisa para mostrar).
pub fn tipo_do_arquivo(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html" | "htm") => "text/html",
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("txt" | "md") => "text/plain",
        Some("xml") => "application/xml",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "oga") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// Padrões de página (match patterns do Chrome) e o script que vai para a página
// ---------------------------------------------------------------------------

/// Um endereço bate com um padrão "esquema://host/caminho" (com "*" nos três lugares).
pub fn casa(padrao: &str, url: &str) -> bool {
    if padrao == "<all_urls>" {
        return url.starts_with("http://") || url.starts_with("https://");
    }
    let Some((esq_p, resto_p)) = padrao.split_once("://") else { return false };
    let Some((esq_u, resto_u)) = url.split_once("://") else { return false };
    let esquema_ok = match esq_p {
        "*" => esq_u == "http" || esq_u == "https",
        outro => outro == esq_u,
    };
    if !esquema_ok {
        return false;
    }
    let (host_p, caminho_p) = resto_p.split_once('/').map(|(h, c)| (h, format!("/{c}"))).unwrap_or((resto_p, "/".into()));
    // Como no Chrome, a porta não entra no padrão (só o host).
    let host_u = resto_u.split(['/', '?', '#']).next().unwrap_or("").split(':').next().unwrap_or("").to_ascii_lowercase();
    let caminho_u = match resto_u.find('/') {
        Some(i) => resto_u[i..].split('#').next().unwrap_or("/").to_string(),
        None => "/".to_string(),
    };
    let host_ok = host_p == "*" || host_p == host_u || (host_p.starts_with("*.") && (host_u == &host_p[2..] || host_u.ends_with(&host_p[1..])));
    host_ok && glob(caminho_p.as_bytes(), caminho_u.as_bytes())
}

pub fn glob(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&p[1..], s) || (!s.is_empty() && glob(p, &s[1..])),
        (Some(a), Some(b)) if a == b => glob(&p[1..], &s[1..]),
        _ => false,
    }
}

pub fn js_str(t: &str) -> String {
    serde_json::to_string(t).unwrap_or_else(|_| "\"\"".into()).replace('<', "\\u003c").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

pub fn js_valor(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into()).replace('<', "\\u003c").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// O mundo isolado dos scripts de conteúdo de uma extensão. É também o nome do canal dele
/// para a Jangada: a mensagem que chega por ele é dessa extensão e de nenhuma outra.
pub fn mundo_de(id: &str) -> String {
    format!("jangada-x-{id}")
}

/// O que vai para uma página: para cada extensão ativa com uma regra que bate com o
/// endereço, (a extensão, o mundo dela, os pedaços em ordem): a API, o CSS e os scripts, na
/// ordem do manifesto. Cada pedaço é avaliado à parte (um erro de escrita num script não
/// leva os outros), e cada script recebe a API da própria extensão, como `jangada`, `chrome`
/// e `browser`.
/// Os scripts das regras `document_start` de cada extensão ativa, para entrar no começo
/// de todo documento da aba: (extensão, mundo, código). Cada regra vem com o teste do
/// endereço (os padrões do `matches` em expressões regulares), e a API só é montada quando
/// a página casa: nas outras, o script não faz nada.
pub fn scripts_no_inicio() -> Vec<(String, String, String)> {
    let mut saida = Vec::new();
    for e in listar().into_iter().filter(|e| e.ativa) {
        let mundo = mundo_de(&e.id);
        let api = api_js(&e, &mundo, false);
        let mut corpo: Vec<String> = Vec::new();
        for r in e.regras.iter().filter(|r| r.inicio) {
            let mut partes = String::new();
            for c in &r.css {
                if let Some(t) = arquivo(&e.pasta, c).and_then(|p| std::fs::read_to_string(p).ok()) {
                    partes.push_str(&format!("(function(){{var s=document.createElement('style');s.textContent={};(document.head||document.documentElement).appendChild(s)}})();", js_str(&t)));
                }
            }
            for j in &r.js {
                if let Some(t) = arquivo(&e.pasta, j).and_then(|p| std::fs::read_to_string(p).ok()) {
                    partes.push_str(&format!(
                        "(function(jangada){{var chrome=jangada,browser=jangada;try{{\n{t}\n}}catch(e){{console.error({}+': '+(e&&e.stack||e))}}}})(window.__jangadaApi);",
                        js_str(&format!("{} ({j})", e.nome))
                    ));
                }
            }
            let padroes: Vec<String> = r.matches.iter().filter_map(|m| regex_do_padrao(m)).map(|s| js_str(&s)).collect();
            if partes.is_empty() || padroes.is_empty() {
                continue;
            }
            corpo.push(format!("if([{}].some(function(s){{return new RegExp(s).test(location.href)}})){{{api}{partes}}}", padroes.join(",")));
        }
        if !corpo.is_empty() {
            saida.push((e.id.clone(), mundo, corpo.join("\n")));
        }
    }
    saida
}

/// Um padrão do `matches` (como `*://*.exemplo.com/*`) em expressão regular, com o mesmo
/// sentido de `casa`: o esquema `*` vale por http e https, `*.` pede o site e os
/// subdomínios, a porta não entra no padrão. (Contribuição de darkstarfire.)
fn regex_do_padrao(padrao: &str) -> Option<String> {
    fn escapa(t: &str) -> String {
        t.chars().map(|c| if "\\^$.|?*+()[]{}".contains(c) { format!("\\{c}") } else { c.to_string() }).collect()
    }
    if padrao == "<all_urls>" {
        return Some(r"^https?://".to_string());
    }
    let (esq, resto) = padrao.split_once("://")?;
    let esq_re = if esq == "*" { "https?".to_string() } else { escapa(esq) };
    let (host, caminho) = resto.split_once('/').map(|(h, c)| (h, format!("/{c}"))).unwrap_or((resto, "/".into()));
    let host_re = if host == "*" {
        r"[^/?#:]+".to_string()
    } else if let Some(sub) = host.strip_prefix("*.") {
        format!(r"(?:[^/?#:]+\.)?{}", escapa(sub))
    } else {
        escapa(host)
    };
    let caminho_re = caminho.split('*').map(escapa).collect::<Vec<_>>().join(".*");
    Some(format!(r"^{esq_re}://{host_re}(?::\d+)?{caminho_re}(?:#.*)?$"))
}

/// Os scripts das regras que entram com a página pronta, para `url`: (extensão, mundo,
/// pedaços). Uma extensão com só regras `document_start` que casam vem com a lista vazia
/// (os scripts dela já entraram; a aba só fica sabendo que a usa).
pub fn scripts_para(url: &str) -> Vec<(String, String, Vec<String>)> {
    let mut saida = Vec::new();
    for e in listar().into_iter().filter(|e| e.ativa) {
        let mut partes = Vec::new();
        let mut usa = false;
        for r in &e.regras {
            if !r.matches.iter().any(|m| casa(m, url)) {
                continue;
            }
            usa = true;
            if r.inicio {
                continue;
            }
            for c in &r.css {
                if let Some(t) = arquivo(&e.pasta, c).and_then(|p| std::fs::read_to_string(p).ok()) {
                    partes.push(format!("(function(){{var s=document.createElement('style');s.textContent={};(document.head||document.documentElement).appendChild(s)}})();", js_str(&t)));
                }
            }
            for j in &r.js {
                if let Some(t) = arquivo(&e.pasta, j).and_then(|p| std::fs::read_to_string(p).ok()) {
                    partes.push(format!(
                        "(function(jangada){{var chrome=jangada,browser=jangada;try{{\n{t}\n}}catch(e){{console.error({}+': '+(e&&e.stack||e))}}}})(window.__jangadaApi);",
                        js_str(&format!("{} ({j})", e.nome))
                    ));
                }
            }
        }
        if partes.is_empty() {
            if usa {
                saida.push((e.id.clone(), mundo_de(&e.id), Vec::new()));
            }
            continue;
        }
        partes.insert(0, api_js(&e, &mundo_de(&e.id), false));
        let mundo = mundo_de(&e.id);
        saida.push((e.id.clone(), mundo, partes));
    }
    saida
}

/// O script que monta a API nas páginas de uma extensão (a janelinha, as opções): entra
/// no mundo principal, no começo do documento, e só faz algo nas páginas dela.
pub fn api_das_paginas(e: &Extensao) -> String {
    format!(
        "(function(){{if(location.protocol!=={}||location.host!=={})return;{}}})();",
        js_str(&format!("{ESQUEMA}:")),
        js_str(&e.id),
        api_js(e, CANAL_PAGINAS, true)
    )
}

/// A API de uma extensão (o objeto `jangada`, que também atende por `chrome` e `browser`),
/// montada uma vez por mundo em `window.__jangadaApi`. `canal`: o nome do canal para a
/// Jangada (o do mundo da extensão, ou o das páginas dela). `pagina`: é uma página da
/// extensão (tem tabs e window.close fecha a janelinha).
fn api_js(e: &Extensao, canal: &str, pagina: bool) -> String {
    let cfg = json!({
        "id": e.id,
        "canal": canal,
        "pagina": pagina,
        "manifesto": e.manifesto,
        "mensagens": e.mensagens,
        "lingua": lingua(),
        "esquema": ESQUEMA,
    });
    format!("({API_JS})({});", js_valor(&cfg))
}

/// A API, numa função que recebe a configuração da extensão. As respostas da Jangada
/// chegam por `__jangadaResposta(k, {v} | {e})`; as mensagens para a extensão, por
/// `__jangadaEntrega`; as mudanças nos dados, por `__jangadaMudou`; o clique no botão, por
/// `__jangadaClique`.
const API_JS: &str = r#"function (cfg) {
  "use strict";
  var W = window;
  if (W.__jangadaApi) return;
  var id = cfg.id, canal = cfg.canal;
  var pend = {}, n = 0;
  function enviar(op, d, k) {
    W.webkit.messageHandlers[canal].postMessage(JSON.stringify({ op: op, k: k || 0, d: d === undefined ? null : d }));
  }
  function pedir(op, d) {
    var k = ++n;
    var p = new Promise(function (res, rej) {
      pend[k] = function (r) {
        if (r && typeof r === "object" && "e" in r) rej(new Error(r.e));
        else res(r && typeof r === "object" ? r.v : undefined);
      };
    });
    enviar(op, d, k);
    return p;
  }
  W.__jangadaResposta = function (k, r) {
    var f = pend[k];
    if (f) { delete pend[k]; f(r); }
  };
  var api = {};
  // Promessa e função de retorno, como no Chrome: com a função, o erro vai para
  // runtime.lastError durante a chamada dela.
  function chamar(op, d, cb) {
    var p = pedir(op, d);
    if (typeof cb === "function") {
      p.then(function (r) { cb(r); }, function (e) {
        api.runtime.lastError = { message: e.message };
        try { cb(); } finally { api.runtime.lastError = null; }
      });
      return undefined;
    }
    return p;
  }
  function evento() {
    var l = [];
    return {
      addListener: function (f) { if (typeof f === "function" && l.indexOf(f) < 0) l.push(f); },
      removeListener: function (f) { var i = l.indexOf(f); if (i >= 0) l.splice(i, 1); },
      hasListener: function (f) { return l.indexOf(f) >= 0; },
      hasListeners: function () { return l.length > 0; },
      _l: l,
      _disparar: function () {
        var a = arguments;
        l.slice().forEach(function (f) { try { f.apply(null, a); } catch (e) { console.error(e); } });
      }
    };
  }
  function url(p) {
    p = String(p == null ? "" : p);
    if (/^[a-z][a-z0-9+.-]*:/i.test(p)) return p;
    return cfg.esquema + "://" + id + "/" + p.replace(/^\/+/, "");
  }
  function mensagem(nome, subs) {
    nome = String(nome == null ? "" : nome);
    var m = cfg.mensagens[nome.toLowerCase()];
    if (!m) {
      if (nome === "@@extension_id") return id;
      if (nome === "@@ui_locale") return cfg.lingua.replace("-", "_");
      if (nome === "@@bidi_dir") return "ltr";
      if (nome === "@@bidi_reversed_dir") return "rtl";
      if (nome === "@@bidi_start_edge") return "left";
      if (nome === "@@bidi_end_edge") return "right";
      return "";
    }
    if (subs == null) subs = [];
    else if (!Array.isArray(subs)) subs = [subs];
    subs = subs.map(String);
    var ph = m.placeholders || {};
    var s = String(m.message || "");
    s = s.replace(/\$([a-zA-Z0-9_@]+)\$/g, function (t, k) {
      var p = ph[k.toLowerCase()];
      if (!p) return t;
      return String(p.content || "").replace(/\$(\d)/g, function (_, i) { return subs[i - 1] || ""; });
    });
    s = s.replace(/\$(\d)/g, function (_, i) { return subs[i - 1] || ""; });
    return s.replace(/\$\$/g, "$");
  }
  var mudou = evento(), mudouLocal = evento(), mudouSync = evento();
  function area(nome, ev) {
    return {
      get: function (c, cb) { if (typeof c === "function") { cb = c; c = null; } return chamar("storage.get", { area: nome, c: c == null ? null : c }, cb); },
      set: function (o, cb) { return chamar("storage.set", { area: nome, c: o || {} }, cb); },
      remove: function (c, cb) { return chamar("storage.remove", { area: nome, c: c }, cb); },
      clear: function (cb) { return chamar("storage.clear", { area: nome, c: null }, cb); },
      getBytesInUse: function (c, cb) { if (typeof c === "function") { cb = c; c = null; } return chamar("storage.bytes", { area: nome, c: c == null ? null : c }, cb); },
      QUOTA_BYTES: 10485760,
      onChanged: ev
    };
  }
  var aoMensagem = evento(), aoClicar = evento();
  // runtime.sendMessage([extensionId], message, [options], [callback])
  function argsDaMensagem(a) {
    a = Array.prototype.slice.call(a);
    var cb = typeof a[a.length - 1] === "function" ? a.pop() : null;
    var msg;
    if (a.length >= 3) msg = a[1];
    else if (a.length === 2) msg = (a[0] === id || a[0] === null) ? a[1] : a[0];
    else msg = a[0];
    return { msg: msg === undefined ? null : msg, cb: cb };
  }
  api.runtime = {
    id: id,
    lastError: null,
    getURL: url,
    getManifest: function () { return JSON.parse(JSON.stringify(cfg.manifesto)); },
    sendMessage: function () { var a = argsDaMensagem(arguments); return chamar("runtime.sendMessage", a.msg, a.cb); },
    onMessage: aoMensagem,
    onMessageExternal: evento(),
    onInstalled: evento(),
    onStartup: evento(),
    onSuspend: evento(),
    onConnect: evento(),
    openOptionsPage: function (cb) { return chamar("runtime.openOptionsPage", null, cb); },
    getPlatformInfo: function (cb) {
      var i = { os: "linux", arch: "x86-64", nacl_arch: "x86-64" };
      if (typeof cb === "function") { cb(i); return undefined; }
      return Promise.resolve(i);
    },
    setUninstallURL: function (u, cb) { if (typeof cb === "function") { cb(); return undefined; } return Promise.resolve(); }
  };
  api.storage = { local: area("local", mudouLocal), sync: area("sync", mudouSync), onChanged: mudou };
  api.i18n = {
    getMessage: mensagem,
    getUILanguage: function () { return cfg.lingua; },
    getAcceptLanguages: function (cb) {
      var l = [cfg.lingua];
      if (typeof cb === "function") { cb(l); return undefined; }
      return Promise.resolve(l);
    }
  };
  function cor(c) {
    if (Array.isArray(c)) return "rgba(" + (c[0] | 0) + "," + (c[1] | 0) + "," + (c[2] | 0) + "," + ((c.length > 3 ? c[3] : 255) / 255) + ")";
    return String(c == null ? "" : c);
  }
  api.action = {
    onClicked: aoClicar,
    setBadgeText: function (o, cb) { return chamar("action.set", { selo: o && o.text != null ? String(o.text) : "" }, cb); },
    getBadgeText: function (o, cb) { if (typeof o === "function") cb = o; return chamar("action.get", "selo", cb); },
    setBadgeBackgroundColor: function (o, cb) { return chamar("action.set", { cor: cor(o && o.color) }, cb); },
    getBadgeBackgroundColor: function (o, cb) { if (typeof o === "function") cb = o; return chamar("action.get", "cor", cb); },
    setBadgeTextColor: function (o, cb) { return chamar("action.set", { corTexto: cor(o && o.color) }, cb); },
    setTitle: function (o, cb) { return chamar("action.set", { titulo: o && o.title != null ? String(o.title) : "" }, cb); },
    getTitle: function (o, cb) { if (typeof o === "function") cb = o; return chamar("action.get", "titulo", cb); },
    setPopup: function (o, cb) { return chamar("action.set", { popup: o && o.popup != null ? String(o.popup) : "" }, cb); },
    getPopup: function (o, cb) { if (typeof o === "function") cb = o; return chamar("action.get", "popup", cb); },
    openPopup: function (o, cb) { if (typeof o === "function") cb = o; return chamar("action.openPopup", null, cb); },
    setIcon: function (o, cb) { if (typeof cb === "function") { cb(); return undefined; } return Promise.resolve(); },
    enable: function (t, cb) { if (typeof t === "function") cb = t; return chamar("action.set", { ligado: true }, cb); },
    disable: function (t, cb) { if (typeof t === "function") cb = t; return chamar("action.set", { ligado: false }, cb); }
  };
  api.browserAction = api.action;
  if (cfg.pagina) {
    api.tabs = {
      TAB_ID_NONE: -1,
      query: function (q, cb) { return chamar("tabs.query", q || {}, cb); },
      get: function (t, cb) { return chamar("tabs.get", t, cb); },
      getCurrent: function (cb) { return chamar("tabs.getCurrent", null, cb); },
      create: function (p, cb) {
        p = Object.assign({}, p);
        if (p.url != null) p.url = new URL(String(p.url), location.href).href;
        return chamar("tabs.create", p, cb);
      },
      update: function (t, p, cb) {
        if (t !== null && typeof t === "object") { cb = p; p = t; t = null; }
        p = Object.assign({}, p);
        if (p.url != null) p.url = new URL(String(p.url), location.href).href;
        return chamar("tabs.update", { tab: t == null ? null : t, p: p }, cb);
      },
      reload: function (t, o, cb) {
        if (typeof t === "function") { cb = t; t = null; } else if (typeof o === "function") { cb = o; }
        if (t !== null && typeof t === "object") t = null;
        return chamar("tabs.reload", t == null ? null : t, cb);
      },
      remove: function (t, cb) { return chamar("tabs.remove", Array.isArray(t) ? t : [t], cb); },
      sendMessage: function (t, m, o, cb) {
        if (typeof o === "function") { cb = o; o = null; }
        return chamar("tabs.sendMessage", { tab: t, msg: m === undefined ? null : m }, cb);
      },
      onUpdated: evento(), onActivated: evento(), onCreated: evento(), onRemoved: evento()
    };
    // Fechar a página da extensão: a janelinha some (ou a aba das opções fecha).
    W.close = function () { enviar("janela.fechar", null); };
  }
  // Uma mensagem para esta extensão (de outra página dela ou de um script de conteúdo).
  // A resposta vai uma vez só: a primeira chamada de sendResponse, ou o valor da promessa
  // que um ouvinte devolver. Devolver true diz que a resposta vem depois.
  W.__jangadaEntrega = function (rid, msg, remetente) {
    var l = aoMensagem._l.slice();
    if (!l.length) { enviar("msg.resposta", { rid: rid, ouvinte: false }); return; }
    var feito = false, espera = false;
    function responder(v) {
      if (feito) return;
      feito = true;
      enviar("msg.resposta", { rid: rid, ouvinte: true, tem: true, valor: v === undefined ? null : v });
    }
    l.forEach(function (f) {
      try {
        var r = f(msg, remetente, responder);
        if (r === true) espera = true;
        else if (r && typeof r.then === "function") {
          espera = true;
          r.then(responder, function (e) { console.error(e); responder(undefined); });
        }
      } catch (e) { console.error(e); }
    });
    if (!feito && !espera) enviar("msg.resposta", { rid: rid, ouvinte: true });
  };
  W.__jangadaMudou = function (mudancas, nomeArea) {
    mudou._disparar(mudancas, nomeArea);
    (nomeArea === "sync" ? mudouSync : mudouLocal)._disparar(mudancas);
  };
  W.__jangadaClique = function (aba) { aoClicar._disparar(aba); };
  W.__jangadaApi = api;
  if (cfg.pagina) { W.chrome = api; W.browser = api; W.jangada = api; }
}"#;

// ---------------------------------------------------------------------------
// Os dados guardados (storage.local e storage.sync, o mesmo lugar)
// ---------------------------------------------------------------------------

/// O nome de pasta de uma extensão instalada (o pedido vem de um script: nada de
/// caminhos, só uma extensão que existe).
fn instalada(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains(['/', '\\']) && pasta_extensoes().join(id).join("manifest.json").is_file()
}

/// A operação de storage pedida pela extensão (ext, op, dados) → o valor de volta.
#[cfg(test)]
pub fn armazenar(id: &str, op: &str, dados: &Value) -> Value {
    armazenar_e_mudancas(id, op, dados).0
}

/// A operação de storage → (o valor de volta, o que mudou, no formato do
/// `storage.onChanged`: {chave: {oldValue, newValue}}). Vazio se nada mudou.
pub fn armazenar_e_mudancas(id: &str, op: &str, dados: &Value) -> (Value, Map<String, Value>) {
    let mut mudancas = Map::new();
    if !instalada(id) {
        return (json!({}), mudancas);
    }
    let arq = pasta_extensoes().join(id).join("storage.json");
    let mut mapa: Map<String, Value> = std::fs::read_to_string(&arq).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let gravar = |mapa: &Map<String, Value>| {
        // Primeiro num arquivo ao lado, depois no lugar: uma queda no meio não perde tudo.
        let tmp = arq.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string(mapa).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &arq);
        }
    };
    let resposta = match op {
        "get" => {
            let mut r = Map::new();
            match dados {
                Value::Null => r = mapa.clone(),
                Value::String(k) => {
                    if let Some(v) = mapa.get(k) {
                        r.insert(k.clone(), v.clone());
                    }
                }
                Value::Array(ks) => {
                    for k in ks.iter().filter_map(|k| k.as_str()) {
                        if let Some(v) = mapa.get(k) {
                            r.insert(k.to_string(), v.clone());
                        }
                    }
                }
                Value::Object(padrao) => {
                    for (k, v) in padrao {
                        r.insert(k.clone(), mapa.get(k).cloned().unwrap_or(v.clone()));
                    }
                }
                _ => {}
            }
            Value::Object(r)
        }
        "bytes" => {
            let tamanho = |k: &str, v: &Value| k.len() + serde_json::to_string(v).map(|s| s.len()).unwrap_or(0);
            let n: usize = match dados {
                Value::Null => mapa.iter().map(|(k, v)| tamanho(k, v)).sum(),
                Value::String(k) => mapa.get(k).map(|v| tamanho(k, v)).unwrap_or(0),
                Value::Array(ks) => ks.iter().filter_map(|k| k.as_str()).filter_map(|k| mapa.get(k).map(|v| tamanho(k, v))).sum(),
                _ => 0,
            };
            json!(n)
        }
        "set" => {
            if let Value::Object(o) = dados {
                for (k, v) in o {
                    let velho = mapa.insert(k.clone(), v.clone());
                    if velho.as_ref() != Some(v) {
                        let mut m = Map::new();
                        if let Some(velho) = velho {
                            m.insert("oldValue".into(), velho);
                        }
                        m.insert("newValue".into(), v.clone());
                        mudancas.insert(k.clone(), Value::Object(m));
                    }
                }
            }
            if !mudancas.is_empty() {
                gravar(&mapa);
            }
            json!({})
        }
        "remove" => {
            let ks: Vec<String> = match dados {
                Value::String(k) => vec![k.clone()],
                Value::Array(a) => a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect(),
                _ => vec![],
            };
            for k in ks {
                if let Some(velho) = mapa.remove(&k) {
                    mudancas.insert(k, json!({ "oldValue": velho }));
                }
            }
            if !mudancas.is_empty() {
                gravar(&mapa);
            }
            json!({})
        }
        "clear" => {
            for (k, velho) in std::mem::take(&mut mapa) {
                mudancas.insert(k, json!({ "oldValue": velho }));
            }
            gravar(&mapa);
            json!({})
        }
        _ => json!({}),
    };
    (resposta, mudancas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_do_github_com_pasta() {
        assert_eq!(subpasta_github("https://github.com/u/r/tree/master/exemplos/destaque").as_deref(), Some("exemplos/destaque"));
        assert!(subpasta_github("https://github.com/u/r").is_none());
        assert!(subpasta_github("https://github.com/u/r/tree/master").is_none());
    }

    #[test]
    fn acha_o_manifesto_na_subpasta() {
        let base = std::env::temp_dir().join(format!("jangada-manifesto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("repo-main/exemplos/destaque")).unwrap();
        std::fs::write(base.join("repo-main/exemplos/destaque/manifest.json"), "{}").unwrap();
        let achado = achar_manifesto(&base.join("repo-main")).expect("tem de achar");
        assert!(achado.ends_with("exemplos/destaque"));
        assert!(achar_manifesto(&base.join("repo-main/exemplos")).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nome_de_pasta_da_extensao() {
        assert!(id_para("Olá, Jangada", "{}").starts_with("ola-jangada-"));
        assert!(id_para("Destaque de exemplo", "{}").starts_with("destaque-de-exemplo-"));
        assert!(id_para("!!!", "{}").starts_with("extensao-"));
        assert_ne!(id_para("A", "{1}"), id_para("A", "{2}"));
    }

    #[test]
    fn instalar_de_novo_troca_a_anterior() {
        let nome = format!("Teste de troca {}", std::process::id());
        let fonte = std::env::temp_dir().join(format!("jangada-troca-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fonte);
        std::fs::create_dir_all(&fonte).unwrap();
        let manifesto = |versao: &str| format!(r#"{{"manifest_version":3,"name":"{nome}","version":"{versao}","content_scripts":[{{"matches":["https://exemplo.com/*"],"js":["a.js"]}}]}}"#);
        std::fs::write(fonte.join("a.js"), "1;").unwrap();
        std::fs::write(fonte.join("manifest.json"), manifesto("1.0")).unwrap();
        // O que começa com ponto (o .git de um clone) não vai junto.
        std::fs::create_dir_all(fonte.join(".git")).unwrap();
        std::fs::write(fonte.join(".git/HEAD"), "ref").unwrap();
        let um = instalar(&ler(&fonte).unwrap()).unwrap();
        assert!(!um.pasta.join(".git").exists());
        armazenar(&um.id, "set", &json!({"guardado": true}));
        // O manifesto mudou (versão nova): continua uma só, na mesma pasta, com os dados.
        std::fs::write(fonte.join("manifest.json"), manifesto("2.0")).unwrap();
        let dois = instalar(&ler(&fonte).unwrap()).unwrap();
        assert_eq!(um.id, dois.id);
        let minhas: Vec<Extensao> = listar().into_iter().filter(|e| e.nome == nome).collect();
        assert_eq!(minhas.len(), 1);
        assert_eq!(minhas[0].versao, "2.0");
        assert_eq!(armazenar(&dois.id, "get", &Value::Null)["guardado"], true);
        remover(&dois.id);
        assert!(listar().iter().all(|e| e.nome != nome));
        let _ = std::fs::remove_dir_all(&fonte);
    }

    #[test]
    fn padroes_do_chrome() {
        assert!(casa("<all_urls>", "https://exemplo.com.br/a?b=1"));
        assert!(casa("https://*.exemplo.com/*", "https://www.exemplo.com/x"));
        assert!(casa("https://*.exemplo.com/*", "https://exemplo.com/"));
        assert!(!casa("https://*.exemplo.com/*", "https://outro.com/x"));
        assert!(casa("*://*/*", "http://qualquer.site/p"));
        assert!(!casa("https://exemplo.com/docs/*", "https://exemplo.com/outro"));
        assert!(casa("http://127.0.0.1/*", "http://127.0.0.1:8765/pagina"));
        assert!(casa("https://exemplo.com/docs/*", "https://exemplo.com/docs/a/b"));
    }

    #[test]
    fn repositorios_do_github() {
        assert_eq!(repositorio_github("https://github.com/fulano/ext").map(|r| (r.0, r.1)), Some(("fulano".into(), "ext".into())));
        assert_eq!(repositorio_github("fulano/ext.git").map(|r| r.1), Some("ext".into()));
        assert_eq!(repositorio_github("https://github.com/fulano/ext/tree/dev/sub").and_then(|r| r.2), Some("dev".into()));
        assert!(repositorio_github("/home/fulano/ext").is_none());
        assert!(repositorio_github("./pasta").is_none());
    }

    #[test]
    fn storage_guarda_e_le() {
        let id = format!("teste-{}", std::process::id());
        let dir = pasta_extensoes().join(&id);
        let _ = std::fs::create_dir_all(&dir);
        // Só uma extensão instalada guarda dados; nada de caminhos no lugar do nome.
        armazenar(&id, "set", &json!({"n": 3}));
        assert!(!dir.join("storage.json").exists());
        std::fs::write(dir.join("manifest.json"), "{}").unwrap();
        assert!(armazenar(&format!("../extensoes/{id}"), "get", &Value::Null).as_object().is_some_and(|o| o.is_empty()));
        armazenar(&id, "set", &json!({"n": 3, "cor": "azul"}));
        assert_eq!(armazenar(&id, "get", &json!("n"))["n"], 3);
        assert_eq!(armazenar(&id, "get", &json!(["cor"]))["cor"], "azul");
        armazenar(&id, "remove", &json!("n"));
        assert!(armazenar(&id, "get", &Value::Null).get("n").is_none());
        armazenar(&id, "clear", &Value::Null);
        assert!(armazenar(&id, "get", &Value::Null).as_object().is_some_and(|o| o.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_conta_o_que_mudou() {
        let id = format!("teste-mudou-{}", std::process::id());
        let dir = pasta_extensoes().join(&id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), "{}").unwrap();
        let (_, m) = armazenar_e_mudancas(&id, "set", &json!({"a": 1, "b": "x"}));
        assert_eq!(m["a"], json!({"newValue": 1}));
        // Gravar o mesmo valor não é mudança.
        let (_, m) = armazenar_e_mudancas(&id, "set", &json!({"a": 1, "b": "y"}));
        assert!(m.get("a").is_none());
        assert_eq!(m["b"], json!({"oldValue": "x", "newValue": "y"}));
        let (_, m) = armazenar_e_mudancas(&id, "remove", &json!(["a", "nao-existe"]));
        assert_eq!(m.len(), 1);
        assert_eq!(m["a"], json!({"oldValue": 1}));
        let (_, m) = armazenar_e_mudancas(&id, "clear", &Value::Null);
        assert_eq!(m["b"], json!({"oldValue": "y"}));
        assert!(armazenar(&id, "bytes", &Value::Null) == json!(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Uma extensão completa numa pasta temporária: ação com janelinha, opções, ícones,
    /// recursos acessíveis e textos traduzidos.
    fn extensao_completa(nome_pasta: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("{nome_pasta}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("_locales/en")).unwrap();
        std::fs::create_dir_all(d.join("_locales/pt_BR")).unwrap();
        std::fs::create_dir_all(d.join("img")).unwrap();
        std::fs::write(
            d.join("manifest.json"),
            r#"{"manifest_version":3,"name":"__MSG_nome__","version":"1","default_locale":"en",
                "action":{"default_popup":"popup.html","default_title":"__MSG_titulo__","default_icon":{"16":"img/16.png","48":"img/48.png"}},
                "icons":{"128":"img/128.png"},
                "options_ui":{"page":"opcoes.html"},
                "host_permissions":["*://api.exemplo.com/*","<all_urls>"],
                "web_accessible_resources":[{"resources":["img/*"],"matches":["<all_urls>"]}]}"#,
        )
        .unwrap();
        std::fs::write(d.join("_locales/en/messages.json"), r#"{"nome":{"message":"Name EN"},"titulo":{"message":"Title EN"},"so_en":{"message":"only $1"}}"#).unwrap();
        std::fs::write(d.join("_locales/pt_BR/messages.json"), r#"{"Nome":{"message":"Nome PT"}}"#).unwrap();
        for f in ["popup.html", "opcoes.html", "img/16.png", "img/48.png", "img/128.png"] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        d
    }

    #[test]
    fn manifesto_completo() {
        let d = extensao_completa("jangada-completa");
        let m = ler_mensagens(&d, Some("en"), "pt_BR");
        // A língua da pessoa cobre a padrão; o que falta nela vem da padrão.
        assert_eq!(m["nome"]["message"], "Nome PT");
        assert_eq!(m["so_en"]["message"], "only $1");
        assert_eq!(traduzir("[__MSG_Titulo__] __MSG_nao_existe__!", &m), "[Title EN] !");
        let e = ler(&d).unwrap();
        let a = e.acao.as_ref().expect("tem ação");
        assert_eq!(a.popup.as_deref(), Some("popup.html"));
        assert_eq!(e.opcoes.as_deref(), Some("opcoes.html"));
        assert!(e.icone(16).unwrap().ends_with("img/16.png"));
        assert!(e.icone(32).unwrap().ends_with("img/48.png"));
        assert!(e.icone(64).unwrap().ends_with("img/48.png"), "os do botão vêm antes dos da extensão");
        assert!(e.acessivel("img/16.png") && e.acessivel("/img/x/y.png"));
        assert!(!e.acessivel("popup.html"));
        assert_eq!(e.hosts_do_motor(), vec!["http://api.exemplo.com/*", "https://api.exemplo.com/*", "http://*/*", "https://*/*"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn manifesto_com_arquivo_que_falta() {
        let d = extensao_completa("jangada-falta");
        std::fs::remove_file(d.join("popup.html")).unwrap();
        assert!(ler(&d).unwrap_err().contains("popup.html"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn arquivos_so_de_dentro_da_pasta() {
        let d = extensao_completa("jangada-ext-arquivos");
        assert!(arquivo(&d, "popup.html").is_some());
        assert!(arquivo(&d, "/img/16.png").is_some());
        assert!(arquivo(&d, "img/%31%36.png").is_some(), "o caminho chega codificado");
        assert!(arquivo(&d, "../manifest.json").is_none());
        assert!(arquivo(&d, "img/../../x").is_none());
        assert!(arquivo(&d, "%2e%2e/etc/passwd").is_none());
        assert!(arquivo(&d, "img").is_none(), "pasta não é arquivo");
        assert!(arquivo(&d, "").is_none());
        // Um atalho que aponta para fora não serve.
        std::os::unix::fs::symlink("/etc/hostname", d.join("fora")).unwrap();
        assert!(arquivo(&d, "fora").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn enderecos_das_extensoes() {
        assert_eq!(url_de("abc-1", "/a/b.html"), "jangada-extension://abc-1/a/b.html");
        assert_eq!(partes_da_url("jangada-extension://abc-1/a/b.html?x=1#y"), Some(("abc-1".into(), "a/b.html".into())));
        assert_eq!(partes_da_url("jangada-extension://abc-1"), Some(("abc-1".into(), "".into())));
        assert!(partes_da_url("https://abc-1/a").is_none());
        assert_eq!(tipo_do_arquivo(Path::new("a/b.JS")), "text/javascript");
        assert_eq!(decodificar("a%20b%2"), "a b%2");
        assert_eq!(decodificar("%é%"), "%é%");
    }

    #[test]
    fn padrao_vira_regex_como_casa() {
        let casa_regex = |p: &str, u: &str| regex_do_padrao(p).is_some_and(|r| regex::Regex::new(&r).unwrap().is_match(u));
        for (p, u, esperado) in [
            ("*://*.youtube.com/*", "https://www.youtube.com/watch?v=1", true),
            ("*://*.youtube.com/*", "http://youtube.com:8080/", true),
            ("*://*.youtube.com/*", "https://youtube.com.evil.net/", false),
            ("*://*.youtube.com/*", "https://evilyoutube.com/", false),
            ("https://example.com/docs/*", "https://example.com/docs/a#x", true),
            ("https://example.com/docs/*", "https://example.com/outro", false),
            ("<all_urls>", "https://a.b/c", true),
            ("<all_urls>", "jangada-extension://x/y", false),
        ] {
            assert_eq!(casa_regex(p, u), esperado, "{p} x {u}");
            assert_eq!(casa(p, u), esperado, "casa {p} x {u}");
        }
    }

    #[test]
    fn regra_do_comeco_do_documento() {
        let m: serde_json::Value = serde_json::from_str(r#"{"content_scripts":[{"matches":["*://*.youtube.com/*"],"js":["a.js"],"run_at":"document_start"},{"matches":["<all_urls>"],"js":["b.js"]}]}"#).unwrap();
        let regras: Vec<bool> = m["content_scripts"].as_array().unwrap().iter().map(|r| r["run_at"].as_str() == Some("document_start")).collect();
        assert_eq!(regras, [true, false]);
    }
}
