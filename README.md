# Jangada

O navegador do **Glove OS**. O motor é o [WebKitGTK](https://webkitgtk.org/), o do sistema: as
correções de segurança dele chegam pelas atualizações da distribuição, como as de qualquer outro
programa. A janela, as abas, a barra de endereço e as páginas da própria Jangada (início e erro) são
do Glove, desenhadas pelo [glove-ui](https://github.com/fabioaacarneiro/glove-ui), no visual
clássico ou no Moderno (claro e escuro).

*English: Jangada is the Glove OS web browser: the system WebKitGTK engine in a Glove window.*

A versão 1.x usava o motor do Chromium (CEF) e continua na branch `cef` (é a do Slackware 15.0).

## Leve

Feita para computadores modestos:

- a página é um widget do sistema dentro da janela do Glove: a placa de vídeo a desenha direto na
  tela, sem cópia nenhuma pelo processador (rolar feeds longos, vídeos e mapas fica leve);
- as barras da Jangada são desenhadas pelo glove-ui por cima, só quando algo muda: parada, a
  Jangada não gasta processador nenhum (nada de relógio rodando de fundo);
- os escudos são filtros nativos do motor (o formato do Safari, compilados uma vez): nenhum script
  dos escudos roda nas páginas;
- o pacote tem uns 4 MB: o motor, os codecs de vídeo (GStreamer) e o resto vêm da distribuição.

Para desenhar só com o processador (placa de vídeo com problema): desmarque *Usar a placa de
vídeo* em Preferências ▸ Avançado, ou abra com `JANGADA_NO_GPU=1 jangada`.

## Sites e vídeos

- Os sites veem a Jangada como o Safari (o motor é o mesmo, o WebKit): mandam a versão completa
  deles, e os que conferem o navegador (WhatsApp Web, bancos) aceitam. O motor ainda corrige, site
  a site, o que algum exigir.
- **Vídeo:** o GStreamer do sistema decodifica H.264, AAC, VP9, AV1 e Opus (no Fedora, o H.264 vem
  do OpenH264; o conjunto completo, do `gstreamer1-plugin-libav` do RPM Fusion). Com os plugins do
  VA-API (`gstreamer1.0-plugins-bad`), a placa de vídeo decodifica.
- **Tela cheia** dos vídeos: as barras da Jangada somem; Esc volta.
- **PDF:** abre na própria aba, no visualizador do motor.

### O que ainda não funciona

- **Chamadas de vídeo e de voz** (Google Meet, Jitsi, as chamadas do WhatsApp Web) precisam do
  WebRTC, e isso depende de como a distribuição compila o WebKitGTK. No Debian 13, e portanto no
  Glove OS, ele vem sem WebRTC: as chamadas não funcionam.
- **Vídeo protegido** (Netflix, Prime Video, o Spotify no navegador): não toca no Debian 13 nem no
  Glove OS, em que o motor vem sem o módulo de proteção (DRM).
- **Telas em 125% ou 150%:** o motor só desenha em escalas inteiras: a página é desenhada em 200% e
  reduzida. Fica um pouco menos nítida que as barras da Jangada (desenhadas na escala exata) e
  gasta mais placa de vídeo do que em 100% ou 200%.
- **Prender o mouse no Glove:** a Jangada permite, mas o ambiente gráfico precisa oferecer o
  travamento do ponteiro, e o Glove ainda não oferece: nele, os jogos que prendem o mouse
  continuam sem prender.

## Desenvolvimento web

- **Ver código-fonte** (Ctrl+U, ou no menu Visualizar): abre o HTML da página numa aba.
- **Ferramentas do desenvolvedor** (Ctrl+Shift+I, ou Visualizar ▸ Ferramentas do Desenvolvedor):
  o inspetor do WebKit (elementos, console, rede, fontes, armazenamento, linha do tempo) abre junto
  da página da aba atual. Não abre porta nenhuma: só a Jangada fala com ele.
  Preferências ▸ Avançado põe também o **Inspecionar Elemento** no menu do botão direito.
- **Depurar de outro navegador:** `jangada --debug [PORTA]` (padrão 9222) abre o inspetor em
  `http://127.0.0.1:PORTA` (só neste computador) para outro navegador. Só vale ao abrir a Jangada:
  se ela já estiver aberta, feche-a antes.

## Permissões dos sites

Quando um site pede a **câmera**, o **microfone**, a **localização** ou as **notificações**, a Jangada
pergunta numa janela própria: *Negar*, *Só desta vez* ou *Permitir*. "Permitir" e "Negar" ficam
guardados para aquele site (Preferências ▸ Privacidade mostra quantos sites têm permissões, e
"Esquecer as Permissões" apaga tudo). "Só desta vez" vale para a página aberta: ao recarregar, a
pergunta volta.

- **Câmera e microfone** vêm pelo GStreamer (com o PipeWire, pelo `gstreamer1.0-pipewire`).
- **Localização:** pelo serviço de localização do sistema (GeoClue), se houver.
- **Notificações** permitidas aparecem no avisador do sistema (o do Glove).
- **Área de transferência:** quando um site quer ler o que você copiou (o botão "colar" dele), a
  pergunta aparece numa faixa no alto da página, com as mesmas três respostas. Ctrl+V nunca
  pergunta.
- **Colar e soltar arquivos:** uma imagem copiada (uma captura de tela, uma figura de outro app)
  ou arquivos copiados no Arquivos chegam à página como arquivos com Ctrl+V; arquivos arrastados
  da mesa ou do Arquivos também. É assim que o Telegram, o WhatsApp e o e-mail recebem anexos.
  (O WebKitGTK sozinho não entrega arquivos à página: quem os lê e entrega é a Jangada.)
- **Prender o mouse** (jogos, visualizadores 3D): a Jangada permite, e o Esc solta (no Glove ainda
  não prende: veja "O que ainda não funciona").
- **Captura da tela:** a Jangada não concede.

## Extensões

A Jangada tem um formato **próprio** de extensões: uma pasta com um `manifest.json` e arquivos
JavaScript, CSS e HTML. Uma extensão põe scripts e estilos nas páginas que você escolher, ganha um
**botão na barra** (com ícone e selo) que abre a **janelinha** dela (uma página HTML da
extensão: botões, menus, um aplicativo inteiro), pode ter uma **página de opções**, guarda dados
(com aviso de mudança em todas as abas), troca mensagens entre a janelinha e as páginas e fala a
língua de quem usa (`_locales`).

Os nomes de campo e de função são os do Chrome (Manifest V3, `chrome.storage`, `chrome.tabs`,
`chrome.runtime`, `chrome.action`, `chrome.i18n`): muitas janelinhas e páginas de opções escritas
para o Chrome funcionam sem mudar nada. **Mas não são as extensões do Chrome:** não há script de
fundo (service worker) nem `chrome.webRequest`, `scripting`, `contextMenus` e afins, então as da
loja do Chrome não funcionam por inteiro.

O menor exemplo são dois arquivos numa pasta. O `manifest.json`:

```json
{
  "manifest_version": 3,
  "name": "Olá, Jangada",
  "version": "1.0",
  "content_scripts": [
    { "matches": ["https://example.com/*"], "js": ["ola.js"] }
  ]
}
```

E o `ola.js`, JavaScript comum com acesso à página:

```js
var aviso = document.createElement("div");
aviso.textContent = "Olá da Jangada!";
aviso.style.cssText =
  "position:fixed; right:12px; bottom:12px; padding:8px 12px; background:#1d6fe0; color:#fff;";
document.body.append(aviso);
```

```sh
jangada --install ./ola                  # pasta ou .zip; mostra o que ela pede e pergunta
jangada --install https://github.com/usuario/repositorio   # ou .../tree/RAMO/PASTA
jangada --extensions                     # lista as instaladas (ativa ou não)
jangada --remove ID                      # tira uma
jangada --help                           # todas as opções
```

Também dá para instalar, ligar, desligar, abrir as opções e remover em Preferências ▸
**Extensões** (a mesma aba tem as que vêm com a Jangada). Nada entra sem a sua confirmação.

- **Na barra:** cada extensão com `action` ganha um botão à esquerda dos escudos; o clique abre a
  janelinha dela embaixo do botão, no tamanho que a página pede (até 800 × 600). Clicar fora, Esc
  ou trocar de aba a fecha. A peça de quebra-cabeça ao lado lista todas e as opções de cada uma.
- **O que há:** scripts e CSS por página (`content_scripts`), a janelinha (`action`), as opções
  (`options_page`), os arquivos da extensão em `jangada-extension://ID/...`,
  `web_accessible_resources`, `host_permissions` (as páginas da extensão pedem a esses sites sem
  a trava de CORS) e a API `jangada` (também `chrome` e `browser`): `storage` (com `onChanged`),
  `runtime` (mensagens, `getURL`, `getManifest`, `openOptionsPage`), `tabs` (`query`,
  `sendMessage`, `create`, `update`, `reload`, `remove`), `action` (selo, título, janelinha) e
  `i18n`.
- **Quando roda:** o script de página entra assim que o documento fica pronto (sem esperar as
  imagens), na página da aba (não nos quadros internos). O `run_at` do manifesto é ignorado.
- **Mundo isolado, um por extensão:** os scripts de cada extensão rodam num mundo só dela: veem a
  página, mas a página não os vê, e uma extensão não fala pela outra nem lê os dados dela. Os
  sites não abrem páginas de extensão e só carregam os arquivos que ela declarou acessíveis.
  Ainda assim, só instale extensões de quem você confia: elas leem as páginas em que entram.
- **O guia de quem escreve uma extensão** (o manifesto campo a campo, a API, como achar erros)
  está em [`docs/EXTENSOES.md`](docs/EXTENSOES.md). Os exemplos: `exemplos/ola/` (o de cima),
  `exemplos/destaque/` (com CSS e contagem de visitas) e `exemplos/links/` (botão, janelinha,
  opções, mensagens e selo).

## Janela privada

**Arquivo ▸ Nova Janela Privada** (Ctrl+Shift+N) abre uma janela que não deixa rastro: o histórico,
os favoritos, as abas da última vez e as senhas não são gravados, e os cookies, o cache e o
armazenamento dos sites ficam só na memória (uma sessão efêmera do motor, sem nada em disco). Não
oferece para guardar senhas nem as preenche.
O que você baixa continua na pasta Downloads (é um arquivo seu, não um rastro). Uma janela privada
é um processo à parte, então ela não se mistura com a normal.

## Escudos, senhas e importar

- **Escudos:** anúncios e rastreadores barrados pelo próprio motor antes de carregar, e os espaços
  de anúncio escondidos na página. As listas são as do Brave e do uBlock (EasyList, EasyPrivacy, a
  portuguesa, a do Peter Lowe e os filtros do uBlock Origin, renovadas toda semana em
  `~/.cache/jangada/shields`); o motor de bloqueio do Brave (o `adblock`) as converte para os
  filtros de conteúdo do WebKit, que ele compila e guarda. O clique no escudo da barra liga ou
  desliga os escudos no site. **Barrar cookies de terceiros** liga também a prevenção de
  rastreamento do motor (a do Safari: os rastreadores entre sites perdem os cookies).
- **Senhas:** ao entrar num site, a Jangada oferece guardar a senha (no chaveiro do sistema: aparece
  no Acesso às Chaves do Glove) e a preenche da próxima vez. Preferências ▸ Senhas lista, mostra,
  copia e apaga.
- **Importar de outro navegador** (Arquivo ▸ Importar de Outro Navegador…): do Google Chrome,
  Chromium, Brave, Microsoft Edge ou Vivaldi deste computador, os favoritos, o histórico, as sessões
  abertas nos sites (os cookies: continua com as contas abertas) e as senhas, decifrados com a chave
  que o próprio navegador guarda no chaveiro. A sincronização com a conta do Google só existe no
  Chrome; o que vem é o que ele guarda no computador.
- **Extensões** (menu Extensões): as que vêm com a Jangada (escudos, cookies de terceiros, modo
  escuro nas páginas) e as instaladas (veja acima). As da loja do Chrome não funcionam por
  inteiro: não há script de fundo nem as APIs de rede, menus e downloads do Chrome.

## Como usar

- **Janela sem barra de título**, como a do Safari: os botões de fechar, minimizar e zoom ficam na
  própria barra da Jangada, ao lado de voltar e avançar; arrastar a barra (ou o espaço livre das
  abas) move a janela, e dois cliques dão o zoom. Sobra mais espaço para a página.
- **Sugestões da barra:** ao digitar, a barra mostra a busca pelo que foi escrito e os favoritos e
  sites do histórico que começam a palavra digitada (depois, os que apenas contêm). As setas
  escolhem uma sugestão e o Enter a abre; o clique também.
- **Barra de endereço** (como a do Safari): fora de foco mostra só o site, com o cadeado; clique ou
  Ctrl+L e digite um endereço ou uma busca (DuckDuckGo). Esc volta. Recarregar e parar ficam dentro
  dela.
- **Abas** como as do Safari: todas da mesma largura, a escolhida em destaque, as outras um tom
  acima da faixa, o ícone do site e o título no meio, o × ao passar o mouse e um + logo depois da
  última. Ctrl+T abre, Ctrl+W fecha, Ctrl+Shift+T reabre a última fechada, Ctrl+Tab passa; links
  que abrem janela nova viram abas, e o botão do meio (ou Ctrl + clique) abre o link numa aba por
  trás. O botão do meio numa aba a fecha. Quando não cabem todas, a faixa rola: uma seta em
  cada ponta, a roda do mouse sobre a faixa, e ela acompanha a aba escolhida.
- **Página inicial** própria, com a busca e os atalhos dos sites; **página de erro** própria. Em
  Preferências ▸ Geral dá para escolher o fundo (padrão, cinza claro, azul claro, grafite ou uma
  imagem do computador), e mostrar ou esconder os atalhos e a caixa de busca.
- **Botão direito:** o menu do Glove, solto, por cima de tudo (abrir o link em nova aba, copiar,
  colar, buscar o texto escolhido). Fora do Glove, um menu do GTK com os mesmos itens.
- **Touchpad:** dois dedos para os lados voltam e avançam.
- **Downloads** direto na pasta Downloads (o que o motor não sabe mostrar, como um `.zip` ou um
  `.deb`, é baixado), com a janela Downloads (Ctrl+J, ou a seta na barra): o andamento, abrir,
  mostrar na pasta e cancelar; **avisos das páginas** (alert, confirm e prompt) como alertas do
  Glove, em janela própria; a mãozinha nos links e a barra de texto nos campos.
- **Favoritos:** a estrela no campo de endereço (ou Ctrl+D) guarda a página; a barra de favoritos
  (Ctrl+Shift+B) e o menu Favoritos levam a elas, e a página inicial mostra os favoritos.
- **Histórico:** os últimos sites no menu Histórico e todos na janela Histórico (Ctrl+H), com
  busca; Reabrir a Última Aba Fechada e **Reabrir as Abas da Última Sessão** (as abas que estavam
  abertas da última vez que a Jangada fechou; cada uma carrega quando é escolhida, para não abrir
  vinte páginas de uma vez); Limpar o Histórico… nas Preferências.
- **Buscar na página** (Ctrl+F), com quantos achou e anterior/próximo; **Imprimir** (Ctrl+P), pela
  folha Imprimir do Glove (impressoras do CUPS, cópias, páginas, pré-visualizar), e **Salvar como
  PDF** (menu Arquivo), na pasta Downloads.
- **Preferências** (Ctrl+,), como as do Safari:
  - *Geral:* página inicial (a da Jangada ou um endereço), reabrir as abas da última vez, a barra de
    favoritos e o tamanho padrão das páginas;
  - *Contas:* **iniciar a sessão no Google** (Gmail, YouTube, Drive e os outros serviços com a sua
    conta; também no menu Jangada), gerenciar a conta e sair. A sessão fica guardada na Jangada;
  - *Busca:* DuckDuckGo, Google, Bing, Ecosia, Brave Search ou Startpage;
  - *Privacidade:* bloquear as janelas que as páginas abrem sozinhas, pedir para não rastrear (Não
    Rastrear e GPC), limpar o histórico, apagar cookies e dados dos sites;
  - *Extensões:* o **bloqueador de anúncios e rastreadores** (vem ligado) e o **modo escuro nas
    páginas**;
  - *Avançado:* o endereço completo na barra, usar a placa de vídeo, o Inspecionar Elemento e a
    pasta Downloads.
- Abrir de novo (`jangada endereço`) abre uma aba na janela que já está aberta.
- No pacote, a Jangada vira o navegador padrão do sistema (`x-www-browser`).

## Instalar

Os pacotes saem na release de cada versão (`github.com/fabioaacarneiro/jangada/releases`):

- **Debian, Ubuntu e Glove OS:** `jangada_VERSÃO_amd64.deb` (`sudo apt install ./jangada_*.deb`:
  o apt traz o WebKitGTK, o GTK 4 e o GStreamer). Vira o navegador padrão do sistema. Pelo
  repositório do Glove OS: `sudo apt install jangada`.
- **Fedora:** `jangada-VERSÃO-1.x86_64.rpm` (`sudo dnf install ./jangada-*.rpm`), ou
  `sudo dnf install jangada` pelo repositório do Glove.
- **Arch:** `jangada-VERSÃO-1-x86_64.pkg.tar.zst` (`sudo pacman -U ./jangada-*.pkg.tar.zst`), ou
  `sudo pacman -S jangada` pelo repositório do Glove.
- **Slackware -current:** `jangada-VERSÃO-x86_64-1.tgz` (`sudo installpkg ./jangada-*.tgz`). O
  Slackware não traz o WebKitGTK: ele vem compilado dentro do pacote (em `/usr/lib/jangada/motor`;
  a Jangada sobe a versão dele a cada correção). O resto (GTK 4, GStreamer, libsoup 3) é do
  Slackware; a descrição do pacote lista.
- **Slackware 15.0:** o GTK 4 e o compilador dele são velhos para o WebKitGTK atual. Nele, a Jangada
  fica na **1.15** (motor do Chromium, branch `cef`), que o instalador do Glove escolhe sozinho.

Nos três últimos o pacote não troca o navegador padrão sozinho: escolha a Jangada nas configurações
do sistema (ou com `xdg-settings set default-web-browser jangada.desktop`).

## Compilar e empacotar

Precisa do Rust 1.89 ou mais novo e do WebKitGTK 6.0 e do GTK 4 para desenvolvimento
(`libwebkitgtk-6.0-dev libgtk-4-dev` no Debian e no Ubuntu, `webkitgtk6.0-devel gtk4-devel` no
Fedora, `webkitgtk-6.0 gtk4` no Arch).

```sh
git clone --recursive https://github.com/fabioaacarneiro/jangada
cd jangada
cargo build --release         # target/release/jangada
./empacotar.sh                # pacotes/jangada_VERSÃO_amd64.deb
./empacotar-rpm.sh            # pacotes/jangada-VERSÃO-1.x86_64.rpm (precisa do rpmbuild)
./empacotar-arch.sh           # pacotes/jangada-VERSÃO-1-x86_64.pkg.tar.zst
./empacotar-slackware.sh      # pacotes/jangada-VERSÃO-x86_64-1.tgz
```
A árvore dos arquivos vem sempre de `preparar.sh`, a mesma para todos os formatos. O `ci/` tem os
scripts que compilam e conferem cada distribuição em contêiner (é o que a CI roda).

A cada tag `v*`, o CI gera os pacotes (.deb, .rpm, Arch e Slackware), confere cada um instalando-o
na própria distribuição (o .deb também no Ubuntu 24.04) e publica tudo na release. Os repositórios
do Glove (apt, dnf e pacman) pegam os pacotes de lá. O do Slackware compila o motor
(`ci/motor-slackware.sh`) e o guarda no cache do CI.

## Licença

GPL-3.0-or-later. O motor (WebKitGTK) é LGPL-2.1, da distribuição (no Slackware, compilado do
código publicado em webkitgtk.org).
Nenhuma marca de terceiros: Jangada é arte e nome próprios do Glove OS.
