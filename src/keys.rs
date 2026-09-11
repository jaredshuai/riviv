//! Custom keyboard shortcuts (#25): the per-command key-binding model
//! (`_viv_key_list`, viv.c:1127-1134), its ini persistence, and the pure
//! halves of the Options Controls-page editor.
//!
//! Upstream keeps one OWNED linked list of `config_key_t` per command
//! (`WORD key` = modifiers<<8 | VK), seeds it from `_viv_default_keys[]`
//! (viv.c:969-1129) at init, overlays the ini (`config.c:180-222`: for
//! every command with a `*_keys` line, clear-then-rebuild from the
//! comma-separated ints), and saves it back unconditionally — an empty
//! list writes an empty line (config.c:352-383). The keyboard path walks
//! the lists in command order, first exact modifier+VK match wins
//! (viv.c:6353-6406). riviv keeps the same shape with `Vec`s; the ini
//! key NAMES are regenerated with upstream's filter so the files are
//! spelling-compatible (`viv_menu_name_to_ini_name`, viv.c:12127-12165).
//!
//! Deviations the router fixes on the way (both toward upstream, see the
//! equivalence test): the old hardcoded router answered Ctrl+Alt+Shift+O
//! as Add File (upstream's exact-mask rule says no command) and did NOT
//! answer the bare O behind Options (upstream registers it).

use crate::ini;
use crate::loc;
use crate::menu::{self, Cmd, KeyDef, Slot};

/// `CONFIG_KEYFLAG_*` (config.h:37-41): the wire format of one binding —
/// modifiers in the high byte, the VK in the low one.
pub(crate) const FLAG_CTRL: u16 = 0x0100;
pub(crate) const FLAG_SHIFT: u16 = 0x0200;
pub(crate) const FLAG_ALT: u16 = 0x0400;
pub(crate) const FLAG_VK_MASK: u16 = 0x00ff;

// Win32 virtual-key codes used by the default keys (WinUser.h; raw values
// keep this module pure-logic with no windows crate dependency).
const VK_RETURN: u16 = 0x0d;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_NEXT: u16 = 0x22;
const VK_PRIOR: u16 = 0x21;
const VK_F1: u16 = 0x70;
const VK_ADD: u16 = 0x6b;
const VK_SUBTRACT: u16 = 0x6d;
const VK_OEM_PLUS: u16 = 0xbb;
const VK_OEM_MINUS: u16 = 0xbd;

/// The chord helper for table rows: modifiers plus a VK.
const fn key(ctrl: bool, alt: bool, shift: bool, vk: u16) -> KeyDef {
    KeyDef {
        ctrl,
        alt,
        shift,
        vk,
    }
}

/// `KeyDef` ⇄ the ini/wire form (`key->key`). The VK keeps only its low
/// byte on the way in — upstream's VK mask is 0xff, so a hand-edited
/// `305` (Ctrl | 0x31) is Ctrl+'1' exactly like upstream reads it.
pub(crate) fn to_flags(k: KeyDef) -> u16 {
    ((k.ctrl as u16) * FLAG_CTRL)
        | ((k.alt as u16) * FLAG_ALT)
        | ((k.shift as u16) * FLAG_SHIFT)
        | (k.vk & FLAG_VK_MASK)
}

pub(crate) fn from_flags(flags: u16) -> KeyDef {
    KeyDef {
        ctrl: flags & FLAG_CTRL != 0,
        alt: flags & FLAG_ALT != 0,
        shift: flags & FLAG_SHIFT != 0,
        vk: flags & FLAG_VK_MASK,
    }
}

/// The default bindings (upstream `_viv_default_keys[]`, viv.c:969-1129,
/// pruned to riviv's implemented commands, upstream row order). Multiple
/// rows per command are the registered alternates — upstream's Zoom In
/// owns `+`, numpad `+` AND Ctrl+numpad `+` (viv.c:1017-1024), Next owns
/// Right and PgDn (viv.c:1040-1045). Commands with no row here have no
/// default binding (upstream registers none for Menu / Best Fit either).
const DEFAULT_KEYS: &[(Cmd, &[KeyDef])] = &[
    (Cmd::FileOpenFile, &[key(true, false, false, b'O' as u16)]),
    (Cmd::FileOpenFolder, &[key(true, false, false, b'B' as u16)]),
    (
        Cmd::FileOpenEverythingSearch,
        &[key(true, false, false, b'E' as u16)],
    ),
    (Cmd::FileAddFile, &[key(true, false, true, b'O' as u16)]),
    (
        Cmd::FileAddEverythingSearch,
        &[key(true, false, true, b'E' as u16)],
    ),
    (Cmd::FileExit, &[key(true, false, false, b'Q' as u16)]),
    (Cmd::ViewOneToOne, &[key(true, true, false, b'0' as u16)]),
    (Cmd::ViewFullscreen, &[key(false, true, false, VK_RETURN)]),
    (
        Cmd::ViewZoomIn,
        &[
            key(false, false, false, VK_OEM_PLUS),
            key(false, false, false, VK_ADD),
            key(true, false, false, VK_ADD),
        ],
    ),
    (
        Cmd::ViewZoomOut,
        &[
            key(false, false, false, VK_OEM_MINUS),
            key(false, false, false, VK_SUBTRACT),
            key(true, false, false, VK_SUBTRACT),
        ],
    ),
    (Cmd::ViewZoomReset, &[key(true, false, false, b'0' as u16)]),
    (Cmd::ViewOptions, &[key(false, false, false, b'O' as u16)]),
    (
        Cmd::NavNext,
        &[
            key(false, false, false, VK_RIGHT),
            key(false, false, false, VK_NEXT),
        ],
    ),
    (
        Cmd::NavPrev,
        &[
            key(false, false, false, VK_LEFT),
            key(false, false, false, VK_PRIOR),
        ],
    ),
    (Cmd::NavHome, &[key(false, false, false, VK_HOME)]),
    (Cmd::NavEnd, &[key(false, false, false, VK_END)]),
    (Cmd::HelpAbout, &[key(true, false, false, VK_F1)]),
];

/// Every command's binding list. `Default` IS the upstream default table
/// (the ini overlay only replaces commands whose `*_keys` line exists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyMap {
    per_cmd: Vec<Vec<KeyDef>>,
}

impl Default for KeyMap {
    fn default() -> Self {
        let mut per_cmd = vec![Vec::new(); Cmd::COUNT];
        for (cmd, keys) in DEFAULT_KEYS {
            per_cmd[usize::from(cmd.id() - 1)] = keys.to_vec();
        }
        KeyMap { per_cmd }
    }
}

impl KeyMap {
    /// The bindings registered for `cmd`, in registration order (the ini
    /// order / default-table order — lookup prefers the earlier command,
    /// not the earlier key).
    pub(crate) fn keys(&self, cmd: Cmd) -> &[KeyDef] {
        &self.per_cmd[usize::from(cmd.id() - 1)]
    }

    /// The first registered binding (the one whose label the menu shows —
    /// upstream `_viv_key_list->start[...]`, viv.c:12365-12367).
    pub(crate) fn first(&self, cmd: Cmd) -> Option<KeyDef> {
        self.keys(cmd).first().copied()
    }

    /// The keyboard route (upstream viv.c:6390-6405): walk the commands in
    /// table order, each command's keys in order; the first EXACT
    /// modifier+VK match wins. A chord bound to two commands belongs to
    /// whichever command registers earlier in `Cmd::ALL` — the same
    /// first-match rule upstream's scan implements.
    pub(crate) fn lookup(&self, ctrl: bool, alt: bool, shift: bool, vk: u16) -> Option<Cmd> {
        let pressed = KeyDef {
            ctrl,
            alt,
            shift,
            vk,
        };
        Cmd::ALL
            .into_iter()
            .find(|cmd| self.keys(*cmd).contains(&pressed))
    }

    /// Replace `cmd`'s bindings wholesale (the ini overlay: upstream
    /// `viv_key_clear_all(i)` then `viv_key_add` per token, config.c:190-217).
    pub(crate) fn set(&mut self, cmd: Cmd, keys: Vec<KeyDef>) {
        self.per_cmd[usize::from(cmd.id() - 1)] = keys;
    }

    /// Append one binding (the Add path — upstream `_viv_key_add` to the
    /// list tail, viv.c:12384-12403).
    pub(crate) fn add(&mut self, cmd: Cmd, k: KeyDef) {
        self.per_cmd[usize::from(cmd.id() - 1)].push(k);
    }

    /// Replace the binding at `index` (the Edit path — upstream walks the
    /// list to the index and overwrites `key->key`, viv.c:8207-8221).
    /// An out-of-range index is a no-op (upstream's walk just ends).
    pub(crate) fn replace(&mut self, cmd: Cmd, index: usize, k: KeyDef) {
        if let Some(slot) = self.per_cmd[usize::from(cmd.id() - 1)].get_mut(index) {
            *slot = k;
        }
    }

    /// Remove every binding equal to `k` from `cmd` (upstream
    /// `_viv_key_remove`, viv.c:12611-12642 — it filters by VALUE, so a
    /// duplicate chord on one command vanishes in one press).
    pub(crate) fn remove(&mut self, cmd: Cmd, k: KeyDef) {
        self.per_cmd[usize::from(cmd.id() - 1)].retain(|other| *other != k);
    }

    /// Remove `k` from EVERY command — the edit dialog's ownership rule
    /// (`_viv_edit_key_remove_currently_used_by`, viv.c:12644-12651): a
    /// chord newly assigned to one command stops belonging to any other.
    pub(crate) fn remove_all(&mut self, k: KeyDef) {
        for list in &mut self.per_cmd {
            list.retain(|other| *other != k);
        }
    }

    /// The commands currently bound to `k` (the edit dialog's
    /// "currently used by" list, `_viv_options_edit_key_changed`,
    /// viv.c:12555-12584).
    pub(crate) fn owners(&self, k: KeyDef) -> Vec<Cmd> {
        Cmd::ALL
            .into_iter()
            .filter(|cmd| self.keys(*cmd).contains(&k))
            .collect()
    }

    /// The ini value for `cmd` (upstream config.c:352-383): the binding
    /// flags as decimal ints, comma-separated; an EMPTY list writes an
    /// empty line — present-but-empty clears the command's bindings.
    pub(crate) fn to_ini(&self, cmd: Cmd) -> String {
        let parts: Vec<String> = self
            .keys(cmd)
            .iter()
            .map(|k| i32::from(to_flags(*k)).to_string())
            .collect();
        parts.join(",")
    }

    /// Parse one `*_keys` ini value into bindings (upstream's scan,
    /// config.c:193-217): split at commas, every token through
    /// `utf8_to_int` (garbage = 0, partial parse keeps the prefix) and
    /// appended — so `"78,,"` is [78, 0-key] but `"78,"` is just [78]:
    /// the comma only opens a new token when content follows.
    pub(crate) fn parse_ini(value: &str) -> Vec<KeyDef> {
        let mut parts: Vec<&str> = value.split(',').collect();
        if parts.last() == Some(&"") {
            parts.pop();
        }
        parts
            .iter()
            .map(|t| from_flags(ini::parse_int(t) as u16))
            .collect()
    }

    /// Apply one `*_keys` line read from the ini (present ⇒ replace;
    /// the caller already knows the line exists — upstream's
    /// `if (key_list)` guard, config.c:183-185).
    pub(crate) fn apply_ini(&mut self, cmd: Cmd, value: &str) {
        self.set(cmd, Self::parse_ini(value));
    }
}

/// One character of an ini key name (upstream
/// `_viv_convert_menu_ini_name_ch`, viv.c:12048-12068): A-Z → a-z, a-z and
/// 0-9 kept, space → '_', everything else (mnemonic '&', punctuation)
/// dropped.
fn ini_name_char(ch: char) -> Option<char> {
    match ch {
        'A'..='Z' => Some(ch.to_ascii_lowercase()),
        'a'..='z' | '0'..='9' => Some(ch),
        ' ' => Some('_'),
        _ => None,
    }
}

/// The English menu caption of a slot path segment, filtered for an ini
/// key (upstream builds the path recursively then the command's own name,
/// each parent ending in '_', the whole ending in "_keys",
/// `viv_menu_name_to_ini_name` + `_viv_cat_command_menu_ini_name_path`,
/// viv.c:12075-12165). English regardless of the UI language — upstream
/// reads the en-US table directly so the ini names are stable.
pub(crate) fn ini_name(cmd: Cmd) -> String {
    let mut name = String::new();
    push_slot_path(&mut name, parent_slot_of(cmd));
    for ch in loc::get_for(loc::Language::English, command_loc(cmd)).chars() {
        if let Some(c) = ini_name_char(ch) {
            name.push(c);
        }
    }
    name.push_str("_keys");
    name
}

/// The parent-slot chain, root first, each name filtered + '_'-ended.
fn push_slot_path(name: &mut String, slot: Slot) {
    if slot == Slot::Root {
        return;
    }
    let (parent, loc_id) = popup_of(slot);
    push_slot_path(name, parent);
    for ch in loc::get_for(loc::Language::English, loc_id).chars() {
        if let Some(c) = ini_name_char(ch) {
            name.push(c);
        }
    }
    name.push('_');
}

/// The menu entry of `cmd` (its caption + parent slot). Every command has
/// exactly one row (the menu table's tested invariant).
fn entry_of(cmd: Cmd) -> menu::Entry {
    *menu::ENTRIES
        .iter()
        .find(|e| matches!(e, menu::Entry::Item { cmd: c, .. } if *c == cmd))
        .expect("every command has a menu row")
}

fn command_loc(cmd: Cmd) -> loc::Id {
    match entry_of(cmd) {
        menu::Entry::Item { loc, .. } => loc,
        _ => unreachable!("entry_of only returns Item rows"),
    }
}

fn parent_slot_of(cmd: Cmd) -> Slot {
    match entry_of(cmd) {
        menu::Entry::Item { parent, .. } => parent,
        _ => unreachable!(),
    }
}

/// The popup row that introduces `slot` (its caption + parent slot).
fn popup_of(slot: Slot) -> (Slot, loc::Id) {
    match menu::ENTRIES
        .iter()
        .find(|e| matches!(e, menu::Entry::Popup { slot: s, .. } if *s == slot))
    {
        Some(menu::Entry::Popup { loc, parent, .. }) => (*parent, *loc),
        _ => (Slot::Root, loc::Id::MenuFile), // unreachable for declared slots
    }
}

/// Strip '&' mnemonics the way the command list displays names (upstream
/// `_viv_get_menu_display_name`, viv.c:12167-12201: '&&' renders one '&',
/// a lone '&' is the mnemonic and disappears).
fn strip_mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            // '&&' collapses to one '&'; a lone '&' is the mnemonic and
            // disappears. The peek keeps the char AFTER a lone '&' —
            // consuming it would eat "E&xit"'s x.
            if chars.clone().next() == Some('&') {
                chars.next();
                out.push('&');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The command's display name for the Controls page list (upstream
/// `_viv_get_command_name`, viv.c:12022-12033): the parent-menu path in
/// UI language, ' | '-separated, then the item caption —
/// "File | Exit".
pub(crate) fn command_display_name(cmd: Cmd) -> String {
    let mut name = String::new();
    push_display_path(&mut name, parent_slot_of(cmd));
    name.push_str(&strip_mnemonic(loc::get(command_loc(cmd))));
    name
}

fn push_display_path(name: &mut String, slot: Slot) {
    if slot == Slot::Root {
        return;
    }
    let (parent, loc_id) = popup_of(slot);
    push_display_path(name, parent);
    name.push_str(&strip_mnemonic(loc::get(loc_id)));
    name.push_str(" | ");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(ctrl: bool, alt: bool, shift: bool, vk: u16) -> KeyDef {
        KeyDef {
            ctrl,
            alt,
            shift,
            vk,
        }
    }

    #[test]
    fn flags_round_trip_the_upstream_wire_format() {
        // config.h:37-41 — Ctrl 0x100, Shift 0x200, Alt 0x400, VK low
        // byte. Ctrl+Shift+O = 0x300 | 0x4F = 0x34F = 847.
        assert_eq!(to_flags(k(true, false, true, b'O' as u16)), 0x34f);
        assert_eq!(from_flags(0x34f), k(true, false, true, b'O' as u16));
        assert_eq!(from_flags(0x00ff), k(false, false, false, 0xff));
        assert_eq!(to_flags(k(false, false, false, 0)), 0);
        // A VK above the mask keeps only its low byte, exactly like
        // upstream's WORD storage would hand to the compare.
        assert_eq!(to_flags(k(false, false, false, 0x1bb)), 0xbb);
    }

    #[test]
    fn default_keys_match_the_upstream_table() {
        // viv.c:969-1129, the rows of every command riviv implements.
        // Order inside a command is the registration order (matters for
        // the menu label: first key wins).
        let m = KeyMap::default();
        let vks = |cmd: Cmd| m.keys(cmd).to_vec();
        assert_eq!(
            vks(Cmd::FileOpenFile),
            vec![k(true, false, false, b'O' as u16)]
        );
        assert_eq!(
            vks(Cmd::ViewZoomIn),
            vec![
                k(false, false, false, 0xbb),
                k(false, false, false, 0x6b),
                k(true, false, false, 0x6b),
            ]
        );
        assert_eq!(
            vks(Cmd::ViewZoomOut),
            vec![
                k(false, false, false, 0xbd),
                k(false, false, false, 0x6d),
                k(true, false, false, 0x6d),
            ]
        );
        assert_eq!(
            vks(Cmd::NavNext),
            vec![k(false, false, false, 0x27), k(false, false, false, 0x22)]
        );
        assert_eq!(
            vks(Cmd::NavPrev),
            vec![k(false, false, false, 0x25), k(false, false, false, 0x21)]
        );
        // No default binding (absent from upstream's table).
        assert!(vks(Cmd::ViewMenu).is_empty());
        assert!(vks(Cmd::ViewBestFit).is_empty());
        // Every command has a slot in the map.
        assert_eq!(m.per_cmd.len(), Cmd::COUNT);
    }

    #[test]
    fn default_bindings_are_globally_unique() {
        // One chord must not route to two commands — upstream's scan
        // would hand it to whichever command sits earlier in the table,
        // so the shipped table never registers a duplicate (the same
        // guarantee menu.rs' table test used to pin for the label keys).
        let m = KeyMap::default();
        let mut seen: Vec<KeyDef> = Vec::new();
        for cmd in Cmd::ALL {
            for key in m.keys(cmd) {
                assert!(!seen.contains(key), "{key:?} bound twice");
                seen.push(*key);
            }
        }
    }

    #[test]
    fn lookup_matches_the_exact_chord_and_first_command_wins() {
        let mut m = KeyMap::default();
        // Every default binding routes home.
        for cmd in Cmd::ALL {
            for key in m.keys(cmd) {
                assert_eq!(
                    m.lookup(key.ctrl, key.alt, key.shift, key.vk),
                    Some(cmd),
                    "{key:?} should route to {cmd:?}"
                );
            }
        }
        // Modifier masks are exact (viv.c:6396-6402): Ctrl+Alt+O is NOT
        // Open File, Shift+Right is NOT Next.
        assert_eq!(m.lookup(true, true, false, b'O' as u16), None);
        assert_eq!(m.lookup(false, false, true, 0x27), None);
        // An unbound key stays unrouted.
        assert_eq!(m.lookup(false, false, false, b'Z' as u16), None);
        // A chord bound to two commands goes to the earlier command —
        // NavPrev sits before NavEnd in the table, so a duplicate of
        // End under Prev reroutes it.
        m.add(Cmd::NavPrev, k(false, false, false, 0x23));
        assert_eq!(m.lookup(false, false, false, 0x23), Some(Cmd::NavPrev));
    }

    #[test]
    fn router_equivalence_against_the_hardcoded_keydown() {
        // The regression the routing swap owes: with the DEFAULT table,
        // every chord the old hardcoded on_keydown dispatched still lands
        // on the same command. Two deliberate fixes toward upstream are
        // pinned here too: Ctrl+Alt+Shift+O used to leak into Add File
        // (the old code only tested ctrl && !alt before consulting
        // shift) — upstream's exact mask says no command; and the bare O
        // behind Options now answers (upstream registers it; the old
        // router never did — the menu showed the label all along).
        let m = KeyMap::default();
        // The old router's dispatched set. lookup(ctrl, alt, shift, vk).
        assert_eq!(
            m.lookup(true, false, false, b'O' as u16),
            Some(Cmd::FileOpenFile)
        );
        assert_eq!(
            m.lookup(true, false, true, b'O' as u16),
            Some(Cmd::FileAddFile)
        );
        assert_eq!(
            m.lookup(true, false, false, b'B' as u16),
            Some(Cmd::FileOpenFolder)
        );
        assert_eq!(
            m.lookup(true, false, false, b'Q' as u16),
            Some(Cmd::FileExit)
        );
        assert_eq!(
            m.lookup(false, true, false, 0x0d),
            Some(Cmd::ViewFullscreen)
        );
        assert_eq!(
            m.lookup(true, true, false, b'0' as u16),
            Some(Cmd::ViewOneToOne)
        );
        assert_eq!(
            m.lookup(true, false, false, b'0' as u16),
            Some(Cmd::ViewZoomReset)
        );
        assert_eq!(m.lookup(false, false, false, 0xbb), Some(Cmd::ViewZoomIn));
        assert_eq!(m.lookup(false, false, false, 0x6b), Some(Cmd::ViewZoomIn));
        assert_eq!(m.lookup(true, false, false, 0x6b), Some(Cmd::ViewZoomIn));
        assert_eq!(m.lookup(false, false, false, 0xbd), Some(Cmd::ViewZoomOut));
        assert_eq!(m.lookup(false, false, false, 0x6d), Some(Cmd::ViewZoomOut));
        assert_eq!(m.lookup(true, false, false, 0x6d), Some(Cmd::ViewZoomOut));
        assert_eq!(m.lookup(false, false, false, 0x27), Some(Cmd::NavNext));
        assert_eq!(m.lookup(false, false, false, 0x22), Some(Cmd::NavNext));
        assert_eq!(m.lookup(false, false, false, 0x25), Some(Cmd::NavPrev));
        assert_eq!(m.lookup(false, false, false, 0x21), Some(Cmd::NavPrev));
        assert_eq!(m.lookup(false, false, false, 0x24), Some(Cmd::NavHome));
        assert_eq!(m.lookup(false, false, false, 0x23), Some(Cmd::NavEnd));
        assert_eq!(m.lookup(true, false, false, 0x70), Some(Cmd::HelpAbout));
        // The two fixes: upstream semantics, not the old quirks.
        assert_eq!(
            m.lookup(true, true, true, b'O' as u16),
            None,
            "old router leaked this into Add File; upstream says no command"
        );
        assert_eq!(
            m.lookup(false, false, false, b'O' as u16),
            Some(Cmd::ViewOptions),
            "old router ignored the Options key; upstream registers it"
        );
        // And the chords the old router deliberately swallowed as no-ops
        // (its trailing any-modifier bail before navigation) stay no-ops.
        assert_eq!(m.lookup(true, false, false, 0x27), None);
        assert_eq!(m.lookup(false, false, true, 0x24), None);
    }

    #[test]
    fn ini_value_round_trips() {
        let mut m = KeyMap::default();
        assert_eq!(m.to_ini(Cmd::FileOpenFile), "335"); // Ctrl+'O' = 0x14F
        assert_eq!(m.to_ini(Cmd::NavNext), "39,34"); // Right, PgDn
        assert_eq!(m.to_ini(Cmd::ViewMenu), ""); // no bindings
        m.apply_ini(Cmd::FileOpenFile, "78"); // bare 'N'
        assert_eq!(m.keys(Cmd::FileOpenFile), &[k(false, false, false, 78)]);
        // Empty value clears (upstream clears then adds zero keys).
        m.apply_ini(Cmd::FileOpenFile, "");
        assert!(m.keys(Cmd::FileOpenFile).is_empty());
        // Multi-token rebuild replaces, not appends.
        m.apply_ini(Cmd::NavNext, "78,79");
        assert_eq!(
            m.keys(Cmd::NavNext),
            &[k(false, false, false, 78), k(false, false, false, 79)]
        );
    }

    #[test]
    fn ini_value_parsing_matches_the_upstream_scan() {
        // config.c:193-217: a token only exists where content follows the
        // comma — "78," is [78], ",78" is [0, 78] (the empty token is a
        // real 0-key, harmless upstream and here), garbage parses to 0
        // through utf8_to_int.
        assert_eq!(KeyMap::parse_ini("78,"), vec![k(false, false, false, 78)]);
        assert_eq!(
            KeyMap::parse_ini(",78"),
            vec![k(false, false, false, 0), k(false, false, false, 78)]
        );
        assert_eq!(
            KeyMap::parse_ini("78,abc"),
            vec![k(false, false, false, 78), k(false, false, false, 0)]
        );
        assert!(KeyMap::parse_ini("").is_empty());
        // 0x-hex parses like the rest of the ini ints.
        assert_eq!(
            KeyMap::parse_ini("0x16f"),
            vec![k(true, false, false, 0x6f)]
        );
    }

    #[test]
    fn edit_operations_match_the_dialog_semantics() {
        let mut m = KeyMap::default();
        // Add appends (viv.c:12384-12403).
        m.add(Cmd::FileExit, k(false, false, false, b'X' as u16));
        assert_eq!(m.keys(Cmd::FileExit).len(), 2);
        // Replace by index (viv.c:8207-8221); out of range is a no-op.
        m.replace(Cmd::FileExit, 1, k(false, false, false, b'Z' as u16));
        assert_eq!(m.keys(Cmd::FileExit)[1].vk, u16::from(b'Z'));
        m.replace(Cmd::FileExit, 9, k(false, false, false, b'A' as u16));
        assert_eq!(m.keys(Cmd::FileExit).len(), 2);
        // Ownership: assigning a chord anywhere strips it from every
        // other command first (viv.c:12644-12651).
        let q = k(true, false, false, b'Q' as u16);
        assert_eq!(m.owners(q), vec![Cmd::FileExit]);
        m.add(Cmd::HelpAbout, q);
        assert_eq!(m.owners(q), vec![Cmd::FileExit, Cmd::HelpAbout]);
        m.remove_all(q);
        assert!(m.owners(q).is_empty());
        // Remove filters by value (viv.c:12611-12642).
        m.add(Cmd::FileExit, k(false, false, false, b'X' as u16)); // duplicate
        m.remove(Cmd::FileExit, k(false, false, false, b'X' as u16));
        assert_eq!(m.keys(Cmd::FileExit).len(), 1); // both copies gone
    }

    #[test]
    fn ini_names_pin_the_upstream_spellings() {
        // Generated with upstream's filter (viv.c:12048-12165) over the
        // en-US captions: lowercase alphanumerics, spaces as '_', parents
        // 'a'-'z' too, mnemonics and punctuation dropped, "_keys" tail.
        // "1:1" loses its colon → view_11_keys; the zoom items nest two
        // levels → view_zoom_zoom_in_keys.
        let expect = [
            (Cmd::FileOpenFile, "file_open_file_keys"),
            (Cmd::FileOpenFolder, "file_open_folder_keys"),
            (
                Cmd::FileOpenEverythingSearch,
                "file_open_everything_search_keys",
            ),
            (Cmd::FileAddFile, "file_add_file_keys"),
            (
                Cmd::FileAddEverythingSearch,
                "file_add_everything_search_keys",
            ),
            (Cmd::FileExit, "file_exit_keys"),
            (Cmd::ViewMenu, "view_menu_keys"),
            (Cmd::ViewFullscreen, "view_fullscreen_keys"),
            (Cmd::ViewOneToOne, "view_11_keys"),
            (Cmd::ViewBestFit, "view_best_fit_keys"),
            (Cmd::ViewZoomIn, "view_zoom_zoom_in_keys"),
            (Cmd::ViewZoomOut, "view_zoom_zoom_out_keys"),
            (Cmd::ViewZoomReset, "view_zoom_reset_keys"),
            (Cmd::ViewOptions, "view_options_keys"),
            (Cmd::NavNext, "navigate_next_keys"),
            (Cmd::NavPrev, "navigate_previous_keys"),
            (Cmd::NavHome, "navigate_home_keys"),
            (Cmd::NavEnd, "navigate_end_keys"),
            (Cmd::HelpAbout, "help_about_keys"),
        ];
        for (cmd, name) in expect {
            assert_eq!(ini_name(cmd), name);
        }
    }

    #[test]
    fn display_names_carry_the_menu_path_without_mnemonics() {
        // _viv_get_command_name (viv.c:12022-12033): parents ' | '-joined
        // in UI language, mnemonic '&'s stripped ("E&xit" → "Exit").
        // Unit tests see English (loc::init never ran).
        assert_eq!(command_display_name(Cmd::FileExit), "File | Exit");
        assert_eq!(
            command_display_name(Cmd::ViewZoomIn),
            "View | Zoom | Zoom In"
        );
        assert_eq!(command_display_name(Cmd::NavEnd), "Navigate | End");
    }

    #[test]
    fn mnemonic_stripping_follows_the_upstream_pairs_rule() {
        // '&&' renders one '&'; a lone '&' vanishes (viv.c:12167-12201).
        assert_eq!(strip_mnemonic("E&xit"), "Exit");
        assert_eq!(strip_mnemonic("Pa&&n"), "Pa&n");
        assert_eq!(strip_mnemonic("1:1"), "1:1");
        assert_eq!(strip_mnemonic("&File"), "File");
    }
}
