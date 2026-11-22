#!/bin/sh
# Compila a Jangada e gera o .tgz do Slackware num contêiner Slackware 15.0 (a instalação
# mínima). O Slackware não resolve dependências: a lista é o que o motor e a Jangada pedem.
set -e
cd "$(dirname "$0")/.."
. ci/comum.sh
sp() { slackpkg -batch=on -default_answer=y "$@"; }
yes | sp update gpg >/dev/null
sp update >/dev/null
sp install gcc binutils flex kernel-headers pkg-config libmpc mpfr gmp isl zlib make guile gc \
    libinput libevdev mtdev libwacom eudev libgudev glib2 pcre libffi mesa libglvnd libdrm libpciaccess \
    wayland expat libxkbcommon dejavu-fonts-ttf meson ninja python3 ca-certificates openssl perl \
    fontconfig wget \
    nss nspr cups pango cairo alsa-lib dbus atk at-spi2-core at-spi2-atk libXcomposite libXdamage libXrandr libXext libXfixes libXrender \
    libX11 libxcb libXau libXdmcp libxshmfence libXi pixman freetype fribidi harfbuzz graphite2 libxml2 brotli libpng bzip2 >/dev/null
update-ca-certificates --fresh >/dev/null 2>&1
# O cmake oficial (o do Slackware 15.0 é velho para o motor), com a soma publicada no release.
v=3.31.6; base=https://github.com/Kitware/CMake/releases/download/v$v
wget -q -O /tmp/cmake-$v-linux-x86_64.tar.gz "$base/cmake-$v-linux-x86_64.tar.gz"
wget -q -O /tmp/cmake.sha "$base/cmake-$v-SHA-256.txt"
(cd /tmp && grep "cmake-$v-linux-x86_64.tar.gz" cmake.sha | sha256sum -c -)
mkdir -p /opt/cmake && tar xzf /tmp/cmake-$v-linux-x86_64.tar.gz -C /opt/cmake --strip-components=1
export PATH="/opt/cmake/bin:$PATH"
instalar_rust
./empacotar-slackware.sh
ls pacotes/jangada-*.tgz
installpkg pacotes/jangada-*.tgz
conferir
