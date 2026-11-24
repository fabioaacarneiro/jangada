#!/bin/sh
# Compila a Jangada e monta a árvore dos arquivos do pacote numa pasta (a mesma para
# todos os formatos: o .deb, o .rpm, o pacote do Arch e o .tgz do Slackware).
#
#   ./preparar.sh PASTA
#
# O motor é o WebKitGTK 6.0 do sistema (os pacotes dependem dele); a compilação precisa
# dele e do GTK 4 para desenvolvimento (libwebkitgtk-6.0-dev, webkitgtk6.0-devel,
# webkitgtk-6.0). Onde a distribuição não tem o WebKitGTK (o Slackware), JANGADA_MOTOR
# aponta a pasta do motor compilado à parte, que vai junto em /usr/lib/jangada/motor.
# A Jangada fica em /usr/lib/jangada; /usr/bin/jangada aponta para lá.
# Quem chama lê a versão em `versao` (uma linha) dentro da pasta, para o nome do pacote.
set -e
cd "$(dirname "$0")"
die() { echo "preparar.sh: $*" >&2; exit 1; }
dest=${1:?uso: ./preparar.sh PASTA}
command -v pkg-config >/dev/null || die "falta o pkg-config"
pkg-config --exists webkitgtk-6.0 || die "falta o WebKitGTK 6.0 para desenvolvimento (libwebkitgtk-6.0-dev, webkitgtk6.0-devel ou webkitgtk-6.0)"
[ -f ui/Cargo.toml ] || git submodule update --init --recursive
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
echo "==> Compilando a Jangada $v (WebKitGTK $(pkg-config --modversion webkitgtk-6.0))"
cargo build --release
# A pasta dos artefatos (CARGO_TARGET_DIR, se o build usar outra).
alvo=${CARGO_TARGET_DIR:-target}
d=$alvo/release
[ -x "$d/jangada" ] || die "a compilação não deixou o programa"
mkdir -p "$dest"
tmp=$dest
lib="$tmp/usr/lib/jangada"
mkdir -p "$lib" "$tmp/usr/bin" "$tmp/usr/share/doc/jangada" "$tmp/usr/share/applications" "$tmp/usr/share/icons/hicolor"
cp "$d/jangada" "$lib/"
strip --strip-unneeded "$lib/jangada" 2>/dev/null || true
# O motor compilado à parte (só onde a distribuição não o tem).
if [ -n "$JANGADA_MOTOR" ]; then
    [ -d "$JANGADA_MOTOR" ] || die "JANGADA_MOTOR não é uma pasta: $JANGADA_MOTOR"
    mkdir -p "$lib/motor"
    cp -a "$JANGADA_MOTOR"/. "$lib/motor/"
fi
ln -sf ../lib/jangada/jangada "$tmp/usr/bin/jangada"
# O ícone nos tamanhos do tema (menus e lançadores de fora do Glove).
"$d/jangada" --icones "$tmp/usr/share/icons/hicolor" || die "não deu para gerar os ícones"
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
if [ -n "$JANGADA_MOTOR" ]; then
    printf 'Jangada %s: motor WebKitGTK (LGPL-2.1, compilado do código de https://webkitgtk.org/, em /usr/lib/jangada/motor).\n' "$v" > "$tmp/usr/share/doc/jangada/SOURCE"
else
    printf 'Jangada %s: motor WebKitGTK do sistema (LGPL-2.1, https://webkitgtk.org/).\n' "$v" > "$tmp/usr/share/doc/jangada/SOURCE"
fi
find "$tmp/usr" -type d -exec chmod 755 {} +
find "$tmp/usr/share" -type f -exec chmod 644 {} +
chmod 755 "$lib/jangada"
printf '%s\n' "$v" > "$tmp/versao"
echo "==> Árvore pronta em $dest"
