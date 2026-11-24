#!/bin/sh
# Compila a Jangada e gera o .rpm num contêiner Fedora (.github/workflows/release.yml).
set -e
cd "$(dirname "$0")/.."
. ci/comum.sh
dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ make pkgconf-pkg-config rpm-build curl ca-certificates findutils tar gzip which xz git \
    fontconfig dejavu-sans-fonts libxkbcommon-devel systemd-devel \
    webkitgtk6.0-devel gtk4-devel glib2-devel
instalar_rust
./empacotar-rpm.sh
ls pacotes/jangada-*.rpm
# O dnf instala também as dependências do pacote: confere os nomes e as bibliotecas.
dnf install -y --setopt=install_weak_deps=False ./pacotes/jangada-*.rpm
conferir
