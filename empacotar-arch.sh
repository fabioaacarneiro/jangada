#!/bin/sh
# Gera o pacote do Arch (.pkg.tar.zst) da Jangada em pacotes/. A árvore dos arquivos
# vem de preparar.sh; aqui só se escreve o .PKGINFO e se empacota (sem o makepkg, que
# não roda como root e a receita baixaria o código de novo).
#
#   ./empacotar-arch.sh
set -e
cd "$(dirname "$0")"
die() { echo "empacotar-arch.sh: $*" >&2; exit 1; }
command -v tar >/dev/null || die "falta o tar"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
./preparar.sh "$tmp/raiz"
raiz="$tmp/raiz"
v=$(cat "$raiz/versao"); rm -f "$raiz/versao"
tamanho=$(du -sb "$raiz" | cut -f1)
cat > "$raiz/.PKGINFO" <<PKG
pkgname = jangada
pkgbase = jangada
pkgver = $v-1
pkgdesc = Jangada, the Glove OS web browser (WebKitGTK engine)
url = https://github.com/fabioaacarneiro/jangada
builddate = $(date +%s)
packager = Glove OS
size = $tamanho
arch = x86_64
license = GPL-3.0-or-later
depend = webkitgtk-6.0
depend = gtk4
depend = glib2
depend = libxkbcommon
depend = systemd-libs
depend = gst-plugins-good
depend = gst-plugins-bad
depend = gst-libav
provides = glove-web
conflicts = glove-web
optdepend = wl-clipboard: copiar e colar no Wayland
optdepend = gnome-keyring: guardar as senhas (o chaveiro do sistema)
optdepend = gst-plugin-pipewire: câmera e tela pelo PipeWire
optdepend = hunspell-en_us: corretor ortográfico
PKG
mkdir -p pacotes
saida="$PWD/pacotes/jangada-$v-1-x86_64.pkg.tar.zst"
# A lista na ordem do pacote, sem o "./" (o pacman espera os caminhos assim).
(cd "$raiz" && find . -mindepth 1 | sed 's|^\./||' | sort) > "$tmp/lista"
# O .MTREE (somas e permissões dos arquivos), como o makepkg faz: o pacman confere com ele.
(cd "$raiz" && bsdtar -cf - --format=mtree --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' --files-from "$tmp/lista" | gzip -n > .MTREE)
echo .MTREE >> "$tmp/lista"
# --no-recursion: cada caminho entra uma vez (a lista já traz as pastas e os arquivos).
tar --zstd --owner=0 --group=0 --numeric-owner --no-recursion -cf "$saida" -C "$raiz" --files-from "$tmp/lista"
echo "==> Pacote: $saida"
