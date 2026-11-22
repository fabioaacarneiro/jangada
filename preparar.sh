#!/bin/sh
# Compila a Jangada e monta a árvore dos arquivos do pacote numa pasta (a mesma para
# todos os formatos: o .deb, o .rpm, o pacote do Arch e o .tgz do Slackware).
#
#   ./preparar.sh PASTA
#
# A compilação baixa o motor do Chromium (CEF, ~200 MB) na primeira vez e precisa do cmake.
# O motor fica em /usr/lib/jangada, ao lado do programa; /usr/bin/jangada aponta para lá.
# Quem chama lê a versão em `versao` (uma linha) dentro da pasta, para o nome do pacote.
set -e
cd "$(dirname "$0")"
die() { echo "preparar.sh: $*" >&2; exit 1; }
dest=${1:?uso: ./preparar.sh PASTA}
command -v cmake >/dev/null || die "falta o cmake (o motor precisa)"
[ -f ui/Cargo.toml ] || git submodule update --init --recursive
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
echo "==> Compilando a Jangada $v"
cargo build --release
# A pasta dos artefatos (CARGO_TARGET_DIR, se o build usar outra).
alvo=${CARGO_TARGET_DIR:-target}
d=$alvo/release
# O motor vem de onde o CEF foi extraído (a cópia ao lado do programa depende da versão do Cargo).
cefd=$(find "$alvo" -name libcef.so -path '*cef_linux*' 2>/dev/null | head -1)
cefd=${cefd:+$(dirname "$cefd")}
[ -n "$cefd" ] || cefd=$d
[ -f "$cefd/libcef.so" ] || die "não achei o motor (libcef.so)"
mkdir -p "$dest"
tmp=$dest
lib="$tmp/usr/lib/jangada"
mkdir -p "$lib/locales" "$tmp/usr/bin" "$tmp/usr/share/doc/jangada" "$tmp/usr/share/applications" "$tmp/usr/share/icons/hicolor"
cp "$d/jangada" "$lib/"
for f in libcef.so chrome-sandbox icudtl.dat v8_context_snapshot.bin resources.pak chrome_100_percent.pak chrome_200_percent.pak libvk_swiftshader.so libvulkan.so.1 vk_swiftshader_icd.json; do
    if [ -e "$cefd/$f" ]; then cp "$cefd/$f" "$lib/"; fi
done
[ -f "$lib/chrome-sandbox" ] || die "o motor veio incompleto (sem chrome-sandbox)"
# Só os idiomas que a Jangada fala (o motor traz uns 200).
cp "$cefd"/locales/pt-BR*.pak "$cefd"/locales/en-US*.pak "$lib/locales/" 2>/dev/null || true
strip --strip-unneeded "$lib/libcef.so" "$lib/libvk_swiftshader.so" 2>/dev/null || true
ln -sf ../lib/jangada/jangada "$tmp/usr/bin/jangada"
# O ícone nos tamanhos do tema (menus e lançadores de fora do Glove).
LD_LIBRARY_PATH="$cefd" "$d/jangada" --icones "$tmp/usr/share/icons/hicolor" || die "não deu para gerar os ícones"
cat > "$tmp/usr/share/applications/jangada.desktop" <<'DESK'
[Desktop Entry]
Type=Application
Name=Jangada
GenericName=Navegador
GenericName[en]=Web Browser
Comment=O navegador do Glove OS
Comment[en]=The Glove OS web browser
Exec=jangada %U
Icon=jangada
Terminal=false
Categories=Network;WebBrowser;
MimeType=text/html;application/xhtml+xml;x-scheme-handler/http;x-scheme-handler/https;
StartupWMClass=glove.Navegador
Keywords=web;internet;navegador;browser;
DESK
cp README.md "$tmp/usr/share/doc/jangada/"
cp "$cefd/CREDITS.html" "$tmp/usr/share/doc/jangada/" 2>/dev/null || true
printf 'Jangada %s: Chromium Embedded Framework (CEF), licença BSD (veja CREDITS.html).\n' "$v" > "$tmp/usr/share/doc/jangada/SOURCE"
find "$tmp/usr" -type d -exec chmod 755 {} +
find "$tmp/usr/share" -type f -exec chmod 644 {} +
chmod 644 "$lib"/*.pak "$lib"/*.dat "$lib"/*.bin "$lib"/locales/* "$lib"/*.json 2>/dev/null || true
chmod 755 "$lib/jangada" "$lib"/*.so* 2>/dev/null || true
# A caixa de areia do Chromium (para os sistemas sem o isolamento por namespace).
chmod 4755 "$lib/chrome-sandbox"
printf '%s\n' "$v" > "$tmp/versao"
echo "==> Árvore pronta em $dest"
