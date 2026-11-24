#!/bin/sh
# Compila a Jangada e gera o .tgz do Slackware num contêiner Slackware -current.
#
# O Slackware não traz o WebKitGTK: o motor é compilado à parte (ci/motor-slackware.sh,
# guardado no cache do CI em motor-slackware/) e vai junto no pacote, em
# /usr/lib/jangada/motor. O resto do que ele usa (GTK 4, GStreamer, libsoup 3) é o do
# Slackware -current. O Slackware 15.0 não tem um GTK 4 e um compilador novos o bastante
# para o WebKitGTK atual: nele, a Jangada fica na 1.15 (a do Chromium, branch cef).
set -e
cd "$(dirname "$0")/.."
. ci/comum.sh
sp() { slackpkg -batch=on -default_answer=y "$@"; }
yes | sp update gpg >/dev/null
sp update >/dev/null
# As séries d (compiladores) e l (bibliotecas), e da x o que o motor usa (mesa, drm,
# wayland, X11 para os cabeçalhos).
sp install d >/dev/null
sp install l >/dev/null
sp install x >/dev/null
# A série n inteira (o curl e o wget com todas as bibliotecas deles: ngtcp2, nghttp3…).
sp install n >/dev/null
# As séries a (base: zlib, xz, bzip2) e ap (sqlite e outras que o motor usa).
sp install a >/dev/null
sp install ap >/dev/null
sp install kernel-headers dbus elogind eudev libgudev >/dev/null || true
update-ca-certificates --fresh >/dev/null 2>&1 || true
motor_raiz="$PWD/motor-slackware"
sh ci/motor-slackware.sh "$motor_raiz"
motor="$motor_raiz/usr/lib/jangada/motor"
# A Jangada compila contra o motor da pasta (os .pc dizem /usr/lib/jangada/motor: o
# sysroot do pkg-config troca pela pasta) e o procura instalado em /usr/lib/jangada/motor.
export PKG_CONFIG_PATH="$motor/lib/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$motor_raiz"
export RUSTFLAGS="-C link-args=-Wl,-rpath,/usr/lib/jangada/motor/lib"
# Para gerar os ícones na compilação (o programa roda antes de instalado).
export LD_LIBRARY_PATH="$motor/lib"
instalar_rust
JANGADA_MOTOR="$motor" ./empacotar-slackware.sh
unset LD_LIBRARY_PATH
ls pacotes/jangada-*.tgz
installpkg pacotes/jangada-*.tgz
conferir
# Os processos do motor (o da página e o da rede) no lugar instalado.
for f in /usr/lib/jangada/motor/libexec/webkitgtk-6.0/WebKitWebProcess /usr/lib/jangada/motor/libexec/webkitgtk-6.0/WebKitNetworkProcess; do
    [ -x "$f" ] || { echo "ERRO: falta $f"; exit 1; }
done
