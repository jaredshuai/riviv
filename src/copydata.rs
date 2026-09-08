//! The single-instance handoff payload (#21): the wire format of the
//! WM_COPYDATA message a second riviv sends the first before exiting
//! (upstream send viv.c:5278-5341 / receive viv.c:3680-3722 / the string
//! walker `_viv_get_copydata_string` viv.c:7876-7904).
//!
//! Layout (upstream viv.c:5298-5322): a little-endian DWORD show command,
//! then the sender's command line and working directory as NUL-terminated
//! UTF-16. The strings go out at full length; the RECEIVER truncates each
//! to 1023 UTF-16 units (`_viv_get_copydata_string`'s STRING_SIZE buffer,
//! string.h:28) — encode/decode mirror that split exactly.

use windows::core::{PCSTR, s};

/// The single-instance mutex name (upstream `"VOIDIMAGEVIEWER"`,
/// viv.c:5283). Deliberately renamed so riviv and upstream coexist on one
/// machine (README Differences); pairs with the find-class below.
pub(crate) const MUTEX_NAME: PCSTR = s!("RIVIV");

/// The window class the second process searches for (upstream
/// `FindWindowA("VOIDIMAGEVIEWER")`, viv.c:5288) — must equal window.rs's
/// `CLASS_NAME` `"riviv"`; the name is ASCII-only, so the A and W spellings
/// address the same registered class.
pub(crate) const FIND_CLASS: PCSTR = s!("riviv");

/// The WM_COPYDATA payload id for the command-line handoff — upstream's
/// `_VIV_COPYDATA_COMMAND_LINE`, first member of its enum (viv.c:293-294),
/// so the value is 0.
pub(crate) const COPYDATA_COMMAND_LINE: usize = 0;

/// Upstream's `STRING_SIZE` (string.h:28): the receiver's string buffers —
/// the per-string truncation cap (STRING_SIZE-1 units actually copied).
pub(crate) const STRING_SIZE: usize = 1024;

/// One decoded handoff: the second launch's requested show state, its
/// original command line, and its working directory (UTF-16 units, the
/// terminating NULs stripped).
pub(crate) struct CommandLineHandoff {
    pub(crate) show_cmd: u32,
    pub(crate) command_line: Vec<u16>,
    pub(crate) cwd: Vec<u16>,
}

/// Build the wire payload (upstream's size calc + fill, viv.c:5298-5322):
/// DWORD + NUL-terminated command line + NUL-terminated cwd, nothing more.
/// Interior NULs in the inputs are not sanitized — a `GetCommandLineW` /
/// `GetCurrentDirectoryW` result never has one, and decode stops at the
/// first anyway, exactly like upstream's byte-walking receiver.
pub(crate) fn encode(show_cmd: u32, command_line: &[u16], cwd: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + (command_line.len() + 1) * 2 + (cwd.len() + 1) * 2);
    out.extend_from_slice(&show_cmd.to_le_bytes());
    for part in [command_line, cwd] {
        for unit in part {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// Parse the payload (upstream viv.c:3704-3710). `None` covers upstream's
/// "no show command" gate — anything under 4 bytes is handled-and-ignored.
/// The strings decode fail-soft like upstream's walker: a missing
/// terminator ends the string at the payload's end, and each is capped at
/// 1023 units.
pub(crate) fn decode(bytes: &[u8]) -> Option<CommandLineHandoff> {
    let show_cmd = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
    let (command_line, rest) = take_wide_string(&bytes[4..]);
    let (cwd, _) = take_wide_string(rest);
    Some(CommandLineHandoff {
        show_cmd,
        command_line,
        cwd,
    })
}

/// One NUL-terminated UTF-16 string out of `bytes`, plus what follows it
/// (upstream `_viv_get_copydata_string`, viv.c:7876-7904): consume UTF-16
/// units until a NUL (consumed with it) or the end of the data; copy at
/// most STRING_SIZE-1 units (the walker's `bufsize > 1` gate) but always
/// ADVANCE past what was read — an oversized first string truncates without
/// ever swallowing the second. A dangling odd byte is ignored.
fn take_wide_string(bytes: &[u8]) -> (Vec<u16>, &[u8]) {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        let unit = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        i += 2;
        if unit == 0 {
            break;
        }
        if out.len() < STRING_SIZE - 1 {
            out.push(unit);
        }
    }
    (out, &bytes[i..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn unwide(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    #[test]
    fn round_trips_all_three_sections() {
        // SW_MAXIMIZE (3), a quoted exe + quoted path with spaces, a CJK cwd.
        let cl = wide("\"C:\\a b\\riviv.exe\" \"C:\\pic dir\\img 1.png\"");
        let cwd = wide("C:\\用户\\jared");
        let h = decode(&encode(3, &cl, &cwd)).unwrap();
        assert_eq!(h.show_cmd, 3);
        assert_eq!(h.command_line, cl);
        assert_eq!(h.cwd, cwd);
    }

    #[test]
    fn wire_layout_is_dword_then_two_nul_terminated_wide_strings() {
        // Pins the exact byte layout the sender builds (viv.c:5309-5322) —
        // DWORD LE, then each string NUL-terminated UTF-16.
        assert_eq!(
            encode(10, &wide("A"), &wide("B")),
            [10, 0, 0, 0, b'A', 0, 0, 0, b'B', 0, 0, 0]
        );
    }

    #[test]
    fn round_trips_empty_strings() {
        let bytes = encode(1, &[], &[]);
        assert_eq!(bytes.len(), 4 + 2 + 2); // just the two NUL terminators
        let h = decode(&bytes).unwrap();
        assert_eq!(h.show_cmd, 1);
        assert!(h.command_line.is_empty());
        assert!(h.cwd.is_empty());
    }

    #[test]
    fn payload_under_four_bytes_is_none() {
        // Upstream's `e-p >= sizeof(DWORD)` gate (viv.c:3706) — the decoder
        // must not read a show command that is not fully there.
        for n in 0..4 {
            assert!(decode(&[0u8; 4][..n]).is_none(), "len {n}");
        }
    }

    #[test]
    fn four_bare_bytes_decode_to_empty_strings() {
        // DWORD consumed, nothing after: the walker NUL-terminates both
        // buffers at once — empty command line, empty cwd.
        let h = decode(&[7, 0, 0, 0]).unwrap();
        assert_eq!(h.show_cmd, 7);
        assert!(h.command_line.is_empty());
        assert!(h.cwd.is_empty());
    }

    #[test]
    fn missing_terminator_reads_to_the_end() {
        // A truncated payload (sender died mid-send is impossible over
        // WM_COPYDATA's copy, but a foreign/malformed one can be short):
        // the string ends at the data's end, cwd stays empty.
        let bytes = encode(2, &wide("abc"), &wide("cwd"));
        let h = decode(&bytes[..4 + 6]).unwrap();
        assert_eq!(unwide(&h.command_line), "abc");
        assert!(h.cwd.is_empty());
    }

    #[test]
    fn oversized_first_string_truncates_but_never_swallows_the_second() {
        // STRING_SIZE-1 = 1023 copied units (the bufsize>1 gate), but the
        // walk advanced past all 1500 — the cwd parses intact after it
        // (upstream's advance-then-copy split, viv.c:7885-7897).
        let long = vec![0x4E2D; 1500]; // 中 × 1500
        let h = decode(&encode(4, &long, &wide("cwd"))).unwrap();
        assert_eq!(h.command_line.len(), STRING_SIZE - 1);
        assert_eq!(h.command_line, vec![0x4E2D; STRING_SIZE - 1]);
        assert_eq!(unwide(&h.cwd), "cwd");
    }

    #[test]
    fn interior_nul_ends_the_string_like_the_receiver_would() {
        // A malformed payload whose cl terminator is really a character:
        // everything up to the NEXT NUL is the cl, the cwd comes up empty —
        // the same byte walk upstream's receiver does.
        let mut bytes = encode(4, &wide("AB"), &wide("CD"));
        let cl_nul_at = 4 + 2 * 2; // byte offset of cl's NUL terminator
        bytes[cl_nul_at] = b'E';
        bytes[cl_nul_at + 1] = 0;
        let h = decode(&bytes).unwrap();
        assert_eq!(unwide(&h.command_line), "ABECD");
        assert!(h.cwd.is_empty());
    }

    #[test]
    fn odd_trailing_byte_is_ignored() {
        // The walker consumes whole UTF-16 units only (`p + sizeof(wchar_t)
        // <= e`, viv.c:7881) — a dangling byte never enters either string.
        let mut bytes = encode(4, &wide("A"), &wide("B"));
        bytes.push(0xFF);
        let h = decode(&bytes).unwrap();
        assert_eq!(unwide(&h.command_line), "A");
        assert_eq!(unwide(&h.cwd), "B");
    }
}
