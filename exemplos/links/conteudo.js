// O script da página: conta os links, destaca quando a janelinha pede, e põe o número
// no botão da extensão.
function contar() {
  return document.querySelectorAll("a[href]").length;
}

jangada.action.setBadgeText({ text: String(contar()) });

jangada.runtime.onMessage.addListener(function (msg, remetente, responder) {
  if (msg.tipo === "contar") {
    responder({ links: contar(), titulo: document.title });
  } else if (msg.tipo === "destacar") {
    jangada.storage.local.get({ cor: "#ffd54a" }).then(function (d) {
      document.querySelectorAll("a[href]").forEach(function (a) {
        a.style.outline = "2px solid " + d.cor;
      });
      responder({ feito: true });
    });
    return true; // a resposta vem depois (o storage é uma promessa)
  }
});

// A cor mudou nas opções: os links já destacados mudam junto.
jangada.storage.onChanged.addListener(function (mudancas) {
  if (!mudancas.cor) return;
  document.querySelectorAll("a[href]").forEach(function (a) {
    if (a.style.outline) a.style.outline = "2px solid " + mudancas.cor.newValue;
  });
});
