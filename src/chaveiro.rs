//! O chaveiro do sistema (Secret Service: gnome-keyring, KWallet, KeePassXC…), pelo
//! D-Bus: as senhas que a Jangada guarda (aparecem também no Acesso às Chaves do Glove)
//! e a chave com que o Chrome e os parecidos cifram as senhas e os cookies deles.

use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const ITEM: &str = "org.freedesktop.Secret.Item";
const COLLECTION: &str = "org.freedesktop.Secret.Collection";
const PROPS: &str = "org.freedesktop.DBus.Properties";

/// O esquema das senhas da Jangada.
pub const ESQUEMA: &str = "org.glove.Jangada.Senha";

/// O segredo como o Secret Service manda: (sessão, parâmetros, valor, tipo).
type Segredo = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

pub struct Chaveiro {
    conn: Connection,
    sessao: OwnedObjectPath,
}

/// Uma senha guardada (sem o segredo).
#[derive(Clone, Debug, PartialEq)]
pub struct Login {
    pub caminho: String,
    pub origem: String,
    pub usuario: String,
}

impl Chaveiro {
    /// Abre a conversa com o chaveiro (sessão "plain": o segredo não sai do computador).
    pub fn abrir() -> Result<Self, String> {
        let conn = Connection::session().map_err(|e| e.to_string())?;
        let r = conn.call_method(Some(DEST), ROOT, Some(SERVICE), "OpenSession", &("plain", Value::from(""))).map_err(|e| e.to_string())?;
        let (_, sessao): (OwnedValue, OwnedObjectPath) = r.body().deserialize().map_err(|e| e.to_string())?;
        Ok(Self { conn, sessao })
    }

    /// Os itens com estes atributos (destrancando o chaveiro, se preciso).
    fn buscar(&self, attrs: &HashMap<&str, &str>) -> Result<Vec<OwnedObjectPath>, String> {
        let r = self.conn.call_method(Some(DEST), ROOT, Some(SERVICE), "SearchItems", &(attrs,)).map_err(|e| e.to_string())?;
        let (mut abertos, trancados): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = r.body().deserialize().map_err(|e| e.to_string())?;
        if !trancados.is_empty() {
            // O chaveiro pede a senha dele (a janela do próprio chaveiro).
            if let Ok(r) = self.conn.call_method(Some(DEST), ROOT, Some(SERVICE), "Unlock", &(trancados.clone(),)) {
                if let Ok((feitos, prompt)) = r.body().deserialize::<(Vec<OwnedObjectPath>, OwnedObjectPath)>() {
                    if prompt.as_str() != "/" {
                        let _ = self.conn.call_method(Some(DEST), prompt.as_str(), Some("org.freedesktop.Secret.Prompt"), "Prompt", &("",));
                        // A resposta do pedido chega depois: tenta de novo em seguida.
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        let r = self.conn.call_method(Some(DEST), ROOT, Some(SERVICE), "SearchItems", &(attrs,)).map_err(|e| e.to_string())?;
                        let (a, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = r.body().deserialize().map_err(|e| e.to_string())?;
                        return Ok(a);
                    }
                    abertos.extend(feitos);
                }
            }
        }
        Ok(abertos)
    }

    fn atributos(&self, item: &str) -> HashMap<String, String> {
        self.conn
            .call_method(Some(DEST), item, Some(PROPS), "Get", &(ITEM, "Attributes"))
            .ok()
            .and_then(|r| r.body().deserialize::<OwnedValue>().ok())
            .and_then(|v| HashMap::<String, String>::try_from(v).ok())
            .unwrap_or_default()
    }

    /// O segredo de um item.
    pub fn segredo(&self, item: &str) -> Result<Vec<u8>, String> {
        let r = self.conn.call_method(Some(DEST), item, Some(ITEM), "GetSecret", &(self.sessao.clone(),)).map_err(|e| e.to_string())?;
        let s: Segredo = r.body().deserialize().map_err(|e| e.to_string())?;
        Ok(s.2)
    }

    /// As senhas da Jangada (sem os segredos), ordenadas pelo site.
    pub fn logins(&self) -> Result<Vec<Login>, String> {
        let mut a = HashMap::new();
        a.insert("xdg:schema", ESQUEMA);
        let mut v: Vec<Login> = self
            .buscar(&a)?
            .into_iter()
            .map(|p| {
                let at = self.atributos(p.as_str());
                Login { caminho: p.to_string(), origem: at.get("origem").cloned().unwrap_or_default(), usuario: at.get("usuario").cloned().unwrap_or_default() }
            })
            .filter(|l| !l.origem.is_empty())
            .collect();
        v.sort_by(|a, b| (&a.origem, &a.usuario).cmp(&(&b.origem, &b.usuario)));
        Ok(v)
    }

    /// Guarda (ou troca) a senha de `usuario` em `origem`.
    pub fn guardar(&self, origem: &str, usuario: &str, senha: &str) -> Result<(), String> {
        let mut attrs: HashMap<&str, &str> = HashMap::new();
        attrs.insert("xdg:schema", ESQUEMA);
        attrs.insert("origem", origem);
        attrs.insert("usuario", usuario);
        let rotulo = format!("Jangada: {} ({})", origem.split("://").nth(1).unwrap_or(origem), usuario);
        let mut props: HashMap<&str, Value> = HashMap::new();
        props.insert("org.freedesktop.Secret.Item.Label", Value::from(rotulo));
        props.insert("org.freedesktop.Secret.Item.Attributes", Value::from(attrs));
        let segredo: Segredo = (self.sessao.clone(), Vec::new(), senha.as_bytes().to_vec(), "text/plain".to_string());
        let colecao = "/org/freedesktop/secrets/aliases/default";
        let r = self.conn.call_method(Some(DEST), colecao, Some(COLLECTION), "CreateItem", &(props, segredo, true)).map_err(|e| e.to_string())?;
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = r.body().deserialize().map_err(|e| e.to_string())?;
        if prompt.as_str() != "/" {
            let _ = self.conn.call_method(Some(DEST), prompt.as_str(), Some("org.freedesktop.Secret.Prompt"), "Prompt", &("",));
        }
        Ok(())
    }

    /// Apaga um item.
    pub fn apagar(&self, item: &str) -> Result<(), String> {
        self.conn.call_method(Some(DEST), item, Some(ITEM), "Delete", &()).map(|_| ()).map_err(|e| e.to_string())
    }

    /// A chave com que um navegador da família do Chrome cifra os dados dele ("Chrome
    /// Safe Storage" e afins): o segredo do item com `application` = `app`.
    pub fn chave_de(&self, app: &str) -> Option<Vec<u8>> {
        let mut a = HashMap::new();
        a.insert("application", app);
        let itens = self.buscar(&a).ok()?;
        itens.iter().find_map(|p| self.segredo(p.as_str()).ok()).filter(|s| !s.is_empty())
    }
}
