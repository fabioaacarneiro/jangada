#!/bin/sh
# Gera o .tgz do Slackware da Jangada em pacotes/. A árvore dos arquivos vem de
# preparar.sh; o Slackware não resolve dependências, então a descrição diz o que o
# sistema precisa (instale pelo slackpkg se faltar).
#
#   ./empacotar-slackware.sh
set -e
cd "$(dirname "$0")"
die() { echo "empacotar-slackware.sh: $*" >&2; exit 1; }
command -v tar >/dev/null || die "falta o tar"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
./preparar.sh "$tmp/raiz"
raiz="$tmp/raiz"
v=$(cat "$raiz/versao"); rm -f "$raiz/versao"
mkdir -p "$raiz/install"
cat > "$raiz/install/slack-desc" <<DESC
jangada: jangada (o navegador do Glove OS)
jangada:
jangada: Navegador com o motor do Chromium (CEF) e a janela do Glove. Leve
jangada: para computadores modestos. Precisa das bibliotecas do sistema: nss,
jangada: nspr, cups, pango, cairo, alsa-lib, dbus, expat, glib2, mesa (gbm e
jangada: drm), libxkbcommon e libX11 com as extensões. Instale pelo slackpkg
jangada: o que faltar.
jangada:
jangada: https://github.com/fabioaacarneiro/jangada
jangada:
DESC
mkdir -p pacotes
saida="$PWD/pacotes/jangada-$v-x86_64-1.tgz"
tar -czf "$saida" --owner=0 --group=0 --numeric-owner -C "$raiz" .
echo "==> Pacote: $saida"
