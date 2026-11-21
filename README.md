# Jangada

O navegador do **Glove OS**. O motor é o do Chromium (via [CEF](https://github.com/chromiumembedded/cef)),
com os recursos da web de hoje; a janela, as abas, a barra de endereço e as páginas da própria
Jangada (início e erro) são do Glove, desenhadas pelo [glove-ui](https://github.com/fabioaacarneiro/glove-ui),
no visual clássico ou no Moderno (claro e escuro).

*English: Jangada is the Glove OS web browser: the Chromium engine (CEF) in a Glove window.*

## Leve

Feita para computadores modestos:

- sem o processo da placa de vídeo: a página é desenhada pelo processador e copiada para a janela;
- no máximo 4 processos de página, sem o cache de voltar (páginas inteiras na memória) e sem rede
  de fundo (atualizações de componentes, tradução, sugestões).

Para usar a placa de vídeo: `JANGADA_GPU=1 jangada`.

## Como usar

- **Barra de endereço** (como a do Safari): fora de foco mostra só o site, com o cadeado; clique ou
  Ctrl+L e digite um endereço ou uma busca (DuckDuckGo). Esc volta. Recarregar e parar ficam dentro
  dela.
- **Abas:** Ctrl+T abre (na página inicial), Ctrl+W fecha, Ctrl+Tab passa; links que abrem janela
  nova viram abas. Cada aba mostra o ícone do site.
- **Página inicial** própria, com a busca e os atalhos dos sites; **página de erro** própria.
- **Botão direito:** o menu do Glove, solto, por cima de tudo (abrir o link em nova aba, copiar,
  colar, buscar o texto escolhido).
- **Downloads** direto na pasta Downloads; **avisos das páginas** (alert e confirm) como alertas do
  Glove, em janela própria; a mãozinha nos links e a barra de texto nos campos.
- Abrir de novo (`jangada endereço`) abre uma aba na janela que já está aberta.
- No pacote, a Jangada vira o navegador padrão do sistema (`x-www-browser`).

## Compilar e empacotar

Precisa do Rust 1.89 ou mais novo, do cmake e de um compilador C++. A primeira compilação baixa o
motor do Chromium (~200 MB).

```sh
git clone --recursive https://github.com/fabioaacarneiro/jangada
cd jangada
cargo build --release         # target/release/jangada (o motor fica ao lado)
./empacotar.sh                # pacotes/jangada_VERSÃO_amd64.deb
```

A cada tag `v*`, o CI gera o pacote e o publica na release; o repositório apt do Glove OS o pega de
lá.

## Licença

GPL-3.0-or-later. O motor (CEF/Chromium) tem a licença BSD dele (veja `CREDITS.html` no pacote).
Nenhuma marca de terceiros: Jangada é arte e nome próprios do Glove OS.
