// A janelinha: pergunta ao script da aba quantos links ela tem e manda destacar.
async function aba() {
  var abas = await jangada.tabs.query({ active: true, currentWindow: true });
  return abas[0];
}

aba().then(function (a) {
  return jangada.tabs.sendMessage(a.id, { tipo: "contar" });
}).then(function (r) {
  document.getElementById("numero").textContent = r.links + (r.links === 1 ? " link" : " links");
  document.getElementById("titulo").textContent = r.titulo;
}).catch(function () {
  // Sem script nesta aba (a página inicial, uma página da Jangada).
  document.getElementById("numero").textContent = "Esta página não tem o script.";
  document.getElementById("destacar").disabled = true;
});

document.getElementById("destacar").addEventListener("click", async function () {
  await jangada.tabs.sendMessage((await aba()).id, { tipo: "destacar" });
  window.close();
});

document.getElementById("opcoes").addEventListener("click", function () {
  jangada.runtime.openOptionsPage();
});
