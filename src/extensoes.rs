//! As extensões da Jangada: um formato próprio e pequeno. Uma pasta com `manifest.json`,
//! scripts (JavaScript) e CSS que entram nas páginas que batem com `matches`, e uma API
//! mínima para o script: `jangada.storage.local` e `jangada.runtime.id` (o mesmo objeto
//! atende por `chrome`, para quem traz um script escrito para o Chrome). O manifesto usa
//! os nomes de campo do Chrome (Manifest V3), mas isto NÃO roda as extensões da loja do
//! Chrome: não há script de fundo, janela de ação, páginas de opções nem as outras APIs.
//! O guia de quem escreve uma extensão está em docs/EXTENSOES.md.
//!
//! Instala-se de uma pasta, de um `.zip` ou de um repositório do GitHub (pela linha de
//! comando, `jangada --instalar`, ou em Preferências ▸ Instaladas). Antes de instalar, a
//! Jangada mostra o que a extensão pede.
//!
//! Os scripts rodam assim que o documento da página fica pronto, no quadro principal, num mundo
//! isolado (o "jangada", o mesmo dos scripts da própria Jangada): veem o documento, mas
//! não as variáveis da página, e a página não os vê.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// A marca das mensagens das extensões para a Jangada (pelo canal do mundo isolado).
pub const MARCA: &str = "\u{1}JANGADA:x:";

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
}

#[derive(Clone, Debug)]
pub struct Regra {
    pub matches: Vec<String>,
    pub js: Vec<String>,
    pub css: Vec<String>,
}

fn pasta_extensoes() -> PathBuf {
    // Nos testes, uma pasta à parte: nunca a das extensões da pessoa.
    #[cfg(test)]
    let p = std::env::temp_dir().join(format!("jangada-extensoes-de-teste-{}", std::process::id()));
    #[cfg(not(test))]
    let p = {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("jangada/extensoes")
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
    pasta_extensoes().join("ativas.txt")
}

fn desativadas() -> Vec<String> {
    std::fs::read_to_string(ativas_arquivo()).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

/// Lê o manifesto de uma pasta de extensão e devolve a extensão (sem instalar).
pub fn ler(pasta: &Path) -> Result<Extensao, String> {
    let texto = std::fs::read_to_string(pasta.join("manifest.json")).map_err(|_| glove_config::tr!("A pasta não tem um manifest.json.").to_string())?;
    let m: Value = serde_json::from_str(&texto).map_err(|e| glove_config::tr!("O manifest.json não é um JSON válido: {}", e))?;
    let versao_manifesto = m["manifest_version"].as_u64().unwrap_or(0);
    if versao_manifesto != 3 {
        return Err(glove_config::tr!("A Jangada aceita extensões do Manifest V3 (o manifest_version é {}).", versao_manifesto));
    }
    let nome = m["name"].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| glove_config::tr!("O manifesto não tem o nome."))?.to_string();
    let versao = m["version"].as_str().unwrap_or("0").to_string();
    let descricao = m["description"].as_str().unwrap_or("").to_string();
    let mut regras = Vec::new();
    for r in m["content_scripts"].as_array().cloned().unwrap_or_default() {
        let lista = |k: &str| -> Vec<String> { r[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let js = lista("js");
        let css = lista("css");
        for f in js.iter().chain(css.iter()) {
            if !pasta.join(f).is_file() {
                return Err(glove_config::tr!("O arquivo {} do manifesto não existe.", f));
            }
        }
        regras.push(Regra { matches: lista("matches"), js, css });
    }
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
    Ok(Extensao { id: id.clone(), nome, versao, descricao, pasta: pasta.to_path_buf(), regras, permissoes, ativa: !desativadas().contains(&id) })
}

/// As extensões instaladas. A identidade de uma instalada é o nome da pasta dela (os
/// dados guardados ficam ali): não muda quando o manifesto muda.
pub fn listar() -> Vec<Extensao> {
    let mut v: Vec<Extensao> = std::fs::read_dir(pasta_extensoes())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("manifest.json").is_file())
        .filter_map(|e| {
            let mut x = ler(&e.path()).ok()?;
            x.id = e.file_name().to_string_lossy().into_owned();
            x.ativa = !desativadas().contains(&x.id);
            Some(x)
        })
        .collect();
    v.sort_by(|a, b| a.nome.to_lowercase().cmp(&b.nome.to_lowercase()));
    v
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
    let mut inst = ler(&destino)?;
    inst.id = id;
    inst.ativa = true;
    Ok(inst)
}

fn copiar(de: &Path, para: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(para)?;
    for e in std::fs::read_dir(de)? {
        let e = e?;
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
}

pub fn alternar(id: &str) {
    let mut v = desativadas();
    if let Some(i) = v.iter().position(|x| x == id) {
        v.remove(i);
    } else {
        v.push(id.to_string());
    }
    let _ = std::fs::write(ativas_arquivo(), v.join("\n") + "\n");
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

fn baixar_github(repo: &(String, String, Option<String>), dir: &Path) -> Result<PathBuf, String> {
    let (usuario, nome, ramo) = repo;
    let ramos: Vec<String> = match ramo {
        Some(r) => vec![r.clone()],
        None => vec!["main".into(), "master".into()],
    };
    for r in ramos {
        let url = format!("https://github.com/{usuario}/{nome}/archive/refs/heads/{r}.zip");
        let arq = dir.join("repo.zip");
        let ok = std::process::Command::new("curl").args(["-fsSL", "--max-time", "120", "-o"]).arg(&arq).arg(&url).status().is_ok_and(|s| s.success())
            || std::process::Command::new("wget").args(["-q", "-T", "120", "-O"]).arg(&arq).arg(&url).status().is_ok_and(|s| s.success());
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

fn glob(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&p[1..], s) || (!s.is_empty() && glob(p, &s[1..])),
        (Some(a), Some(b)) if a == b => glob(&p[1..], &s[1..]),
        _ => false,
    }
}

fn js_str(t: &str) -> String {
    serde_json::to_string(t).unwrap_or_else(|_| "\"\"".into()).replace('<', "\\u003c")
}

/// O que a Jangada injeta numa página: para cada extensão ativa com uma regra que bate
/// com o endereço, o CSS e os scripts, na ordem do manifesto. Cada pedaço é avaliado à
/// parte (um erro de escrita num script não leva os outros) e cada script recebe a API
/// da própria extensão, como `jangada` e como `chrome`. Vazio se nenhuma se aplica.
pub fn scripts_para(url: &str) -> Vec<String> {
    let mut partes = Vec::new();
    for e in listar().into_iter().filter(|e| e.ativa) {
        for r in &e.regras {
            if !r.matches.iter().any(|m| casa(m, url)) {
                continue;
            }
            for c in &r.css {
                if let Ok(t) = std::fs::read_to_string(e.pasta.join(c)) {
                    partes.push(format!("(function(){{var s=document.createElement('style');s.textContent={};(document.head||document.documentElement).appendChild(s)}})();", js_str(&t)));
                }
            }
            for j in &r.js {
                if let Ok(t) = std::fs::read_to_string(e.pasta.join(j)) {
                    partes.push(format!("(function(jangada){{var chrome=jangada;try{{\n{t}\n}}catch(e){{console.error({}+': '+e)}}}})({});", js_str(&format!("{} ({j})", e.nome)), api_de(&e.id)));
                }
            }
        }
    }
    partes
}

/// A API de uma extensão (o objeto `jangada`, que também atende por `chrome`), montada
/// na página: runtime.id e storage.local (get, set, remove e clear, que vão para a
/// Jangada pelo canal do mundo isolado). Os pedidos à espera e o número deles são um só
/// para o mundo inteiro: com um por script, as respostas do segundo script da página
/// (outro arquivo, outra extensão) iam para o primeiro e nunca chegavam.
fn api_de(id: &str) -> String {
    format!(
        "(function(){{var id={};var W=window;W.__jangadaPend=W.__jangadaPend||{{}};W.__jangadaN=W.__jangadaN||0;\
W.__jangadaResposta=W.__jangadaResposta||function(k,d){{var f=W.__jangadaPend[k];if(f){{delete W.__jangadaPend[k];f(d)}}}};\
function op(t,d,cb){{var k=++W.__jangadaN;var p=new Promise(function(res){{W.__jangadaPend[k]=function(r){{if(typeof cb=='function')cb(r);res(r)}}}});W.webkit.messageHandlers.jangada.postMessage({marca}+JSON.stringify({{ext:id,op:t,k:k,d:d}}));return p}}\
var local={{get:function(c,cb){{if(typeof c=='function'){{cb=c;c=null}}return op('get',c==null?null:c,cb)}},set:function(o,cb){{return op('set',o,cb)}},remove:function(c,cb){{return op('remove',c,cb)}},clear:function(cb){{return op('clear',null,cb)}}}};\
var listener={{addListener:function(){{}},removeListener:function(){{}},hasListener:function(){{return false}}}};\
return {{runtime:{{id:id,getURL:function(p){{return 'jangada-extension://'+id+'/'+String(p||'').replace(/^\\//,'')}},onMessage:listener,lastError:null}},storage:{{local:local,onChanged:listener}}}}}})()",
        js_str(id),
        marca = js_str(MARCA),
    )
}

/// O nome de pasta de uma extensão instalada (o `ext` de um pedido vem do script: nada
/// de caminhos, só uma extensão que existe).
fn instalada(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains(['/', '\\']) && pasta_extensoes().join(id).join("manifest.json").is_file()
}

/// A operação de storage pedida pela página (ext, op, dados) → o valor de volta.
pub fn armazenar(id: &str, op: &str, dados: &Value) -> Value {
    if !instalada(id) {
        return json!({});
    }
    let arq = pasta_extensoes().join(id).join("storage.json");
    let mut mapa: serde_json::Map<String, Value> = std::fs::read_to_string(&arq).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let resposta = match op {
        "get" => {
            let mut r = serde_json::Map::new();
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
        "set" => {
            if let Value::Object(o) = dados {
                for (k, v) in o {
                    mapa.insert(k.clone(), v.clone());
                }
            }
            let _ = std::fs::write(&arq, serde_json::to_string(&mapa).unwrap_or_default());
            json!({})
        }
        "remove" => {
            let ks: Vec<String> = match dados {
                Value::String(k) => vec![k.clone()],
                Value::Array(a) => a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect(),
                _ => vec![],
            };
            for k in ks {
                mapa.remove(&k);
            }
            let _ = std::fs::write(&arq, serde_json::to_string(&mapa).unwrap_or_default());
            json!({})
        }
        "clear" => {
            let _ = std::fs::write(&arq, "{}");
            json!({})
        }
        _ => json!({}),
    };
    resposta
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
        let um = instalar(&ler(&fonte).unwrap()).unwrap();
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
}
