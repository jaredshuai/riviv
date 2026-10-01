//! The fault-injection seam (#172): a TEST-ONLY env knob that turns real
//! SUCCESSES into synthetic failures at gpu.rs's classification
//! boundaries, so the failure WIRING — the device-loss ladder (one loss →
//! same-kind rebuild, 3-in-10s → permanent WARP, WARP failing too →
//! deferred fatal), the three-layer session ratchet (ac_surface_latched /
//! wide_effect_latched / stage_latched) and the prepare escalation — can
//! be driven end-to-end by smoke scripts. No synthesizable real trigger
//! exists on the dev machine (docs/spikes/s-fault-injection.md: the
//! graphics-reset hotkey ignores synthetic input, a TDR storm courts a
//! bugcheck, PnP/VM need elevation); the real-chain confirmation is a
//! one-minute manual runbook item instead. The `chain` kind (#187, ADR
//! 0006 D9) drives the user effect chain's build failure: a consumed
//! count refuses a chain build whose CreateEffect(Sharpen) genuinely
//! succeeded, so the session-level drop (not the narrow latch) ends the
//! failure sequence.
//!
//! Semantics: a count is a number of REMAINING injections; every consume
//! decrements one. Injection happens where the real call SUCCEEDED — the
//! ladder/ratchet/drain/rebuild/re-derive/re-paint machinery downstream
//! of the classification runs for real; only the OS→HRESULT leg is
//! synthetic (the loss codes themselves are upstream-documented). Note
//! the unit is the BOUNDARY CALL, not the paint: the AC face's dump
//! channel draws two passes (warm-up first, then the timed one) and a
//! consumed count fails THE PASS THAT REACHES IT — with any armed count
//! the dump aborts at the warm-up pass and the timed pass never runs.
//! Size counts accordingly in scripted scenarios.
//!
//! Contract: with `RIVIV_FAULT` unset every counter is zero and every
//! consume answers false — the seam is behaviorally inert in production
//! (the zero-perturbation control re-runs the narrow-image smoke subset
//! byte-for-byte).

use std::sync::atomic::{AtomicU8, Ordering};

static DEVICE_LOSS_REMAINING: AtomicU8 = AtomicU8::new(0);
static EFFECT_REMAINING: AtomicU8 = AtomicU8::new(0);
static AC_CREATE_REMAINING: AtomicU8 = AtomicU8::new(0);
static PREPARE_REMAINING: AtomicU8 = AtomicU8::new(0);
static CHAIN_BUILD_REMAINING: AtomicU8 = AtomicU8::new(0);

/// The parsed knob: how many synthetic failures each boundary still owes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FaultPlan {
    pub(crate) device: u8,
    pub(crate) effect: u8,
    pub(crate) ac_create: u8,
    pub(crate) prepare: u8,
    pub(crate) chain: u8,
}

/// Grammar: comma-separated `kind=count` tokens, whitespace-tolerant;
/// counts clamp to 0..=255 (0 = disarmed; values above `u8::MAX` clamp
/// DOWN, e.g. `device=999` arms 255 — but a token that does not parse as
/// `u32` at all, like `device=4294967296` or `device=-1`, is SKIPPED
/// like any garbage, not clamped); a duplicate kind takes the LAST
/// token (`device=1,device=2` arms 2) and a leading `+` parses (Rust
/// `u32` semantics); unknown kinds and malformed tokens are SKIPPED,
/// never a parse failure (a typo must not abort startup and must not
/// arm the wrong slot either).
pub(crate) fn parse(value: &str) -> FaultPlan {
    let mut plan = FaultPlan::default();
    for token in value.split(',') {
        let token = token.trim();
        let Some((kind, count)) = token.split_once('=') else {
            continue;
        };
        let Ok(count) = count.trim().parse::<u32>() else {
            continue;
        };
        let count = count.clamp(0, u32::from(u8::MAX)) as u8;
        match kind.trim() {
            "device" => plan.device = count,
            "effect" => plan.effect = count,
            "ac_create" => plan.ac_create = count,
            "prepare" => plan.prepare = count,
            "chain" => plan.chain = count,
            _ => {}
        }
    }
    plan
}

/// Read the env once at startup and arm the counters. Unset (or a value
/// that parses to all-zero — an empty string or pure garbage) = the seam
/// stays inert, and the arming breadcrumb is NOT printed for the
/// all-zero case (external review P3: an `RIVIV_FAULT=""` must not lie
/// "armed"). The breadcrumb therefore only ever accompanies a live
/// arming, so a production session's stderr contract is untouched.
pub(crate) fn init_from_env() {
    let Some(value) = std::env::var_os("RIVIV_FAULT") else {
        return;
    };
    let value = value.to_string_lossy();
    let plan = parse(&value);
    if plan == FaultPlan::default() {
        return;
    }
    DEVICE_LOSS_REMAINING.store(plan.device, Ordering::Relaxed);
    EFFECT_REMAINING.store(plan.effect, Ordering::Relaxed);
    AC_CREATE_REMAINING.store(plan.ac_create, Ordering::Relaxed);
    PREPARE_REMAINING.store(plan.prepare, Ordering::Relaxed);
    CHAIN_BUILD_REMAINING.store(plan.chain, Ordering::Relaxed);
    eprintln!(
        "riviv: fault injection armed (device={} effect={} ac_create={} prepare={} chain={})",
        plan.device, plan.effect, plan.ac_create, plan.prepare, plan.chain
    );
}

/// Take one synthetic device loss (a successful draw reclassified as
/// `DXGI_ERROR_DEVICE_REMOVED` at `draw_pass`'s entry).
pub(crate) fn consume_device_loss() -> bool {
    take_one(&DEVICE_LOSS_REMAINING)
}

/// Take one synthetic display-effect failure (an `effect_pass` refused
/// before drawing; the drain classifies it by the CURRENT arm — AcFace on
/// the scRGB arm, WideLegacy on the legacy wide arms, Narrow otherwise).
pub(crate) fn consume_effect() -> bool {
    take_one(&EFFECT_REMAINING)
}

/// Take one synthetic AC-surface creation refusal (gpu::create's AC arm
/// fails before SetColorspace1, latching ac_surface_latched at creation
/// and folding the session to the legacy face).
pub(crate) fn consume_ac_create() -> bool {
    take_one(&AC_CREATE_REMAINING)
}

/// Take one synthetic upload failure (prepare refuses; the paint blanks
/// the frame and the consecutive counter feeds the escalation ladder).
pub(crate) fn consume_prepare() -> bool {
    take_one(&PREPARE_REMAINING)
}

/// Take one synthetic user-chain build failure (#187, ADR 0006 D9): the
/// CreateEffect(Sharpen) call itself SUCCEEDED, then the build refuses —
/// `ensure_effect_graph`'s Err arm drops the chain for the session (the
/// breadcrumb line) and the next paint collapses onto the chain-less
/// shape, ending the failure sequence. Driving this proves the drop
/// actually happens and never loops.
pub(crate) fn consume_chain_build() -> bool {
    take_one(&CHAIN_BUILD_REMAINING)
}

fn take_one(slot: &AtomicU8) -> bool {
    loop {
        let n = slot.load(Ordering::Relaxed);
        if n == 0 {
            return false;
        }
        if slot
            .compare_exchange_weak(n, n - 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(value: &str) -> FaultPlan {
        parse(value)
    }

    #[test]
    fn a_full_line_arms_every_slot() {
        let plan = plan_of("device=3,effect=4,ac_create=1,prepare=5,chain=2");
        assert_eq!(
            plan,
            FaultPlan {
                device: 3,
                effect: 4,
                ac_create: 1,
                prepare: 5,
                chain: 2,
            }
        );
    }

    #[test]
    fn a_single_kind_arms_only_its_slot() {
        let plan = plan_of("device=1");
        assert_eq!(
            plan,
            FaultPlan {
                device: 1,
                ..FaultPlan::default()
            }
        );
    }

    #[test]
    fn whitespace_around_kinds_and_counts_is_tolerated() {
        assert_eq!(plan_of(" device = 2 , effect = 1 ").device, 2);
        assert_eq!(plan_of(" device = 2 , effect = 1 ").effect, 1);
    }

    #[test]
    fn unknown_kinds_and_garbage_are_skipped_not_fatal() {
        // A typo must neither abort parsing nor arm the wrong slot.
        let plan = plan_of("devicee=9,=3,bogus,7,device=2");
        assert_eq!(
            plan,
            FaultPlan {
                device: 2,
                ..FaultPlan::default()
            }
        );
    }

    #[test]
    fn counts_clamp_into_the_u8_range() {
        assert_eq!(plan_of("device=99999").device, u8::MAX);
        assert_eq!(plan_of("device=0").device, 0);
    }

    #[test]
    fn a_count_beyond_u32_is_skipped_not_clamped() {
        // 4294967296 does not parse as u32: the token is garbage, so the
        // slot stays disarmed (it never clamps down to 255).
        assert_eq!(plan_of("device=4294967296"), FaultPlan::default());
        assert_eq!(plan_of("device=-1"), FaultPlan::default());
    }

    #[test]
    fn an_empty_or_all_garbage_line_disarms_everything() {
        assert_eq!(plan_of(""), FaultPlan::default());
        assert_eq!(plan_of(",,"), FaultPlan::default());
        assert_eq!(plan_of("no-equals-sign"), FaultPlan::default());
    }

    #[test]
    fn take_one_yields_true_exactly_count_times() {
        let slot = AtomicU8::new(3);
        assert!(take_one(&slot));
        assert!(take_one(&slot));
        assert!(take_one(&slot));
        assert!(!take_one(&slot));
        assert_eq!(slot.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_zero_slot_never_yields() {
        let slot = AtomicU8::new(0);
        assert!(!take_one(&slot));
    }
}
