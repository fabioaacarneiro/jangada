#!/bin/sh
# Gera o pacote .deb da Jangada em pacotes/ (Debian, Ubuntu e o Glove OS).
#
#   ./empacotar.sh
#
# A compilação baixa o motor do Chromium (CEF, ~200 MB) na primeira vez e precisa do
# cmake. O motor fica em /usr/lib/jangada, ao lado do programa; /usr/bin/jangada aponta
# para lá. O pacote substitui o glove-web (o nome de antes) e vira o navegador padrão
# do sistema (x-www-browser).
set -e
cd "$(dirname "$0")"
die() { echo "empacotar.sh: $*" >&2; exit 1; }
command -v dpkg-deb >/dev/null || die "falta o dpkg-deb"
command -v cmake >/dev/null || die "falta o cmake (o motor precisa)"
[ -f ui/Cargo.toml ] || git submodule update --init --recursive
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
echo "==> Compilando a Jangada $v"
cargo build --release
d=target/release
# O motor vem de onde o CEF foi extraído (a cópia ao lado do programa depende da versão
# do Cargo).
cefd=$(find target -name libcef.so -path '*cef_linux*' 2>/dev/null | head -1)
cefd=${cefd:+$(dirname "$cefd")}
[ -n "$cefd" ] || cefd=$d
[ -f "$cefd/libcef.so" ] || die "não achei o motor (libcef.so)"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
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
ln -s ../lib/jangada/jangada "$tmp/usr/bin/jangada"
# O ícone nos tamanhos do tema (menus e lançadores de fora do Glove).
LD_LIBRARY_PATH="$cefd" "$d/jangada" --icones "$tmp/usr/share/icons/hicolor" || die "não deu para gerar os ícones"
cat > "$tmp/usr/share/applications/jangada.desktop" <<'EOF'
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
EOF
cp README.md "$tmp/usr/share/doc/jangada/"
cp "$cefd/CREDITS.html" "$tmp/usr/share/doc/jangada/" 2>/dev/null || true
printf 'Jangada %s: Chromium Embedded Framework (CEF), licença BSD (veja CREDITS.html).\n' "$v" > "$tmp/usr/share/doc/jangada/SOURCE"
find "$tmp" -type d -exec chmod 755 {} +
find "$tmp/usr/share" -type f -exec chmod 644 {} +
chmod 644 "$lib"/*.pak "$lib"/*.dat "$lib"/*.bin "$lib"/locales/* "$lib"/*.json 2>/dev/null || true
chmod 755 "$lib/jangada" "$lib"/*.so* 2>/dev/null || true
# A caixa de areia do Chromium (para os sistemas sem o isolamento por namespace).
chmod 4755 "$lib/chrome-sandbox"
mkdir -p "$tmp/DEBIAN"
cat > "$tmp/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    update-alternatives --install /usr/bin/x-www-browser x-www-browser /usr/bin/jangada 200
    update-alternatives --install /usr/bin/gnome-www-browser gnome-www-browser /usr/bin/jangada 200
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -f /usr/share/icons/hicolor 2>/dev/null || true
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q 2>/dev/null || true
fi
EOF
cat > "$tmp/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ] || [ "$1" = deconfigure ]; then
    update-alternatives --remove x-www-browser /usr/bin/jangada || true
    update-alternatives --remove gnome-www-browser /usr/bin/jangada || true
fi
EOF
chmod 755 "$tmp/DEBIAN/postinst" "$tmp/DEBIAN/prerm"
size=$(du -sk "$tmp" | cut -f1)
cat > "$tmp/DEBIAN/control" <<EOF
Package: jangada
Version: $v
Architecture: amd64
Maintainer: Fabio Carneiro <fabioaacarneiro@gmail.com>
Installed-Size: $size
Depends: libnss3, libnspr4, libatk1.0-0t64 | libatk1.0-0, libatk-bridge2.0-0t64 | libatk-bridge2.0-0, libcups2t64 | libcups2, libxcomposite1, libxdamage1, libxrandr2, libxext6, libxfixes3, libx11-6, libxcb1, libxkbcommon0, libgbm1, libdrm2, libpango-1.0-0, libcairo2, libasound2t64 | libasound2, libdbus-1-3, libexpat1, libudev1, libglib2.0-0t64 | libglib2.0-0
Recommends: wl-clipboard
Provides: glove-web, www-browser
Conflicts: glove-web
Replaces: glove-web
Section: web
Priority: optional
Homepage: https://github.com/fabioaacarneiro/jangada
Description: Jangada, the Glove OS web browser (Chromium engine)
 The browser of the Glove desktop: the Chromium engine (CEF) with Glove's own window,
 tabs, address bar and pages (start page, errors), in the classic or the modern look.
 Light for modest computers: no GPU process, few page processes, nothing running in
 the background.
EOF
mkdir -p pacotes
f="pacotes/jangada_${v}_amd64.deb"
dpkg-deb --root-owner-group -Zxz --build "$tmp" "$f" >/dev/null
echo "==> Pacote: $f"
