#!/bin/sh
# Gera o .rpm da Jangada em pacotes/ (Fedora e as da mesma família). A árvore dos
# arquivos vem de preparar.sh; o rpmbuild só monta o pacote.
#
#   ./empacotar-rpm.sh
set -e
cd "$(dirname "$0")"
die() { echo "empacotar-rpm.sh: $*" >&2; exit 1; }
command -v rpmbuild >/dev/null || die "falta o rpmbuild (pacote rpm-build)"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
./preparar.sh "$tmp/raiz"
raiz="$tmp/raiz"
v=$(cat "$raiz/versao"); rm -f "$raiz/versao"
top="$tmp/rpm"
mkdir -p "$top/SPECS" "$top/BUILD" "$top/RPMS" "$top/SOURCES" "$top/SRPMS" "$top/BUILDROOT"
# A lista dos arquivos, para o %files.
lista="$tmp/lista"
# Só os arquivos: os diretórios são dos pacotes do sistema (filesystem, hicolor…) ou se
# criam com os arquivos.
(cd "$raiz" && find . -type f -o -type l | sed 's|^\.||') > "$lista"
sed -i -e '/^%dir /!s|^/|%attr(-,root,root) /|' "$lista"
cat > "$top/SPECS/jangada.spec" <<SPEC
Name: jangada
Version: $v
Release: 1
Summary: Jangada, the Glove OS web browser (WebKitGTK engine)
License: GPL-3.0-or-later
URL: https://github.com/fabioaacarneiro/jangada
Requires: webkitgtk6.0 >= 2.42 gtk4 >= 4.14 glib2 libxkbcommon systemd-libs gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugin-openh264
Provides: glove-web
Obsoletes: glove-web < 1.0.0
Recommends: wl-clipboard curl gstreamer1-plugin-libav pipewire-gstreamer
%global debug_package %{nil}
%description
The browser of the Glove desktop: the system WebKitGTK engine (kept up to date by the
distribution) with Glove's own window, tabs, address bar and pages, in the classic or the
modern look. Video codecs come from GStreamer (H.264 by OpenH264; the full set with the
RPM Fusion gstreamer1-plugin-libav).

%install
cp -a $raiz/. %{buildroot}/

%files -f $lista
SPEC
rpmbuild --define "_topdir $top" --define "_build_id_links none" -bb "$top/SPECS/jangada.spec" >/dev/null
mkdir -p pacotes
rpm=$(ls "$top"/RPMS/*/jangada-"$v"-1.*.rpm | head -1)
cp "$rpm" pacotes/
echo "==> Pacote: pacotes/$(basename "$rpm")"
