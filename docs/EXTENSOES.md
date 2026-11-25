# Extensões da Jangada

Uma extensão da Jangada é uma pasta com um `manifest.json` e arquivos **JavaScript**, CSS e HTML.
Ela pode:

- pôr scripts e estilos nas páginas que você escolher (esconder um pedaço, mudar um estilo,
  acrescentar um botão ou um aviso);
- ter um **botão na barra**, com ícone e selo, que abre a **janelinha** da extensão: uma página
  HTML dela, onde cabem botões, menus, formulários ou um aplicativo inteiro;
- ter uma **página de opções**, aberta numa aba;
- guardar dados e ser avisada quando eles mudam, em todas as abas;
- trocar mensagens entre a janelinha, as opções e os scripts das páginas;
- falar a língua de quem usa (`_locales`).

É um formato **próprio da Jangada**, com os nomes de campo e de função do Chrome (Manifest V3):
quem já escreveu uma extensão para o Chrome se acha aqui, e muitas janelinhas e páginas de opções
funcionam sem mudar nada.

> **Não são as extensões do Chrome.** As da loja do Chrome não instalam aqui por inteiro: não há
> script de fundo (o `background` / service worker), nem `chrome.webRequest`, `scripting`,
> `contextMenus`, `downloads`, `cookies`, `history` e as outras APIs. Veja
> [o que a Jangada tem](#a-api-o-objeto-jangada) e [o que não tem](#o-que-não-tem).

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
// Roda em cada página que bate com "matches", assim que o documento dela fica pronto.
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
jangada --install ./ola       # mostra o que a extensão pede e pergunta antes de instalar
jangada https://example.com
```

O aviso aparece no canto de baixo. Mudou o script? Instale de novo (`jangada --install ./ola`) e
recarregue a página: a versão nova entra no lugar da anterior.

Esses dois arquivos estão em [`exemplos/ola/`](../exemplos/ola/). Um exemplo com CSS e com dados
guardados está em [`exemplos/destaque/`](../exemplos/destaque/), e um com botão, janelinha,
opções e mensagens em [`exemplos/links/`](../exemplos/links/).

## O botão e a janelinha

Com `action` no manifesto, a extensão ganha um botão na barra da Jangada, à esquerda dos escudos:

```json
"action": {
  "default_title": "Links da página",
  "default_popup": "janelinha.html",
  "default_icon": { "16": "icone16.png", "32": "icone32.png" }
}
```

- **`default_popup`**: a página que abre na janelinha, embaixo do botão. É HTML comum, com
  `<script src="...">`, CSS, imagens e `fetch` para os arquivos da extensão. A janelinha toma o
  tamanho do conteúdo (a largura que a página pede, até 800 × 600; dê uma largura ao `body`
  para controlar). Clicar fora, apertar Esc, trocar de aba ou chamar `window.close()` a fecha; ela
  é aberta de novo (do zero) a cada clique, como no Chrome. Guarde o que precisa durar com
  `storage`.
- **Sem `default_popup`**, o clique chega aos scripts da extensão na aba ativa, por
  `jangada.action.onClicked`.
- **`default_icon`**: PNG, JPEG ou SVG. A Jangada escolhe o tamanho que fica nítido na tela.
  Sem ícone, o botão mostra a inicial do nome. Os `icons` do manifesto valem se o `action` não
  tiver ícone.
- **`default_title`**: a dica ao parar o mouse no botão (sem ela, o nome da extensão).

O botão da **peça de quebra-cabeça**, ao lado, lista todas as extensões ligadas (inclusive as
que não couberam na barra) e as opções de cada uma. O menu **Extensões** da barra de menus
também.

Os scripts mudam o botão em tempo real:

```js
jangada.action.setBadgeText({ text: "12" });          // o selo ("" tira)
jangada.action.setBadgeBackgroundColor({ color: "#1e8e3e" });
jangada.action.setTitle({ title: "12 links" });
jangada.action.setPopup({ popup: "outra.html" });     // "" tira a janelinha
```

O estado do botão vale para todas as abas e volta ao do manifesto quando a Jangada reabre.

**Para achar erros na janelinha:** com o teclado nela, **Ctrl+Shift+I** abre as Ferramentas do
Desenvolvedor da própria janelinha.

## A página de opções

```json
"options_page": "opcoes.html"
```

(ou `"options_ui": { "page": "opcoes.html" }`). Ela abre numa aba: pelo menu da peça, por
Preferências ▸ Instaladas (o botão **Opções…**) ou por `jangada.runtime.openOptionsPage()`. Se já
estiver aberta, a Jangada vai para a aba dela.

## Os endereços da extensão

Os arquivos da extensão têm endereço próprio: `jangada-extension://IDENTIFICADOR/caminho`. A
janelinha e as opções vêm daí; `jangada.runtime.getURL("img/a.png")` devolve o endereço de um
arquivo. Uma página da extensão lê o que quiser da pasta dela.

Uma **página da web** não lê os arquivos da extensão, a não ser os que ela declarar acessíveis
(por exemplo, uma imagem que o script de conteúdo põe na página):

```json
"web_accessible_resources": [{ "resources": ["img/*"], "matches": ["<all_urls>"] }]
```

Um site também não consegue abrir uma página da extensão (nem numa aba, nem num quadro).

## O `manifest.json`

| Campo | O que é |
|---|---|
| `manifest_version` | Sempre `3`. Obrigatório. |
| `name` | O nome da extensão. Obrigatório. É ele que identifica a extensão: instalar outra com o mesmo nome troca a anterior. |
| `version` | A versão, como texto (`"1.0"`). Aparece na lista das instaladas. |
| `description` | Uma linha sobre o que ela faz. Aparece antes de instalar e na lista. |
| `content_scripts` | As regras dos scripts de página: em que páginas, quais arquivos. Veja abaixo. |
| `action` | O botão na barra e a janelinha. Veja [O botão e a janelinha](#o-botão-e-a-janelinha). |
| `icons` | Os ícones da extensão (`{"16": "a.png", "128": "b.png"}`). |
| `options_page`, `options_ui.page` | A página de opções. |
| `web_accessible_resources` | Os arquivos que as páginas da web podem carregar. |
| `host_permissions` | Os sites que as páginas da extensão (janelinha, opções) podem pedir com `fetch` sem a trava de CORS, como no Chrome. Também deixam a extensão ver o endereço das abas desses sites. |
| `permissions` | Aparecem na pergunta antes de instalar. `"tabs"` deixa a extensão ver o endereço e o título de todas as abas; `"activeTab"`, os da aba em que você clicou no botão dela. As outras são só aviso. |
| `default_locale` | A língua padrão dos textos traduzidos (obrigatória com `_locales`). |

Cada regra de `content_scripts` tem:

| Campo | O que é |
|---|---|
| `matches` | Os endereços em que a regra vale (veja a seção seguinte). |
| `js` | Os arquivos JavaScript, na ordem em que rodam. |
| `css` | Os arquivos CSS, que entram na página antes dos scripts da regra. |

Os arquivos citados (scripts, CSS, janelinha, opções) precisam existir na pasta, senão a extensão
não instala. Os outros campos do manifesto do Chrome (`background`, `commands`, `run_at`,
`all_frames`…) são ignorados.

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

## Quando e onde o script de página roda

- **Quando:** assim que o documento da página fica pronto (o HTML já lido; as imagens podem
  ainda estar chegando), uma vez por carregamento. Num site que
  troca de tela sem recarregar (YouTube, redes sociais), o script não roda de novo sozinho: se
  precisar acompanhar, observe a página (`MutationObserver`) ou o endereço (`location.href`).
- **Onde:** na página da aba (não nos quadros internos, os `<iframe>`).
- **Mundo isolado, só da extensão:** o script vê e muda o documento (`document`, os elementos,
  os estilos), mas não vê as variáveis e funções do JavaScript da página, e a página não vê as do
  script. Cada extensão tem o mundo dela: uma não enxerga as variáveis nem fala pelo canal da
  outra. O CSS da extensão vale para a página inteira.
- **Cada arquivo é uma função à parte:** uma variável declarada num arquivo não aparece no outro.
  Para dividir o código entre arquivos, pendure o que for comum num objeto de `window` (por
  exemplo `window.minhaExtensao = { ... }`): esse `window` é o do mundo da extensão, a página não o
  enxerga.
- JavaScript comum do navegador: sem `import` e sem `await` solto no topo do arquivo (use
  `.then(...)` ou uma função `async`).

As páginas da extensão (janelinha, opções) são páginas comuns: `<script src>` de arquivos da
extensão, módulos (`type="module"`), `await` dentro de funções `async`.

## A API: o objeto `jangada`

Os scripts de página recebem o objeto `jangada`, que também atende por `chrome` e por `browser`.
Nas páginas da extensão ele está em `window.jangada` (e em `window.chrome` e `window.browser`).
Toda função que pede algo à Jangada devolve uma **promessa** e também aceita uma **função de
retorno** como último argumento, como no Chrome (com a função, um erro aparece em
`jangada.runtime.lastError` durante a chamada dela).

### Dados: `storage`

```js
// Lê (com um valor padrão para o que ainda não foi guardado) e grava.
jangada.storage.local.get({ visitas: 0 }).then(function (dados) {
  jangada.storage.local.set({ visitas: dados.visitas + 1 });
});

// Avisa quando algo muda, venha de onde vier (a janelinha, as opções, outra aba).
jangada.storage.onChanged.addListener(function (mudancas, area) {
  if (mudancas.cor) console.log("de", mudancas.cor.oldValue, "para", mudancas.cor.newValue);
});
```

| Chamada | O que faz |
|---|---|
| `storage.local.get()` | Devolve tudo o que a extensão guardou. |
| `storage.local.get("chave")` / `get(["a", "b"])` | Só essas chaves. |
| `storage.local.get({ a: 1 })` | Essas chaves, com o valor dado para as que não existem. |
| `storage.local.set({ a: 1 })` | Grava (as outras chaves ficam como estavam). |
| `storage.local.remove("a")` | Apaga uma chave (ou uma lista delas). |
| `storage.local.clear()` | Apaga tudo. |
| `storage.local.getBytesInUse()` | Quanto a extensão guardou, em bytes. |
| `storage.onChanged`, `storage.local.onChanged` | O aviso de mudança, em todos os scripts e páginas da extensão. |
| `storage.sync` | O mesmo lugar do `local` (a Jangada não sincroniza contas). |

Os valores são os que cabem em JSON: texto, número, verdadeiro/falso, listas e objetos. Os dados
são de cada extensão, valem em todos os sites e páginas dela e ficam em
`~/.local/share/jangada/extensions/IDENTIFICADOR/storage.json`. Instalar a extensão de novo os
mantém; remover a extensão os apaga.

### Mensagens: `runtime.sendMessage` e `tabs.sendMessage`

A janelinha (ou as opções) fala com o script de uma aba:

```js
// Na janelinha
var [aba] = await jangada.tabs.query({ active: true, currentWindow: true });
var resposta = await jangada.tabs.sendMessage(aba.id, { tipo: "contar" });

// No script da página
jangada.runtime.onMessage.addListener(function (msg, remetente, responder) {
  if (msg.tipo === "contar") responder({ links: document.links.length });
  // Para responder depois (depois de uma promessa), devolva true:
  // return true;
});
```

O script da página fala com as páginas abertas da extensão por `jangada.runtime.sendMessage`
(quem recebe é a janelinha ou as opções, se estiverem abertas). Sem ninguém para receber, a
promessa falha com *"Could not establish connection. Receiving end does not exist."*, como no
Chrome. `remetente` traz o `id` da extensão, o `url` de quem mandou e, se veio de uma aba, a `tab`.

### Abas: `tabs` (só nas páginas da extensão)

| Chamada | O que faz |
|---|---|
| `tabs.query({ active, url, title, status, index })` | As abas que batem com o pedido. `currentWindow` e `lastFocusedWindow` valem sempre (uma janela). |
| `tabs.get(id)`, `tabs.getCurrent()` | Uma aba; a aba da própria página (nas opções). |
| `tabs.create({ url, active })` | Abre uma aba (um caminho como `"ajuda.html"` é da extensão). |
| `tabs.update(id, { url, active })` | Muda o endereço de uma aba, ou a traz para a frente. |
| `tabs.reload(id)`, `tabs.remove(id)` | Recarrega; fecha. |
| `tabs.sendMessage(id, msg)` | Uma mensagem para os scripts da extensão nessa aba. |

O endereço (`url`) e o título (`title`) de uma aba só aparecem para a extensão que pode vê-los:
com a permissão `"tabs"`, com o site em `host_permissions` ou `matches`, com `"activeTab"` na aba
em que você clicou no botão dela, ou se a página for da própria extensão.

### O resto

| Chamada | O que faz |
|---|---|
| `runtime.id` | O identificador da extensão instalada. |
| `runtime.getURL(caminho)` | O endereço de um arquivo da extensão. |
| `runtime.getManifest()` | O manifesto (com os textos já traduzidos). |
| `runtime.openOptionsPage()` | Abre (ou mostra) a página de opções. |
| `action.*` | O botão: `setBadgeText`, `setBadgeBackgroundColor`, `setBadgeTextColor`, `setTitle`, `setPopup`, os `get…` de cada um, `openPopup`, `enable`, `disable` e o aviso `onClicked`. |
| `i18n.getMessage(nome, [substituições])` | Um texto traduzido (veja abaixo). |
| `i18n.getUILanguage()` | A língua de quem usa (`"pt-BR"`). |

### Textos traduzidos: `_locales`

```
minha-extensao/
├── manifest.json            "default_locale": "en", "name": "__MSG_nome__"
└── _locales/
    ├── en/messages.json     { "nome": { "message": "Link counter" } }
    └── pt_BR/messages.json  { "nome": { "message": "Contador de links" } }
```

A Jangada usa a língua de quem usa (`pt_BR`, depois `pt`), e o que faltar vem da
`default_locale`. Valem os `__MSG_nome__` no manifesto, `jangada.i18n.getMessage("nome")` nos
scripts e os marcadores (`$1`, `"placeholders"`) do formato do Chrome.

## O que não tem

- **Script de fundo** (`background`, service worker): nada roda quando nenhuma página da
  extensão está aberta. O que o Chrome faria no fundo, aqui vai no script de página (que roda em
  cada aba) ou na janelinha.
- `chrome.webRequest`, `declarativeNetRequest`, `scripting`, `contextMenus`, `downloads`,
  `cookies`, `history`, `bookmarks`, `notifications`, `alarms`, `commands`, `runtime.connect` e as
  outras APIs que não estão nas tabelas acima. Chamar uma delas dá `undefined` (confira antes:
  `if (jangada.contextMenus) ...`).
- `action.setIcon` não troca o ícone (não dá erro).
- Os eventos de abas (`tabs.onUpdated` etc.) existem para o script não quebrar, e não avisam.

## Instalar, atualizar, desligar e remover

```sh
jangada --install ./minha-extensao        # uma pasta
jangada --install ./minha-extensao.zip    # um .zip dela
jangada --install https://github.com/usuario/repositorio
jangada --install https://github.com/usuario/repositorio/tree/RAMO/PASTA
jangada --install ./minha-extensao --yes  # sem perguntar (para scripts)
jangada --extensions                      # lista: identificador, nome, versão, ativa ou desligada
jangada --remove IDENTIFICADOR
```

Pela janela: **Preferências ▸ Instaladas**. Digite a pasta, o `.zip` ou o link, clique em
Verificar, confira o que a extensão pede e confirme. A mesma tela liga, desliga, abre as opções e
remove.

- Do GitHub, sem ramo no link, vale o ramo padrão do repositório (`main`, `master` ou outro).
- O que começa com ponto na pasta (`.git`, `.github`) não é copiado.
- **Atualizar** é instalar de novo: a extensão de mesmo nome é trocada e os dados dela ficam.
- A mudança vale nas páginas abertas **depois**: recarregue a aba para ver.

## Achar erros

- **Ctrl+Shift+I** abre as Ferramentas do Desenvolvedor na aba; no Console aparecem os
  `console.log` dos scripts de página e os erros deles, com o nome da extensão e o do arquivo na
  frente. Com o teclado na janelinha, o mesmo atalho abre as dela.
- Um erro de escrita num arquivo não impede os outros arquivos (nem as outras extensões) de rodar.
- Para ver o que está guardado, abra o `storage.json` da extensão.
- Se nada acontece, confira o `matches` (o esquema `http`/`https`, o `/*` no fim) e se a extensão
  está ligada em Preferências ▸ Instaladas.

## Segurança

Uma extensão lê e muda as páginas em que entra, inclusive o que você digita nelas. Instale só as
de quem você confia e olhe o `matches` antes: `<all_urls>` é a extensão em todos os sites. A
Jangada mostra o que a extensão pede e não instala nada sem a sua confirmação.

O que a Jangada garante:

- os scripts de cada extensão rodam num mundo só dela: a página não os vê, e uma extensão não
  fala com a Jangada pelo canal de outra (nem lê os dados dela);
- quem pede algo à Jangada é sempre conhecido por ela (pelo canal e pelo endereço da página), não
  pelo que vem escrito no pedido;
- os sites não abrem páginas de extensão e só carregam os arquivos declarados em
  `web_accessible_resources`;
- a trava de CORS só cai nas páginas da própria extensão, só para os sites de
  `host_permissions`, e volta assim que a aba sai da página da extensão;
- os arquivos servidos ficam dentro da pasta da extensão (nada de `..` ou atalhos para fora).
