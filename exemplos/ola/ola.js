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
