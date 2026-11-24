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
jangada: Navegador com o motor WebKitGTK e a janela do Glove. O motor vem
jangada: junto (em /usr/lib/jangada/motor); o resto é do Slackware -current:
jangada: gtk4, glib2, libsoup3, gstreamer e os plugins (base, good,
jangada: bad-free, libav), icu4c, harfbuzz, libwebp, woff2, libsecret, mesa.
jangada: Para o Slackware 15.0, use a Jangada 1.15 (motor do Chromium).
jangada:
jangada: https://github.com/fabioaacarneiro/jangada
jangada:
DESC
mkdir -p pacotes
saida="$PWD/pacotes/jangada-$v-x86_64-1.tgz"
tar -czf "$saida" --owner=0 --group=0 --numeric-owner -C "$raiz" .
echo "==> Pacote: $saida"
