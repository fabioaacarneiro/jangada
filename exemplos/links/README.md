# Links da página

Uma extensão de exemplo para a Jangada com tudo o que vai além de um script na página:

- um **botão na barra** com ícone e um **selo** (o número de links da página);
- a **janelinha** (`janelinha.html`), que conversa com o script da aba
  (`tabs.query` e `tabs.sendMessage`);
- a **página de opções** (`opcoes.html`), aberta numa aba;
- os **dados guardados** com aviso de mudança: trocar a cor nas opções muda os links já
  destacados nas abas abertas (`storage.onChanged`).

Instalar: `jangada --install exemplos/links` (ou pelo link do repositório, em
Preferências ▸ Instaladas).

O guia de como escrever uma extensão está em [`docs/EXTENSOES.md`](../../docs/EXTENSOES.md).
