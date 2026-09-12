# riviv

Unofficial Rust rewrite of [voidtools/voidImageViewer](https://github.com/voidtools/voidImageViewer) (MIT) — a lightweight, single-executable image viewer for Windows.

Based on voidImageViewer by David Carpenter / voidtools. See [LICENSE](LICENSE). The original C implementation is preserved under [`c-original/`](c-original/) as a read-only behavioral reference.

> **Status: early development (M4 planned).** Current scope: Win32 window + GDI rendering, animated GIF/WebP playback at author timing, alpha-composited transparency for every supported format (PNG, JPEG, BMP, ICO, TIFF, GIF and WebP), drag & drop, a keyboard-navigable playlist, zoom/pan over the upstream 16-level preset curve, the settings foundation (the window rect is remembered across runs in a `[riviv]` ini with upstream's 60% first-run auto-fit), en/zh-CN localization driven by the system UI language, single-instance command-line forwarding (a second launch hands its image to the running viewer and exits), a menu bar over a command table (File/View/Navigate/Help carrying the implemented commands, toggled by `show_menu`), the Options dialog (General/View/Controls pages over the same ini — filters, fit/fill, auto-size, background colors, frame-minus, mouse actions, multiple instances, appdata location, start-menu shortcuts and file associations; OK applies live and saves), custom keyboard shortcuts (per-command bindings over the same ini — upstream's `*_keys` lines — editable on the Controls page; the keyboard route and the menu accelerator labels follow the live table), and the install family (the `/install`-family CLI, an embedded app icon, and an NSIS installer), and the Everything IPC search (Ctrl+E / Ctrl+Shift+E open a hand-built Search Everything dialog; the query goes out as a QUERY2 WM_COPYDATA to a running Everything, the LIST2 results fill the playlist — Open replaces, Add appends — and the Randomize checkbox arms an endless one-image-at-a-time mode bound to next/prev/home), and the slideshow (#37: F11 or View→Slideshow auto-enters fullscreen and advances the playlist on a timer; the Slideshow menu carries Play/Pause, the 17 rate presets and a Custom dialog; Space/↑/↓ and Esc follow upstream). M3 is feature-complete; M4 (the remaining upstream features) is ticketed as #37–#50; see [Roadmap](#roadmap).

## Build

Requires the Rust toolchain with the MSVC target.

```text
cargo build --release
```

The NSIS installer (optional; needs NSIS 3, e.g. `winget install NSIS.NSIS`):

```text
installer/build-installer.ps1              # zh-CN installer (upstream default)
installer/build-installer.ps1 -Lang English
```

## Usage

```text
riviv.exe <image path>    open an image
riviv.exe <folder>        build a playlist from the folder (recursive) and open the newest image
riviv.exe <a.png> <b.png> build a playlist from the arguments and open the first
riviv.exe                 empty window; press Ctrl+O to pick a file
```

Install-family switches (upstream semantics; the process exits without a window after handling them): `riviv.exe /png` associates an extension (`/no<ext>` removes it), `/appdata` / `/noappdata` switch the settings location, `/startmenu` / `/nostartmenu` manage the all-users start-menu shortcuts, and `-install <dir>` / `-uninstall [dir]` (used by the installer) copy or remove the program. The elevated work re-executes itself via UAC when needed.

Drag & drop works the same way (upstream `WM_DROPFILES` semantics): a single dropped file replaces the current image; dropping a folder, multiple files, or holding Shift builds/extends the playlist (Shift appends instead of replacing). Navigate with Right/PgDn (next), Left/PgUp (previous), Home/End (first/last) — by the upstream default sort: date modified, newest first. With no playlist, navigation walks the current image's folder.

Zoom & pan (upstream preset semantics): the mouse wheel and `+`/`-` step through the 16-level zoom curve (level 0 = fit, top level = 1600%) anchored at the cursor; drag with the left button to pan while the image exceeds the window; Ctrl+0 returns to fit; Ctrl+Alt+0 toggles a temporary pixel-exact 1:1 view.

Fullscreen (upstream semantics): double-click or Alt+Enter toggles a borderless cover of the current monitor (Esc also leaves it) — the pre-toggle window rect (and its maximized state) is restored on exit, the status bar is hidden for the cover, and an idle cursor hides after 2 s, reappearing on any movement.

Slideshow (upstream semantics): F11 or View→Slideshow starts a fullscreen auto-advancing playlist run; Space toggles pause/resume, ↑/↓ step the rate through the 17-preset ladder (250 ms … 60 s), the Slideshow menu picks a preset or opens the Custom dialog (value + ms/s/min unit), manual navigation re-arms the timer, and Esc both leaves fullscreen and pauses the run. With `loop_animations_once` on (the default), the advance holds until the current animation has played through once. While the run is up the status bar's main part reads "Slideshow playing" (windowed runs only — the fullscreen cover has no status bar, and the Loading/File-not-found/Failed verdicts outrank it) and the process holds the display awake (`prevent_sleep`).

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
  - [x] Everything IPC search (#22: Ctrl+E/Ctrl+Shift+E dialog, QUERY2/LIST2 codec, random mode)
  - [x] menu bar + command table (#23)
  - [x] options dialog (General/View/Controls) (#24)
  - [x] custom shortcuts (#25)
  - [x] file associations + NSIS installer (#26: `/install`-family CLI, General-page association checkboxes, app icon, `installer/` NSIS port)
- [ ] M4 — the remaining upstream features (interaction & shell completion)
  - [x] slideshow: View→Slideshow + Slideshow menu + rate presets (#37)
  - [ ] animation controls: play/pause, jumps, frame step, rate, loop-once (#38)
  - [ ] navigation sort / shuffle / jump-to (#39)
  - [ ] preload next + last-image cache + PRELOAD status part (#40)
  - [ ] clipboard: cut / copy / copy filename / copy image / paste (#41)
  - [ ] shell verbs: print / edit / preview / location / properties / wallpaper / close (#42)
  - [ ] file management: rename / delete ×3 / rotate / copy-to / move-to (#43)
  - [ ] panscan + input completion: NUMPAD family, X buttons, middle-drag scroll (#44)
  - [ ] toolbar (#45, blocked by #37)
  - [ ] view presets + window size + always-on-top + view toggles (#46, blocked by #45)
  - [ ] status-bar POS/RGB/temp-text + title-bar format (#47)
  - [ ] config CLI second pass + usage dialog (#48, blocked by #37/#39)
  - [ ] full context menu (#49, blocked by #37/#39/#41/#42/#43)
  - [ ] ICM evaluation: embedded ICC profiles (#50)

## Differences from upstream (intentional)

- The settings live in a `riviv.ini` with a `[riviv]` section next to the exe (or `%APPDATA%\riviv\` once switched) — upstream's `voidImageViewer.ini`; the namespaces are separate so both viewers coexist on one machine. The first run centers a 60%-of-monitor window on the cursor's monitor in that monitor's own coordinate frame, exactly like upstream (including the secondary-monitor quirk where the relative rect is re-anchored onto the window's monitor — viv.c:5383-5406); later runs restore the remembered rect/maximized state, saved on exit.
- The DEFAULT fit never upscales past 100% (upstream `fill_window=0`; `keep_aspect_ratio`/`fill_window`/`fullscreen_fill_window` are configurable in Options since #24, matching upstream semantics including the fill-mode zoom ladder).
- The window never resizes when switching images via drag & drop (upstream behavior).
- File associations and the install family (#26) live in riviv's own namespace: progids `riviv.<ext>` under `HKCU\SOFTWARE\Classes` (backup value `riviv.Backup`, upstream's is `voidImageViewer.Backup`) and a `riviv` folder for the all-users start-menu shortcuts — so both viewers' associations coexist and each uninstalls only its own. Uninstall removes the progid with `RegDeleteTreeW` where upstream calls the single-level `RegDeleteKey` — which FAILS on a key with subkeys (a progid always has DefaultIcon + shell\open\command), so upstream silently leaks the whole progid tree on every uninstall; riviv cleans up fully instead (the issue's "no residue after uninstall"). The `-install` copy skips upstream's `Changes.txt` (riviv ships none). The NSIS installer (`installer/nsis/riviv.nsi`, build with `installer/build-installer.ps1`) is a port of upstream's script onto MUI2 (upstream ships MUI v1) and keeps upstream's trick of staying `RequestExecutionLevel user` — the staged exe's own `/install` pass elevates via its `runas` re-execution.
- Animated WebP frames shorter than 10 ms play quantized to the `USER_TIMER_MINIMUM` timer period — a two-frame 5 ms animation advances two frames per tick and can appear frozen. Upstream's primary path additionally drives a 1 ms timer-queue timer (`CreateTimerQueueTimer`, viv.c:9132-9141) for those; GIF delays are 10 ms multiples and never hit this.
- A user-level load failure (bad path, undecodable file, decode-budget overflow) never pops up or exits. The window title adopts the requested file at REQUEST time (upstream viv.c:1574-1578) and a failure never reverts it. Before the first frame, the old image stays on screen (upstream blanks here) while the title bar already carries the failed file's name; once the new image's first frame is already on screen, a later failure of that same load (e.g. a budget overflow mid-animation) cannot roll the old image back and clears to a blank window instead, the title keeping the failed file's name — like upstream's async FAILED handler, which clears the frame data only (viv.c:2832-2840; only blank clears the title too, viv.c:7919-7923). The same family covers navigating to a file deleted after the playlist was built: riviv's pre-open check shows "File not found." (upstream would show "Failed to load image." after blanking).
- Mipmap-chain failures degrade to a shallower level (or the original) and keep rendering; upstream propagates a `NULL` level into not drawing the image at all (viv.c:14226→4167-4169). Mip objects (level DDBs + paint scratch DCs) are capped by a process-wide GDI-object budget that covers worker pre-generation AND paint-time lazy fills (upstream has no such cap — riviv's fail-loud wrap of a base-frame DC makes the 10000-object process quota a crash otherwise). Per-tile StretchBlt failures inside a level keep stitching the remaining tiles (upstream aborts the rest of the chain's tiles on the first failure, viv.c:14999-15002), and degenerate zero-area tiles are skipped rather than handed to GDI.
- A giant-extent HALFTONE shrink (dest or selected level ≥ 32768 px) wraps the full-rect StretchBlt in a single simple clip region = the update paint rect intersected with the viewport. Upstream iterates the update region's rects one by one (viv.c:4262-4284); riviv's coarser clip redraws the update bounding box on every WM_PAINT instead of each exact dirty rect. Non-giant shrinks keep the #7 single full-rect blt (pixel-identical to upstream's per-rect loop, performance-only difference on complex update regions).
- Magnified rendering re-anchors the source sub-rect (`clip_blit`) instead of upstream's full-rect-accurate `_viv_stretch_blt`/stitched stretch — up to ±1 source pixel of sampling phase on very high zooms, and the visible region is re-stretched every paint rather than only the dirty rect's source area (inherited from the #7 zoom work).
- The status bar is a simplified form of upstream's: it keeps the main text (Loading / File not found. / Failed to load image.), the frame counter, and the `W x H (N KB)` dimension parts, but drops upstream's PRELOAD / pixel POS / RGB parts (riviv has neither feature yet), the temp-text line (upstream's position/zoom readout is tied to the panscan/pixel-info features), and the status-bar-click frames-remaining toggle (frame_minus is an Options checkbox instead, #24). The frame counter's `m` counts the *loaded* prefix and grows while an animation streams in — image frame iterators cannot report the total up front (GDI+/libwebp can).
- No toolbar yet (upstream shows it by default) — not in M3's scope either; deferred.
- Embedded ICC color profiles are not applied (upstream enables GDI+ ICM); non-sRGB images may show slightly inaccurate colors.
- The single-instance handoff lives in riviv's own namespace: mutex `RIVIV` + window class `riviv` (upstream: `VOIDIMAGEVIEWER`), so a riviv launch never hands its command line to a running upstream viewer — both coexist. `multiple_instances=1` in the ini skips the mutex entirely and every launch opens its own window, like upstream.
- The Everything IPC search (#22) keeps upstream's protocol byte-for-byte (QUERY2W over WM_COPYDATA to the `EVERYTHING_TASKBAR_NOTIFICATION` window, LIST2 replies multiplexed beside the single-instance handoff, the extension-wrapped term, the probe-driven request flags, the random offset `(r1*32767+r2)%total` over the MSVC CRT rand) with these deliberate divergences: the Search Everything dialog is hand-assembled like the Options dialog (#24's no-rc-template decision) with the initial focus set on the edit; a malformed/forged LIST2 reply drops the offending item (or ends the item walk when the headers run out) where upstream's raw pointer walk reads out of bounds, and a random retry arriving with random mode disarmed is a no-op where upstream string_cats NULL (UB); playlist mtimes from the reply convert the IPC FILETIME onto riviv's signed unix-tick epoch instead of re-statting; and the receiving window returns the DefWindowProc value for the Everything reply ids like upstream's break-to-default (the IPC documents a TRUE return, Everything does not check it). Upstream's `/everything`/`/random` CLI pair stays with the deferred config-switch family (above), and the menu rows are visible (the menu bullet above). QA note: with an ELEVATED Everything, a standard riviv's queries are blocked by UIPI and the search silently does nothing — upstream behaves identically; test at the same integrity level.
- The Options dialog (#24) is hand-assembled from CreateWindowExW rather than an rc-template DialogBox: no build-time resource compiler for dialog templates (the app icon and version info ARE embedded, via winresource writing the resource object itself — #26), dialog semantics (Tab/Enter/Esc) via a local IsDialogMessageW pump, owner-drawn color swatches instead of upstream's BS_BITMAP ones — and no EnableThemeDialogTexture page gradient (that API needs real dialog windows). Its appdata toggle applies directly in-process (the `/appdata` CLI switches exist since #26, but the toggle does not re-execute elevated like upstream, viv.c:8654-8670 — a write-protected install dir loses the exe-dir marker write, diagnosed on stderr, and the toggle reverts next launch). The General page carries the start-menu checkbox and the nine association checkboxes since #26 (associations apply directly at standard-user level like upstream, viv.c:8693-8709; the start-menu toggle re-executes the exe with `/startmenu`-family switches, elevation happening in the child); the View page omits title-bar-format/loop-animations-once/preload/cache (features not implemented) and instead carries keep-aspect/fill/fullscreen-fill/frame-minus as checkboxes — upstream exposes those through View-menu commands and a status-bar click (viv.c:2032-2060/3994-3999) that riviv does not register. frame_minus counts down against the LOADED prefix (the m= deviation above). The Controls combos list only implemented actions; an ini value outside them shows a blank combo and OK preserves it (upstream lists every action value). The Ctrl+wheel action stays `ctrl_mouse_wheel_action` from the ini only, like upstream (no control ships for it); `windowed_hide_cursor` is likewise ini-only with the upstream default 1 — configurable, aligned with upstream since #24. A background-color change repaints letterboxes live but does not re-composite transparent pixels until the next decode — the decode-time flatten (#3) snapshots the windowed color at request time, the same stale composite upstream shows after an Options color change.
- The menu carries only riviv's implemented commands and grows with each feature (upstream's table registers every command, greyed or not — riviv's Edit/Slideshow/Animation menus appear when their commands do; per issue #23). The two Everything-search rows show in riviv's File menu where upstream MF_OWNERDRAW-hides them (viv.c:804/807 — keyboard-only upstream, like the Add File row riviv already shows). About is a message box rather than upstream's branded IDD_ABOUT dialog, and its body stays English — upstream localizes its About dialog's caption and contents, riviv's box carries version + URLs. A blank window never shows a 1:1 checkmark (upstream's raw render==image size compare degenerates to 0==0 there). The right-click context menu ships only its bar-recovery row — "Menu" appears on right-click (or Shift+F10, centered) only while the menu bar is hidden (upstream gates the same row identically, viv.c:3427); the rest of upstream's context menu lands with its features. Custom shortcuts (#25) are bound per command in the ini (`<menu path>_keys = flags,flags,...`, upstream's spelling) and edited on the Controls page (commands list + Add/Edit/Remove over a capture dialog; a newly assigned chord is stripped from every other command first, like upstream). The Commands list and the keyboard route carry only implemented commands; menu accelerator labels show each command's FIRST registered binding from the live map (upstream viv.c:12365-12367) and relabel on the Options OK that changes them.
- Command-line switches: the install family is parsed exactly like upstream (`-install <path>`, `-install-options <opts>`, `-uninstall [path]`, `-appdata`/`-noappdata`, `-startmenu`/`-nostartmenu`, `-isrunas`, and `<ext>`/`-no<ext>` for the nine associations — case-insensitive, run before the window exists; #26). Config switches like `/sort`, `/slideshow`, `/fullscreen` — and the Everything pair `/everything <term>` / `/random <term>`, which upstream runs in the same window-exists pass — are still ignored (upstream parses them in a second pass and shows a usage dialog for unknown switches — that family stays deferred, along with the usage dialog); their parameter words leak into the file arguments the same way `/x 100`'s does. Switch detection matches upstream's quirk of treating dotted words like `-foo.png` as filenames; quoted switches are visible to the install parser (it reads the raw `GetCommandLineW`, like upstream) but stay indistinguishable to the file-argument parser through `args_os`.
- Upstream's default-on decode-ahead (preload next image) and last-image caches are not implemented — every open, including navigation back to a just-seen image, decodes from disk.
- Navigation always navigates by the default upstream sort (date modified, newest first). The sort-mode/ascending menu options and shuffle are M3 config work.
- The NUMPAD panscan commands are not implemented (size/width/height steps and move/center, upstream `VIV_ID_VIEW_PANSCAN_*`): zoom/pan uses the 16-level preset curve, the wheel and left-drag only. Middle-button drag-to-scroll (upstream `_VIV_DOING_MSCROLL`) is not implemented either.
- The mouse back/forward buttons (X buttons) do not navigate yet — upstream maps them to next/prev by default (`config_xbutton_action = 2`); lands with the slideshow/input work.

Agent workflow: see [AGENTS.md](AGENTS.md). Decisions: `docs/adr/`.

## License

MIT — same as upstream. Original C implementation © voidtools / David Carpenter.
