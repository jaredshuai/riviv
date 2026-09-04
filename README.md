# riviv

Unofficial Rust rewrite of [voidtools/voidImageViewer](https://github.com/voidtools/voidImageViewer) (MIT) — a lightweight, single-executable image viewer for Windows.

Based on voidImageViewer by David Carpenter / voidtools. See [LICENSE](LICENSE). The original C implementation is preserved under [`c-original/`](c-original/) as a read-only behavioral reference.

> **Status: early development (M2 in progress).** Current scope: Win32 window + GDI rendering, animated GIF/WebP playback at author timing, and alpha-composited transparency (PNG / JPEG / BMP / ICO / TIFF included) with drag & drop. Playlist, zoom/pan and settings land in M2/M3 — see [Roadmap](#roadmap).

## Build

Requires the Rust toolchain with the MSVC target.

```text
cargo build --release
```

## Usage

```text
riviv.exe <image path>    open an image
riviv.exe                 empty window; press Ctrl+O to pick a file
```

Drag & drop a file onto the window to switch images (a single dropped file replaces the current image, matching upstream behavior).

## Roadmap

- [x] M1 — skeleton: Win32 window + GDI rendering + static image display
- [ ] M2 — animated GIF/WebP, playlist, zoom/pan, background decoding
  - [x] animation + transparency compositing
- [ ] M3 — settings & custom shortcuts, Everything IPC, file associations, localization, installer

## Differences from upstream (intentional)

- The window opens at image size (upstream: remembered window rect, or 60% auto-fit on first run — restored in M3 with config persistence).
- Images are never upscaled beyond 100% (upstream default `fill_window=0`).
- The window never resizes when switching images via drag & drop (upstream behavior).
- Default window icon for now (upstream ships its own icon).
- Large images decode synchronously on the UI thread — the window may briefly freeze while opening them; background decoding lands in M2.
- Images with a source dimension ≥ 32768 px are not rendered until M2 (upstream stitches tiled stretches, viv.c `_viv_StretchBltStitch`); downscaled repainting of large images is not mip-cached until M2 either (upstream `_viv_get_mipmap`).
- No status bar / toolbar yet (upstream shows them by default) — the status bar lands in M2 (it carries the "Failed to load image." text).
- Embedded ICC color profiles are not applied (upstream enables GDI+ ICM); non-sRGB images may show slightly inaccurate colors.
- No single-instance handoff yet: a second launch opens a new window instead of forwarding its command line to the existing viewer (upstream default). Planned for M3 together with Everything IPC.
- Double-click does not toggle fullscreen yet (upstream default action) — lands with the fullscreen work in M2.
- Dropping multiple files (or Shift-dropping) only opens the first file; playlist-building drops land in M2.
- No menu bar yet (upstream shows File/View/Navigate by default) — planned for M2+.

Agent workflow: see [AGENTS.md](AGENTS.md). Decisions: `docs/adr/`.

## License

MIT — same as upstream. Original C implementation © voidtools / David Carpenter.
