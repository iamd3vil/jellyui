# Changelog

## 0.2.0 - 2026-10-17

- Up next: the next episode is offered in the corner for the last 20 seconds,
  and the player stays on it when an episode finishes instead of closing.
- Next episode button on episode pages.
- Fixed a crash when clicking the seek bar during playback.
- Fixed starting another item while one is playing: the old one now keeps its
  watched progress, and the new one no longer closes the player or starts paused.

## 0.1.0 - 2026-10-17

First release.

- Multiple Jellyfin servers and user profiles, switchable from the sidebar.
- Home (Continue Watching, Next Up, Latest), library grids with sorting and paging,
  series → seasons → episodes, and search.
- Embedded libmpv playback inside the window: resume, seek bar, audio and subtitle
  track menus, fullscreen, keyboard shortcuts, and auto-hiding controls.
- Playback progress and watched state reported back to Jellyfin.
- Playback keeps running when you return to the library, with a now-playing bar.
- Keeps the screen awake during playback (macOS and Linux).
- Dark and light appearance.
- Prebuilt binaries for Linux x86_64 and macOS Apple Silicon with libmpv linked in.
