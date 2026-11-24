// Conta quantas vezes você abriu o site (nos dados guardados da extensão) e mostra um aviso.
jangada.storage.local.get({ visitas: 0 }).then(function (dados) {
  var visitas = dados.visitas + 1;
  jangada.storage.local.set({ visitas: visitas });
  var aviso = document.createElement("div");
  aviso.id = "destaque-de-exemplo";
  aviso.textContent = "Destaque de exemplo: esta é a visita número " + visitas + " a este site.";
  document.body.prepend(aviso);
});
