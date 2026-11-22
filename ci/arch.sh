#!/bin/sh
# Compila a Jangada e gera o pacote do Arch (.pkg.tar.zst) num contêiner Arch.
set -e
cd "$(dirname "$0")/.."
. ci/comum.sh
pacman -Syu --noconfirm --needed base-devel cmake rust pkgconf curl zstd git fontconfig ttf-dejavu \
    libxkbcommon libinput mesa libdrm wayland nss nspr atk at-spi2-atk libcups libxcomposite libxdamage \
    libxrandr libxext libxfixes libx11 libxcb pango cairo alsa-lib dbus expat systemd-libs glib2
./empacotar-arch.sh
ls pacotes/jangada-*.pkg.tar.zst
pacman -U --noconfirm pacotes/jangada-*.pkg.tar.zst
conferir
