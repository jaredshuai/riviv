# riviv

Unofficial Rust rewrite of [voidtools/voidImageViewer](https://github.com/voidtools/voidImageViewer) (MIT) — a lightweight, single-executable image viewer for Windows.

Based on voidImageViewer by David Carpenter / voidtools. See [LICENSE](LICENSE). The original C implementation is preserved under [`c-original/`](c-original/) as a read-only behavioral reference.

> **Status: early development (M3 in progress).** Current scope: Win32 window + GDI rendering, animated GIF/WebP playback at author timing, alpha-composited transparency for every supported format (PNG, JPEG, BMP, ICO, TIFF, GIF and WebP), drag & drop, a keyboard-navigable playlist, zoom/pan over the upstream 16-level preset curve, the settings foundation (the window rect is remembered across runs in a `[riviv]` ini with upstream's 60% first-run auto-fit), and en/zh-CN localization driven by the system UI language. Remaining M3: menu, options dialog, custom shortcuts, single instance, Everything IPC, file associations, installer — see [Roadmap](#roadmap).

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

Fullscreen (upstream semantics): double-click or Alt+Enter toggles a borderless cover of the current monitor (Esc also leaves it) — the pre-toggle window rect (and its maximized state) is restored on exit, the status bar is hidden for the cover, and an idle cursor hides after 2 s, reappearing on any movement.

Giant images (upstream semantics): panoramas ≥ 32768 px render through 512-px stitched stretches during mipmap generation (`_viv_StretchBltStitch`), and zoomed-out repaints blit from the cached mipmap level selected for the render size (`_viv_get_mipmap`), pre-generated on the decode thread against the request-time viewport and extended lazily at paint time.

## Roadmap

- [x] M1 — skeleton: Win32 window + GDI rendering + static image display
- [x] M2 — animated GIF/WebP, playlist, zoom/pan, background decoding
  - [x] animation + transparency compositing
  - [x] playlist + keyboard navigation
  - [x] zoom & pan: 16-level presets + wheel + drag + temporary 1:1
  - [x] fullscreen: double-click / Alt+Enter / Esc + idle cursor hide
  - [x] ≥32768-px giant images: stitched mip generation + per-zoom-level mipmap cache
- [ ] M3 — settings & custom shortcuts, Everything IPC, file associations, localization, installer
  - [x] config foundation: `[riviv]` ini read/write + remembered window rect + 60% first-run auto-fit (#19)
  - [x] localization: en/zh-CN string tables + system language detection (#20)
  - [x] single-instance command-line forwarding (#21)
  - [ ] Everything IPC search (#22)
  - [ ] menu bar + command table (#23)
  - [ ] options dialog (General/View/Controls) (#24)
  - [ ] custom shortcuts (#25)
  - [ ] file associations + NSIS installer (#26)

## Differences from upstream (intentional)

- The settings live in a `riviv.ini` with a `[riviv]` section next to the exe (or `%APPDATA%\riviv\` once switched) — upstream's `voidImageViewer.ini`; the namespaces are separate so both viewers coexist on one machine. The first run centers a 60%-of-monitor window on the cursor's monitor in that monitor's own coordinate frame, exactly like upstream (including the secondary-monitor quirk where the relative rect is re-anchored onto the window's monitor — viv.c:5383-5406); later runs restore the remembered rect/maximized state, saved on exit.
- Images are never upscaled beyond 100% (upstream default `fill_window=0`).
- The window never resizes when switching images via drag & drop (upstream behavior).
- Default window icon for now (upstream ships its own icon).
- Animated WebP frames shorter than 10 ms play quantized to the `USER_TIMER_MINIMUM` timer period — a two-frame 5 ms animation advances two frames per tick and can appear frozen. Upstream's primary path additionally drives a 1 ms timer-queue timer (`CreateTimerQueueTimer`, viv.c:9132-9141) for those; GIF delays are 10 ms multiples and never hit this.
- A user-level load failure (bad path, undecodable file, decode-budget overflow) keeps the old image and title untouched — nothing seems to happen (no popup, no exit). Once a new image's first frame is already on screen, a later failure of that same load (e.g. a budget overflow mid-animation) cannot roll the old image back and clears to a blank window instead, like upstream's async FAILED handler (viv.c:2832-2840). Upstream blanks in both cases. The same family covers navigating to a file deleted after the playlist was built: riviv's pre-open check shows "File not found." (upstream would show "Failed to load image." after blanking).
- Mipmap-chain failures degrade to a shallower level (or the original) and keep rendering; upstream propagates a `NULL` level into not drawing the image at all (viv.c:14226→4167-4169). Mip objects (level DDBs + paint scratch DCs) are capped by a process-wide GDI-object budget that covers worker pre-generation AND paint-time lazy fills (upstream has no such cap — riviv's fail-loud wrap of a base-frame DC makes the 10000-object process quota a crash otherwise). Per-tile StretchBlt failures inside a level keep stitching the remaining tiles (upstream aborts the rest of the chain's tiles on the first failure, viv.c:14999-15002), and degenerate zero-area tiles are skipped rather than handed to GDI.
- A giant-extent HALFTONE shrink (dest or selected level ≥ 32768 px) wraps the full-rect StretchBlt in a single simple clip region = the update paint rect intersected with the viewport. Upstream iterates the update region's rects one by one (viv.c:4262-4284); riviv's coarser clip redraws the update bounding box on every WM_PAINT instead of each exact dirty rect. Non-giant shrinks keep the #7 single full-rect blt (pixel-identical to upstream's per-rect loop, performance-only difference on complex update regions).
- Magnified rendering re-anchors the source sub-rect (`clip_blit`) instead of upstream's full-rect-accurate `_viv_stretch_blt`/stitched stretch — up to ±1 source pixel of sampling phase on very high zooms, and the visible region is re-stretched every paint rather than only the dirty rect's source area (inherited from the #7 zoom work).
- The status bar is a simplified form of upstream's: it keeps the main text (Loading / File not found. / Failed to load image.), the frame counter, and the `W x H (N KB)` dimension parts, but drops upstream's PRELOAD / pixel POS / RGB parts (riviv has neither feature yet), the temp-text line (upstream's position/zoom readout is tied to the panscan/pixel-info features), and the click-to-toggle-frames-remaining behavior. The frame counter's `m` counts the *loaded* prefix and grows while an animation streams in — image frame iterators cannot report the total up front (GDI+/libwebp can).
- No toolbar yet (upstream shows it by default) — not in M3's scope either; deferred.
- Embedded ICC color profiles are not applied (upstream enables GDI+ ICM); non-sRGB images may show slightly inaccurate colors.
- The single-instance handoff lives in riviv's own namespace: mutex `RIVIV` + window class `riviv` (upstream: `VOIDIMAGEVIEWER`), so a riviv launch never hands its command line to a running upstream viewer — both coexist. `multiple_instances=1` in the ini skips the mutex entirely and every launch opens its own window, like upstream.
- The cursor does not hide while idle in WINDOWED mode (upstream defaults `windowed_hide_cursor=1`, hiding it there too; riviv hides it in fullscreen only, per issue #8's scope). The fullscreen background also stays the windowed background color — upstream separates `fullscreen_background_color`.
- The fullscreen toggle's zoom-offset is implemented but inert: both `fill_window` and `fullscreen_fill_window` are off (no config yet), so the zoom level is preserved entering/leaving fullscreen. Upstream defaults `fullscreen_fill_window=1`, which drops to the largest level that still covers the monitor on entry.
- No menu bar yet (upstream shows File/View/Navigate by default) — planned for M3 (#23).
- Command-line switches are ignored (upstream parses config switches like `/sort` and install switches like `-install`, and shows a usage dialog for unknown ones; riviv parses neither yet — planned with #26). Switch detection matches upstream's quirk of treating dotted words like `-foo.png` as filenames; quoted switches cannot be distinguished from unquoted ones through `args_os` and are skipped either way.
- Upstream's default-on decode-ahead (preload next image) and last-image caches are not implemented — every open, including navigation back to a just-seen image, decodes from disk.
- Navigation always navigates by the default upstream sort (date modified, newest first). The sort-mode/ascending menu options and shuffle are M3 config work.
- The NUMPAD panscan commands are not implemented (size/width/height steps and move/center, upstream `VIV_ID_VIEW_PANSCAN_*`): zoom/pan uses the 16-level preset curve, the wheel and left-drag only. Middle-button drag-to-scroll (upstream `_VIV_DOING_MSCROLL`) is not implemented either.
- The mouse back/forward buttons (X buttons) do not navigate yet — upstream maps them to next/prev by default (`config_xbutton_action = 2`); lands with the slideshow/input work.

Agent workflow: see [AGENTS.md](AGENTS.md). Decisions: `docs/adr/`.

## License

MIT — same as upstream. Original C implementation © voidtools / David Carpenter.
