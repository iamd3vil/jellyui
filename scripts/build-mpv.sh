#!/usr/bin/env bash
# Copyright (C) 2026 Sarat Chandra
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Builds libmpv and its media stack as static libraries for release builds,
# so the shipped binary needs only system libraries (glibc, xcb, xkbcommon,
# ALSA/PulseAudio on Linux; system frameworks on macOS).
#
# Usage: scripts/build-mpv.sh [prefix]   (default: target/mpv-static)
# Then build with MPV_STATIC_DIR=<prefix>; see build.rs.
#
# Needs: a C/C++ toolchain, git, pkg-config, cmake, meson >= 1.3, ninja, nasm,
# and on Linux gperf plus the zlib, ALSA and PulseAudio development packages.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "${1:-$root/target/mpv-static}"
prefix=$(cd "${1:-$root/target/mpv-static}" && pwd)
src="$prefix-src"
os=$(uname -s)
jobs=$(getconf _NPROCESSORS_ONLN)

mkdir -p "$src"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$PKG_CONFIG_PATH:$(pkg-config --variable pc_path pkg-config)"
export CFLAGS="${CFLAGS:-} -O2 -fPIC" CXXFLAGS="${CXXFLAGS:-} -O2 -fPIC"
licenses="$prefix/THIRD-PARTY-LICENSES.txt"
: >"$licenses"

fetch() { # name url tag
    [ -d "$src/$1" ] || git -c advice.detachedHead=false clone -q --depth 1 --recurse-submodules --shallow-submodules \
        --branch "$3" "$2" "$src/$1"
    for f in "$src/$1"/{LICENSE,COPYING,Copyright,LICENSE.md,LICENSE.TXT,COPYING.LGPLv3,LICENSE.LGPL,LICENSE.GPL,docs/FTL.TXT,expat/COPYING}; do
        [ -f "$f" ] && printf '\n==== %s %s: %s ====\n\n' "$1" "$3" "${f#"$src/$1/"}" >>"$licenses" && cat "$f" >>"$licenses"
    done
    return 0
}

meson_build() { # name [meson options...]
    local name=$1
    shift
    local reconf=()
    [ -d "$src/$name/build" ] && reconf=(--reconfigure)
    quiet meson setup ${reconf[@]+"${reconf[@]}"} "$src/$name/build" "$src/$name" --prefix "$prefix" --libdir lib \
        --buildtype release --default-library static -Db_staticpic=true "$@"
    quiet ninja -C "$src/$name/build"
    # Only libraries, headers and .pc files; fontconfig would also write into /etc.
    quiet meson install -C "$src/$name/build" --no-rebuild --tags devel
}

cmake_build() { # name [cmake options...]
    local name=$1
    shift
    quiet cmake -S "$src/$name" -B "$src/$name/build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib \
        -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF "$@"
    quiet ninja -C "$src/$name/build" install
}

step() { echo "==> $1"; }

# Runs a command with its output in $src/build.log, shown only on failure.
quiet() {
    "$@" >"$src/build.log" 2>&1 || {
        tail -n 50 "$src/build.log"
        return 1
    }
}

step freetype
fetch freetype https://gitlab.freedesktop.org/freetype/freetype VER-2-14-3
meson_build freetype -Dbrotli=disabled -Dbzip2=disabled -Dharfbuzz=disabled \
    -Dpng=disabled -Dzlib=disabled -Dtests=disabled

step fribidi
fetch fribidi https://github.com/fribidi/fribidi v1.0.17
meson_build fribidi -Ddocs=false -Dbin=false -Dtests=false

step harfbuzz
fetch harfbuzz https://github.com/harfbuzz/harfbuzz 14.6.0
meson_build harfbuzz -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled \
    -Dchafa=disabled -Dicu=disabled -Dtests=disabled -Ddocs=disabled -Dutilities=disabled \
    -Dintrospection=disabled

ass_fonts=(-Dfontconfig=disabled -Dcoretext=enabled)
if [ "$os" = Linux ]; then
    step expat
    fetch expat https://github.com/libexpat/libexpat R_2_9_0
    cmake_build expat/expat -DEXPAT_BUILD_TOOLS=OFF -DEXPAT_BUILD_EXAMPLES=OFF \
        -DEXPAT_BUILD_TESTS=OFF -DEXPAT_BUILD_DOCS=OFF

    # Reads the system's /etc/fonts configuration and font cache at runtime.
    step fontconfig
    fetch fontconfig https://gitlab.freedesktop.org/fontconfig/fontconfig 2.18.3
    meson_build fontconfig --sysconfdir /etc --localstatedir /var -Ddoc=disabled \
        -Dnls=disabled -Dtests=disabled -Dtools=disabled -Dcache-build=disabled \
        -Dxml-backend=expat -Dfontations=disabled
    ass_fonts=(-Dfontconfig=enabled -Dcoretext=disabled)
fi

step libass
fetch libass https://github.com/libass/libass 0.17.5
meson_build libass "${ass_fonts[@]}" -Dlibunibreak=disabled -Drequire-system-font-provider=false \
    -Dtest=disabled -Dcompare=disabled -Dprofile=disabled -Dfuzz=disabled -Dcheckasm=disabled

step dav1d
fetch dav1d https://code.videolan.org/videolan/dav1d 1.5.4
meson_build dav1d -Denable_tools=false -Denable_tests=false -Denable_examples=false

# TLS for https:// Jellyfin streams.
step mbedtls
fetch mbedtls https://github.com/Mbed-TLS/mbedtls v3.6.7
cmake_build mbedtls -DENABLE_PROGRAMS=OFF -DENABLE_TESTING=OFF -DMBEDTLS_FATAL_WARNINGS=OFF

step ffmpeg
fetch ffmpeg https://github.com/FFmpeg/FFmpeg n8.1.3
ffmpeg_platform=(--enable-zlib)
[ "$os" = Darwin ] && ffmpeg_platform+=(--enable-videotoolbox --enable-audiotoolbox)
(
    cd "$src/ffmpeg"
    quiet ./configure --prefix="$prefix" --pkg-config-flags=--static --enable-static --disable-shared \
        --enable-pic --disable-debug --disable-doc --disable-programs --disable-autodetect \
        --enable-version3 --enable-mbedtls --enable-libdav1d --disable-encoders --disable-muxers \
        --disable-indevs --disable-outdevs "${ffmpeg_platform[@]}"
    quiet make -j"$jobs" install
)

step libplacebo
fetch libplacebo https://code.videolan.org/videolan/libplacebo v7.360.1
meson_build libplacebo -Dvulkan=disabled -Dopengl=disabled -Dd3d11=disabled -Dglslang=disabled \
    -Dshaderc=disabled -Dlcms=disabled -Ddovi=disabled -Dlibdovi=disabled -Dunwind=disabled \
    -Dxxhash=disabled -Ddemos=false -Dtests=false

# Only what the embedded software renderer needs: decoding, subtitles, audio out.
step mpv
fetch mpv https://github.com/mpv-player/mpv v0.41.0
mpv_platform=(-Dalsa=enabled -Dpulse=enabled)
# coreaudio needs osdep/utils-mac.c, which mpv only builds with cocoa.
[ "$os" = Darwin ] && mpv_platform=(-Dcoreaudio=enabled -Dcocoa=enabled -Dswift-build=disabled)
meson_build mpv -Dauto_features=disabled -Dlibmpv=true -Dcplayer=false -Dtests=false -Dgl=disabled \
    -Dbuild-date=false -Dzlib=enabled -Diconv=enabled "${mpv_platform[@]}"

echo "libmpv installed to $prefix"
