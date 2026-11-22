#!/bin/sh
# Gera o pacote .deb da Jangada em pacotes/ (Debian, Ubuntu e o Glove OS).
#
#   ./empacotar.sh
#
# A árvore dos arquivos vem de preparar.sh (a mesma dos outros formatos). O pacote
# substitui o glove-web (o nome de antes) e vira o navegador padrão do sistema
# (x-www-browser).
set -e
cd "$(dirname "$0")"
die() { echo "empacotar.sh: $*" >&2; exit 1; }
command -v dpkg-deb >/dev/null || die "falta o dpkg-deb"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
./preparar.sh "$tmp/raiz"
raiz="$tmp/raiz"
v=$(cat "$raiz/versao"); rm -f "$raiz/versao"
mkdir -p "$raiz/DEBIAN"
cp debian/postinst debian/prerm "$raiz/DEBIAN/"
chmod 755 "$raiz/DEBIAN/postinst" "$raiz/DEBIAN/prerm"
size=$(du -sk "$raiz" | cut -f1)
sed -e "s/@VERSAO@/$v/" -e "s/@TAMANHO@/$size/" debian/control > "$raiz/DEBIAN/control"
mkdir -p pacotes
f="pacotes/jangada_${v}_amd64.deb"
dpkg-deb --root-owner-group -Zxz --build "$raiz" "$f" >/dev/null
echo "==> Pacote: $f"
