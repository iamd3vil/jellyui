# Jellyui

A fast, native [Jellyfin](https://jellyfin.org) client written in Rust.
It renders with [GPUI](https://www.gpui.rs/) (the UI framework behind Zed), uses
editable [gpuicn](https://ui.jha.sh) components for a shadcn-style look, and plays
video inside the window through an embedded [libmpv](https://mpv.io) core.

## Features

- **Multiple servers, multiple profiles.** Save any number of Jellyfin servers and any
  number of user profiles per server. Switch between them from the sidebar footer.
- **Browse.** Home with Continue Watching, Next Up and Latest per library. Library
  grids with sorting and paging. Series → seasons → episodes. Search across movies,
  shows and episodes.
- **Embedded playback.** Direct-play through libmpv rendered inside the app: resume
  from where you left off, seek bar, pause, ±10/30s, audio and subtitle track menus
  (embedded tracks), fullscreen, keyboard shortcuts. Playback progress is reported back
  to Jellyfin so other clients pick up where you stopped.
- **Leave the player running.** Escape returns to the library while the video keeps
  playing; a now-playing bar brings it back.
- **Dark and light appearance**, Geist typography, GPU-rendered UI.

## Install

Download the archive for your platform from
[Releases](https://github.com/iamd3vil/jellyui/releases), extract it and run `jellyui`.
libmpv and its media libraries are linked into the binary, so nothing else needs to be
installed.

- **Linux x86_64**: glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later).
  It also loads libasound, libpulse, libxcb and libxkbcommon, which desktop installs
  already have.
- **macOS Apple Silicon**: macOS 11 or newer. The binary is not signed, so clear the
  quarantine flag once: `xattr -d com.apple.quarantine jellyui`.

Verify a download with `sha256sum --check --ignore-missing checksums.txt`.

## Building from source

| Dependency | Notes |
| --- | --- |
| Rust 1.97+ | `rustup update stable` |
| Xcode command line tools | macOS builds need the Metal toolchain |
| libmpv 2.x | `brew install mpv`, your distro's `mpv`/`libmpv` package, or the Nix `mpv` package |
| ffmpeg (optional) | Only for the playback test; it encodes a short clip |
| [just](https://github.com/casey/just) (optional) | Task runner for the recipes below |

Runs on macOS and Linux. Windows is supported by GPUI, gpui-kit and libmpv, but this
app has not been run there yet.

## Quick start

```sh
just run            # or: cargo run
```

On first launch, add your server address (for example `https://jelly.example.com` or
`192.168.1.10:8096`), then sign in. The users the server publishes on its login screen
appear as tiles; click one to prefill the form. Signed-in profiles are saved and open
with a single click next time.

### Where things are stored

Servers, profiles and access tokens live in `jellyui/config.json` under the platform
config directory (`~/Library/Application Support` on macOS, `~/.config` on Linux),
written with owner-only permissions. Use `just reset-config` to forget everything.

### Finding libmpv

`build.rs` looks for libmpv next to the `mpv` binary on your `PATH`, then via
`pkg-config`, then in the usual library directories. If linking fails, point at it
explicitly:

```sh
MPV_LIB_DIR=/opt/homebrew/lib cargo build
```

## Keyboard shortcuts (player)

| Key | Action |
| --- | --- |
| `space`, `k` | Play / pause |
| `←` / `→` | Seek −5s / +5s |
| `j` / `l` | Seek −10s / +10s |
| `f` | Toggle fullscreen |
| `esc` | Leave fullscreen, or return to the library while playback continues |

## Development

```sh
just            # list recipes
just check      # fmt --check, clippy, tests
just test       # unit tests + a real libmpv playback round-trip
just lint
just fmt
```

### Releasing

Release archives are built by [rlsr](https://rlsr.sarat.dev) from `rlsr.yml`, linking
the static libmpv that `scripts/build-mpv.sh` compiles into `target/mpv-static`.
Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds Linux and macOS on
their own runners and publishes a GitHub release. Running the workflow by hand builds
snapshot archives without publishing.

To try a release build locally (the first `just mpv-static` takes a while):

```sh
just mpv-static
just release-snapshot
```

### Layout

| Path | Purpose |
| --- | --- |
| `src/main.rs` | Window setup, fonts, theme bootstrap |
| `src/app.rs` | Root view: sessions, navigation, background fetching, player polling |
| `src/views/` | Screens: connect, shell (sidebar, top bar, now-playing), home, library, detail, player, shared cards |
| `src/jellyfin.rs` | Blocking Jellyfin REST client and item model |
| `src/player.rs` | libmpv worker thread, software frame rendering, Jellyfin playback reporting |
| `src/images.rs` | Remote artwork cache |
| `src/config.rs` | Persisted servers, profiles and preferences |
| `src/ui/` | gpuicn components installed as editable source (see below) |
| `build.rs` | Locates libmpv for linking, or links the static build |
| `scripts/build-mpv.sh` | Builds libmpv and its dependencies as static libraries for releases |
| `rlsr.yml`, `.github/workflows/release.yml` | Release packaging and CI |

### How playback works

`src/player.rs` owns one libmpv core on a worker thread. Video is rendered with mpv's
software render API into BGRA frames sized to the visible video area (never larger
than the source), and each frame is handed to GPUI as an image; the previous GPU
texture is released as the next one arrives. This keeps the player fully inside the
GPUI window and works on every platform libmpv supports, at the cost of one CPU copy
per frame. mpv handles demuxing, decoding (hardware where available), subtitles and
audio, so anything mpv plays, Jellyui plays.

Progress is posted to Jellyfin on start, every ten seconds while playing, and on stop.

### UI components

The files in `src/ui` come from the gpuicn registry and belong to this app: edit them
freely. `gpuicn.toml` pins the registry snapshot. To pull updated components:

```sh
just ui-cli       # install the pinned gpuicn CLI once
just ui-diff      # see which files an update would replace
just ui-update -- --overwrite
```

gpuicn pins `gpui-kit 0.6.1` and the `gpui-pre 0.3.4` runtime, so keep those versions
in `Cargo.toml` aligned with the registry you install from.

## Known limitations

- Only subtitle tracks embedded in the media file are offered; external subtitle files
  managed by Jellyfin are not loaded yet.
- Music, books, live TV and photo libraries are hidden from the sidebar.
- Transcoding is not requested; playback relies on mpv's broad direct-play support.
- Linux release builds decode video in software; build from source against a system
  libmpv for VA-API hardware decoding.

## License

Jellyui is free software under the [GNU Affero General Public License v3.0 or later](LICENSE).

Third-party components keep their own licenses: the bundled Geist fonts are under the
SIL Open Font License (`assets/fonts/OFL.txt`), the gpuicn components in `src/ui` are
MIT licensed. Source builds link the system libmpv dynamically. Release binaries link
libmpv, FFmpeg, libplacebo, libass, dav1d, Mbed TLS, FreeType, HarfBuzz, FriBidi and (on
Linux) Fontconfig and Expat statically; their licenses ship in
`THIRD-PARTY-LICENSES.txt` inside each archive.
