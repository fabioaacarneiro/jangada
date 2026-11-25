var campo = document.getElementById("cor");
jangada.storage.local.get({ cor: "#ffd54a" }).then(function (d) {
  campo.value = d.cor;
});
campo.addEventListener("input", function () {
  jangada.storage.local.set({ cor: campo.value }).then(function () {
    document.getElementById("aviso").textContent = "Guardada. As abas abertas já mudaram.";
  });
});
