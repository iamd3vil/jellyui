#!/bin/sh
# Installs jellyui from a GitHub release: on Linux x86_64 with a desktop
# launcher and icon, on Apple Silicon Macs as Jellyui.app. Run with --help
# for options.
set -eu

repo=iamd3vil/jellyui
prefix=
version=latest
archive=
uninstall=0

usage() {
    cat <<EOF
Usage: install.sh [options]

Linux: installs jellyui and its desktop launcher under ~/.local.
macOS: installs Jellyui.app into /Applications, or ~/Applications when
       /Applications isn't writable.

  --prefix DIR     Linux: install under DIR instead, e.g. /usr/local (as root)
                   macOS: put Jellyui.app in DIR instead
  --version TAG    release to install, e.g. v0.3.0 (default: latest)
  --archive FILE   install from a downloaded release file instead
                   (jellyui-*-linux-amd64.tar.gz or jellyui-*-darwin-arm64.dmg)
  --uninstall      remove what this script installed
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

os=$(uname -s)
case $os in
Linux)
    asset=linux-amd64.tar.gz
    settings=${XDG_CONFIG_HOME:-$HOME/.config}/jellyui
    : "${prefix:=$HOME/.local}"
    ;;
Darwin)
    asset=darwin-arm64.dmg
    settings="$HOME/Library/Application Support/jellyui"
    if [ -z "$prefix" ]; then
        if [ -w /Applications ]; then
            prefix=/Applications
        else
            prefix=$HOME/Applications
        fi
    fi
    ;;
*) die "no builds are published for $os; build from source" ;;
esac

# Linux layout.
bin=$prefix/bin/jellyui
apps=$prefix/share/applications
desktop=$apps/jellyui.desktop
icons=$prefix/share/icons/hicolor
icon=$icons/scalable/apps/jellyui.svg
docs=$prefix/share/doc/jellyui
doc_files="README.md CHANGELOG.md LICENSE OFL.txt THIRD-PARTY-LICENSES.txt"
# macOS layout.
app=$prefix/Jellyui.app

# Only refresh an icon cache that already exists: creating one in a user's
# hicolor directory hides icons other apps add there later.
refresh_caches() {
    if [ -f "$icons/icon-theme.cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t "$icons" 2>/dev/null || true
    fi
    # KDE apps (the Plasma menu included) cache icon lookups until told to reload.
    if command -v dbus-send >/dev/null 2>&1; then
        dbus-send --session --type=signal /KIconLoader org.kde.KIconLoader.iconChanged int32:0 2>/dev/null || true
    fi
}

if [ "$uninstall" = 1 ]; then
    if [ "$os" = Darwin ]; then
        rm -rf "$app"
    else
        rm -f "$bin" "$desktop" "$icon"
        for f in $doc_files; do
            rm -f "$docs/$f"
        done
        rmdir "$docs" 2>/dev/null || true
        refresh_caches
    fi
    echo "Removed jellyui from $prefix. Settings in $settings are kept."
    exit 0
fi

if [ "$os" = Darwin ]; then
    # uname -m says x86_64 in a shell running under Rosetta, so ask the hardware.
    [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ] ||
        die "only Apple Silicon builds are published; build from source on Intel Macs"
else
    [ "$(uname -m)" = x86_64 ] ||
        die "only x86_64 builds are published; build from source on $(uname -m)"
fi

tmp=$(mktemp -d)
mnt=
cleanup() {
    if [ -n "$mnt" ]; then
        hdiutil detach -quiet "$mnt" 2>/dev/null || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1"
    else
        shasum -a 256 "$1"
    fi | cut -d' ' -f1
}

if [ -z "$archive" ]; then
    command -v curl >/dev/null 2>&1 || die "curl is required"
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
    name=jellyui-$version-$asset
    base=https://github.com/$repo/releases/download/$version
    echo "Downloading jellyui $version"
    curl -fL# --proto '=https' --tlsv1.2 -o "$tmp/$name" "$base/$name" ||
        die "could not download $base/$name"
    curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/checksums.txt" "$base/checksums.txt" ||
        die "could not download $base/checksums.txt"
    expected=$(awk -v name="$name" '$2 == name { print $1 }' "$tmp/checksums.txt")
    [ -n "$expected" ] && [ "$(sha256 "$tmp/$name")" = "$expected" ] ||
        die "checksum mismatch for $name"
    archive=$tmp/$name
fi

if [ "$os" = Darwin ]; then
    mkdir "$tmp/mnt"
    hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$tmp/mnt" "$archive" >/dev/null ||
        die "could not open $archive"
    mnt=$tmp/mnt
    [ -d "$mnt/Jellyui.app" ] || die "$archive has no Jellyui.app"
    mkdir -p "$prefix"
    rm -rf "$app"
    ditto "$mnt/Jellyui.app" "$app"
    # curl doesn't quarantine what it downloads, but a browser download passed
    # with --archive is, and Gatekeeper blocks the unnotarized app until cleared.
    xattr -dr com.apple.quarantine "$app" 2>/dev/null || true
    echo "Installed Jellyui to $app"
    exit 0
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
