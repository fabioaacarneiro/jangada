# Extensões da Jangada

Uma extensão da Jangada é uma pasta com um `manifest.json` e arquivos **JavaScript** e CSS que
entram nas páginas que você escolher. Serve para ajustar sites: esconder um pedaço, mudar um
estilo, acrescentar um botão ou um aviso, lembrar de alguma coisa entre uma visita e outra.

É um formato **próprio da Jangada**, pequeno de propósito.

> **Não são as extensões do Chrome.** As da loja do Chrome (e as do Firefox e do Safari) não
> instalam nem funcionam aqui. O `manifest.json` usa os mesmos nomes de campo do Chrome, para quem
> já os conhece, e a semelhança acaba aí: não há script de fundo, botão na barra, página de
> opções, nem `chrome.tabs`, `chrome.webRequest` e as outras APIs.

## A primeira extensão

Vamos pôr um aviso no canto das páginas de `example.com`. São dois arquivos numa pasta:

```
ola/
├── manifest.json
└── ola.js
```

**`manifest.json`** diz o nome da extensão e em que páginas o script entra:

```json
{
  "manifest_version": 3,
  "name": "Olá, Jangada",
  "version": "1.0",
  "description": "O menor exemplo: um aviso no canto das páginas de example.com.",
  "content_scripts": [
    {
      "matches": ["https://example.com/*"],
      "js": ["ola.js"]
    }
  ]
}
```

**`ola.js`** é JavaScript comum, com acesso à página (`document`):

```js
// Roda em cada página que bate com "matches", quando ela termina de carregar.
var aviso = document.createElement("div");
aviso.textContent = "Olá da Jangada! Clique para fechar.";
aviso.style.cssText =
  "position:fixed; right:12px; bottom:12px; z-index:2147483647;" +
  "padding:8px 12px; border-radius:8px; cursor:pointer;" +
  "background:#1d6fe0; color:#fff; font:14px sans-serif;";
aviso.addEventListener("click", function () {
  aviso.remove();
});
document.body.append(aviso);
```

Instale e abra o site:

```sh
jangada --instalar ./ola      # mostra o que a extensão pede e pergunta antes de instalar
jangada https://example.com
```

O aviso aparece no canto de baixo. Mudou o script? Instale de novo (`jangada --instalar ./ola`) e
recarregue a página: a versão nova entra no lugar da anterior.

Esses dois arquivos estão em [`exemplos/ola/`](../exemplos/ola/). Um exemplo com CSS e com dados
guardados está em [`exemplos/destaque/`](../exemplos/destaque/).

## O `manifest.json`

| Campo | O que é |
|---|---|
| `manifest_version` | Sempre `3`. Obrigatório. |
| `name` | O nome da extensão. Obrigatório. É ele que identifica a extensão: instalar outra com o mesmo nome troca a anterior. |
| `version` | A versão, como texto (`"1.0"`). Aparece na lista das instaladas. |
| `description` | Uma linha sobre o que ela faz. Aparece antes de instalar e na lista. |
| `content_scripts` | A lista das regras: em que páginas, quais arquivos. Veja abaixo. |
| `permissions`, `host_permissions` | Aparecem na pergunta antes de instalar. São só um aviso para quem instala: a Jangada não libera nem bloqueia nada por eles. O que decide onde o script entra é o `matches`. |

Cada regra de `content_scripts` tem:

| Campo | O que é |
|---|---|
| `matches` | Os endereços em que a regra vale (veja a seção seguinte). |
| `js` | Os arquivos JavaScript, na ordem em que rodam. |
| `css` | Os arquivos CSS, que entram na página antes dos scripts da regra. |

Os arquivos citados precisam existir na pasta, senão a extensão não instala. Os outros campos do
manifesto do Chrome (`background`, `action`, `options_page`, `icons`, `run_at`, `all_frames`,
`web_accessible_resources`…) são ignorados.

## Em que páginas: o `matches`

Cada padrão tem a forma `esquema://site/caminho`, e o `*` vale por "qualquer coisa":

| Padrão | Onde vale |
|---|---|
| `https://example.com/*` | Todas as páginas de `example.com` em https. |
| `https://*.example.com/*` | `example.com` e os subdomínios dele (`www.example.com`, `loja.example.com`). |
| `*://example.com/*` | `example.com` em http e em https. |
| `https://example.com/docs/*` | Só o que está em `/docs/`. |
| `*://*/*` ou `<all_urls>` | Todos os sites (http e https). |

A porta não entra no padrão: `http://127.0.0.1/*` vale para `http://127.0.0.1:8080/`. O `*` no
lugar do esquema e o `<all_urls>` valem só para http e https.

## Quando e onde o script roda

- **Quando:** depois que a página termina de carregar, uma vez por carregamento. Num site que
  troca de tela sem recarregar (YouTube, redes sociais), o script não roda de novo sozinho: se
  precisar acompanhar, observe a página (`MutationObserver`) ou o endereço (`location.href`).
- **Onde:** na página da aba (não nos quadros internos, os `<iframe>`).
- **Mundo isolado:** o script vê e muda o documento (`document`, os elementos, os estilos), mas
  não vê as variáveis e funções do JavaScript da página, e a página não vê as do script. O CSS da
  extensão vale para a página inteira.
- **Cada arquivo é uma função à parte:** uma variável declarada num arquivo não aparece no outro.
  Para dividir o código entre arquivos, pendure o que for comum num objeto de `window` com um
  nome seu (por exemplo `window.minhaExtensao = { ... }`): esse `window` é o do mundo isolado, a
  página não o enxerga. Ele é o mesmo para todas as extensões que entram na página, por isso o
  nome próprio.
- JavaScript comum do navegador: sem `import` e sem `await` solto no topo do arquivo (use
  `.then(...)` ou uma função `async`).

## Guardar dados: `jangada.storage.local`

Todo script recebe o objeto `jangada`, a API da extensão. O mesmo objeto atende pelo nome
`chrome`, para quem traz um script escrito para o Chrome.

```js
// Lê (com um valor padrão para o que ainda não foi guardado) e grava.
jangada.storage.local.get({ visitas: 0 }).then(function (dados) {
  jangada.storage.local.set({ visitas: dados.visitas + 1 });
});
```

| Chamada | O que faz |
|---|---|
| `jangada.storage.local.get()` | Devolve tudo o que a extensão guardou. |
| `jangada.storage.local.get("chave")` | Só essa chave. |
| `jangada.storage.local.get(["a", "b"])` | Essas chaves. |
| `jangada.storage.local.get({ a: 1 })` | Essas chaves, com o valor dado para as que não existem. |
| `jangada.storage.local.set({ a: 1 })` | Grava (as outras chaves ficam como estavam). |
| `jangada.storage.local.remove("a")` | Apaga uma chave (ou uma lista delas). |
| `jangada.storage.local.clear()` | Apaga tudo. |
| `jangada.runtime.id` | O identificador da extensão instalada. |

Todas devolvem uma promessa (e aceitam uma função de retorno como último argumento, como no
Chrome). Os valores são os que cabem em JSON: texto, número, verdadeiro/falso, listas e objetos.
Os dados são de cada extensão, valem em todos os sites em que ela entra e ficam em
`~/.local/share/jangada/extensoes/IDENTIFICADOR/storage.json`. Instalar a extensão de novo os
mantém; remover a extensão os apaga.

Não há mais nada na API: nada de `jangada.tabs`, mensagens entre scripts, pedidos de rede com
permissão especial ou avisos de mudança (`storage.onChanged` existe para o script não quebrar, e
nunca avisa). Para pedir dados a um servidor, o script usa `fetch`, com as mesmas regras da
página em que está.

## Instalar, atualizar, desligar e remover

```sh
jangada --instalar ./minha-extensao      # uma pasta
jangada --instalar ./minha-extensao.zip  # um .zip dela
jangada --instalar https://github.com/usuario/repositorio
jangada --instalar https://github.com/usuario/repositorio/tree/RAMO/PASTA
jangada --instalar ./minha-extensao --sim   # sem perguntar (para scripts)
jangada --extensoes                      # lista: identificador, nome, versão, ativa ou desligada
jangada --remover IDENTIFICADOR
```

Pela janela: **Preferências ▸ Instaladas**. Digite a pasta, o `.zip` ou o link, clique em
Verificar, confira o que a extensão pede e confirme. A mesma tela liga, desliga e remove.

- **Atualizar** é instalar de novo: a extensão de mesmo nome é trocada e os dados dela ficam.
- A mudança vale nas páginas abertas **depois**: recarregue a aba para ver.

## Achar erros

- **Ctrl+Shift+I** abre as Ferramentas do Desenvolvedor na aba; no Console aparecem os
  `console.log` do script e os erros dele, com o nome da extensão e o do arquivo na frente.
- Um erro de escrita num arquivo não impede os outros arquivos (nem as outras extensões) de rodar.
- Para ver o que está guardado, abra o `storage.json` da extensão.
- Se nada acontece, confira o `matches` (o esquema `http`/`https`, o `/*` no fim) e se a extensão
  está ligada em Preferências ▸ Instaladas.

## Segurança

Uma extensão lê e muda as páginas em que entra, inclusive o que você digita nelas. Instale só as
de quem você confia e olhe o `matches` antes: `<all_urls>` é a extensão em todos os sites. A
Jangada mostra o que a extensão pede e não instala nada sem a sua confirmação.
