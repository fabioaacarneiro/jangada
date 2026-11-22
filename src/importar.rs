//! Importar de outro navegador da família do Chrome (Google Chrome, Chromium, Brave,
//! Microsoft Edge, Vivaldi), do perfil dele neste computador: os favoritos, o histórico,
//! as sessões abertas nos sites (os cookies) e as senhas.
//!
//! Os cookies e as senhas vêm cifrados (AES-128-CBC). A chave sai do chaveiro do sistema
//! ("Chrome Safe Storage" e afins: v11) ou é a fixa do Chromium sem chaveiro (v10).
//! A sincronização com a conta do Google não existe fora do Chrome; o que vem é o que
//! está guardado no computador (inclusive as senhas da conta que o Chrome guarda aqui).

use std::path::{Path, PathBuf};

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};

/// Um perfil de navegador encontrado.
#[derive(Clone, Debug)]
pub struct Fonte {
    /// "Google Chrome — Pessoa 1".
    pub nome: String,
    pub pasta: PathBuf,
    /// Os nomes do navegador no chaveiro (o atributo `application`).
    apps: &'static [&'static str],
}

const NAVEGADORES: [(&str, &str, &[&str]); 5] = [
    ("Google Chrome", ".config/google-chrome", &["chrome"]),
    ("Chromium", ".config/chromium", &["chromium"]),
    ("Brave", ".config/BraveSoftware/Brave-Browser", &["brave"]),
    ("Microsoft Edge", ".config/microsoft-edge", &["microsoft-edge", "chromium"]),
    ("Vivaldi", ".config/vivaldi", &["vivaldi"]),
];

/// Os perfis dos navegadores instalados (os que têm dados).
pub fn fontes() -> Vec<Fonte> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut v = Vec::new();
    for (nome, dir, apps) in NAVEGADORES {
        let base = home.join(dir);
        let Ok(ler) = std::fs::read_dir(&base) else { continue };
        // Os nomes dos perfis ("Pessoa 1", o nome da conta) ficam no Local State.
        let estado: serde_json::Value = std::fs::read_to_string(base.join("Local State")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let mut perfis: Vec<PathBuf> = ler
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == "Default" || n.starts_with("Profile ")))
            .filter(|p| p.join("History").exists() || p.join("Login Data").exists())
            .collect();
        perfis.sort();
        let varios = perfis.len() > 1;
        for p in perfis {
            let id = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let rotulo = estado["profile"]["info_cache"][&id]["name"].as_str().map(str::to_string).unwrap_or_else(|| id.clone());
            let nome = if varios { format!("{nome} — {rotulo}") } else { nome.to_string() };
            v.push(Fonte { nome, pasta: p, apps });
        }
    }
    v
}

/// O que importar.
#[derive(Clone, Copy, Debug)]
pub struct Partes {
    pub favoritos: bool,
    pub historico: bool,
    pub sessoes: bool,
    pub senhas: bool,
}

/// O resultado (quantos de cada) e os problemas.
#[derive(Clone, Debug, Default)]
pub struct Resultado {
    pub favoritos: usize,
    pub historico: usize,
    pub senhas: usize,
    /// Os cookies (as sessões) decifrados, para o motor gravar (na linha principal).
    pub cookies: Vec<Biscoito>,
    pub problemas: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Biscoito {
    pub host: String,
    pub nome: String,
    pub valor: String,
    pub caminho: String,
    pub seguro: bool,
    pub http_only: bool,
    /// Microssegundos desde 1601 (0 = da sessão).
    pub expira: i64,
    pub same_site: i64,
}

/// As chaves possíveis: a do chaveiro (v11) e a fixa (v10), já derivadas.
struct Chaves {
    v11: Option<[u8; 16]>,
    v10: [u8; 16],
}

fn derivar(segredo: &[u8]) -> [u8; 16] {
    let mut k = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(segredo, b"saltysalt", 1, &mut k);
    k
}

fn chaves(f: &Fonte) -> Chaves {
    let v11 = crate::chaveiro::Chaveiro::abrir().ok().and_then(|c| f.apps.iter().find_map(|a| c.chave_de(a))).map(|s| derivar(&s));
    Chaves { v11, v10: derivar(b"peanuts") }
}

/// Decifra um valor do Chrome ("v10"/"v11" + AES-128-CBC). Sem prefixo, é texto puro.
fn decifrar(dados: &[u8], k: &Chaves) -> Option<Vec<u8>> {
    let (chave, resto) = match dados.get(..3) {
        Some(b"v11") => (k.v11?, &dados[3..]),
        Some(b"v10") => (k.v10, &dados[3..]),
        _ => return Some(dados.to_vec()),
    };
    let iv = [b' '; 16];
    let mut buf = resto.to_vec();
    let d = cbc::Decryptor::<aes::Aes128>::new(&chave.into(), &iv.into());
    d.decrypt_padded_mut::<Pkcs7>(&mut buf).ok().map(|s| s.to_vec())
}

/// Abre uma cópia do banco (o navegador aberto o deixa travado).
fn abrir_copia(origem: &Path) -> Option<(rusqlite::Connection, PathBuf)> {
    if !origem.exists() {
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("jangada-importar-{}-{}", std::process::id(), origem.file_name()?.to_string_lossy().replace(' ', "_")));
    std::fs::copy(origem, &tmp).ok()?;
    let c = rusqlite::Connection::open_with_flags(&tmp, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    Some((c, tmp))
}

fn favoritos_de(f: &Fonte) -> Vec<(String, String)> {
    fn juntar(n: &serde_json::Value, out: &mut Vec<(String, String)>) {
        if n["type"] == "url" {
            if let (Some(t), Some(u)) = (n["name"].as_str(), n["url"].as_str()) {
                if u.starts_with("http") {
                    out.push((t.to_string(), u.to_string()));
                }
            }
        }
        if let Some(c) = n["children"].as_array() {
            for x in c {
                juntar(x, out);
            }
        }
    }
    let mut v = Vec::new();
    for arq in ["Bookmarks", "AccountBookmarks"] {
        let Some(j) = std::fs::read_to_string(f.pasta.join(arq)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        if let Some(raizes) = j["roots"].as_object() {
            for r in raizes.values() {
                juntar(r, &mut v);
            }
        }
    }
    v
}

fn historico_de(f: &Fonte) -> Vec<(String, String)> {
    let Some((c, tmp)) = abrir_copia(&f.pasta.join("History")) else { return Vec::new() };
    let v = c
        .prepare("SELECT title, url FROM urls WHERE hidden = 0 ORDER BY last_visit_time DESC LIMIT 2000")
        .and_then(|mut s| s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map(|it| it.flatten().collect::<Vec<_>>()))
        .unwrap_or_default();
    drop(c);
    let _ = std::fs::remove_file(tmp);
    v.into_iter().filter(|(_, u)| u.starts_with("http")).collect()
}

fn cookies_de(f: &Fonte, k: &Chaves) -> (Vec<Biscoito>, usize) {
    let arq = if f.pasta.join("Network/Cookies").exists() { f.pasta.join("Network/Cookies") } else { f.pasta.join("Cookies") };
    let Some((c, tmp)) = abrir_copia(&arq) else { return (Vec::new(), 0) };
    // A partir da versão 24 o valor decifrado começa com 32 bytes do domínio (SHA-256).
    let versao: i64 = c.query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut falhas = 0;
    let linhas: Vec<(String, String, String, Vec<u8>, String, bool, bool, i64, i64)> = c
        .prepare("SELECT host_key, name, value, encrypted_value, path, is_secure, is_httponly, expires_utc, samesite FROM cookies")
        .and_then(|mut s| {
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get::<_, i64>(5)? != 0, r.get::<_, i64>(6)? != 0, r.get(7)?, r.get(8)?)))
                .map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    drop(c);
    let _ = std::fs::remove_file(tmp);
    let mut v = Vec::new();
    for (host, nome, valor, cifrado, caminho, seguro, http_only, expira, same_site) in linhas {
        let valor = if !valor.is_empty() || cifrado.is_empty() {
            valor
        } else {
            match decifrar(&cifrado, k) {
                Some(mut d) => {
                    if versao >= 24 && d.len() >= 32 {
                        d.drain(..32);
                    }
                    match String::from_utf8(d) {
                        Ok(s) => s,
                        Err(_) => {
                            falhas += 1;
                            continue;
                        }
                    }
                }
                None => {
                    falhas += 1;
                    continue;
                }
            }
        };
        v.push(Biscoito { host, nome, valor, caminho, seguro, http_only, expira, same_site });
    }
    (v, falhas)
}

/// As senhas: (origem, usuário, senha) e quantas não deu para decifrar.
fn senhas_de(f: &Fonte, k: &Chaves) -> (Vec<(String, String, String)>, usize) {
    let mut v = Vec::new();
    let mut falhas = 0;
    for arq in ["Login Data", "Login Data For Account"] {
        let Some((c, tmp)) = abrir_copia(&f.pasta.join(arq)) else { continue };
        let linhas: Vec<(String, String, Vec<u8>)> = c
            .prepare("SELECT origin_url, username_value, password_value FROM logins WHERE blacklisted_by_user = 0")
            .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|it| it.flatten().collect()))
            .unwrap_or_default();
        drop(c);
        let _ = std::fs::remove_file(tmp);
        for (url, usuario, cifrada) in linhas {
            match decifrar(&cifrada, k).and_then(|d| String::from_utf8(d).ok()) {
                Some(s) if !s.is_empty() => v.push((origem(&url), usuario, s)),
                Some(_) => {}
                None => falhas += 1,
            }
        }
    }
    v.sort();
    v.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    (v, falhas)
}

/// "https://site.com" (sem o caminho) de um endereço.
pub fn origem(url: &str) -> String {
    let Some((esquema, resto)) = url.split_once("://") else { return url.to_string() };
    let host = resto.split(['/', '?', '#']).next().unwrap_or("");
    format!("{esquema}://{host}")
}

/// Importa (numa linha à parte: pode demorar). Os cookies voltam no resultado para o
/// motor gravar na linha principal.
pub fn importar(f: &Fonte, partes: Partes) -> Resultado {
    let mut r = Resultado::default();
    let k = chaves(f);
    if partes.favoritos {
        let mut atuais = crate::ajustes::favoritos();
        for (t, u) in favoritos_de(f) {
            if !atuais.iter().any(|(_, x)| *x == u) {
                atuais.push((t, u));
                r.favoritos += 1;
            }
        }
        crate::ajustes::gravar_favoritos(&atuais);
    }
    if partes.historico {
        r.historico = crate::ajustes::juntar_historico(&historico_de(f));
    }
    if (partes.sessoes || partes.senhas) && k.v11.is_none() {
        r.problemas.push(glove_config::tr!("Não achei a chave do navegador no chaveiro: só os dados sem a chave do sistema puderam ser lidos.").to_string());
    }
    if partes.sessoes {
        let (c, falhas) = cookies_de(f, &k);
        if falhas > 0 {
            r.problemas.push(glove_config::tr!("{} cookies não puderam ser decifrados.", falhas));
        }
        r.cookies = c;
    }
    if partes.senhas {
        let (s, falhas) = senhas_de(f, &k);
        if falhas > 0 {
            r.problemas.push(glove_config::tr!("{} senhas não puderam ser decifradas.", falhas));
        }
        match crate::chaveiro::Chaveiro::abrir() {
            Ok(c) => {
                for (o, u, p) in &s {
                    if c.guardar(o, u, p).is_ok() {
                        r.senhas += 1;
                    }
                }
            }
            Err(e) => r.problemas.push(glove_config::tr!("O chaveiro do sistema não respondeu: {}", e)),
        }
    }
    r
}

/// Só confere (para os testes e o diagnóstico): quantas senhas e cookies dá para
/// decifrar, sem guardar nada.
#[cfg(test)]
pub fn conferir(f: &Fonte) -> (usize, usize, usize, usize, bool) {
    let k = chaves(f);
    let (s, fs) = senhas_de(f, &k);
    let (c, fc) = cookies_de(f, &k);
    (s.len(), fs, c.len(), fc, k.v11.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decifra_como_o_chromium() {
        use aes::cipher::BlockEncryptMut;
        let k = Chaves { v11: Some(derivar(b"segredo")), v10: derivar(b"peanuts") };
        for (marca, chave) in [(&b"v10"[..], k.v10), (&b"v11"[..], k.v11.unwrap())] {
            let iv = [b' '; 16];
            let mut buf = b"minha senha".to_vec();
            let n = buf.len();
            buf.resize(n + 16, 0);
            let c = cbc::Encryptor::<aes::Aes128>::new(&chave.into(), &iv.into()).encrypt_padded_mut::<Pkcs7>(&mut buf, n).unwrap().to_vec();
            let dados = [marca, &c[..]].concat();
            assert_eq!(decifrar(&dados, &k).unwrap(), b"minha senha");
        }
        assert_eq!(origem("https://contas.exemplo.com/entrar?x=1"), "https://contas.exemplo.com");
    }

    /// No computador de quem desenvolve: confere, sem guardar nada, que os dados do
    /// Chrome dali se decifram (`cargo test -- --ignored conferir_chrome_local`).
    #[test]
    #[ignore]
    fn conferir_chrome_local() {
        for f in fontes() {
            let (s, fs, c, fc, chave) = conferir(&f);
            eprintln!("{}: chave no chaveiro={chave} senhas={s} (falhas {fs}) cookies={c} (falhas {fc})", f.nome);
        }
    }
}
