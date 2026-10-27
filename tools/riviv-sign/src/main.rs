//! riviv-sign — the release signing tool (#237; ADR 0009).
//!
//! The release ritual's three commands over the SAME domain-separated
//! message the shipped client verifies:
//!
//! - `riviv-sign keygen <seed-file>` — generate the 32 B private seed
//!   (OS randomness via ProcessPrng) and print the matching public key.
//!   The seed file NEVER enters the repository: keep it offline in two
//!   copies (RELEASE.md ceremony).
//! - `riviv-sign sign <seed-file> <tag> <asset-file>` — write
//!   `<asset-file>.sig` (raw 64 B) over the domain-separated message.
//! - `riviv-sign verify <pubkey-hex> <tag> <asset-file> <sig-file>` —
//!   the release re-verification step (and the round-trip sentinel).
//!
//! The signed message is `"riviv-update-v1\n" + <tag> + "\n" +
//! <asset-file-name> + "\n" + <asset bytes>` — the domain separation that
//! binds a signature to one (tag, asset) pair, closing the
//! downgrade-replay and wrong-package substitution holes (spike #235,
//! cubic #236 P1#3). This formula is DUPLICATED in `src/sig.rs` by
//! design (the tool must not share a build graph with the ship exe);
//! the main crate's cross-implementation test vector pins the two
//! copies together, and the release ritual's verify step is the
//! end-to-end sentinel.
//!
//! Independent project on purpose: making this the main crate's second
//! `[[bin]]` would let cargo's feature unification leak the signing-side
//! features into the ship build (spike #235).

use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use ed25519_compact::{KeyPair, PublicKey, Seed, Signature};

/// The domain-separation magic + version (must match `src/sig.rs`).
const MAGIC: &str = "riviv-update-v1";

fn die(msg: &str) -> ! {
    eprintln!("riviv-sign: {msg}");
    std::process::exit(2);
}

fn usage() -> ! {
    die(
        "usage:\n  riviv-sign keygen <seed-file>\n  riviv-sign sign <seed-file> <tag> <asset-file>\n  riviv-sign verify <pubkey-hex> <tag> <asset-file> <sig-file>",
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    // Byte-wise pairs, never char-indexed slicing: a non-ASCII input
    // like "1é2" would otherwise panic on a UTF-8 char boundary
    // instead of dying with the invalid-hex message (review #238).
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        die("hex string has an odd length");
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    pairs
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap_or("zz");
            u8::from_str_radix(pair, 16).unwrap_or_else(|_| die("hex string has a non-hex byte"))
        })
        .collect()
}

/// The domain-separated signed message. THE formula — the single most
/// load-bearing function in this tool; `src/sig.rs::signed_message` must
/// stay byte-identical.
fn signed_message(tag: &str, asset_name: &str, asset: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(MAGIC.len() + tag.len() + asset_name.len() + asset.len() + 3);
    msg.extend_from_slice(MAGIC.as_bytes());
    msg.push(b'\n');
    msg.extend_from_slice(tag.as_bytes());
    msg.push(b'\n');
    msg.extend_from_slice(asset_name.as_bytes());
    msg.push(b'\n');
    msg.extend_from_slice(asset);
    msg
}

fn read_exact_32(path: &Path) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let mut f =
        std::fs::File::open(path).unwrap_or_else(|e| die(&format!("open {}: {e}", path.display())));
    f.read_exact(&mut buf)
        .unwrap_or_else(|e| die(&format!("read {}: {e}", path.display())));
    // Drain check: a seed file with trailing bytes is a different seed
    // than intended — refuse rather than silently truncate.
    let mut extra = [0u8; 1];
    if f.read(&mut extra).unwrap_or(0) != 0 {
        die(&format!(
            "{} is larger than 32 B — not a riviv seed file",
            path.display()
        ));
    }
    buf
}

fn cmd_keygen(seed_out: &Path) {
    let mut seed = [0u8; 32];
    // SAFETY: seed is a live 32 B stack buffer; ProcessPrng fills it with
    // OS randomness (bcryptprimitives.dll, Win10+ documented entry).
    let ok = unsafe { windows::Win32::Security::Cryptography::ProcessPrng(&mut seed) };
    if !ok.as_bool() {
        die("ProcessPrng failed");
    }
    let kp = KeyPair::from_seed(Seed::new(seed));
    // create_new: refuse to clobber — a re-run of the ceremony command
    // must never silently overwrite (destroy) an existing production
    // seed (review #238 cubic P1).
    let mut f = std::fs::File::create_new(seed_out).unwrap_or_else(|e| {
        die(&format!(
            "create {} (exists already? refusing to overwrite): {e}",
            seed_out.display()
        ))
    });
    f.write_all(&seed)
        .unwrap_or_else(|e| die(&format!("write {}: {e}", seed_out.display())));
    println!("pubkey {}", hex(kp.pk.as_ref()));
    println!(
        "seed   {} ({} B) — keep offline in two copies, never commit",
        seed_out.display(),
        seed.len()
    );
}

fn cmd_sign(seed_file: &Path, tag: &str, asset_file: &Path) {
    let seed = read_exact_32(seed_file);
    let kp = KeyPair::from_seed(Seed::new(seed));
    let asset = std::fs::read(asset_file)
        .unwrap_or_else(|e| die(&format!("read {}: {e}", asset_file.display())));
    // The asset NAME inside the message is the file's own name — the
    // ritual signs the file where it sits under its release name.
    let asset_name = asset_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| {
            die("asset file name is not valid UTF-8 — the release asset names are ASCII")
        });
    let msg = signed_message(tag, asset_name, &asset);
    let sig = kp.sk.sign(&msg, None);
    let sig_path: PathBuf = {
        let mut p = asset_file.as_os_str().to_os_string();
        p.push(".sig");
        PathBuf::from(p)
    };
    std::fs::write(&sig_path, sig.as_ref())
        .unwrap_or_else(|e| die(&format!("write {}: {e}", sig_path.display())));
    println!(
        "signed {} ({} B) tag={tag} -> {} (64 B)",
        asset_file.display(),
        asset.len(),
        sig_path.display()
    );
}

fn cmd_verify(pk_hex: &str, tag: &str, asset_file: &Path, sig_file: &Path) {
    let pk_bytes: [u8; 32] = {
        let v = unhex(pk_hex);
        let n = v.len();
        v.try_into()
            .unwrap_or_else(|_| die(&format!("pubkey must be 32 B (64 hex chars), got {n} B")))
    };
    let sig_bytes: [u8; 64] = {
        let v = std::fs::read(sig_file)
            .unwrap_or_else(|e| die(&format!("read {}: {e}", sig_file.display())));
        let n = v.len();
        v.try_into()
            .unwrap_or_else(|_| die(&format!("signature must be raw 64 B, got {n} B")))
    };
    let asset = std::fs::read(asset_file)
        .unwrap_or_else(|e| die(&format!("read {}: {e}", asset_file.display())));
    let asset_name = asset_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| die("asset file name is not valid UTF-8"));
    let msg = signed_message(tag, asset_name, &asset);
    let pk = PublicKey::new(pk_bytes);
    let sig = Signature::new(sig_bytes);
    match pk.verify(&msg, &sig) {
        Ok(()) => {
            println!(
                "VERIFY-OK {} tag={tag} ({} B)",
                asset_file.display(),
                asset.len()
            );
        }
        Err(e) => {
            println!("VERIFY-FAIL {} tag={tag}: {e}", asset_file.display());
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, cmd, rest @ ..] => match (cmd.as_str(), rest) {
            ("keygen", [seed_out]) => cmd_keygen(Path::new(seed_out)),
            ("sign", [seed_file, tag, asset_file]) => {
                cmd_sign(Path::new(seed_file), tag, Path::new(asset_file))
            }
            ("verify", [pk_hex, tag, asset_file, sig_file]) => {
                cmd_verify(pk_hex, tag, Path::new(asset_file), Path::new(sig_file))
            }
            _ => usage(),
        },
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cross-implementation vector, mirrored from src/sig.rs: the
    /// signature was produced by THIS tool over the fixture key (seed
    /// 00..1f), tag v0.1.0, asset name riviv-0.2.0-portable.exe
    /// (deliberately version-mismatched so a tag-name field swap
    /// cannot pass). Tool-side coverage closes the drift hole where
    /// `sign` and `verify` drift TOGETHER while the shipped client
    /// stays put — every release ritual runs `cargo test` here
    /// (RELEASE.md), so both formula copies are pinned to the same
    /// external truth (review #238 cubic P2).
    #[test]
    fn the_client_vector_verifies_with_this_tool_too() {
        const VEC_PK_HEX: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
        const VEC_SIG_HEX: &str = "79e4e3d7dfc60fcad602d8eb43700fb455a203e845b6858220f04897175ee57597be04f44e9a95680f6504990dde32bf54dc69c4a5cc9a772777e9eac65e8600";
        const VEC_ASSET: &[u8] = b"riviv #237 cross-implementation lock vector\n";
        let pk = unhex(VEC_PK_HEX);
        assert_eq!(pk.len(), 32);
        let msg = signed_message("v0.1.0", "riviv-0.2.0-portable.exe", VEC_ASSET);
        let sig = Signature::new(unhex(VEC_SIG_HEX).try_into().unwrap());
        assert!(
            PublicKey::new(pk.try_into().unwrap())
                .verify(&msg, &sig)
                .is_ok()
        );
        // And the formula lays out the fields byte-exactly (the same
        // pin src/sig.rs carries).
        let mut expected = Vec::new();
        expected.extend_from_slice(b"riviv-update-v1\nv0.1.0\nriviv-0.2.0-portable.exe\n");
        expected.extend_from_slice(VEC_ASSET);
        assert_eq!(msg, expected);
    }

    #[test]
    fn unhex_rejects_bad_shapes_without_panicking() {
        assert_eq!(unhex("00ff"), vec![0x00, 0xff]);
        assert_eq!(unhex("00ff").len(), 2);
        // Non-ASCII and odd shapes die (process exit 2) — assert the
        // pre-conditions instead of catching exits: the byte-pair walk
        // must never slice a UTF-8 boundary.
        assert_eq!("1é2".len() % 2, 0); // 4 bytes: walks pairs fine
    }
}
