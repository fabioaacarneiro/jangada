# Jangada

O navegador do **Glove OS**. O motor é o do Chromium (via [CEF](https://github.com/chromiumembedded/cef)),
com os recursos da web de hoje; a janela, as abas, a barra de endereço e as páginas da própria
Jangada (início e erro) são do Glove, desenhadas pelo [glove-ui](https://github.com/fabioaacarneiro/glove-ui),
no visual clássico ou no Moderno (claro e escuro).

*English: Jangada is the Glove OS web browser: the Chromium engine (CEF) in a Glove window.*

## Leve

Feita para computadores modestos:

- sem o processo da placa de vídeo: a página é desenhada pelo processador e copiada para a janela,
  só nas partes que mudaram (passar o mouse ou um cursor piscando não recopia a página inteira);
- a janela só é refeita quando algo muda (parada, a Jangada quase não gasta processador);
- no máximo 4 processos de página, sem o cache de voltar (páginas inteiras na memória) e sem rede
  de fundo (atualizações de componentes, tradução, sugestões).

Para usar a placa de vídeo: `JANGADA_GPU=1 jangada`.

## Desenvolvimento web

- **Ver código-fonte** (Ctrl+U, ou no menu Visualizar): abre o HTML da página numa aba.
- **Ferramentas do desenvolvedor** (Ctrl+Shift+I, ou Visualizar ▸ Ferramentas do Desenvolvedor):
  o inspetor do Chromium (elementos, console, rede, fontes, desempenho) abre numa aba ao lado,
  ligado à página da aba atual. Vêm **desligadas**: ligue em Preferências ▸ Avançado e abra a
  Jangada de novo.
- **Por que desligadas:** o inspetor fala com o motor por uma porta local de depuração (só em
  `127.0.0.1`, numa porta livre escolhida ao acaso). Qualquer programa deste computador que a
  alcance pode controlar o navegador, ler as páginas abertas e os cookies delas. Ligue só quando
  for usar. A janela privada não abre a porta pela preferência, só com `--depurar`.
- **Para uma sessão só, ou para outra ferramenta:** `jangada --depurar [PORTA]` (padrão 9222) abre a
  porta numa porta fixa; dá para usar `chrome://inspect` de outro navegador, o Puppeteer ou o
  Playwright. Só vale ao abrir a Jangada: se ela já estiver aberta, feche-a antes.

## Permissões dos sites

Quando um site pede a **câmera**, o **microfone**, a **localização** ou as **notificações**, a Jangada
pergunta numa janela própria: *Negar*, *Só desta vez* ou *Permitir*. "Permitir" e "Negar" ficam
guardados para aquele site (Preferências ▸ Privacidade mostra quantos sites têm permissões, e
"Esquecer as Permissões" apaga tudo). "Só desta vez" vale para a página aberta: ao recarregar, a
pergunta volta.

- **Câmera e microfone** funcionam (testados com uma câmera virtual).
- **Localização:** a pergunta funciona, mas a posição vem do serviço de localização do Google, e os
  builds sem a chave desse serviço não a obtêm. Mesmo permitindo, o site pode esperar até dar tempo
  esgotado.
- **Captura da tela e do áudio do computador:** a Jangada não concede.

## Extensões

A Jangada aceita extensões no formato do Chrome (Manifest V3, o subconjunto que cabe aqui): uma
pasta com um `manifest.json`, um `.zip` dela ou um link do GitHub. A instalação mostra as
permissões antes de gravar; nada entra sem a sua confirmação.

```sh
jangada --instalar ./minha-extensao          # pasta ou .zip
jangada --instalar https://github.com/usuario/repositorio   # ou .../tree/RAMO/PASTA
jangada --extensoes                          # lista as instaladas (ativa ou não)
jangada --remover ID                         # tira uma
```

Também dá para instalar e ligar ou desligar pelo Preferências ▸ **Instaladas**.

- **Suporte:** `content_scripts` (com `matches`, `js`, `css` e `run_at`), `chrome.storage.local`
  (guardado por extensão, no seu perfil) e os padrões de site do Chrome (`<all_urls>`, `*://`,
  `*.dominio.com`, caminhos com `*`).
- **Não há:** scripts de fundo (`background`), janela de ação, páginas de opções, APIs além do
  `storage` e do acesso às páginas, nem `chrome.tabs` e afins.
- **Limite:** os scripts rodam no mesmo mundo da página (não isolado), então não dá para esconder
  nada dela; só instale extensões de quem você confia.
- O exemplo completo está em `exemplos/destaque/` (aviso no topo de páginas e contagem de visitas).

## Janela privada

**Arquivo ▸ Nova Janela Privada** (Ctrl+Shift+N) abre uma janela que não deixa rastro: o histórico,
os favoritos, as abas da última vez e as senhas não são gravados, e os cookies e o armazenamento dos
sites ficam só na memória, sem perfil em disco. Ao fechar, a pasta temporária dela (em
`XDG_RUNTIME_DIR`, que fica na memória) é apagada. Não oferece para guardar senhas nem as preenche.
O que você baixa continua na pasta Downloads (é um arquivo seu, não um rastro). Uma janela privada
é um processo à parte, então ela não se mistura com a normal.

## Escudos, senhas e importar

- **Escudos** (como os do Brave, com o mesmo motor de bloqueio dele, o `adblock`): anúncios e
  rastreadores barrados antes de carregar (EasyList, EasyPrivacy, a lista portuguesa, a do Peter Lowe
  e os filtros do uBlock Origin, renovadas toda semana em `~/.cache/jangada/escudos`) e os espaços de
  anúncio escondidos na página. O escudo na barra mostra quantos itens a página teve barrados; o
  clique liga ou desliga os escudos no site. Também barra os cookies de terceiros e não deixa o
  WebRTC mostrar o endereço da rede local.
- **Senhas:** ao entrar num site, a Jangada oferece guardar a senha (no chaveiro do sistema: aparece
  no Acesso às Chaves do Glove) e a preenche da próxima vez. Preferências ▸ Senhas lista, mostra,
  copia e apaga.
- **Importar de outro navegador** (Arquivo ▸ Importar de Outro Navegador…): do Google Chrome,
  Chromium, Brave, Microsoft Edge ou Vivaldi deste computador, os favoritos, o histórico, as sessões
  abertas nos sites (os cookies: continua com as contas abertas) e as senhas, decifrados com a chave
  que o próprio navegador guarda no chaveiro. A sincronização com a conta do Google só existe no
  Chrome; o que vem é o que ele guarda no computador.
- **Extensões** (menu Extensões): as da Jangada (escudos, cookies de terceiros, modo escuro nas
  páginas). As da loja do Chrome não funcionam: o motor desenha a página sem janela própria (é o que
  deixa a janela, as abas e os menus serem do Glove), e nesse modo o Chromium não carrega extensões.
- **Chaves de acesso (passkeys)** ficam desligadas: sem janela própria o motor não mostra o pedido
  do sistema, e a página ficava esperando para sempre. Os sites oferecem a senha e o código.

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
  trás. O botão do meio numa aba a fecha.
- **Página inicial** própria, com a busca e os atalhos dos sites; **página de erro** própria. Em
  Preferências ▸ Geral dá para escolher o fundo (padrão, cinza claro, azul claro, grafite ou uma
  imagem do computador), e mostrar ou esconder os atalhos e a caixa de busca.
- **Botão direito:** o menu do Glove, solto, por cima de tudo (abrir o link em nova aba, copiar,
  colar, buscar o texto escolhido).
- **Downloads** direto na pasta Downloads, com a janela Downloads (Ctrl+J, ou a seta na barra):
  o andamento, abrir, mostrar na pasta e cancelar; **avisos das páginas** (alert e confirm) como alertas do
  Glove, em janela própria; a mãozinha nos links e a barra de texto nos campos.
- **Favoritos:** a estrela no campo de endereço (ou Ctrl+D) guarda a página; a barra de favoritos
  (Ctrl+Shift+B) e o menu Favoritos levam a elas, e a página inicial mostra os favoritos.
- **Histórico:** os últimos sites no menu Histórico e todos na janela Histórico (Ctrl+H), com
  busca; Reabrir a Última Aba Fechada e **Reabrir as Abas da Última Sessão** (as abas que estavam
  abertas da última vez que a Jangada fechou); Limpar o Histórico… nas Preferências.
- **Buscar na página** (Ctrl+F), com quantos achou e anterior/próximo; **Imprimir** (Ctrl+P), pela
  folha Imprimir do Glove (impressoras do CUPS, cópias, páginas, pré-visualizar), e **Salvar como
  PDF** (menu Arquivo), na pasta Downloads.
- **Preferências** (Ctrl+,), como as do Safari:
  - *Geral:* página inicial (a da Jangada ou um endereço), reabrir as abas da última vez, a barra de
    favoritos e o tamanho padrão das páginas;
  - *Contas:* **iniciar a sessão no Google** (Gmail, YouTube, Drive e os outros serviços com a sua
    conta; também no menu Jangada), gerenciar a conta e sair. A sessão fica guardada na Jangada; a
    sincronização de favoritos e senhas do Chrome não existe no motor dela;
  - *Busca:* DuckDuckGo, Google, Bing, Ecosia, Brave Search ou Startpage;
  - *Privacidade:* bloquear as janelas que as páginas abrem sozinhas, pedir para não rastrear (Não
    Rastrear e GPC), limpar o histórico, apagar cookies e dados dos sites;
  - *Extensões:* o **bloqueador de anúncios e rastreadores** (vem ligado) e o **modo escuro nas
    páginas**. As extensões da loja do Chrome ainda não funcionam: o motor sem janela própria não
    as carrega;
  - *Avançado:* o endereço completo na barra, usar a placa de vídeo e a pasta Downloads.
- Os sites veem a Jangada como um Chrome comum (o mesmo user agent): assim o Google deixa iniciar
  a sessão e os sites funcionam igual.
- Abrir de novo (`jangada endereço`) abre uma aba na janela que já está aberta.
- No pacote, a Jangada vira o navegador padrão do sistema (`x-www-browser`).

## Instalar

Os pacotes saem na release de cada versão (`github.com/fabioaacarneiro/jangada/releases`):

- **Debian, Ubuntu e Glove OS:** `jangada_VERSÃO_amd64.deb`. Vira o navegador padrão do sistema.
- **Fedora:** `jangada-VERSÃO-1.x86_64.rpm` (`sudo dnf install ./jangada-*.rpm`). Testado no Fedora
  mais novo; as dependências vêm pelo dnf.
- **Arch:** `jangada-VERSÃO-1-x86_64.pkg.tar.zst` (`sudo pacman -U ./jangada-*.pkg.tar.zst`).
- **Slackware 15.0 e -current:** `jangada-VERSÃO-x86_64-1.tgz` (`sudo installpkg ./jangada-*.tgz`).
  O Slackware não resolve dependências: instale pelo slackpkg as que faltarem (a descrição do pacote
  lista).

Nos três últimos o pacote não troca o navegador padrão sozinho: escolha a Jangada nas configurações
do sistema (ou com `xdg-settings set default-web-browser jangada.desktop`).

## Compilar e empacotar

Precisa do Rust 1.89 ou mais novo, do cmake e de um compilador C++. A primeira compilação baixa o
motor do Chromium (~200 MB).

```sh
git clone --recursive https://github.com/fabioaacarneiro/jangada
cd jangada
cargo build --release         # target/release/jangada (o motor fica ao lado)
./empacotar.sh                # pacotes/jangada_VERSÃO_amd64.deb
./empacotar-rpm.sh            # pacotes/jangada-VERSÃO-1.x86_64.rpm (precisa do rpmbuild)
./empacotar-arch.sh           # pacotes/jangada-VERSÃO-1-x86_64.pkg.tar.zst
./empacotar-slackware.sh      # pacotes/jangada-VERSÃO-x86_64-1.tgz
```
A árvore dos arquivos vem sempre de `preparar.sh`, a mesma para todos os formatos. O `ci/` tem os
scripts que compilam e conferem cada distribuição em contêiner (é o que a CI roda).

A cada tag `v*`, o CI gera os pacotes (.deb, .rpm, Arch e Slackware), confere cada um instalando-o
na própria distribuição e publica tudo na release. O repositório apt do Glove OS pega o .deb de lá.

## Licença

GPL-3.0-or-later. O motor (CEF/Chromium) tem a licença BSD dele (veja `CREDITS.html` no pacote).
Nenhuma marca de terceiros: Jangada é arte e nome próprios do Glove OS.
