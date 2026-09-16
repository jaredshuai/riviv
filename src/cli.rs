//! The second-pass command-line parse (#48; upstream
//! `_viv_process_command_line`, viv.c:4744-5148): the config-switch family
//! that runs once the window exists — at startup and on every
//! single-instance handoff receive. Everything here is pure wide-string
//! logic; the shell half (applying config, sending Everything queries,
//! toggling fullscreen) lives in window.rs.
//!
//! Upstream's walk is over the RAW command line with its own tokenizer
//! (`string_get_word`, string.c:804-835): quotes group words and `""`
//! embeds a literal quote — NOT CommandLineToArgvW's backslash rules —
//! and a word only counts as a switch when it was UNQUOTED, starts with
//! `/` or `-`, and contains no '.' anywhere (string.c:856-871 —
//! `-foo.png` is a file). `args_os` cannot see the quoting, so both the
//! startup line and the forwarded handoff line come in raw.
//!
//! Upstream quirks kept faithfully:
//! - `/appdata`-family words have no second-pass arm (they belong to the
//!   install pass, which runs first and does not exit for them) — they
//!   land in the unknown arm and pop the usage box (viv.c:4988).
//! - `/rate`'s value is stored as the slideshow PRESET INDEX; the usage
//!   text saying "milliseconds" is upstream's own doc/impl mismatch.
//! - `string_to_int` (string.c:260-285) skips non-digits WITHOUT stopping
//!   ("1x2" parses as 12) and an absent parameter word parses as 0.

use crate::playlist::SortMode;

/// One tokenizer word (string.c:804-835): the unquoted text. Whether the
/// raw word STARTED with a quote (viv.c:4801-4806's `was_quote`) is the
/// caller's to read off the slice BEFORE consuming the word.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    text: Vec<u16>,
}

/// `wchar_is_ws` (wchar.c:37-47).
fn is_ws(c: u16) -> bool {
    c == b' ' as u16 || c == b'\t' as u16 || c == b'\r' as u16 || c == b'\n' as u16
}

/// `string_get_word` + `string_skip_ws` in one step: consume the next
/// word starting at `i`, leaving `i` past the following whitespace.
fn next_word(cl: &[u16], i: &mut usize) -> Word {
    let mut text = Vec::new();
    let mut quoted = false;
    while *i < cl.len() {
        let c = cl[*i];
        if c == u16::from(b'"') && *i + 1 < cl.len() && cl[*i + 1] == u16::from(b'"') {
            *i += 2;
            text.push(u16::from(b'"'));
        } else if c == u16::from(b'"') {
            quoted = !quoted;
            *i += 1;
        } else if !quoted && is_ws(c) {
            break;
        } else {
            text.push(c);
            *i += 1;
        }
    }
    while *i < cl.len() && is_ws(cl[*i]) {
        *i += 1;
    }
    Word { text }
}

/// `string_to_int` (string.c:260-285): one optional leading '-', then
/// every digit ANYWHERE in the word accumulates (non-digits neither stop
/// nor reset the accumulator — "1x2" is 12).
pub(crate) fn to_int(s: &[u16]) -> i32 {
    let mut sign = 1i32;
    let mut i = 0usize;
    if s.first() == Some(&u16::from(b'-')) {
        sign = -1;
        i = 1;
    }
    let mut acc = 0i32;
    while i < s.len() {
        let c = s[i];
        if u8::try_from(c).is_ok_and(|b| b.is_ascii_digit()) {
            acc = acc
                .wrapping_mul(10)
                .wrapping_add(i32::from(c - b'0' as u16));
        }
        i += 1;
    }
    sign.wrapping_mul(acc)
}

/// The `stdin:` pseudo-filename (#65; upstream wishlist viv.c:81 — "open
/// a file with the filename stdin: to open stdin"): one word's text,
/// ASCII case-insensitive, exactly `stdin:`. Quoting is irrelevant (the
/// tokenizer strips it into the word — a quoted `"stdin:"` is still the
/// filename); the switch form `/stdin:` keeps its slash and never
/// matches. NTFS reserves ':' as the alternate-data-stream separator, so
/// no on-disk file can collide with the pseudo-name.
pub(crate) fn is_stdin_word(word: &[u16]) -> bool {
    word.len() == 6
        && word.iter().zip("stdin:".as_bytes()).all(|(c, b)| {
            // eq_switch's manual lowercase (u16 has no to_ascii_lowercase).
            let lower = if (b'A' as u16..=b'Z' as u16).contains(c) {
                *c + 32
            } else {
                *c
            };
            lower == u16::from(*b)
        })
}

/// Whether a raw command line contains the `stdin:` pseudo-filename as
/// any word (#65): the single-instance gate reads this BEFORE forwarding
/// — a piped launch must keep its own stdin (the pipe belongs to this
/// process; see the run() handoff block) — and nothing else may resolve
/// the word to a path.
pub(crate) fn has_stdin_word(cl: &[u16]) -> bool {
    let mut i = 0usize;
    while i < cl.len() && is_ws(cl[i]) {
        i += 1;
    }
    let _exe = next_word(cl, &mut i);
    loop {
        if i >= cl.len() {
            return false;
        }
        let word = next_word(cl, &mut i);
        if is_stdin_word(&word.text) {
            return true;
        }
    }
}

/// ASCII case-insensitive compare against a switch name (upstream
/// `string_icompare_lowercase_ascii`).
fn eq_switch(word: &[u16], name: &str) -> bool {
    word.len() == name.len()
        && word.iter().zip(name.as_bytes()).all(|(c, b)| {
            let lower = if (b'A' as u16..=b'Z' as u16).contains(c) {
                *c + 32
            } else {
                *c
            };
            lower == *b as u16
        })
}

/// The `/x /y /width /height` overrides — unset axes keep the window's
/// current rect (upstream seeds all four from GetWindowRect, viv.c:4774).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RectOverride {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub wide: Option<i32>,
    pub high: Option<i32>,
}

impl RectOverride {
    pub(crate) fn any(&self) -> bool {
        self.x.is_some() || self.y.is_some() || self.wide.is_some() || self.high.is_some()
    }
}

/// One config-write switch arm's fields — replayed at its walk position,
/// last write wins per field (upstream assigns the globals mid-walk,
/// viv.c:4838-4953).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ConfigWrite {
    /// `/name`-family: the mode arm also fixes the direction (name/path →
    /// ascending, dm/dc/size → descending, viv.c:4917-4953).
    pub nav_sort: Option<crate::playlist::SortMode>,
    /// `/ascending` or `/descending` alone.
    pub sort_ascending: Option<i32>,
    pub shuffle: bool,
    /// `/rate <index>` (the PRESET index — the usage text saying
    /// milliseconds is upstream's own doc/impl mismatch).
    pub slideshow_rate: Option<i32>,
}

/// The ordered side-effect stream of the walk (upstream's loop is
/// order-sensitive: `is_add` can flip via `/add` between words, and a
/// `/random` between file words fires its navigation BEFORE later words
/// — the shell replays one-by-one at the original positions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClAction {
    /// The first file word of the line: random mode exits (viv.c:4998-5006).
    ExitRandom,
    /// A REPLACING line's first file word cleared the playlist (the same
    /// block — only when `!is_add` at that moment).
    ClearPlaylist,
    /// One file word appended to the playlist (viv.c:5008-5023: the stashed
    /// `single` first, then each next word — the shell replays in order).
    AddFile(Vec<u16>),
    /// `/everything <term>` fired its search mid-walk (viv.c:4857-4865).
    Everything(Vec<u16>),
    /// `/random <term>` armed and fired its first query mid-walk
    /// (viv.c:4866-4872 — the randomize arm navigates immediately).
    Random(Vec<u16>),
    /// A config-writing switch arm landed (last write wins per field).
    ConfigWrite(ConfigWrite),
    /// A command-dispatching switch landed (#46): `/ontop` runs the
    /// View→On Top→Always command, `/minimal`/`/compact` the Preset 1/2
    /// commands (upstream `_viv_command`s them mid-walk, viv.c:4853-4888
    /// — the toggle quirks ride along through the shared handlers).
    Command(crate::menu::Cmd),
}

/// The parsed second-pass outcome. The shell replays `actions` in order,
/// then the end blocks (add-mode bootstrap / the open), the usage boxes,
/// and the show tail (`/slideshow`, fullscreen, rect, maximized).
#[derive(Debug, Clone, Default)]
pub(crate) struct Parsed {
    pub actions: Vec<ClAction>,
    /// Upstream's `file_count` — drives the end blocks (add-mode's lone
    /// word append, the single-vs-playlist open).
    pub file_count: usize,
    pub single: Option<Vec<u16>>,
    pub is_add: bool,
    pub start_slideshow: bool,
    pub start_fullscreen: bool,
    pub start_window: bool,
    pub start_maximized: bool,
    pub rect: RectOverride,
    /// Unknown switch words — each pops the usage box (viv.c:4986-4990).
    pub unknown: Vec<Vec<u16>>,
}

/// Parse the raw command line (the full GetCommandLineW string, exe word
/// included — skipped like upstream viv.c:4789-4790). `is_add` is the
/// pre-walk add decision (the handoff timeout window); `has_current` is
/// whether the receiving window has a current file — the `/add` arm only
/// takes when something is loaded (viv.c:4963-4969).
pub(crate) fn parse(cl: &[u16], is_add: bool, has_current: bool) -> Parsed {
    let mut out = Parsed {
        is_add,
        ..Parsed::default()
    };
    let mut i = 0usize;
    while i < cl.len() && is_ws(cl[i]) {
        i += 1;
    }
    let _exe = next_word(cl, &mut i);

    let mut file_count = 0usize;
    let mut single: Option<Vec<u16>> = None;
    loop {
        if i >= cl.len() {
            break;
        }
        let started_quoted = cl[i] == u16::from(b'"');
        let word = next_word(cl, &mut i);
        let text = &word.text;
        // The switch test (viv.c:4817-4820): unquoted, '/'- or '-'-prefixed,
        // and dot-free (dotted "-foo.png" is a filename).
        let is_switch_word = !started_quoted
            && matches!(text.first(), Some(&c) if c == u16::from(b'/') || c == u16::from(b'-'))
            && !text.contains(&u16::from(b'.'));
        if is_switch_word {
            let arm = &text[1..];
            // The parameter words each arm consumes (viv.c reads the next
            // word verbatim — quotes already stripped by the tokenizer).
            let param = |i: &mut usize| next_word(cl, i).text;
            if eq_switch(arm, "slideshow") {
                out.start_slideshow = true;
            } else if eq_switch(arm, "fullscreen") {
                out.start_fullscreen = true;
                out.start_window = false;
            } else if eq_switch(arm, "window") {
                out.start_fullscreen = false;
                out.start_window = true;
            } else if eq_switch(arm, "maximized") {
                out.start_fullscreen = false;
                out.start_window = true;
                out.start_maximized = true;
            } else if eq_switch(arm, "ontop") {
                // Upstream runs the View→On Top→Always command in place
                // (viv.c:4853-4855) — the TOGGLE semantics ride along
                // through the shared handler.
                out.actions
                    .push(ClAction::Command(crate::menu::Cmd::ViewOntopAlways));
            } else if eq_switch(arm, "shuffle") {
                out.actions.push(ClAction::ConfigWrite(ConfigWrite {
                    shuffle: true,
                    ..ConfigWrite::default()
                }));
            } else if eq_switch(arm, "everything") {
                out.actions.push(ClAction::Everything(param(&mut i)));
            } else if eq_switch(arm, "random") {
                out.actions.push(ClAction::Random(param(&mut i)));
            } else if eq_switch(arm, "minimal") || eq_switch(arm, "compact") {
                // The view-preset pair (#46): upstream `_viv_command`s
                // Preset 1 / Preset 2 in place (viv.c:4879-4888).
                out.actions
                    .push(ClAction::Command(if eq_switch(arm, "minimal") {
                        crate::menu::Cmd::ViewPreset1
                    } else {
                        crate::menu::Cmd::ViewPreset2
                    }));
            } else if eq_switch(arm, "x") {
                out.rect.x = Some(to_int(&param(&mut i)));
            } else if eq_switch(arm, "y") {
                out.rect.y = Some(to_int(&param(&mut i)));
            } else if eq_switch(arm, "width") {
                out.rect.wide = Some(to_int(&param(&mut i)));
            } else if eq_switch(arm, "height") {
                out.rect.high = Some(to_int(&param(&mut i)));
            } else if eq_switch(arm, "rate") {
                out.actions.push(ClAction::ConfigWrite(ConfigWrite {
                    slideshow_rate: Some(to_int(&param(&mut i))),
                    ..ConfigWrite::default()
                }));
            } else if eq_switch(arm, "name") {
                out.actions.push(sort_write(SortMode::Name, 1));
            } else if eq_switch(arm, "dm") {
                out.actions.push(sort_write(SortMode::DateModified, 0));
            } else if eq_switch(arm, "dc") {
                out.actions.push(sort_write(SortMode::DateCreated, 0));
            } else if eq_switch(arm, "path") {
                out.actions.push(sort_write(SortMode::FullPath, 1));
            } else if eq_switch(arm, "size") {
                out.actions.push(sort_write(SortMode::Size, 0));
            } else if eq_switch(arm, "ascending") {
                out.actions.push(ClAction::ConfigWrite(ConfigWrite {
                    sort_ascending: Some(1),
                    ..ConfigWrite::default()
                }));
            } else if eq_switch(arm, "descending") {
                out.actions.push(ClAction::ConfigWrite(ConfigWrite {
                    sort_ascending: Some(0),
                    ..ConfigWrite::default()
                }));
            } else if eq_switch(arm, "isrunas") {
                // The install pass's re-execution marker — inert here
                // (viv.c:4958-4961).
            } else if eq_switch(arm, "add") {
                if has_current {
                    out.is_add = true;
                }
            } else {
                out.unknown.push(text.clone());
            }
        } else if !text.is_empty() {
            // The file-word bookkeeping, exactly upstream's ladder
            // (viv.c:4990-5024).
            if file_count == 0 {
                out.actions.push(ClAction::ExitRandom);
                if !out.is_add {
                    out.actions.push(ClAction::ClearPlaylist);
                }
            }
            if file_count == 1 {
                out.actions.push(ClAction::AddFile(single.clone().unwrap()));
            }
            if file_count >= 1 {
                out.actions.push(ClAction::AddFile(text.clone()));
            }
            if file_count == 0 {
                single = Some(text.clone());
            }
            file_count += 1;
        }
    }
    out.file_count = file_count;
    out.single = single;
    out
}

/// A `/name`-family arm's two-field write (the mode fixes the direction,
/// viv.c:4917-4953).
fn sort_write(mode: SortMode, ascending: i32) -> ClAction {
    ClAction::ConfigWrite(ConfigWrite {
        nav_sort: Some(mode),
        sort_ascending: Some(ascending),
        ..ConfigWrite::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::SortMode;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn s(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    /// Fold the stream's ConfigWrite actions into the final config state
    /// (the replay applies them in order — last write wins per field).
    fn config_of(p: &Parsed) -> ConfigWrite {
        let mut out = ConfigWrite::default();
        for a in &p.actions {
            if let ClAction::ConfigWrite(w) = a {
                if let Some(mode) = w.nav_sort {
                    out.nav_sort = Some(mode);
                }
                if let Some(asc) = w.sort_ascending {
                    out.sort_ascending = Some(asc);
                }
                out.shuffle |= w.shuffle;
                if let Some(rate) = w.slideshow_rate {
                    out.slideshow_rate = Some(rate);
                }
            }
        }
        out
    }

    // ---- the tokenizer (string.c:804-835) ----

    #[test]
    fn double_quotes_embed_one_literal_quote() {
        let cl = w("exe a\"\"b c");
        let mut i = 0;
        let _exe = next_word(&cl, &mut i);
        assert_eq!(s(&next_word(&cl, &mut i).text), "a\"b");
        assert_eq!(s(&next_word(&cl, &mut i).text), "c");
    }

    #[test]
    fn quoted_whitespace_groups_one_word() {
        let cl = w("exe \"a b\" c");
        let mut i = 0;
        let _exe = next_word(&cl, &mut i);
        assert_eq!(s(&next_word(&cl, &mut i).text), "a b");
        assert_eq!(s(&next_word(&cl, &mut i).text), "c");
        assert_eq!(i, cl.len());
    }

    // ---- switch recognition (viv.c:4817-4820) ----

    #[test]
    fn a_quoted_switch_is_a_file_word() {
        // The was_quote test: "\"/slideshow\"" opens a (missing) FILE, it
        // does not arm the slideshow.
        let p = parse(&w("exe \"/slideshow\""), false, false);
        assert!(!p.start_slideshow);
        assert_eq!(p.file_count, 1);
        assert_eq!(s(p.single.as_ref().unwrap()), "/slideshow");
    }

    #[test]
    fn dotted_dash_words_are_files_and_arms_ignore_case() {
        let p = parse(&w("exe -foo.png /SLIDESHOW -Fullscreen"), false, false);
        assert_eq!(p.file_count, 1);
        assert_eq!(s(p.single.as_ref().unwrap()), "-foo.png");
        assert!(p.start_slideshow);
        assert!(p.start_fullscreen);
    }

    // ---- parameter consumption ----

    #[test]
    fn rect_and_rate_switches_consume_their_parameter_words() {
        let p = parse(
            &w("exe /x 100 /y 200 /width 300 /height 400 /rate 4 img.png"),
            false,
            false,
        );
        assert_eq!(p.rect.x, Some(100));
        assert_eq!(p.rect.y, Some(200));
        assert_eq!(p.rect.wide, Some(300));
        assert_eq!(p.rect.high, Some(400));
        assert_eq!(config_of(&p).slideshow_rate, Some(4));
        // The parameters never leak into the file words.
        assert_eq!(p.file_count, 1);
        assert_eq!(s(p.single.as_ref().unwrap()), "img.png");
    }

    #[test]
    fn a_missing_parameter_parses_as_zero() {
        // string_get_word returns the empty tail word; string_to_int("")=0
        // (upstream moves the window to 0 for a dangling "/x").
        let p = parse(&w("exe /x"), false, false);
        assert_eq!(p.rect.x, Some(0));
    }

    #[test]
    fn everything_and_random_consume_terms_in_walk_order() {
        let p = parse(
            &w("exe /everything cat /random dog.png a.png"),
            false,
            false,
        );
        assert_eq!(
            p.actions,
            vec![
                ClAction::Everything(w("cat")),
                ClAction::Random(w("dog.png")),
                ClAction::ExitRandom,
                ClAction::ClearPlaylist,
            ]
        );
        assert_eq!(p.file_count, 1); // dog.png was consumed as the term
        assert_eq!(s(p.single.as_ref().unwrap()), "a.png");
    }

    #[test]
    fn a_search_between_file_words_keeps_its_walk_position() {
        // The cubic P1 pin: `/random` between file words fires its
        // navigation BEFORE later words process (viv.c walks one loop).
        let p = parse(&w("exe a.png /random t b.png"), false, false);
        assert_eq!(
            p.actions,
            vec![
                ClAction::ExitRandom,
                ClAction::ClearPlaylist,
                ClAction::Random(w("t")),
                ClAction::AddFile(w("a.png")),
                ClAction::AddFile(w("b.png")),
            ]
        );
    }

    // ---- the sort family (viv.c:4917-4953) ----

    #[test]
    fn sort_arms_fix_both_mode_and_direction() {
        let p = parse(&w("exe /name"), false, false);
        let c = config_of(&p);
        assert_eq!(c.nav_sort, Some(SortMode::Name));
        assert_eq!(c.sort_ascending, Some(1));
        let p = parse(&w("exe /size"), false, false);
        let c = config_of(&p);
        assert_eq!(c.nav_sort, Some(SortMode::Size));
        assert_eq!(c.sort_ascending, Some(0));
        assert_eq!(
            config_of(&parse(&w("exe /dm"), false, false)).nav_sort,
            Some(SortMode::DateModified)
        );
        assert_eq!(
            config_of(&parse(&w("exe /dc"), false, false)).nav_sort,
            Some(SortMode::DateCreated)
        );
        assert_eq!(
            config_of(&parse(&w("exe /path"), false, false)).nav_sort,
            Some(SortMode::FullPath)
        );
    }

    #[test]
    fn bare_direction_switches_and_last_write_wins() {
        let c = config_of(&parse(&w("exe /size /ascending"), false, false));
        assert_eq!(c.nav_sort, Some(SortMode::Size));
        assert_eq!(c.sort_ascending, Some(1));
        // /dm is the last mode write; /descending stays the last direction.
        let c = config_of(&parse(&w("exe /name /descending /dm"), false, false));
        assert_eq!(c.nav_sort, Some(SortMode::DateModified));
        assert_eq!(c.sort_ascending, Some(0));
    }

    #[test]
    fn shuffle_and_isrunas_are_recognized() {
        let p = parse(&w("exe /shuffle /isrunas"), false, false);
        assert!(config_of(&p).shuffle);
        assert!(p.unknown.is_empty());
    }

    // ---- the unknown arm (viv.c:4986-4990) ----

    #[test]
    fn unknown_switches_collect_for_the_usage_box() {
        // Includes the /appdata family: install-pass words with no
        // second-pass arm — upstream's own quirk.
        let p = parse(&w("exe /appdata /frobnicate a.png"), false, false);
        assert_eq!(p.unknown.len(), 2);
        assert_eq!(s(&p.unknown[0]), "/appdata");
        assert_eq!(s(&p.unknown[1]), "/frobnicate");
        assert_eq!(p.file_count, 1);
    }

    // ---- the file-word ladder (viv.c:4990-5024) ----

    #[test]
    fn a_single_word_never_enters_the_playlist_during_the_walk() {
        let p = parse(&w("exe a.png"), false, false);
        assert_eq!(
            p.actions,
            vec![ClAction::ExitRandom, ClAction::ClearPlaylist]
        );
        assert_eq!(p.file_count, 1);
        assert_eq!(s(p.single.as_ref().unwrap()), "a.png");
    }

    #[test]
    fn the_second_word_promotes_the_stashed_single() {
        let p = parse(&w("exe a.png b.png"), false, false);
        assert_eq!(
            p.actions,
            vec![
                ClAction::ExitRandom,
                ClAction::ClearPlaylist,
                ClAction::AddFile(w("a.png")),
                ClAction::AddFile(w("b.png")),
            ]
        );
        assert_eq!(p.file_count, 2);
    }

    #[test]
    fn add_mode_skips_the_clear_but_still_exits_random() {
        let p = parse(&w("exe a.png b.png"), true, false);
        assert_eq!(
            p.actions,
            vec![
                ClAction::ExitRandom,
                ClAction::AddFile(w("a.png")),
                ClAction::AddFile(w("b.png"))
            ]
        );
    }

    #[test]
    fn the_add_switch_only_takes_when_something_is_loaded() {
        // Mid-walk flip (viv.c:4963-4969): the first word's clear already
        // ran with is_add=0 — the upstream ordering quirk.
        let p = parse(&w("exe a.png /add b.png"), false, true);
        assert_eq!(
            p.actions,
            vec![
                ClAction::ExitRandom,
                ClAction::ClearPlaylist,
                ClAction::AddFile(w("a.png")),
                ClAction::AddFile(w("b.png")),
            ]
        );
        assert!(p.is_add);
        assert_eq!(p.file_count, 2);
        // Without a current file the switch is inert.
        let p = parse(&w("exe /add a.png"), false, false);
        assert!(!p.is_add);
    }

    #[test]
    fn empty_words_and_bare_quotes_are_skipped_as_files() {
        // A lone "" tokenizes to one literal quote char (the doubling
        // rule) — a file word, not empty. Truly empty words only arise
        // at end-of-string and never reach the ladder.
        let p = parse(&w("exe   a.png"), false, false);
        assert_eq!(p.file_count, 1);
    }

    // ---- string_to_int (string.c:260-285) ----

    #[test]
    fn to_int_skips_non_digits_without_stopping() {
        assert_eq!(to_int(&w("100")), 100);
        assert_eq!(to_int(&w("-40")), -40);
        assert_eq!(to_int(&w("1x2")), 12);
        assert_eq!(to_int(&w("")), 0);
        assert_eq!(to_int(&w("abc")), 0);
    }

    // ---- the stdin: pseudo-filename (#65) ----

    #[test]
    fn stdin_word_matches_case_insensitively_and_exactly() {
        assert!(is_stdin_word(&w("stdin:")));
        assert!(is_stdin_word(&w("STDIN:")));
        assert!(is_stdin_word(&w("StdIn:")));
        // Not the word: longer, shorter, prefix, switch form, suffixed.
        assert!(!is_stdin_word(&w("stdin")));
        assert!(!is_stdin_word(&w("stdin:x")));
        assert!(!is_stdin_word(&w("xstdin:")));
        assert!(!is_stdin_word(&w("/stdin:")));
        assert!(!is_stdin_word(&w("-stdin:")));
        assert!(!is_stdin_word(&w("stdin.png")));
    }

    #[test]
    fn stdin_word_scan_walks_the_raw_command_line() {
        // Any word position counts, quoted or not (the tokenizer strips
        // quoting); the exe word and switches do not false-positive.
        assert!(has_stdin_word(&w("riviv stdin:")));
        assert!(has_stdin_word(&w("riviv /fullscreen stdin:")));
        assert!(has_stdin_word(&w("riviv \"stdin:\"")));
        assert!(has_stdin_word(&w("riviv a.png stdin:")));
        assert!(has_stdin_word(&w("riviv /x 100 stdin:")));
        assert!(!has_stdin_word(&w("riviv a.png")));
        assert!(!has_stdin_word(&w("riviv /stdin:")));
        assert!(!has_stdin_word(&w("riviv stdin.png")));
        assert!(!has_stdin_word(&w("riviv")));
    }
}
