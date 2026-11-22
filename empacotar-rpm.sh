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
# O bit de caixa de areia (chrome-sandbox) é o único com permissão especial.
sed -i 's|^/usr/lib/jangada/chrome-sandbox$|%attr(4755,root,root) /usr/lib/jangada/chrome-sandbox|' "$lista"
sed -i -e '/^%dir /!s|^/|%attr(-,root,root) /|' -e 's|^%attr(-,root,root) %attr(4755|%attr(4755|' "$lista"
cat > "$top/SPECS/jangada.spec" <<SPEC
Name: jangada
Version: $v
Release: 1
Summary: Jangada, the Glove OS web browser (Chromium engine)
License: GPL-3.0-or-later
URL: https://github.com/fabioaacarneiro/jangada
Requires: nss nspr atk at-spi2-atk cups-libs libXcomposite libXdamage libXrandr libXext libXfixes libX11 libxcb libxkbcommon mesa-libgbm libdrm pango cairo alsa-lib dbus-libs expat systemd-libs glib2
Provides: glove-web
Obsoletes: glove-web < 1.0.0
Recommends: wl-clipboard curl
%global debug_package %{nil}
%description
The browser of the Glove desktop: the Chromium engine (CEF) with Glove's own window,
tabs, address bar and pages (start page, errors), in the classic or the modern look.
Light for modest computers: no GPU process, few page processes, nothing running in the
background.

%install
cp -a $raiz/. %{buildroot}/

%files -f $lista
SPEC
rpmbuild --define "_topdir $top" --define "_build_id_links none" -bb "$top/SPECS/jangada.spec" >/dev/null
mkdir -p pacotes
rpm=$(ls "$top"/RPMS/*/jangada-"$v"-1.*.rpm | head -1)
cp "$rpm" pacotes/
echo "==> Pacote: pacotes/$(basename "$rpm")"
