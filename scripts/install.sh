#!/bin/sh
# Installs jellyui from a GitHub release on Linux x86_64, with a desktop
# launcher and icon. Run with --help for options.
set -eu

repo=iamd3vil/jellyui
prefix=$HOME/.local
version=latest
archive=
uninstall=0

usage() {
    cat <<EOF
Usage: install.sh [options]

Installs jellyui and its desktop launcher under ~/.local.

  --prefix DIR     install under DIR instead, e.g. /usr/local (run as root)
  --version TAG    release to install, e.g. v0.2.0 (default: latest)
  --archive FILE   install from a downloaded jellyui-*-linux-amd64.tar.gz
  --uninstall      remove what this script installed under the prefix
  -h, --help       show this help
EOF
}

die() {
    echo "install.sh: $*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case $1 in
    --prefix | --version | --archive)
        [ $# -ge 2 ] && [ -n "$2" ] || die "$1 needs a value"
        case $1 in
        --prefix) prefix=$2 ;;
        --version) version=$2 ;;
        --archive) archive=$2 ;;
        esac
        shift 2
        ;;
    --uninstall)
        uninstall=1
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *) die "unknown option: $1 (see --help)" ;;
    esac
done

bin=$prefix/bin/jellyui
apps=$prefix/share/applications
desktop=$apps/jellyui.desktop
icons=$prefix/share/icons/hicolor
icon=$icons/scalable/apps/jellyui.svg
docs=$prefix/share/doc/jellyui
doc_files="README.md CHANGELOG.md LICENSE OFL.txt THIRD-PARTY-LICENSES.txt"

# Only refresh an icon cache that already exists: creating one in a user's
# hicolor directory hides icons other apps add there later.
refresh_caches() {
    if [ -f "$icons/icon-theme.cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t "$icons" 2>/dev/null || true
    fi
}

if [ "$uninstall" = 1 ]; then
    rm -f "$bin" "$desktop" "$icon"
    for f in $doc_files; do
        rm -f "$docs/$f"
    done
    rmdir "$docs" 2>/dev/null || true
    refresh_caches
    echo "Removed jellyui from $prefix. Settings in ${XDG_CONFIG_HOME:-$HOME/.config}/jellyui are kept."
    exit 0
fi

[ "$(uname -s)" = Linux ] ||
    die "this installs the Linux build; on macOS use the .dmg from https://github.com/$repo/releases"
[ "$(uname -m)" = x86_64 ] ||
    die "only x86_64 builds are published; build from source on $(uname -m)"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ -z "$archive" ]; then
    command -v curl >/dev/null 2>&1 || die "curl is required"
    command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"
    if [ "$version" = latest ]; then
        # The latest-release page redirects to .../releases/tag/<tag>.
        url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
            "https://github.com/$repo/releases/latest") ||
            die "could not reach GitHub"
        version=${url##*/}
    fi
    case $version in
    v*) ;;
    *) version=v$version ;;
    esac
    name=jellyui-$version-linux-amd64.tar.gz
    base=https://github.com/$repo/releases/download/$version
    echo "Downloading jellyui $version"
    curl -fL# --proto '=https' --tlsv1.2 -o "$tmp/$name" "$base/$name" ||
        die "could not download $base/$name"
    curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/checksums.txt" "$base/checksums.txt" ||
        die "could not download $base/checksums.txt"
    (cd "$tmp" && grep "  $name\$" checksums.txt | sha256sum -c --quiet -) ||
        die "checksum mismatch for $name"
    archive=$tmp/$name
fi

mkdir "$tmp/release"
tar -xzf "$archive" -C "$tmp/release" || die "could not extract $archive"
[ -f "$tmp/release/jellyui" ] || die "$archive has no jellyui binary"

install -Dm755 "$tmp/release/jellyui" "$bin"
# Releases up to v0.2.0 shipped without the icon.
if [ -f "$tmp/release/jellyui.svg" ]; then
    install -Dm644 "$tmp/release/jellyui.svg" "$icon"
fi
for f in $doc_files; do
    if [ -f "$tmp/release/$f" ]; then
        install -Dm644 "$tmp/release/$f" "$docs/$f"
    fi
done

mkdir -p "$apps"
cat >"$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Jellyui
GenericName=Media Player
Comment=Desktop client for Jellyfin
Exec="$bin"
Icon=jellyui
Terminal=false
Categories=AudioVideo;Video;Player;
Keywords=jellyfin;media;video;tv;
StartupWMClass=jellyui
EOF
refresh_caches

echo "Installed jellyui to $bin"
missing=$(ldd "$bin" 2>&1 | grep 'not found' || true)
if [ -n "$missing" ]; then
    echo "Warning: some system libraries jellyui needs are missing:" >&2
    echo "$missing" >&2
    echo "Install ALSA, PulseAudio, xcb and xkbcommon from your distribution (glibc 2.35 or newer)." >&2
fi
case :$PATH: in
*:"$prefix/bin":*) ;;
*) echo "Note: $prefix/bin is not on your PATH. The desktop launcher works either way." ;;
esac
