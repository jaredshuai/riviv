# riviv

Unofficial Rust rewrite of [voidtools/voidImageViewer](https://github.com/voidtools/voidImageViewer) (MIT) — a lightweight, single-executable image viewer for Windows.

Based on voidImageViewer by David Carpenter / voidtools. See [LICENSE](LICENSE). The original C implementation is preserved under [`c-original/`](c-original/) as a read-only behavioral reference.

> **Status: early development (M2 in progress).** Current scope: Win32 window + GDI rendering, animated GIF/WebP playback at author timing, alpha-composited transparency for every supported format (PNG, JPEG, BMP, ICO, TIFF, GIF and WebP), drag & drop, a keyboard-navigable playlist, and zoom/pan over the upstream 16-level preset curve. Settings land in M3 — see [Roadmap](#roadmap).

## Build

Requires the Rust toolchain with the MSVC target.

```text
cargo build --release
```

## Usage

```text
riviv.exe <image path>    open an image
riviv.exe <folder>        build a playlist from the folder (recursive) and open the newest image
riviv.exe <a.png> <b.png> build a playlist from the arguments and open the first
riviv.exe                 empty window; press Ctrl+O to pick a file
```

Drag & drop works the same way (upstream `WM_DROPFILES` semantics): a single dropped file replaces the current image; dropping a folder, multiple files, or holding Shift builds/extends the playlist (Shift appends instead of replacing). Navigate with Right/PgDn (next), Left/PgUp (previous), Home/End (first/last) — by the upstream default sort: date modified, newest first. With no playlist, navigation walks the current image's folder.

Zoom & pan (upstream preset semantics): the mouse wheel and `+`/`-` step through the 16-level zoom curve (level 0 = fit, top level = 1600%) anchored at the cursor; drag with the left button to pan while the image exceeds the window; Ctrl+0 returns to fit; Ctrl+Alt+0 toggles a temporary pixel-exact 1:1 view.

## Roadmap

- [x] M1 — skeleton: Win32 window + GDI rendering + static image display
- [ ] M2 — animated GIF/WebP, playlist, zoom/pan, background decoding
  - [x] animation + transparency compositing
  - [x] playlist + keyboard navigation
  - [x] zoom & pan: 16-level presets + wheel + drag + temporary 1:1
- [ ] M3 — settings & custom shortcuts, Everything IPC, file associations, localization, installer

## Differences from upstream (intentional)

- The window opens at the default size and resizes to the image when the startup image's first frame arrives (upstream: remembered window rect, or 60% auto-fit on first run — restored in M3 with config persistence).
- Images are never upscaled beyond 100% (upstream default `fill_window=0`).
- The window never resizes when switching images via drag & drop (upstream behavior).
- Default window icon for now (upstream ships its own icon).
- Animated WebP frames shorter than 10 ms play quantized to the `USER_TIMER_MINIMUM` timer period — a two-frame 5 ms animation advances two frames per tick and can appear frozen. Upstream's primary path additionally drives a 1 ms timer-queue timer (`CreateTimerQueueTimer`, viv.c:9132-9141) for those; GIF delays are 10 ms multiples and never hit this.
- A user-level load failure (bad path, undecodable file, decode-budget overflow) keeps the old image and title untouched — nothing seems to happen (no popup, no exit). Once a new image's first frame is already on screen, a later failure of that same load (e.g. a budget overflow mid-animation) cannot roll the old image back and clears to a blank window instead, like upstream's async FAILED handler (viv.c:2832-2840). Upstream blanks in both cases. The same family covers navigating to a file deleted after the playlist was built: riviv's pre-open check shows "File not found." (upstream would show "Failed to load image." after blanking).
- Images with a source dimension ≥ 32768 px are not rendered until M2 (upstream stitches tiled stretches, viv.c `_viv_StretchBltStitch`); downscaled repainting of large images is not mip-cached until M2 either (upstream `_viv_get_mipmap`).
- The status bar is a simplified form of upstream's: it keeps the main text (Loading / File not found. / Failed to load image.), the frame counter, and the `W x H (N KB)` dimension parts, but drops upstream's PRELOAD / pixel POS / RGB parts (riviv has neither feature yet), the temp-text line (upstream's position/zoom readout is tied to the panscan/pixel-info features), and the click-to-toggle-frames-remaining behavior. The frame counter's `m` counts the *loaded* prefix and grows while an animation streams in — image frame iterators cannot report the total up front (GDI+/libwebp can).
- No toolbar yet (upstream shows it by default) — lands in M2.
- Embedded ICC color profiles are not applied (upstream enables GDI+ ICM); non-sRGB images may show slightly inaccurate colors.
- No single-instance handoff yet: a second launch opens a new window instead of forwarding its command line to the existing viewer (upstream default). Planned for M3 together with Everything IPC.
- Double-click does not toggle fullscreen yet (upstream default action) — lands with the fullscreen work in M2.
- No menu bar yet (upstream shows File/View/Navigate by default) — planned for M2+.
- Command-line switches are ignored (upstream parses config switches like `/sort` and shows a usage dialog for unknown ones; riviv has no config yet — M3). Switch detection matches upstream's quirk of treating dotted words like `-foo.png` as filenames; quoted switches cannot be distinguished from unquoted ones through `args_os` and are skipped either way.
- Upstream's default-on decode-ahead (preload next image) and last-image caches are not implemented — every open, including navigation back to a just-seen image, decodes from disk.
- Navigation always navigates by the default upstream sort (date modified, newest first). The sort-mode/ascending menu options and shuffle are M3 config work.
- The NUMPAD panscan commands are not implemented (size/width/height steps and move/center, upstream `VIV_ID_VIEW_PANSCAN_*`): zoom/pan uses the 16-level preset curve, the wheel and left-drag only. Middle-button drag-to-scroll (upstream `_VIV_DOING_MSCROLL`) is not implemented either.

Agent workflow: see [AGENTS.md](AGENTS.md). Decisions: `docs/adr/`.

## License

MIT — same as upstream. Original C implementation © voidtools / David Carpenter.
