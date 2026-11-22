#!/bin/sh
# Compila a Jangada e gera o .rpm num contêiner Fedora (.github/workflows/pacotes.yml).
set -e
cd "$(dirname "$0")/.."
. ci/comum.sh
dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ make cmake pkgconf-pkg-config rpm-build curl ca-certificates findutils tar gzip which xz git \
    fontconfig dejavu-sans-fonts libxkbcommon-devel systemd-devel libinput-devel mesa-libgbm-devel \
    libdrm-devel wayland-devel \
    nss nspr atk at-spi2-atk cups-libs libXcomposite libXdamage libXrandr libXext libXfixes libX11 libxcb \
    mesa-libgbm libdrm pango cairo alsa-lib dbus-libs expat systemd-libs glib2 libxkbcommon
instalar_rust
./empacotar-rpm.sh
ls pacotes/jangada-*.rpm
# O dnf instala também as dependências do pacote: confere os nomes e as bibliotecas.
dnf install -y --setopt=install_weak_deps=False ./pacotes/jangada-*.rpm
conferir
