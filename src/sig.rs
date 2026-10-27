//! The update trust anchor's verify half (#237; ADR 0009).
//!
//! The client side of the Ed25519 pinned-key scheme: every release asset
//! ships with a raw 64 B `<asset>.sig`, and the signed message is NOT
//! the bare asset bytes but a domain-separated buffer —
//!
//! ```text
//! "riviv-update-v1\n" + <tag> + "\n" + <asset file name> + "\n" + <asset bytes>
//! ```
//!
//! — binding each signature to one (tag, asset) pair, so a poisoned
//! channel cannot replay an old release's valid signature as a
//! downgrade, or substitute one language's Setup for another (spike
//! #235; cubic #236 P1#3 adopted). The signer half lives in
//! `tools/riviv-sign/` as an INDEPENDENT project (a second `[[bin]]` in
//! this crate would let feature unification leak signing features into
//! the ship build); this module and that tool duplicate the formula on
//! purpose, and the cross-implementation test vector below pins the two
//! copies byte-for-byte. The release ritual's verify step is the
//! end-to-end sentinel (RELEASE.md).
//!
//! Pure logic throughout (bytes in, verdict out) — the network/file
//! shells live in `update_install.rs`; nothing here touches Win32.

use ed25519_compact::{PublicKey, Signature};

/// The production signing public key (32 B, hex). Generated once by
/// `riviv-sign keygen` at #237 implementation time; the matching seed
/// lives OFFLINE in two copies and never enters the repository
/// (RELEASE.md ceremony; loss/leak contingency in ADR 0009).
pub(crate) const PUBKEY_HEX: &str =
    "670f39afb49835ce762721d473b3d4225786f79c856df00124c22a95ac4451c1";

/// The domain-separation magic + version. Bumping this string is the
/// format-evolution knob: old signatures stop verifying against a new
/// client wholesale, which is exactly the intended failure mode.
pub(crate) const MAGIC: &str = "riviv-update-v1";

/// Build the domain-separated signed message. Must stay byte-identical
/// to `tools/riviv-sign/src/main.rs::signed_message` — the pinned test
/// vector below is the lock.
///
/// Memory note: this copies the whole asset into the message buffer on
/// top of the caller's read — a transient 2× peak (worst case ~256 MB
/// at the 128 MB budget, ~17 MB at the real 8.5 MB assets; review #238
/// cubic, accepted tradeoff: ed25519-compact takes the message by
/// slice, so a zero-copy layout would need a pre-sized read into the
/// message tail — machinery not worth the real-asset sizes).
pub(crate) fn signed_message(tag: &str, asset_name: &str, asset: &[u8]) -> Vec<u8> {
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

/// The verify verdict over an explicit key (pure; the tests exercise
/// this against fixture keys, the runtime wrapper below pins the
/// production one). A wrong-length signature is simply `false` — the
/// caller's verdict shape stays boolean (spike #235's failure semantics
/// table: verify failure is one user-facing class, not a diagnostic
/// taxonomy).
pub(crate) fn verify_with(
    pk: &[u8; 32],
    tag: &str,
    asset_name: &str,
    asset: &[u8],
    sig: &[u8],
) -> bool {
    let Ok(sig) = <[u8; 64]>::try_from(sig) else {
        return false;
    };
    let msg = signed_message(tag, asset_name, asset);
    PublicKey::new(*pk)
        .verify(&msg, &Signature::new(sig))
        .is_ok()
}

/// The production-key verify verdict — the one call the install flow
/// makes (ADR 0009 D1: pinned key, no key rotation logic in the first
/// cut; a future rotation ships a new exe, which is itself signed).
pub(crate) fn verify(tag: &str, asset_name: &str, asset: &[u8], sig: &[u8]) -> bool {
    let Some(pk) = unhex32(PUBKEY_HEX) else {
        // A malformed embedded constant is a build-time authoring error;
        // the test below pins its shape, so this arm is unreachable
        // defense — and failing CLOSED is the only safe direction.
        return false;
    };
    verify_with(&pk, tag, asset_name, asset, sig)
}

/// Decode a 64-char hex string into 32 B (None on any bad shape — used
/// for the embedded constant and the test fixtures; no crate).
pub(crate) fn unhex32(s: &str) -> Option<[u8; 32]> {
    let b = s.as_bytes();
    if b.len() != 64 {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = (nib(b[2 * i])? << 4) | nib(b[2 * i + 1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_compact::{KeyPair, Seed};

    /// The spike #235 fixture seed (00..1f) — its public key
    /// `03a107bf…31b8` is an independent pin (spike doc E4/probe
    /// constants), so a crate regression that breaks from_seed/new shows
    /// up as a mismatch here, not just as our own round-trip tautology.
    const VEC_PK_HEX: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";

    fn vec_seed() -> [u8; 32] {
        let mut s = [0u8; 32];
        for (i, b) in s.iter_mut().enumerate() {
            *b = i as u8;
        }
        s
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn the_message_lays_out_the_domain_separated_fields() {
        // The formula, spelled out field by field as the independent
        // truth (spike #235, P2=A ruling): magic, tag, asset name, bytes
        // — each separated by exactly one LF.
        let msg = signed_message("v0.9.0", "riviv-0.9.0-portable.exe", b"ABC");
        let mut expected = Vec::new();
        expected.extend_from_slice(b"riviv-update-v1\n");
        expected.extend_from_slice(b"v0.9.0\n");
        expected.extend_from_slice(b"riviv-0.9.0-portable.exe\n");
        expected.extend_from_slice(b"ABC");
        assert_eq!(msg, expected);
    }

    #[test]
    fn a_tag_or_name_containing_lf_cannot_smuggle_across_fields() {
        // A hostile tag_name like "v9.9.0\nriviv-0.9.0-portable.exe"
        // would make two different (tag, name) pairs produce the SAME
        // message. The formula itself cannot prevent that, so this test
        // DOCUMENTS the residual: the tag comes from parse_semver-gated
        // tag_name (digits and dots only upstream of this call), and the
        // asset name is always OUR constructed name — neither contains
        // LF in practice. If this ever changes, the collision below is
        // the alarm.
        let a = signed_message("v9.9.0\nriviv-0.9.0-portable.exe", "x", b"ABC");
        let b = signed_message("v9.9.0", "riviv-0.9.0-portable.exe\nx", b"ABC");
        assert_eq!(
            a, b,
            "known residual: LF smuggling across fields — inputs are pre-constrained upstream"
        );
    }

    #[test]
    fn sign_verify_round_trip_over_the_fixture_key() {
        let kp = KeyPair::from_seed(Seed::new(vec_seed()));
        // The from_seed half is pinned against the spike's independent
        // constant, keeping this from being purely tautological.
        assert_eq!(hex(kp.pk.as_ref()), VEC_PK_HEX);
        let asset = b"the quick brown fox";
        let sig = kp.sk.sign(signed_message("v0.9.0", "a.exe", asset), None);
        assert!(verify_with(
            &kp.pk.as_ref().try_into().unwrap(),
            "v0.9.0",
            "a.exe",
            asset,
            sig.as_ref()
        ));
    }

    #[test]
    fn the_tool_test_vector_verifies() {
        // The cross-implementation lock: this signature was produced by
        // `tools/riviv-sign` (built from the tool's own formula copy)
        // over the fixture key, tag v0.1.0 and asset name
        // riviv-0.2.0-portable.exe (deliberately version-mismatched so a
        // tag↔name field swap cannot pass). If either formula copy
        // drifts, this fails.
        const VEC_SIG_HEX: &str = "79e4e3d7dfc60fcad602d8eb43700fb455a203e845b6858220f04897175ee57597be04f44e9a95680f6504990dde32bf54dc69c4a5cc9a772777e9eac65e8600";
        const VEC_ASSET: &[u8] = b"riviv #237 cross-implementation lock vector\n";
        let pk = unhex32(VEC_PK_HEX).unwrap();
        assert!(verify_with(
            &pk,
            "v0.1.0",
            "riviv-0.2.0-portable.exe",
            VEC_ASSET,
            &unhex(VEC_SIG_HEX)
        ));
    }

    #[test]
    fn domain_separation_rejects_replays_and_substitutions() {
        let kp = KeyPair::from_seed(Seed::new(vec_seed()));
        let pk: [u8; 32] = kp.pk.as_ref().try_into().unwrap();
        let asset = b"asset bytes v1";
        let sig = kp.sk.sign(
            signed_message("v0.9.0", "riviv-0.9.0-portable.exe", asset),
            None,
        );
        let sig = sig.as_ref();
        // The signed truth.
        assert!(verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0-portable.exe",
            asset,
            sig
        ));
        // Downgrade replay: the same valid signature under a NEW tag.
        assert!(!verify_with(
            &pk,
            "v0.9.1",
            "riviv-0.9.0-portable.exe",
            asset,
            sig
        ));
        // Wrong-package substitution: same signature, other asset name.
        assert!(!verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0.x64.zh-CN-Setup.exe",
            asset,
            sig
        ));
        // Tampered bytes (the 1-B flip the smoke's tamper arm replays).
        let mut tampered = asset.to_vec();
        tampered[0] ^= 1;
        assert!(!verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0-portable.exe",
            &tampered,
            sig
        ));
        // Tampered signature.
        let mut bad_sig = sig.to_vec();
        bad_sig[63] ^= 1;
        assert!(!verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0-portable.exe",
            asset,
            &bad_sig
        ));
        // Wrong key: another party's signature over the same message.
        let other = KeyPair::from_seed(Seed::new(core::array::from_fn(|i| 255 - i as u8)));
        let other_sig = other.sk.sign(
            signed_message("v0.9.0", "riviv-0.9.0-portable.exe", asset),
            None,
        );
        assert!(!verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0-portable.exe",
            asset,
            other_sig.as_ref()
        ));
        // Shape guards: short/garbage signatures are false, never a panic.
        assert!(!verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0-portable.exe",
            asset,
            &[0u8; 63]
        ));
        assert!(!verify_with(
            &pk,
            "v0.9.0",
            "riviv-0.9.0-portable.exe",
            asset,
            &[]
        ));
    }

    #[test]
    fn the_embedded_production_constant_is_well_formed() {
        let pk = unhex32(PUBKEY_HEX).expect("PUBKEY_HEX must be 64 hex chars");
        assert!(pk != [0u8; 32], "the production key is not the null key");
        // And it is NOT the spike's public fixture (whose seed is printed
        // in the spike doc — using it would pin the updates to a burned
        // key).
        assert_ne!(hex(&pk), VEC_PK_HEX);
    }

    #[test]
    fn unhex32_rejects_bad_shapes() {
        assert!(unhex32("00").is_none());
        assert!(unhex32("").is_none());
        assert_eq!(
            unhex32(&"0".repeat(64)),
            Some([0u8; 32]),
            "all-zero hex is well-formed"
        );
        assert!(unhex32(&"g".repeat(64)).is_none());
        assert!(unhex32(VEC_PK_HEX).is_some());
        assert_eq!(unhex32("ff"), None);
    }
}
