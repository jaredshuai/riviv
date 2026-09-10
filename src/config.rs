//! The settings model: a typed mirror of upstream's `config_*` globals
//! with their defaults (c-original/src/config.c:35-100), persisted as a
//! `[riviv]` ini section (upstream's `voidImageViewer.ini` — the riviv
//! namespace keeps the two co-installable).
//!
//! Locations (config.c:268-293/404-429):
//! - `appdata = 0`: `riviv.ini` next to the exe.
//! - `appdata = 1`: the exe-dir ini's `appdata` key switches everything to
//!   `%APPDATA%\riviv\riviv.ini`; the exe-dir file then holds ONLY the
//!   `appdata` marker (so a portable install can still find the switch).
//!
//! Load order is exe-dir first, then the appdata file overlays it: the
//! appdata file's keys override, keys it is MISSING keep the values the
//! exe-dir file already set (upstream `ini_get_int(ini, key, current)` —
//! config.c:104+).
//! Save mirrors `_config_save_settings_by_location` key-for-key, in the
//! upstream order, through a `.tmp` + rename (MOVEFILE_REPLACE_EXISTING
//! semantics via `std::fs::rename`, with upstream's CopyFile+DeleteFile
//! fallback, config.c:394-400).
//!
//! Only the window-position keys have consumers so far (#19); the rest of
//! the table exists so later M3 issues (#23/#24/#25/#26) wire settings
//! without re-touching the persistence layer.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::ini;
use crate::keys;
use crate::keys::KeyMap;
use crate::menu::Cmd;

/// The settings file's section name and basename (upstream uses
/// "voidImageViewer" for both; riviv names its own).
const SECTION: &str = "riviv";
const FILE_NAME: &str = "riviv.ini";
/// The `%APPDATA%` subdirectory (upstream: "voidimageviewer").
const APPDATA_DIR: &str = "riviv";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    // Field order and defaults mirror config.c:35-100 one line each.
    pub(crate) appdata: i32,
    pub(crate) keep_centered: i32,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) wide: i32,
    pub(crate) high: i32,
    pub(crate) maximized: i32,
    pub(crate) slideshow_rate: i32,
    pub(crate) allow_shrinking: i32,
    pub(crate) shrink_blit_mode: i32,
    pub(crate) mag_filter: i32,
    pub(crate) nav_sort: i32,
    pub(crate) nav_sort_ascending: i32,
    pub(crate) keep_aspect_ratio: i32,
    pub(crate) fill_window: i32,
    pub(crate) fullscreen_fill_window: i32,
    pub(crate) auto_zoom: i32,
    pub(crate) auto_zoom_type: i32,
    pub(crate) auto_fit_wide_mul: i32,
    pub(crate) auto_fit_wide_div: i32,
    pub(crate) auto_fit_high_mul: i32,
    pub(crate) auto_fit_high_div: i32,
    pub(crate) frame_minus: i32,
    pub(crate) multiple_instances: i32,
    pub(crate) show_status: i32,
    pub(crate) show_controls: i32,
    pub(crate) prevent_sleep: i32,
    pub(crate) loop_animations_once: i32,
    pub(crate) mouse_wheel_action: i32,
    pub(crate) ctrl_mouse_wheel_action: i32,
    pub(crate) left_click_action: i32,
    pub(crate) right_click_action: i32,
    pub(crate) xbutton_action: i32,
    pub(crate) windowed_background_color_r: i32,
    pub(crate) windowed_background_color_g: i32,
    pub(crate) windowed_background_color_b: i32,
    pub(crate) fullscreen_background_color_r: i32,
    pub(crate) fullscreen_background_color_g: i32,
    pub(crate) fullscreen_background_color_b: i32,
    pub(crate) options_last_page: i32,
    pub(crate) short_jump: i32,
    pub(crate) medium_jump: i32,
    pub(crate) long_jump: i32,
    pub(crate) shuffle: i32,
    pub(crate) browse_file_open_dialog: i32,
    pub(crate) ontop: i32,
    pub(crate) slideshow_custom_rate: i32,
    pub(crate) slideshow_custom_rate_type: i32,
    pub(crate) scroll_window: i32,
    pub(crate) preload_next: i32,
    pub(crate) cache_last: i32,
    pub(crate) icm: i32,
    pub(crate) show_menu: i32,
    pub(crate) show_caption: i32,
    pub(crate) show_thickframe: i32,
    pub(crate) toolbar_move_window: i32,
    pub(crate) windowed_hide_cursor: i32,
    pub(crate) pixel_info: i32,
    pub(crate) orientation: i32,
    pub(crate) title_bar_format: i32,
    pub(crate) add_command_line_timeout: i32,
    /// The per-command keyboard bindings (#25; upstream keeps these OUTSIDE
    /// the `config_*` int globals in `_viv_key_list`, but loads/saves them
    /// through the same ini pass — riding inside `Config` gives them the
    /// same two-location overlay and `.tmp` save discipline for free).
    pub(crate) keys: KeyMap,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            appdata: 0,
            keep_centered: 1,
            x: 0,
            y: 0,
            wide: 0,
            high: 0,
            maximized: 0,
            slideshow_rate: 5000,
            allow_shrinking: 1,
            shrink_blit_mode: 1, // CONFIG_SHRINK_BLIT_MODE_HALFTONE
            mag_filter: 0,       // CONFIG_MAG_FILTER_COLORONCOLOR
            nav_sort: 2,         // CONFIG_NAV_SORT_DATE_MODIFIED
            nav_sort_ascending: 0,
            keep_aspect_ratio: 1,
            fill_window: 0,
            fullscreen_fill_window: 1,
            auto_zoom: 0,
            auto_zoom_type: 1,
            auto_fit_wide_mul: 3,
            auto_fit_wide_div: 5,
            auto_fit_high_mul: 3,
            auto_fit_high_div: 5,
            frame_minus: 0,
            multiple_instances: 0,
            show_status: 1,
            show_controls: 1,
            prevent_sleep: 1,
            loop_animations_once: 1,
            mouse_wheel_action: 0,
            ctrl_mouse_wheel_action: 0,
            left_click_action: 0,
            right_click_action: 0,
            xbutton_action: 2,
            windowed_background_color_r: 255,
            windowed_background_color_g: 255,
            windowed_background_color_b: 255,
            fullscreen_background_color_r: 0,
            fullscreen_background_color_g: 0,
            fullscreen_background_color_b: 0,
            options_last_page: 0,
            short_jump: 500,
            medium_jump: 1000,
            long_jump: 2000,
            shuffle: 0,
            browse_file_open_dialog: 1,
            ontop: 0,
            slideshow_custom_rate: 3,
            slideshow_custom_rate_type: 1,
            scroll_window: 1,
            preload_next: 1,
            cache_last: 1,
            icm: 1,
            show_menu: 1,
            show_caption: 1,
            show_thickframe: 1,
            toolbar_move_window: 1,
            windowed_hide_cursor: 1,
            pixel_info: 0,
            orientation: 1,
            title_bar_format: 1,
            add_command_line_timeout: 500,
            keys: KeyMap::default(),
        }
    }
}

impl Config {
    /// The windowed compositing/letterbox background as one triple
    /// (upstream reads the three `config_windowed_background_color_*`
    /// globals wherever it builds an RGB, e.g. viv.c:4396).
    pub(crate) fn windowed_bg(&self) -> [u8; 3] {
        [
            self.windowed_background_color_r as u8,
            self.windowed_background_color_g as u8,
            self.windowed_background_color_b as u8,
        ]
    }

    /// The fullscreen background as one triple (`config_fullscreen_
    /// background_color_*`, viv.c:4396).
    pub(crate) fn fullscreen_bg(&self) -> [u8; 3] {
        [
            self.fullscreen_background_color_r as u8,
            self.fullscreen_background_color_g as u8,
            self.fullscreen_background_color_b as u8,
        ]
    }

    /// Read the settings (config.c:268-293): the exe-dir file first; if IT
    /// switched `appdata` on, the appdata file's keys overlay (its missing
    /// keys keep the exe-dir values — upstream `ini_get_int(ini, key,
    /// current)`). A missing or unreadable file keeps the defaults — same
    /// as upstream, whose `ini_open` failure leaves the globals untouched.
    pub(crate) fn load() -> Config {
        let mut config = Config::default();
        // Read as bytes + lossy decode: upstream parses raw bytes (only
        // ASCII keys matter), so a stray non-UTF-8 byte in a hand-edited
        // ini must not discard the whole remembered state.
        if let Some(path) = exe_dir_ini()
            && let Ok(bytes) = std::fs::read(&path)
        {
            let text = String::from_utf8_lossy(&bytes);
            config.apply_section(&ini::parse(&text, SECTION), true);
        }
        if config.appdata != 0
            && let Some(path) = appdata_ini()
            && let Ok(bytes) = std::fs::read(&path)
        {
            let text = String::from_utf8_lossy(&bytes);
            config.apply_section(&ini::parse(&text, SECTION), false);
        }
        config
    }

    /// Write the settings to the active location (config.c:404-429): the
    /// appdata dir (created if missing) when switched on, else the exe
    /// dir. With the switch on, the exe-dir file degenerates to the
    /// `appdata` marker (the real table then lives in appdata) — upstream
    /// writes BOTH files in two saves (the elevated `/appdata` helper's
    /// `config_save_settings(config_appdata)` then
    /// `config_save_settings(0)`, viv.c:4652-4662); riviv's direct
    /// switch (#24, no CLI switches yet) performs the same pair in one
    /// save. With the switch on but the appdata path unresolvable,
    /// upstream has NO fallback branch — the save is silently skipped,
    /// never redirected into the exe dir (which would clobber the root
    /// file); the marker is still written so a later launch can find the
    /// (failed) switch. Failures are logged, not fatal: upstream never
    /// checks its writes either, and a read-only install must still run.
    pub(crate) fn save(&self) {
        if self.appdata != 0 {
            if let Some(dir) = appdata_dir() {
                // Upstream CreateDirectory before saving (config.c:416).
                let _ = std::fs::create_dir_all(&dir);
                save_by_location(dir.join(FILE_NAME), self.to_pairs(false));
            }
            if let Some(dir) = exe_dir()
                // An exe living INSIDE the appdata dir would have the
                // marker write clobber the full table just saved to the
                // same path (upstream cannot hit this: its appdata path is
                // fixed to %APPDATA%voidimageviewer while the exe-dir
                // save always goes to the install dir).
                && appdata_dir().is_none_or(|ad| ad != dir)
            {
                save_by_location(dir.join(FILE_NAME), self.to_pairs(true));
            }
        } else if let Some(dir) = exe_dir() {
            save_by_location(dir.join(FILE_NAME), self.to_pairs(true));
        }
    }

    /// Apply one parsed section. `root` marks the exe-dir file, the only
    /// place the `appdata` switch itself is read (config.c:178-182).
    fn apply_section(&mut self, pairs: &ini::Section, root: bool) {
        // NOTE: the ini key spellings differ from the field names for
        // three keys (sort/sort_ascending/statusbar_pixel_info) — kept
        // verbatim for file compatibility with the upstream table
        // (config.c:113-178).
        let v = |key: &str| pairs.get(key).map(|s| ini::parse_int(s));
        // Most upstream globals are BYTEs (config.c:35-100) — the C
        // assignment from int TRUNCATES, so `maximized=256` is 0 there,
        // not a truthy value; mirror that exactly.
        macro_rules! apply_byte {
            ($field:ident = $key:literal) => {
                if let Some(value) = v($key) {
                    self.$field = (value as u8) as i32;
                }
            };
        }
        // The int-width globals: coordinates, rates, jumps, fit fractions.
        macro_rules! apply_int {
            ($field:ident = $key:literal) => {
                if let Some(value) = v($key) {
                    self.$field = value;
                }
            };
        }
        apply_int!(x = "x");
        apply_int!(y = "y");
        apply_int!(wide = "wide");
        apply_int!(high = "high");
        apply_byte!(maximized = "maximized");
        apply_int!(slideshow_rate = "slideshow_rate");
        apply_byte!(allow_shrinking = "allow_shrinking");
        apply_byte!(shrink_blit_mode = "shrink_blit_mode");
        apply_byte!(mag_filter = "mag_filter");
        apply_byte!(keep_aspect_ratio = "keep_aspect_ratio");
        apply_byte!(fill_window = "fill_window");
        apply_byte!(fullscreen_fill_window = "fullscreen_fill_window");
        apply_byte!(nav_sort = "sort");
        apply_byte!(nav_sort_ascending = "sort_ascending");
        apply_byte!(multiple_instances = "multiple_instances");
        apply_byte!(show_caption = "show_caption");
        apply_byte!(show_thickframe = "show_thickframe");
        apply_byte!(show_menu = "show_menu");
        apply_byte!(show_status = "show_status");
        apply_byte!(pixel_info = "statusbar_pixel_info");
        apply_byte!(show_controls = "show_controls");
        apply_byte!(auto_zoom = "auto_zoom");
        apply_byte!(auto_zoom_type = "auto_zoom_type");
        apply_int!(auto_fit_wide_mul = "auto_fit_wide_mul");
        apply_int!(auto_fit_wide_div = "auto_fit_wide_div");
        apply_int!(auto_fit_high_mul = "auto_fit_high_mul");
        apply_int!(auto_fit_high_div = "auto_fit_high_div");
        apply_byte!(frame_minus = "frame_minus");
        apply_byte!(mouse_wheel_action = "mouse_wheel_action");
        apply_byte!(ctrl_mouse_wheel_action = "ctrl_mouse_wheel_action");
        apply_byte!(left_click_action = "left_click_action");
        apply_byte!(right_click_action = "right_click_action");
        apply_byte!(xbutton_action = "xbutton_action");
        apply_byte!(keep_centered = "keep_centered");
        apply_byte!(windowed_background_color_r = "windowed_background_color_r");
        apply_byte!(windowed_background_color_g = "windowed_background_color_g");
        apply_byte!(windowed_background_color_b = "windowed_background_color_b");
        apply_byte!(windowed_hide_cursor = "windowed_hide_cursor");
        apply_byte!(fullscreen_background_color_r = "fullscreen_background_color_r");
        apply_byte!(fullscreen_background_color_g = "fullscreen_background_color_g");
        apply_byte!(fullscreen_background_color_b = "fullscreen_background_color_b");
        apply_byte!(options_last_page = "options_last_page");
        apply_int!(short_jump = "short_jump");
        apply_int!(medium_jump = "medium_jump");
        apply_int!(long_jump = "long_jump");
        apply_byte!(loop_animations_once = "loop_animations_once");
        apply_byte!(prevent_sleep = "prevent_sleep");
        apply_byte!(shuffle = "shuffle");
        apply_byte!(browse_file_open_dialog = "browse_file_open_dialog");
        apply_byte!(ontop = "ontop");
        apply_int!(slideshow_custom_rate = "slideshow_custom_rate");
        apply_byte!(slideshow_custom_rate_type = "slideshow_custom_rate_type");
        apply_byte!(scroll_window = "scroll_window");
        apply_byte!(preload_next = "preload_next");
        apply_byte!(cache_last = "cache_last");
        apply_byte!(icm = "icm");
        apply_byte!(orientation = "orientation");
        apply_byte!(toolbar_move_window = "toolbar_move_window");
        apply_byte!(title_bar_format = "title_bar_format");
        apply_int!(add_command_line_timeout = "add_command_line_timeout");
        if root {
            apply_byte!(appdata = "appdata");
        }
        // The `*_keys` overlays (config.c:180-222): a PRESENT line replaces
        // the command's bindings wholesale (empty = none); a missing line
        // keeps whatever stands — the defaults, or an earlier file's value
        // in the appdata-overlay pass.
        for cmd in Cmd::ALL {
            let name = keys::ini_name(cmd);
            if let Some(value) = pairs.get(&name) {
                self.keys.apply_ini(cmd, value);
            }
        }
    }

    /// The key=value table in upstream save order (config.c:293-383): the
    /// int keys, then one `*_keys` line per command in command order
    /// (present-but-empty clears — upstream writes the line even for an
    /// empty list). The exe-dir (`root`) form degenerates to the
    /// `appdata` marker only when the switch is ON — that is where the
    /// real table then lives (config.c:298-300: `is_root && config_appdata`).
    fn to_pairs(&self, root: bool) -> Vec<(String, String)> {
        let i = |v: i32| v.to_string();
        if root && self.appdata != 0 {
            return vec![("appdata".to_string(), i(self.appdata))];
        }
        let int_pairs: [(&str, String); 60] = [
            ("x", i(self.x)),
            ("y", i(self.y)),
            ("wide", i(self.wide)),
            ("high", i(self.high)),
            ("maximized", i(self.maximized)),
            ("slideshow_rate", i(self.slideshow_rate)),
            ("allow_shrinking", i(self.allow_shrinking)),
            ("shrink_blit_mode", i(self.shrink_blit_mode)),
            ("mag_filter", i(self.mag_filter)),
            ("keep_aspect_ratio", i(self.keep_aspect_ratio)),
            ("fill_window", i(self.fill_window)),
            ("fullscreen_fill_window", i(self.fullscreen_fill_window)),
            ("sort", i(self.nav_sort)),
            ("sort_ascending", i(self.nav_sort_ascending)),
            ("multiple_instances", i(self.multiple_instances)),
            ("show_caption", i(self.show_caption)),
            ("show_thickframe", i(self.show_thickframe)),
            ("show_menu", i(self.show_menu)),
            ("show_status", i(self.show_status)),
            ("statusbar_pixel_info", i(self.pixel_info)),
            ("show_controls", i(self.show_controls)),
            ("auto_zoom", i(self.auto_zoom)),
            ("auto_zoom_type", i(self.auto_zoom_type)),
            ("auto_fit_wide_mul", i(self.auto_fit_wide_mul)),
            ("auto_fit_wide_div", i(self.auto_fit_wide_div)),
            ("auto_fit_high_mul", i(self.auto_fit_high_mul)),
            ("auto_fit_high_div", i(self.auto_fit_high_div)),
            ("frame_minus", i(self.frame_minus)),
            ("mouse_wheel_action", i(self.mouse_wheel_action)),
            ("ctrl_mouse_wheel_action", i(self.ctrl_mouse_wheel_action)),
            ("left_click_action", i(self.left_click_action)),
            ("right_click_action", i(self.right_click_action)),
            ("xbutton_action", i(self.xbutton_action)),
            ("keep_centered", i(self.keep_centered)),
            (
                "windowed_background_color_r",
                i(self.windowed_background_color_r),
            ),
            (
                "windowed_background_color_g",
                i(self.windowed_background_color_g),
            ),
            (
                "windowed_background_color_b",
                i(self.windowed_background_color_b),
            ),
            ("windowed_hide_cursor", i(self.windowed_hide_cursor)),
            (
                "fullscreen_background_color_r",
                i(self.fullscreen_background_color_r),
            ),
            (
                "fullscreen_background_color_g",
                i(self.fullscreen_background_color_g),
            ),
            (
                "fullscreen_background_color_b",
                i(self.fullscreen_background_color_b),
            ),
            ("options_last_page", i(self.options_last_page)),
            ("short_jump", i(self.short_jump)),
            ("medium_jump", i(self.medium_jump)),
            ("long_jump", i(self.long_jump)),
            ("loop_animations_once", i(self.loop_animations_once)),
            ("prevent_sleep", i(self.prevent_sleep)),
            ("shuffle", i(self.shuffle)),
            ("browse_file_open_dialog", i(self.browse_file_open_dialog)),
            ("ontop", i(self.ontop)),
            ("slideshow_custom_rate", i(self.slideshow_custom_rate)),
            (
                "slideshow_custom_rate_type",
                i(self.slideshow_custom_rate_type),
            ),
            ("scroll_window", i(self.scroll_window)),
            ("preload_next", i(self.preload_next)),
            ("cache_last", i(self.cache_last)),
            ("icm", i(self.icm)),
            ("orientation", i(self.orientation)),
            ("toolbar_move_window", i(self.toolbar_move_window)),
            ("title_bar_format", i(self.title_bar_format)),
            ("add_command_line_timeout", i(self.add_command_line_timeout)),
        ];
        let mut pairs: Vec<(String, String)> = int_pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        // The per-command key lists (upstream appends them after the int
        // table, one line per command, config.c:350-383).
        for cmd in Cmd::ALL {
            pairs.push((keys::ini_name(cmd), self.keys.to_ini(cmd)));
        }
        pairs
    }
}

/// The exe's directory (upstream `string_get_exe_path`).
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn exe_dir_ini() -> Option<PathBuf> {
    exe_dir().map(|d| d.join(FILE_NAME))
}

/// `%APPDATA%\riviv` — resolved through the roaming-AppData KNOWN FOLDER
/// like upstream (`SHGetSpecialFolderLocation(CSIDL_APPDATA)`,
/// string.c:637-655), not the `APPDATA` environment variable a launcher
/// can strip or override while the known folder stays correct.
fn appdata_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut buf = [0u16; 260]; // MAX_PATH
    // SAFETY: `buf` is a valid MAX_PATH-sized out-buffer for the duration
    // of the call; no ownership is taken of any parameter.
    let hr = unsafe {
        windows::Win32::UI::Shell::SHGetFolderPathW(
            None,
            windows::Win32::UI::Shell::CSIDL_APPDATA as i32,
            None,
            0,
            &mut buf,
        )
    };
    if hr.is_ok()
        && let Some(len) = buf.iter().position(|&c| c == 0)
        && len > 0
    {
        return Some(PathBuf::from(OsString::from_wide(&buf[..len])).join(APPDATA_DIR));
    }
    None
}

fn appdata_ini() -> Option<PathBuf> {
    appdata_dir().map(|d| d.join(FILE_NAME))
}

/// Write through a `.tmp` then rename over the target — upstream's
/// MoveFileExW(REPLACE_EXISTING) with the CopyFile+DeleteFile fallback
/// (config.c:394-400; `std::fs::rename` IS MoveFileExW with the replace
/// flag on Windows).
fn save_by_location(path: PathBuf, pairs: Vec<(String, String)>) {
    let text = ini::serialize(SECTION, &pairs);
    let tmp = path.with_extension("ini.tmp");
    if let Err(e) = std::fs::write(&tmp, text) {
        eprintln!("riviv: write {} failed: {e}", tmp.display());
        return;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        // The upstream fallback for replace-hostile filesystems.
        if let Err(e) = std::fs::copy(&tmp, &path).and_then(|_| std::fs::remove_file(&tmp)) {
            eprintln!("riviv: save {} failed: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_apply(text: &str, root: bool) -> Config {
        let mut c = Config::default();
        c.apply_section(&ini::parse(text, SECTION), root);
        c
    }

    #[test]
    fn defaults_match_the_upstream_table() {
        // Spot checks against config.c:35-100 (the full table is pinned by
        // the round-trip test below — every key must both read and write).
        let d = Config::default();
        assert_eq!(d.wide, 0);
        assert_eq!(d.maximized, 0);
        assert_eq!(d.slideshow_rate, 5000);
        assert_eq!(d.shrink_blit_mode, 1);
        assert_eq!(d.nav_sort, 2);
        assert_eq!(d.nav_sort_ascending, 0);
        assert_eq!(d.fill_window, 0);
        assert_eq!(d.fullscreen_fill_window, 1);
        assert_eq!(d.auto_fit_wide_mul, 3);
        assert_eq!(d.auto_fit_wide_div, 5);
        assert_eq!(d.multiple_instances, 0);
        assert_eq!(d.windowed_background_color_r, 255);
        assert_eq!(d.fullscreen_background_color_r, 0);
        assert_eq!(d.windowed_hide_cursor, 1);
        assert_eq!(d.show_menu, 1);
        assert_eq!(d.add_command_line_timeout, 500);
    }

    #[test]
    fn every_written_key_reads_back() {
        // Round-trips the save table through the parser: catches a key the
        // save list writes but the load match doesn't read (and vice versa
        // via the key-count assertion).
        let c = Config {
            x: -8,
            y: 17,
            wide: 640,
            high: 480,
            maximized: 1,
            nav_sort: 0,
            pixel_info: 1,
            shuffle: 1,
            ..Config::default()
        };
        let text = ini::serialize(SECTION, &c.to_pairs(false));
        let back = parse_apply(&text, true);
        assert_eq!(back, c, "every save key must be a load key");
        // 60 int keys + one *_keys line per command (17).
        assert_eq!(c.to_pairs(false).len(), 77, "the upstream save table");
    }

    #[test]
    fn missing_keys_keep_defaults_garbage_values_are_zero() {
        // A key present with garbage parses to 0 (upstream utf8_to_int),
        // NOT the default — only a MISSING key falls back.
        let c = parse_apply("[riviv]\nwide=abc\n", true);
        assert_eq!(c.wide, 0);
        let c = parse_apply("[riviv]\nslideshow_rate=abc\n", true);
        assert_eq!(c.slideshow_rate, 0);
        let c = parse_apply("[riviv]\n", true);
        assert_eq!(c.slideshow_rate, 5000);
    }

    #[test]
    fn byte_backed_keys_truncate_like_the_c_assignment() {
        // Upstream stores these as BYTE globals: `maximized=256` truncates
        // to 0 (NOT maximized), 257 to 1, -1 to 255; the int-width keys
        // keep their full value.
        let c = parse_apply("[riviv]\nmaximized=256\nx=100000\n", true);
        assert_eq!(c.maximized, 0, "256 truncates to BYTE 0");
        let c = parse_apply("[riviv]\nmaximized=257\n", true);
        assert_eq!(c.maximized, 1);
        let c = parse_apply("[riviv]\nmaximized=-1\n", true);
        assert_eq!(c.maximized, 255, "(BYTE)(-1) is 255");
        let c = parse_apply("[riviv]\nappdata=256\n", true);
        assert_eq!(c.appdata, 0, "appdata=256 must NOT switch locations");
        let c = parse_apply("[riviv]\nx=100000\n", true);
        assert_eq!(c.x, 100000, "int-width keys do not truncate");
    }

    #[test]
    fn appdata_key_is_only_read_from_the_root_file() {
        // The appdata switch lives in the exe-dir file only
        // (config.c:178-182) — an appdata file cannot flip it.
        let c = parse_apply("[riviv]\nappdata=1\n", true);
        assert_eq!(c.appdata, 1);
        let c = parse_apply("[riviv]\nappdata=1\n", false);
        assert_eq!(c.appdata, 0);
    }

    #[test]
    fn root_save_is_the_appdata_marker_only_when_switched_on() {
        // appdata=0: the exe-dir file IS the active store — full table
        // (config.c:298-300). appdata=1: the exe-dir file keeps only the
        // switch; the table lives in %APPDATA%.
        let c = Config::default();
        assert_eq!(
            c.to_pairs(true).len(),
            77,
            "active store writes the full table"
        );
        let c = Config {
            appdata: 1,
            ..Config::default()
        };
        assert_eq!(
            c.to_pairs(true),
            vec![("appdata".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn keys_lines_replace_and_missing_lines_keep_defaults() {
        // config.c:180-222: a present `*_keys` line rebuilds that
        // command's list (empty = no bindings); a missing line leaves the
        // default table standing.
        let c = parse_apply("[riviv]\nnavigate_next_keys=78\nfile_exit_keys=\n", true);
        assert_eq!(c.keys.keys(Cmd::NavNext).len(), 1);
        assert_eq!(c.keys.keys(Cmd::NavNext)[0].vk, 78);
        assert!(c.keys.keys(Cmd::FileExit).is_empty());
        // Untouched commands keep their defaults (and the ini overlay of a
        // second location only replaces lines IT carries).
        assert_eq!(c.keys.keys(Cmd::FileOpenFile).len(), 1);
        let c = parse_apply("[riviv]\n[riviv]\n", true);
        assert_eq!(c.keys, KeyMap::default());
    }

    #[test]
    fn keys_lines_round_trip_through_save() {
        // A customized map saves to decimal flags and reads back identically
        // ('N'=0x4E=78 bare, Ctrl+F2=0x1F1=497).
        let mut c = Config::default();
        c.keys.set(Cmd::NavNext, vec![crate::keys::from_flags(78)]);
        c.keys.add(Cmd::HelpAbout, crate::keys::from_flags(497));
        let text = ini::serialize(SECTION, &c.to_pairs(false));
        let back = parse_apply(&text, true);
        assert_eq!(back.keys, c.keys);
    }

    #[test]
    fn tmp_extension_sits_alongside_the_target() {
        let p = PathBuf::from(r"C:\dir\riviv.ini");
        assert_eq!(
            p.with_extension("ini.tmp"),
            PathBuf::from(r"C:\dir\riviv.ini.tmp")
        );
    }
}
