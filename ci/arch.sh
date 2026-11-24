#!/bin/sh
# Compila a Jangada e gera o pacote do Arch (.pkg.tar.zst) num contêiner Arch.
set -e
cd "$(dirname "$0")/.."
. ci/comum.sh
pacman -Syu --noconfirm --needed base-devel rust pkgconf curl zstd git fontconfig ttf-dejavu \
    libxkbcommon systemd-libs webkitgtk-6.0 gtk4 glib2
./empacotar-arch.sh
ls pacotes/jangada-*.pkg.tar.zst
# O pacman instala as dependências do pacote (os plugins do GStreamer etc.).
pacman -U --noconfirm pacotes/jangada-*.pkg.tar.zst
conferir
