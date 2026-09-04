# riviv

Unofficial Rust rewrite of [voidtools/voidImageViewer](https://github.com/voidtools/voidImageViewer) (MIT) — a lightweight, single-executable image viewer for Windows.

Based on voidImageViewer by David Carpenter / voidtools. See [LICENSE](LICENSE). The original C implementation is preserved under [`c-original/`](c-original/) as a read-only behavioral reference.

> **Status: early development (M1 skeleton).** Current scope: Win32 window + GDI rendering + static image display (PNG / JPEG / BMP / ICO / TIFF / GIF / WebP first frame) with drag & drop. Animation, playlist, zoom/pan and settings land in M2/M3 — see [Roadmap](#roadmap).

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
- [ ] M3 — settings & custom shortcuts, Everything IPC, file associations, localization, installer

## Differences from upstream (intentional, M1)

- The window opens at image size (upstream: remembered window rect, or 60% auto-fit on first run — restored in M3 with config persistence).
- Images are never upscaled beyond 100% (upstream default `fill_window=0`).
- The window never resizes when switching images via drag & drop (upstream behavior).
- Default window icon for now (upstream ships its own icon).
- Only the first frame of animated images is shown until M2.
- Transparent areas of PNG/WebP are drawn with their raw RGB values (no alpha compositing) until M2.
- Large images decode synchronously on the UI thread — the window may briefly freeze while opening them; background decoding lands in M2.

Agent workflow: see [AGENTS.md](AGENTS.md). Decisions: `docs/adr/`.

## License

MIT — same as upstream. Original C implementation © voidtools / David Carpenter.
