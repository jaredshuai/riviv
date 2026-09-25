//! The display-segment decision table (#127, M8-2; design authority =
//! issuecomment-5831416664 + the #127 design comment 5831945491):
//! which transform, if any, runs between the sRGB master and the
//! swapchain. Pure logic, zero unsafe (AGENTS.md dependency principle
//! 2) — the WCS query, the D2D effect, and the ratchet's failure feed
//! all land with their consumer tickets (#126's dump channel and the
//! static-gpu_effect phase).
//!
//! Pipeline position (disposition D1/D7): the master stays sRGB
//! forever — Stage 1 (#77, `icm.rs`) maps embedded ICCs into it at
//! decode and the composite bakes the background in — and THIS stage
//! is the single viewport-sized pass after composition, applied only
//! when the OS is not already managing the display transform AND the
//! WCS display profile is not sRGB-equivalent.
//!
//! The judge is the WCS profile query, never the ACM bit (D3): under
//! auto color management the getter answers "no profile", while the
//! legacy compatibility helper ("Use legacy display ICC color
//! management", no programmatic enablement) answers with the composite
//! profile — the getter disambiguates both states by itself. The ACM
//! bit from `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` is a diagnostic
//! label that rides along in the output identity, nothing more.
//!
//! WARP never runs the gpu_effect (hard exclusion, ticket) and never
//! runs the CPU pass either (probe P3: a WCS viewport transform
//! measured 9.61 ms median at 1080p with a matrix-shaper profile — an
//! optimistic floor, real display profiles run LUT-heavy — over the
//! 8 ms gate), so a custom profile on WARP displays uncorrected sRGB.
//! `Cpu` stays in the vocabulary for a future re-verdict but is
//! unreachable from today's table.
//!
//! The R2 contract (disposition D6): the status-bar RGB readout is an
//! "sRGB-normalized reading" — it always samples the sRGB master
//! BEFORE this stage, regardless of `TransformStage`. Every cell of
//! the table keeps the content space Srgb (asserted exhaustively
//! below); the end-to-end assertion channel is #126's dump
//! OutputIdentity gen.

// ---------------------------------------------------------------------
// Inputs — the render-environment identity the table decides on.
// ---------------------------------------------------------------------

/// The effective D2D backend after stack creation (auto/d2d resolve
/// here; #90 removed the GDI session). Hardware and WARP disagree on
/// one table row: the gpu_effect only exists where a GPU runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Backend {
    Hardware,
    Warp,
}

impl Backend {
    /// Map the stack's EFFECTIVE renderer kind (config.rs) onto the table's
    /// backend axis (#126's dump wiring): `create` resolves auto/d2d into
    /// D2d or Warp before the kind is stored, so Auto never reaches a live
    /// stack — the total-function pin below maps it to Hardware anyway (a
    /// pre-create read is the only way to see it, and no device means no
    /// output identity either).
    pub(crate) fn from_effective(kind: crate::config::RendererKind) -> Self {
        match kind {
            crate::config::RendererKind::Warp => Backend::Warp,
            _ => Backend::Hardware,
        }
    }
}

/// What the WCS display-profile query came back with — the primary
/// judge (D3). `NoProfile` means the OS owns the display transform
/// (ACM active without the legacy compat helper): riviv's sRGB output
/// is already correct and must not be transformed again. `Unknown`
/// (RDP ACCESS_DENIED, the modern getter absent on Win10) must never
/// transform blindly. A profile means riviv is responsible, and
/// whether it differs from sRGB decides whether there is work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayProfileQuery {
    NoProfile,
    Unknown,
    Profile(DisplayProfileSpace),
}

/// The space a returned display profile encodes, classified the same
/// way Stage 1 classifies embedded profiles: byte-equality against
/// the system sRGB profile short-circuits, a different blob encoding
/// sRGB falls to the 1024-pixel noise-floor probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayProfileSpace {
    SrgbEquivalent,
    Custom,
}

// ---------------------------------------------------------------------
// The table.
// ---------------------------------------------------------------------

/// Where the sRGB->display transform runs, if anywhere. `Cpu` is
/// reserved vocabulary: today's table never emits it (see the module
/// doc's P3 verdict) — WARP with a custom profile shows uncorrected
/// sRGB rather than paying a ~10 ms CPU pass per paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TransformStage {
    /// Identity: nothing between the master and the swapchain.
    None,
    /// A WCS pass over the viewport on the CPU — reserved, currently
    /// unreachable (P3: 9.61 ms median at 1080p over the 8 ms gate).
    Cpu,
    /// The D2D ColorManagement effect in the draw pass (hardware
    /// only — the WARP exclusion is one of the table's hard rules).
    GpuEffect,
    /// The OS owns the display transform (auto color management):
    /// riviv's sRGB output is already correct. Stage 1 is unaffected.
    DwmAcm,
}

/// The pure decision table: one row per `DisplayProfileQuery`, one
/// column per `Backend`, exhaustively pinned by tests. No config
/// input — the `icm` key governs Stage 1 only; the display segment is
/// automatic correctness, and a user gate would be a wiring-phase
/// design of its own.
pub(crate) fn desired_stage(backend: Backend, query: DisplayProfileQuery) -> TransformStage {
    match (backend, query) {
        // The OS is managing: never stack an app-side display segment
        // (D3's rewrite — Stage 1 keeps running regardless).
        (_, DisplayProfileQuery::NoProfile) => TransformStage::DwmAcm,
        // Unreachable judge or an sRGB display: identity either way —
        // never transform on a guess (RDP), never pay for a no-op.
        (_, DisplayProfileQuery::Unknown)
        | (_, DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent)) => {
            TransformStage::None
        }
        // A real destination profile: the GPU effect where there is a
        // GPU to run it; WARP skips the segment (P3 verdict) — the
        // hard exclusion keeps it off the effect, the probe keeps it
        // off the CPU.
        (Backend::Hardware, DisplayProfileQuery::Profile(DisplayProfileSpace::Custom)) => {
            TransformStage::GpuEffect
        }
        (Backend::Warp, DisplayProfileQuery::Profile(DisplayProfileSpace::Custom)) => {
            TransformStage::None
        }
    }
}

// ---------------------------------------------------------------------
// desired vs effective: the session ratchet (D4).
// ---------------------------------------------------------------------

/// Consecutive application failures before the display segment is
/// switched off for the session. Same value as
/// `gpu::PREPARE_ESCALATION_FAILURES` but a separate constant on
/// purpose: that one feeds the device-loss ladder, this one is a
/// quality ratchet — the policies coexist, they are not one knob.
pub(crate) const STAGE_DEGRADE_FAILURES: u32 = 3;

/// Whether `consecutive_failures` failed applications in a row warrant
/// latching the session downgrade — the same shape as
/// `gpu::prepare_escalates` (a single transient must not tear quality
/// down; three in a row is a pattern).
pub(crate) fn stage_degrades(consecutive_failures: u32) -> bool {
    consecutive_failures >= STAGE_DEGRADE_FAILURES
}

/// The stage that actually runs: `desired` clamped by the session
/// latch. A latched degrade switches the segment OFF (`None`) — never
/// down to `Cpu`: on hardware that would mean a readback + WCS +
/// re-upload every paint, a frame-rate price no rare driver quirk is
/// worth (D4, adopting AI-3 over AI-1). A CPU stage failing degrades
/// the same way — there is no lower transform to fall to. The latch
/// itself is the wiring's session state; quality downgrades are
/// breadcrumb-only, never fatal (ADR 0001).
pub(crate) fn effective_stage(desired: TransformStage, degraded: bool) -> TransformStage {
    if degraded && matches!(desired, TransformStage::GpuEffect | TransformStage::Cpu) {
        TransformStage::None
    } else {
        desired
    }
}

// ---------------------------------------------------------------------
// OutputIdentity (D5, the R7 contraction): the output level's key.
// ---------------------------------------------------------------------

/// The one-byte content space of everything upstream of the display
/// segment — master, LevelCache, uploads, tiles. Pinned `Srgb` for
/// the whole 8-bit era (D10's known limitation: wide-gamut sources
/// are clipped through it); the HDR phase's FP16/scRGB pipeline is
/// what introduces a second variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentSpace {
    Srgb,
}

/// The rendering intent of the display segment. Stage 1 pins
/// RELATIVE_COLORIMETRIC (`icm.rs`), and the display segment matches
/// it (D8-1: the D2D effect's PERCEPTUAL default must be overridden
/// on both ends) — in-gamut colors land byte-accurate and the
/// equivalence gates stay meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RenderIntent {
    RelativeColorimetric,
}

/// The transform quality. WCS runs BEST_MODE (`icm.rs`); the D2D
/// effect's quality property joins the fingerprint so a future
/// NORMAL-vs-BEST decision (FL10 float-buffer requirements, D8-1)
/// re-keys the output identity instead of silently mixing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RenderQuality {
    Best,
}

/// Intent + quality as one fingerprintable unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OutputPolicy {
    pub(crate) intent: RenderIntent,
    pub(crate) quality: RenderQuality,
}

/// The ACM diagnostic from `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` —
/// a label, never a judge (D3). Per wingdi.h 26100 (probe P1's SDK
/// transcription; the docs page is gone): bit 0 =
/// advancedColorSupported, bit 1 = advancedColorEnabled, bit 2 =
/// wideColorEnforced, bit 3 = advancedColorForceDisabled. `Unknown`
/// covers every read failure (probe P1 saw the whole API family
/// return ERROR_GEN_FAILURE in agent contexts on build 26200).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AcState {
    Off,
    On,
    Unknown,
}

/// The digest of everything that changes the output: which stage
/// runs, which destination profile it targets (byte hash), the ACM
/// diagnostic, the backend, and the render policy. Equal fingerprints
/// mean interchangeable output resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OutputFingerprint {
    pub(crate) stage: TransformStage,
    pub(crate) profile_hash: u64,
    pub(crate) ac: AcState,
    pub(crate) backend: Backend,
    pub(crate) policy: OutputPolicy,
}

/// The output level's cache key (D5): a monotonic generation plus the
/// fingerprint it was minted for, plus the content space the stage
/// reads. #126's dump channel prints `output_gen` on stderr — a
/// decision change must show as a new generation there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputIdentity {
    pub(crate) output_gen: u64,
    pub(crate) fingerprint: OutputFingerprint,
    pub(crate) content_space: ContentSpace,
}

/// FNV-1a 64 over profile bytes. Not cryptographic — it only has to
/// distinguish profiles within one session's tracker, and to stay
/// stable (the digest is pinned by test so identities never drift
/// across builds).
pub(crate) fn profile_hash(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// The wiring-phase fingerprint (#126): what a dump actually went through
/// TODAY. The judge — the WCS display-profile query — is not wired yet,
/// so the query is [`DisplayProfileQuery::Unknown`] and the table's own
/// answer governs (never transform on a guess: stage None on both
/// backends); no profile bytes exist, so the digest is the empty
/// profile's; the ACM diagnostic is unread (probe P1: the API family
/// failed across the board in agent contexts). Each later phase swaps
/// exactly one placeholder for the real input — the query (the
/// profile-hot-reload phase), the profile bytes, the applied stage (the
/// static gpu_effect phase), the ac read — and the swap is a fingerprint
/// transition the tracker mints a new generation for, which is the whole
/// point of the dump channel's `output_gen` line. Until then the ONE
/// live term is the backend: `-renderer warp` at startup and the failure
/// ladder's hw->warp escalation are both transitions.
pub(crate) fn current_output_fingerprint(backend: Backend) -> OutputFingerprint {
    OutputFingerprint {
        stage: desired_stage(backend, DisplayProfileQuery::Unknown),
        profile_hash: profile_hash(&[]),
        ac: AcState::Unknown,
        backend,
        policy: OutputPolicy {
            intent: RenderIntent::RelativeColorimetric,
            quality: RenderQuality::Best,
        },
    }
}

/// Mints `OutputIdentity` values with a strict division of labor:
/// `output_gen` is the change signal, the fingerprint is the reuse
/// key. Every fingerprint TRANSITION mints the next monotonic gen —
/// including a return to a previous fingerprint (A -> B -> A walks
/// gens 1, 2, 3), so a gen-only consumer (#126's dump channel, whose
/// contract reads "a decision change must show as a new gen") never
/// misses a change. Resource reuse on the round trip is CORRECT reuse
/// (D5) and happens downstream by keying on the fingerprint the
/// identity carries — never by re-issuing an old gen (Codex P2, PR
/// #128: a memoizing tracker hands back gen 1 after the B -> A
/// change, and the change silently disappears from the gen channel).
#[derive(Debug, Default)]
pub(crate) struct OutputTracker {
    next_gen: u64,
    active: Option<OutputFingerprint>,
}

impl OutputTracker {
    /// `output_gen` starts at 1: zero stays free for the wiring's "no
    /// output identity yet" sentinel. Re-identifying the unchanged
    /// active fingerprint is idempotent — same gen, same identity.
    pub(crate) fn identify(&mut self, fingerprint: OutputFingerprint) -> OutputIdentity {
        if self.active != Some(fingerprint) {
            self.next_gen += 1;
            self.active = Some(fingerprint);
        }
        OutputIdentity {
            output_gen: self.next_gen,
            fingerprint,
            content_space: ContentSpace::Srgb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RendererKind;
    use crate::tile::TileKey;

    /// The full judge space: every `DisplayProfileQuery` shape the
    /// wiring can ever resolve a query to.
    fn all_queries() -> [DisplayProfileQuery; 4] {
        [
            DisplayProfileQuery::NoProfile,
            DisplayProfileQuery::Unknown,
            DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent),
            DisplayProfileQuery::Profile(DisplayProfileSpace::Custom),
        ]
    }

    fn both_backends() -> [Backend; 2] {
        [Backend::Hardware, Backend::Warp]
    }

    fn fingerprint(stage: TransformStage, profile_hash: u64) -> OutputFingerprint {
        OutputFingerprint {
            stage,
            profile_hash,
            ac: AcState::Unknown,
            backend: Backend::Hardware,
            policy: OutputPolicy {
                intent: RenderIntent::RelativeColorimetric,
                quality: RenderQuality::Best,
            },
        }
    }

    // ---- the table, cell by cell ----

    #[test]
    fn every_table_cell_is_pinned() {
        use Backend::*;
        use DisplayProfileQuery::*;
        use DisplayProfileSpace::*;
        use TransformStage::*;
        // The design comment's table, verbatim: the OS-managed row is
        // DwmAcm for both backends, the unknown and sRGB rows are
        // identity, and only hardware runs an effect for a custom
        // profile (WARP's cpu cell was dropped by the P3 probe).
        assert_eq!(desired_stage(Hardware, NoProfile), DwmAcm);
        assert_eq!(desired_stage(Warp, NoProfile), DwmAcm);
        assert_eq!(desired_stage(Hardware, Unknown), None);
        assert_eq!(desired_stage(Warp, Unknown), None);
        assert_eq!(desired_stage(Hardware, Profile(SrgbEquivalent)), None);
        assert_eq!(desired_stage(Warp, Profile(SrgbEquivalent)), None);
        assert_eq!(desired_stage(Hardware, Profile(Custom)), GpuEffect);
        assert_eq!(desired_stage(Warp, Profile(Custom)), None);
    }

    #[test]
    fn warp_never_selects_the_gpu_effect_anywhere_in_the_input_space() {
        // Hard exclusion #1 (ticket): the software path must not grow
        // a GPU-only effect, whatever the judge says.
        for query in all_queries() {
            assert_ne!(
                desired_stage(Backend::Warp, query),
                TransformStage::GpuEffect,
                "warp must not run the gpu_effect for {query:?}"
            );
        }
    }

    #[test]
    fn an_os_managed_display_never_stacks_an_app_side_transform() {
        // Hard exclusion #2 (D3's rewrite): "no profile" from the WCS
        // getter means the OS is transforming — the display segment is
        // DwmAcm, never an app-side cpu/gpu pass. Stage 1 is a
        // separate pipeline slot the table does not even see (its
        // input carries no embedded-profile term).
        for backend in both_backends() {
            assert_eq!(
                desired_stage(backend, DisplayProfileQuery::NoProfile),
                TransformStage::DwmAcm
            );
        }
    }

    #[test]
    fn an_unreachable_judge_or_srgb_display_is_always_identity() {
        // Unknown (RDP ACCESS_DENIED, getter absent on Win10) must
        // not transform on a guess; an sRGB display needs no work.
        for backend in both_backends() {
            assert_eq!(
                desired_stage(backend, DisplayProfileQuery::Unknown),
                TransformStage::None
            );
            assert_eq!(
                desired_stage(
                    backend,
                    DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent)
                ),
                TransformStage::None
            );
        }
    }

    #[test]
    fn the_cpu_stage_is_reserved_and_unreachable_from_the_table() {
        // P3 verdict (design comment): a WCS viewport pass measured
        // 9.61 ms median at 1080p on a matrix-shaper profile — an
        // optimistic floor — over the 8 ms gate, so the WARP cpu cell
        // is dropped. The variant stays as vocabulary; if throughput
        // reality changes, this test is the place to revisit.
        for backend in both_backends() {
            for query in all_queries() {
                assert_ne!(
                    desired_stage(backend, query),
                    TransformStage::Cpu,
                    "cpu must stay unreachable for {backend:?}/{query:?}"
                );
            }
        }
    }

    // ---- the ratchet ----

    #[test]
    fn the_session_latch_fires_on_the_third_consecutive_failure() {
        // gpu.rs's prepare-escalation shape: one transient blip must
        // not tear quality down, three in a row is a pattern.
        assert!(!stage_degrades(0));
        assert!(!stage_degrades(1));
        assert!(!stage_degrades(2));
        assert!(stage_degrades(3));
        assert!(stage_degrades(4));
        assert_eq!(STAGE_DEGRADE_FAILURES, 3);
    }

    #[test]
    fn a_latched_degrade_switches_the_segment_off_not_down_to_cpu() {
        // D4: an effect failure on hardware lands on None — never the
        // CPU stage (readback + WCS + re-upload per paint); a failing
        // CPU stage has nothing lower to fall to. The stages that are
        // not ours to apply (None / DwmAcm) pass the latch unchanged.
        assert_eq!(
            effective_stage(TransformStage::GpuEffect, true),
            TransformStage::None
        );
        assert_eq!(
            effective_stage(TransformStage::Cpu, true),
            TransformStage::None
        );
        assert_eq!(
            effective_stage(TransformStage::GpuEffect, false),
            TransformStage::GpuEffect
        );
        assert_eq!(
            effective_stage(TransformStage::None, true),
            TransformStage::None
        );
        assert_eq!(
            effective_stage(TransformStage::DwmAcm, true),
            TransformStage::DwmAcm
        );
    }

    // ---- OutputIdentity ----

    #[test]
    fn every_decision_change_mints_a_new_generation_and_reuse_keys_on_the_fingerprint() {
        // Division of labor (Codex P2, PR #128): the gen is the change
        // signal — every fingerprint TRANSITION bumps it, including the
        // B -> A return, because #126's dump contract reads "a decision
        // change must show as a new gen"; reuse of A's resources on the
        // round trip is what the fingerprint (the identity's reuse key)
        // is for, never a re-issued old gen. A -> B -> A walks 1, 2, 3.
        let mut tracker = OutputTracker::default();
        let a = fingerprint(TransformStage::None, 0xaaa);
        let b = fingerprint(TransformStage::GpuEffect, 0xbbb);
        let a1 = tracker.identify(a);
        assert_eq!(
            a1.output_gen, 1,
            "the first identity is gen 1 (zero stays sentinel)"
        );
        assert_eq!(tracker.identify(b).output_gen, 2);
        let a2 = tracker.identify(a);
        assert_eq!(
            a2.output_gen, 3,
            "the B -> A change must mint a new gen, not restore gen 1"
        );
        assert_eq!(
            a2.fingerprint, a1.fingerprint,
            "the reuse key survives the round trip — A's cached output resources stay valid"
        );
        assert_eq!(
            tracker.identify(a).output_gen,
            3,
            "re-identifying the unchanged decision is idempotent"
        );
        let c = fingerprint(TransformStage::None, 0xccc);
        assert_eq!(
            tracker.identify(c).output_gen,
            3 + 1,
            "a new fingerprint keeps the counter monotonic"
        );
    }

    #[test]
    fn the_ac_diagnostic_participates_in_the_output_identity() {
        // The fingerprint lists the ACM diagnostic explicitly (D5): it
        // explains the stage to the dump reader, so a diagnosis flip
        // mints a new identity even when the transform itself is
        // identical.
        let mut tracker = OutputTracker::default();
        let mut fp = fingerprint(TransformStage::DwmAcm, 0x1234);
        fp.ac = AcState::Off;
        let first = tracker.identify(fp).output_gen;
        fp.ac = AcState::On;
        assert_ne!(tracker.identify(fp).output_gen, first);
    }

    #[test]
    fn the_profile_hash_is_stable_and_distinguishing() {
        // The digest is pinned so identities never drift across
        // builds: FNV-1a 64 has published vectors, and equal bytes
        // must keep hashing equal while different bytes differ.
        assert_eq!(profile_hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(profile_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(profile_hash(b"foobar"), 0x85944171f73967e8);
        assert_ne!(profile_hash(b"abc"), profile_hash(b"abd"));
    }

    // ---- the #126 wiring constructor ----

    #[test]
    fn the_effective_kind_maps_onto_the_table_backend_axis() {
        // create() resolves auto/d2d before the kind is stored, so a live
        // stack is D2d or Warp; the mapping is total anyway (Auto is
        // pinned to Hardware — no device, no output identity, unreachable
        // through the wiring).
        assert_eq!(Backend::from_effective(RendererKind::Warp), Backend::Warp);
        assert_eq!(
            Backend::from_effective(RendererKind::D2d),
            Backend::Hardware
        );
        assert_eq!(
            Backend::from_effective(RendererKind::Auto),
            Backend::Hardware
        );
    }

    #[test]
    fn the_wiring_fingerprint_pins_the_unwired_placeholders() {
        // The dump channel's identity for TODAY (see the constructor's
        // doc): the unwired judge reads Unknown (the table answers None on
        // both backends — never transform on a guess), the profile bytes
        // are the empty digest, the ACM diagnostic is unread, and the
        // policy is the pinned pair. Every field is pinned so a later
        // phase's swap is a visible fingerprint transition, not a drift.
        for backend in both_backends() {
            let fp = current_output_fingerprint(backend);
            assert_eq!(fp.stage, TransformStage::None, "{backend:?}");
            assert_eq!(fp.profile_hash, profile_hash(b""));
            assert_eq!(fp.ac, AcState::Unknown);
            assert_eq!(fp.backend, backend);
            assert_eq!(fp.policy.intent, RenderIntent::RelativeColorimetric);
            assert_eq!(fp.policy.quality, RenderQuality::Best);
        }
    }

    #[test]
    fn the_backend_is_the_one_live_fingerprint_term_and_walks_the_gens() {
        // The erratum semantics (Codex P2, PR #128) exercised through the
        // REAL constructor: today only the backend varies — a hw->warp
        // escalation then a rebuild back are fingerprint transitions, so
        // the dump channel sees 1, 2, 3, and a same-backend re-identify
        // (an ordinary same-kind rebuild) is idempotent.
        let mut tracker = OutputTracker::default();
        let hw = current_output_fingerprint(Backend::Hardware);
        let warp = current_output_fingerprint(Backend::Warp);
        assert_ne!(hw, warp);
        assert_eq!(tracker.identify(hw).output_gen, 1);
        assert_eq!(tracker.identify(warp).output_gen, 2);
        assert_eq!(
            tracker.identify(hw).output_gen,
            3,
            "the return to hardware must mint a new gen, not restore gen 1"
        );
        assert_eq!(
            tracker.identify(hw).output_gen,
            3,
            "a same-kind rebuild re-identifies idempotently"
        );
    }

    // ---- R2 / D5 pins ----

    #[test]
    fn status_rgb_reads_the_pre_stage2_srgb_master_in_every_cell() {
        // R2 (D6): the status readout is an sRGB-normalized reading
        // off the master BEFORE the display segment — no decision
        // cell may change the content space it samples. (The
        // end-to-end channel is #126's dump gen.)
        for backend in both_backends() {
            for query in all_queries() {
                let stage = desired_stage(backend, query);
                let mut tracker = OutputTracker::default();
                let identity = tracker.identify(fingerprint(stage, 0));
                assert_eq!(
                    identity.content_space,
                    ContentSpace::Srgb,
                    "cell {backend:?}/{query:?} must keep the master sRGB"
                );
            }
        }
    }

    #[test]
    fn tile_keys_stay_output_blind() {
        // D5 (the R7 contraction): the four-layer cache — master,
        // LevelCache, upload keys, tiles — is display-independent. A
        // TileKey is exactly {frame_gen, level, tx, ty}; this
        // exhaustive struct literal stops compiling if anyone adds an
        // output-identity field to it (the mistake R7 originally
        // feared: invalidating tiles on a profile change). Two
        // different output identities below, one untouched tile key.
        let mut tracker = OutputTracker::default();
        let _out1 = tracker.identify(fingerprint(TransformStage::None, 1));
        let _out2 = tracker.identify(fingerprint(TransformStage::GpuEffect, 2));
        let key = TileKey {
            frame_gen: 7,
            level: 2,
            tx: 3,
            ty: 4,
        };
        assert_eq!(
            key,
            TileKey {
                frame_gen: 7,
                level: 2,
                tx: 3,
                ty: 4
            }
        );
    }
}
