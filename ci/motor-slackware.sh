#!/bin/sh
# O motor da Jangada no Slackware: o Slackware não traz o WebKitGTK, então ele é
# compilado aqui (do código publicado em webkitgtk.org, conferido pela soma) e vai junto
# no pacote da Jangada, em /usr/lib/jangada/motor. O resto (GTK 4, GStreamer, libsoup 3…)
# é o do Slackware -current.
#
#   sh ci/motor-slackware.sh PASTA
#
# Deixa a árvore em PASTA/usr/lib/jangada/motor. Se ela já existe com a mesma versão (o
# cache do CI), não compila de novo. Leva umas duas horas na primeira vez.
set -e
cd "$(dirname "$0")/.."
destino=${1:?uso: sh ci/motor-slackware.sh PASTA}
# A versão do motor (a mesma linha das distribuições; subir aqui a cada correção).
WEBKIT=2.54.3
WEBKIT_SHA256=2779c1b46f88762e8400650271ba14452dc4e30b37059981552d8b562b7d0862
UNIFDEF=2.12
UNIFDEF_SHA256=43ce0f02ecdcdc723b2475575563ddb192e988c886d368260bc0a63aee3ac400
# Baixa e confere pela soma fixada aqui (IPv4: no contêiner o IPv6 não sai).
baixar() {
    curl -4 -fsSL --retry 3 --max-time 900 -o "$2" "$1" || wget -4 -q -O "$2" "$1"
    echo "$3  $2" | sha256sum -c -
}
motor="$destino/usr/lib/jangada/motor"
if [ -f "$motor/VERSAO" ] && [ "$(cat "$motor/VERSAO")" = "$WEBKIT" ]; then
    echo "==> Motor WebKitGTK $WEBKIT já compilado (cache)"
    exit 0
fi
rm -rf "$destino"
mkdir -p "$destino"
trab=$(mktemp -d)
trap 'rm -rf "$trab"' EXIT
cd "$trab"
# O unifdef (a compilação do WebKit precisa; o Slackware não tem).
baixar "https://dotat.at/prog/unifdef/unifdef-$UNIFDEF.tar.xz" "unifdef-$UNIFDEF.tar.xz" "$UNIFDEF_SHA256"
tar xf "unifdef-$UNIFDEF.tar.xz"
make -s -C "unifdef-$UNIFDEF" prefix=/usr/local install >/dev/null
# O código do WebKitGTK (a soma é a publicada em webkitgtk.org, no .sums da versão).
baixar "https://webkitgtk.org/releases/webkitgtk-$WEBKIT.tar.xz" "webkitgtk-$WEBKIT.tar.xz" "$WEBKIT_SHA256"
tar xf "webkitgtk-$WEBKIT.tar.xz"
cd "webkitgtk-$WEBKIT"
# Só o que a Jangada usa (sem documentação, MiniBrowser, WebDriver, gamepad, voz). A
# caixa de areia do motor (bubblewrap) fica de fora: o Slackware não a tem.
cmake -S . -B build -GNinja \
    -DPORT=GTK -DCMAKE_BUILD_TYPE=Release -DUSE_GTK4=ON \
    -DCMAKE_INSTALL_PREFIX=/usr/lib/jangada/motor -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_INSTALL_LIBEXECDIR=libexec \
    -DENABLE_DOCUMENTATION=OFF -DENABLE_INTROSPECTION=OFF -DENABLE_MINIBROWSER=OFF -DENABLE_WEBDRIVER=OFF \
    -DENABLE_BUBBLEWRAP_SANDBOX=OFF -DENABLE_GAMEPAD=OFF -DENABLE_SPEECH_SYNTHESIS=OFF -DENABLE_JOURNALD_LOG=OFF \
    -DUSE_LIBBACKTRACE=OFF -DUSE_SYSPROF_CAPTURE=OFF -DUSE_AVIF=OFF
# Metade dos núcleos (a ligação das bibliotecas grandes pede muita memória).
n=$(( $(nproc) / 2 ))
[ "$n" -ge 2 ] || n=2
ninja -C build -j "$n"
DESTDIR="$destino" ninja -C build install >/dev/null
strip --strip-unneeded "$motor"/lib/*.so.* 2>/dev/null || true
printf '%s\n' "$WEBKIT" > "$motor/VERSAO"
echo "==> Motor WebKitGTK $WEBKIT em $motor"
