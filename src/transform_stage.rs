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
//! The judge of the sRGB content rows is the WCS profile query, never
//! the ACM bit (D3): under auto color management the getter answers
//! "no profile", while the legacy compatibility helper ("Use legacy
//! display ICC color management", no programmatic enablement) answers
//! with the composite profile — the getter disambiguates both states
//! by itself. On those rows the ACM bit from
//! `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` stays a diagnostic label
//! that rides along in the output identity, nothing more (#127 D3's
//! original text, pinned by test). ADR 0004 D4 revises D3 for the
//! WIDE content rows only: on an `F16P3` master the ACM bit (type 9
//! bit 1) is the AC output arm's primary judge — "does riviv declare
//! itself the interpreter of its own buffer" is exactly the signal
//! bit 1 carries — and it outranks the WCS query there (the P-D probe
//! caught ACM-state getters answering non-empty, so the query must
//! never veto the arm).
//!
//! #130 wired the judge (`display_profile.rs`: the modern getter via
//! dynamic mscms resolution, per output-decision point) and the static
//! gpu_effect application; `fingerprint_for` consumes the REAL query,
//! bytes, and latch. #132 added the event-driven hot reload, and #134
//! swapped the last placeholder: the ACM diagnostic (type 9 bit 1) now
//! rides the fingerprint's `ac` term. #154 added the table's third
//! dimension (ADR 0004 D4): the question is no longer just "which
//! stage" but "which face and which stage" — [`desired_output`] — the
//! fingerprint gained a surface term, the identity carries the display
//! content's real class, and the wide `GpuEffectP3To*` stages joined
//! the vocabulary. #156 wired the draw consumption: [`display_arm`] is
//! the per-frame answer the paint actually executes (the desired cell
//! run through the three session ratchets and the LIVE output face's
//! clamp), [`clamped_surface`] is the face the output resources must
//! physically sit on (the fingerprint's surface term AND the window
//! wiring's face-reconciliation target), and a latched AC arm folds the
//! wide AC cells back onto the legacy face's wide stage in both.
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
    /// Dead in the non-test build on purpose; the unreachable-from-the-
    /// table pin below is the truthful record.
    #[allow(dead_code)]
    Cpu,
    /// The D2D ColorManagement effect in the draw pass (hardware
    /// only — the WARP exclusion is one of the table's hard rules).
    GpuEffect,
    /// The wide master's constant pass-through (ADR 0004 D1, the F16P3
    /// "sRGB screen" column): a ColorManagement effect mapping an
    /// F16P3 master to plain sRGB. Same effect shape as
    /// [`TransformStage::GpuEffect`], but the destination is CONSTANT
    /// and does not depend on the display verdict at all — hence
    /// "pass-through": on hardware the arm runs whatever the judge
    /// answered. Legacy surface. Draw wiring = #156.
    GpuEffectP3ToSrgb,
    /// The domain half's first leg (ADR 0004 D1, the legacy
    /// wide-gamut screen column): a ColorManagement effect mapping an
    /// F16P3 master to the JUDGED display profile — the same pass #130
    /// runs for sRGB content, now fed values Stage 1 kept inside the
    /// P3 container, so the super-sRGB colors survive to the panel.
    /// Legacy surface, hardware only (the WARP exclusion holds).
    /// Draw wiring = #156.
    GpuEffectP3ToDisplay,
    /// The domain half's second leg (ADR 0004 D3, the AC surface's
    /// draw chain): a ColorManagement effect mapping an F16P3 master
    /// to LINEAR scRGB, drawn into the FP16 swapchain whose color
    /// space the OS maps to the panel (the `SetColorSpace1`
    /// declaration). AC surface only — this stage never pairs with
    /// [`OutputSurface::Legacy`]. Draw wiring = #156.
    GpuEffectP3ToScRgb,
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

/// The output face a decision lands on (ADR 0004 D3). `Legacy` is
/// today's one face — the BGRA8 UNORM swapchain gpu.rs freezes in
/// `SWAPCHAIN_FORMAT`. `AcScRgb` is the AC declaration's face (FP16
/// swapchain + `SetColorSpace1(scRGB)`); #154 carries only the
/// VARIANT and the table/fingerprint bookkeeping — building the face
/// (the swapchain's dual arms) is #156, so nothing constructs it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OutputSurface {
    Legacy,
    AcScRgb,
}

/// [`desired_output`]'s answer: which face to present on, and what the
/// display segment runs (ADR 0004 D4's third dimension of the table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DesiredOutput {
    pub(crate) surface: OutputSurface,
    pub(crate) stage: TransformStage,
}

/// The third dimension of the decision table (ADR 0004 D4): the sRGB-era
/// question "which stage?" becomes "which face, and which stage?" — one
/// answer per (backend, judge, ac) cell per content class, exhaustively
/// pinned by tests. The narrow classes (`Srgb`, `F16Srgb`) delegate to
/// [`desired_stage`] verbatim: every narrow cell is bit-identical to the
/// pre-#154 table (D1's hard constraint, pinned). The wide class
/// (`F16P3`) adds the AC arm — the ACM bit outranks the judge there
/// (D4; the P-D probe caught ACM-state getters answering non-empty, so
/// the query never vetoes the arm), and wide content on hardware NEVER
/// falls to identity or DwmAcm: P3 halves shown as sRGB code values is
/// the fake-color outcome D6's philosophy forbids. The WARP wide cell
/// is a total-function completion that must stay unreachable: Stage 1's
/// destination selection (#155) never mints an F16P3 master in a WARP
/// session, so reaching that cell means the #155 production gate is
/// broken.
pub(crate) fn desired_output(
    backend: Backend,
    query: DisplayProfileQuery,
    ac: AcState,
    content_class: ContentSpace,
) -> DesiredOutput {
    match content_class {
        // The narrow rows: #127 D3's original table, word for word —
        // the ac bit is a label here, never an input.
        ContentSpace::Srgb | ContentSpace::F16Srgb => DesiredOutput {
            surface: OutputSurface::Legacy,
            stage: desired_stage(backend, query),
        },
        ContentSpace::F16P3 => match backend {
            // D5/#155: unreachable completion (see the fn doc). A WARP
            // session cannot hold an F16P3 master, so there is no arm
            // to answer with.
            Backend::Warp => DesiredOutput {
                surface: OutputSurface::Legacy,
                stage: TransformStage::None,
            },
            Backend::Hardware => match ac {
                // The AC arm (D4): the bit outranks the judge — the
                // scRGB declaration face, the domain half's second leg.
                AcState::On => DesiredOutput {
                    surface: OutputSurface::AcScRgb,
                    stage: TransformStage::GpuEffectP3ToScRgb,
                },
                // No AC declaration: the legacy face carries the wide
                // content through an effect — the judged display
                // profile when the judge earns one (the domain half's
                // first leg), the constant sRGB pass-through otherwise.
                // Identity/DwmAcm are not on this menu (fake color).
                AcState::Off | AcState::Unknown => match query {
                    DisplayProfileQuery::Profile(DisplayProfileSpace::Custom) => DesiredOutput {
                        surface: OutputSurface::Legacy,
                        stage: TransformStage::GpuEffectP3ToDisplay,
                    },
                    DisplayProfileQuery::NoProfile
                    | DisplayProfileQuery::Unknown
                    | DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent) => {
                        DesiredOutput {
                            surface: OutputSurface::Legacy,
                            stage: TransformStage::GpuEffectP3ToSrgb,
                        }
                    }
                },
            },
        },
    }
}

// ---------------------------------------------------------------------
// The draw-consumption arm (#156): what this frame's paint executes.
// ---------------------------------------------------------------------

/// The display arm ONE FRAME executes (ADR 0004 D3/D6, the draw-side
/// projection of the table): the desired cell ([`desired_output`]) run
/// through the three session ratchets and the LIVE output face's clamp.
/// `Direct` is the untransformed pass (narrow rows' None/DwmAcm answer),
/// `SrgbToDisplay` the narrow effect, the three `P3To*` arms the wide
/// stages, and `WideBlank` the wide row with NO legal draw — the frame
/// blanks and waits for a re-derivation (latched wide effect) or is an
/// unreachable-cell defense (WARP). A wide row NEVER answers `Direct`:
/// P3 halves shown untransformed is the fake-color outcome D6 forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DisplayArm {
    Direct,
    SrgbToDisplay,
    P3ToSrgb,
    P3ToDisplay,
    P3ToScRgb,
    WideBlank,
}

/// The user display-effect chain (#183, ADR 0006): a DISPLAY-ONLY,
/// non-destructive effect stack (O1=A — the master bytes, the read side
/// (status-bar RGB / clipboard / copy) and the save path never see it;
/// only the viewport and the dump do, per #156's contract). Each stage
/// is one knife over one docs enum-page domain as an int-key scale
/// (O4=A/O6=A): sharpen (#185) = 0.0–10.0 SHARPNESS as 0..=10, white
/// balance (#191) = −1.0..1.0 TEMPERATURE as −10..=10, contrast (#193,
/// the chain's closing knife) = −1.0..1.0 CONTRAST as −10..=10, off = 0
/// per stage. `Eq`/`Hash` make the chain a graph-identity term (`built_for`)
/// and later a fingerprint policy term — an f32 could never key either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct EffectChain {
    pub(crate) sharpen: u8,
    /// The white-balance stage (#191, the chain's second knife): the
    /// docs enum page's −1.0..1.0 TEMPERATURE domain as an integer
    /// −10..=10 scale (0.1 per step, off = 0). SIGNED on purpose —
    /// cooling is half the knob — and `i8` covers the domain with room
    /// for the clamp's edges.
    pub(crate) white_balance: i8,
    /// The contrast stage (#193, the chain's third knife — viv.c:76's
    /// "color correction" realized as the tonal axis): the docs enum
    /// page's −1.0..1.0 CONTRAST domain as an integer −10..=10 scale
    /// (0.1 per step, off = 0 — the docs default 0.0f IS the identity,
    /// the exact white_balance shape). SIGNED on purpose — softening is
    /// half the knob.
    pub(crate) contrast: i8,
}

/// The level a fresh toggle-ON lands on when the config key carries no
/// non-zero level to restore (#185; the single-int-key design has no
/// second "last level" to remember): a gentle but clearly visible third
/// of the docs domain. The Options combo stays the level editor; the
/// toggle is the quick switch.
pub(crate) const SHARPEN_TOGGLE_ON: u8 = 3;
/// #191's mirror of [`SHARPEN_TOGGLE_ON`]: a gentle third of the
/// temperature domain, on the WARM side of zero (the positive sign).
pub(crate) const WHITE_BALANCE_TOGGLE_ON: i8 = 3;
/// #193's mirror of [`WHITE_BALANCE_TOGGLE_ON`]: a gentle third of the
/// contrast domain, on the punchy side of zero (the positive sign).
pub(crate) const CONTRAST_TOGGLE_ON: i8 = 3;

impl EffectChain {
    /// The chain's own off state — and the zero-perturbation contract's
    /// structural term: an empty chain must collapse every (backend, arm)
    /// cell onto the pre-#183 dispatch shape (see [`pass_shape`]). A
    /// single stage off is NOT an empty chain — the other stages still
    /// force the two-stage shape (#191, #193).
    pub(crate) fn is_empty(&self) -> bool {
        self.sharpen == 0 && self.white_balance == 0 && self.contrast == 0
    }

    /// The Sharpen toggle's target chain (#185/#191/#193): flips the
    /// sharpen STAGE only, the other stages ride along untouched — each
    /// View row owns its own stage. The ON level re-arms from the config
    /// key when it carries one (a dropped-then-retoggled chain retries
    /// the persisted level — D9's drop is session-scoped, the user's
    /// standing wish is not), else falls to [`SHARPEN_TOGGLE_ON`]. A
    /// hand-edited out-of-domain key clamps here, never at parse (the
    /// raw int stays in the ini like every other int key).
    pub(crate) fn toggle_sharpen(&self, config_sharpen: i32) -> EffectChain {
        let mut target = *self;
        target.sharpen = if self.sharpen != 0 {
            0
        } else if config_sharpen <= 0 {
            SHARPEN_TOGGLE_ON
        } else {
            EffectChain::from_config(
                config_sharpen,
                i32::from(self.white_balance),
                i32::from(self.contrast),
            )
            .sharpen
        };
        target
    }

    /// The White Balance toggle's target chain (#191): the same shape as
    /// [`toggle_sharpen`] with the stages swapped. "No persisted level"
    /// is `config == 0` exactly — a NEGATIVE key is a real standing
    /// level (cooling), not an absence.
    pub(crate) fn toggle_white_balance(&self, config_white_balance: i32) -> EffectChain {
        let mut target = *self;
        target.white_balance = if self.white_balance != 0 {
            0
        } else if config_white_balance == 0 {
            WHITE_BALANCE_TOGGLE_ON
        } else {
            EffectChain::from_config(
                i32::from(self.sharpen),
                config_white_balance,
                i32::from(self.contrast),
            )
            .white_balance
        };
        target
    }

    /// The Contrast toggle's target chain (#193): [`toggle_white_balance`]'s
    /// exact mirror — "no persisted level" is `config == 0` (a NEGATIVE
    /// key is a real standing level, softening).
    pub(crate) fn toggle_contrast(&self, config_contrast: i32) -> EffectChain {
        let mut target = *self;
        target.contrast = if self.contrast != 0 {
            0
        } else if config_contrast == 0 {
            CONTRAST_TOGGLE_ON
        } else {
            EffectChain::from_config(
                i32::from(self.sharpen),
                i32::from(self.white_balance),
                config_contrast,
            )
            .contrast
        };
        target
    }

    /// The chain a config triple implies — the SEED every stack
    /// (re)installation replays (#185, #191/#193 for the later keys):
    /// each stage clamped into its own docs domain, off = 0/0/0. The
    /// seed runs at stack creation AND every rebuild, so a device-loss
    /// rebuild re-arms a chain the old device's build failure had
    /// dropped (D9's drop is a verdict about one device's context, not
    /// the user's wish — a fresh device gets a fresh try, its failure
    /// would drop again with the same breadcrumb).
    pub(crate) fn from_config(
        config_sharpen: i32,
        config_white_balance: i32,
        config_contrast: i32,
    ) -> EffectChain {
        EffectChain {
            sharpen: config_sharpen.clamp(0, 10) as u8,
            white_balance: config_white_balance.clamp(-10, 10) as i8,
            contrast: config_contrast.clamp(-10, 10) as i8,
        }
    }
}

/// The output face a frame's decision lands on AFTER the AC surface
/// ratchet's clamp (D6): [`desired_output`]'s face, folded back to
/// Legacy once the session latched the AC arm off. One function, three
/// consumers that must never drift apart: the fingerprint's surface
/// term, [`display_arm`]'s pre-clamp face answer, and the window
/// wiring's face-reconciliation target (gpu.rs's `GpuStack::face` chases
/// THIS between paints).
pub(crate) fn clamped_surface(
    backend: Backend,
    query: DisplayProfileQuery,
    ac: AcState,
    content_class: ContentSpace,
    ac_surface_latched: bool,
) -> OutputSurface {
    let desired = desired_output(backend, query, ac, content_class);
    match desired.surface {
        OutputSurface::AcScRgb if ac_surface_latched => OutputSurface::Legacy,
        other => other,
    }
}

/// The master space an effect ARM implies (#175's follow-up, external
/// review P1): the P3 arms only ever serve `F16P3` masters, the narrow
/// effect arm narrow ones. A BLANK draw (no plan — a cleared or failed
/// display, a refused upload's letterbox) has no master behind it, so
/// gpu.rs's `draw_pass` feeds the graph build the ARM'S OWN implication
/// instead of the prepare stamp: the stamp may describe a dead frame
/// (a wide master's stamp surviving `clear_display_state` while the arm
/// already answers the now-narrow session), while the arm always reads
/// the current class. `Direct`/`WideBlank` never build a graph — they
/// map to `Srgb` as an inert default no consumer reads.
pub(crate) fn arm_master_space(arm: DisplayArm) -> ContentSpace {
    match arm {
        DisplayArm::P3ToSrgb | DisplayArm::P3ToDisplay | DisplayArm::P3ToScRgb => {
            ContentSpace::F16P3
        }
        DisplayArm::Direct | DisplayArm::WideBlank | DisplayArm::SrgbToDisplay => {
            ContentSpace::Srgb
        }
    }
}

/// The legacy face's wide stage for the JUDGE's answer (the fold-back
/// target of a latched AC arm and the pre-reconciliation legacy-face
/// arm): a custom display profile earns the P3→display leg, every other
/// judge answer the constant P3→sRGB pass-through. Never None/DwmAcm —
/// the fake-color ban holds on the legacy face too.
fn wide_legacy_stage(query: DisplayProfileQuery) -> TransformStage {
    match query {
        DisplayProfileQuery::Profile(DisplayProfileSpace::Custom) => {
            TransformStage::GpuEffectP3ToDisplay
        }
        _ => TransformStage::GpuEffectP3ToSrgb,
    }
}

/// This frame's executed display arm — the answer the paint draws with
/// (D6's philosophy made executable). Layered on [`desired_output`]:
///
/// - The narrow classes (`Srgb`, `F16Srgb`) answer bit-identically to
///   the pre-#156 draw shape: `SrgbToDisplay` exactly where the old
///   `stage == GpuEffect` synced the two-phase pass, `Direct` otherwise
///   (None/Cpu/DwmAcm) — the narrow latch (`narrow_latched`) clamps
///   through [`effective_stage`] as before.
/// - A wide row never goes direct. On hardware the desired cell answers;
///   a latched AC arm (`ac_surface_latched`) folds an `AcScRgb` cell back
///   onto the legacy face's wide stage ([`wide_legacy_stage`]); an
///   UNRECONCILED face (the stack still on Legacy while the desired face
///   is `AcScRgb` — the one frame before the face catches up, or
///   defense) draws the legacy wide stage instead of the scRGB arm, and
///   the reverse mismatch draws blank below. A latched wide effect
///   (`wide_effect_latched`) answers `WideBlank`: the frame is blank
///   while the re-derivation to sRGB masters runs — never a direct draw
///   (D6: correct-and-blank beats fake color).
/// - WARP + wide is the table's unreachable completion; the defense
///   answer here is `WideBlank` too (the #155 production gate should
///   never mint an F16P3 master in a WARP session — if one appears
///   anyway, blank is the honest floor).
// The spec's own signature — the three session latches and the live face
// are four separate inputs by design; collapsing them into a struct would
// hide which terms the ratchets own. The exhaustive pins read every cell
// through exactly this shape.
#[allow(clippy::too_many_arguments)]
pub(crate) fn display_arm(
    backend: Backend,
    query: DisplayProfileQuery,
    ac: AcState,
    content_class: ContentSpace,
    narrow_latched: bool,
    ac_surface_latched: bool,
    wide_effect_latched: bool,
    stack_face: OutputSurface,
) -> DisplayArm {
    match content_class {
        ContentSpace::Srgb | ContentSpace::F16Srgb => {
            // The pre-#156 shape verbatim: the two-phase pass iff the
            // effective stage is the narrow effect.
            match effective_stage(desired_stage(backend, query), narrow_latched) {
                TransformStage::GpuEffect => DisplayArm::SrgbToDisplay,
                _ => DisplayArm::Direct,
            }
        }
        ContentSpace::F16P3 => match backend {
            Backend::Warp => DisplayArm::WideBlank,
            Backend::Hardware => {
                if wide_effect_latched {
                    return DisplayArm::WideBlank;
                }
                let desired = desired_output(backend, query, ac, content_class);
                match desired.surface {
                    // The AC arm: only legal on the face that declares it.
                    OutputSurface::AcScRgb => {
                        if ac_surface_latched {
                            // D6's surface ratchet fired: the session left
                            // the AC arm — the legacy wide stage keeps the
                            // colors correct on the legacy face.
                            return match wide_legacy_stage(query) {
                                TransformStage::GpuEffectP3ToDisplay => DisplayArm::P3ToDisplay,
                                _ => DisplayArm::P3ToSrgb,
                            };
                        }
                        match stack_face {
                            OutputSurface::AcScRgb => DisplayArm::P3ToScRgb,
                            // The face has not caught up (one reconciliation
                            // frame / defense): the legacy wide stage keeps the
                            // colors CORRECT (clipped, not fake).
                            OutputSurface::Legacy => match wide_legacy_stage(query) {
                                TransformStage::GpuEffectP3ToDisplay => DisplayArm::P3ToDisplay,
                                _ => DisplayArm::P3ToSrgb,
                            },
                        }
                    }
                    // The legacy face's wide stages (ac off/unknown).
                    OutputSurface::Legacy => match wide_legacy_stage(query) {
                        TransformStage::GpuEffectP3ToDisplay => DisplayArm::P3ToDisplay,
                        _ => DisplayArm::P3ToSrgb,
                    },
                }
            }
        },
    }
}

/// The pass SHAPE of a draw — orthogonal to the color arm (#183): whether
/// the pass needs the two-stage form (scene → intermediate → effect
/// graph → target) or composites straight into the target. The color arms
/// have been two-stage since #130 for their own reason (the
/// ColorManagement effect); #183 adds the second, independent reason (a
/// user effect chain on the Direct arm, hardware only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassShape {
    /// BeginDraw → Clear → scene → EndDraw, straight into the target.
    Direct,
    /// Phase 1 composites into the intermediate, phase 2 draws the graph's
    /// output image 1:1 into the target (the one post-composition
    /// viewport pass — #130's ruling, never per-tile).
    TwoStage,
}

/// #183's decision table (#127's decision-table-first landing order): the
/// pass shape per (backend, arm, chain) cell.
///
/// - An EMPTY chain reproduces today's dispatch on every cell — the
///   zero-perturbation negative control as a structural guarantee.
/// - A non-empty chain forces TwoStage on the hardware Direct arm — the
///   one NEW shape (the CM-less chain graph of ADR 0006).
/// - WARP NEVER runs the chain: #127's hard exclusion INHERITED (O3=B);
///   unlifting needs the WARP two-stage path plus an 8 ms timing case
///   (ADR 0006 D5), not a table edit.
/// - The color arms stay two-stage whatever the chain: the chain rides
///   INSIDE their graph (before the ColorManagement effect, so the
///   convolution runs in the master's gamma-like domain on every arm).
/// - `WideBlank` stays direct: blank is a constant field, and a
///   convolution of a constant is itself.
pub(crate) fn pass_shape(backend: Backend, arm: DisplayArm, chain: EffectChain) -> PassShape {
    match arm {
        DisplayArm::Direct if backend == Backend::Hardware && !chain.is_empty() => {
            PassShape::TwoStage
        }
        DisplayArm::Direct | DisplayArm::WideBlank => PassShape::Direct,
        DisplayArm::SrgbToDisplay
        | DisplayArm::P3ToSrgb
        | DisplayArm::P3ToDisplay
        | DisplayArm::P3ToScRgb => PassShape::TwoStage,
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
///
/// The latch is the LEGACY sRGB segment's quality ratchet (#130's
/// effect-failure path is its only setter), and the `matches!` arm
/// below names exactly the stages it governs — the wide
/// `GpuEffectP3To*` stages pass through unchanged. That is by design
/// (#154): the wide stages' failure ladder is D6's own SURFACE
/// ratchet (an AC-arm failure drops the face to legacy, total
/// failure re-derives to F16Srgb), which is #156's separate latch —
/// until that lands a wide stage can never SET the latch (it never
/// runs), and an sRGB row's latch must not eat a wide stage either.
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
/// segment — master, LevelCache, uploads, tiles. `Srgb` is the 8-bit
/// era's single space (D10's known limitation: wide-gamut sources are
/// clipped through it); `F16Srgb` is L1's FP16 master (ADR 0003: the
/// sRGB EOTF encoding values held as f16, gamma domain); `F16P3` is
/// L2's wide-gamut master (ADR 0004 D2): a Display-P3-D65 container
/// whose halves carry P3-primary values in the destination profile's
/// gamma domain (the P3 TRC is an sRGB-shaped curve) — 8 bytes per
/// pixel exactly like `F16Srgb`, so every #144 byte budget and its
/// derivations hold unchanged. #154 landed the VARIANT and its
/// bookkeeping only (ADR 0004 impact item 1): the accounting arms and
/// the decision table's third dimension. #155 landed Stage 1's
/// destination selection and the F16P3 production gate
/// (pixels.rs's `from_f16_halves_wide`, reached from the loader's
/// wide arm); `master_content_space` itself still answers Srgb/F16Srgb
/// only — the wide mark travels with the transform's own halves, not
/// through the depth gate. #140 added the first
/// variant and the gating; #141 landed the consumers — the LevelCache,
/// upload and tile keys carry the mark (a master-side property, part of
/// every key per ADR 0003 D1), and the direct-read seams dispatch on it.
/// The ordering/hash derives ride the key structs (`TileKey` orders by
/// field order, the tile HashMap hashes by it); the Ord order only has
/// to be deterministic — `F16P3` sits after `F16Srgb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ContentSpace {
    Srgb,
    F16Srgb,
    /// The wide-gamut master (#155, ADR 0004 D2): minted ONLY through
    /// pixels.rs's `from_f16_halves_wide`, the single production gate —
    /// reached from the loader's transform arm when Stage 1's ICC
    /// transform runs against the P3 destination profile (a hardware
    /// session's foreign tagged source). #154 landed the variant and
    /// its bookkeeping; #155 wired the gate and Stage 1's destination
    /// selection.
    F16P3,
}

/// The L1 master's per-input gate (ADR 0003 D1): an image earns the
/// FP16 master only when there is precision worth keeping — either
/// Stage 1 actually applied the embedded-profile transform (a 16-bit
/// CMM output would be clipped by an 8-bit master) or the source
/// decodes deeper than 8 bits (the loader's old `into_rgba8()` was the
/// truncation). The untagged 8-bit majority stays `Srgb`, byte-ident
/// to the 8-bit era — they gain nothing from f16 and must not pay its
/// ×2 cache footprint.
pub(crate) fn master_content_space(
    source_bits_per_sample: u16,
    transform_applied: bool,
) -> ContentSpace {
    if source_bits_per_sample > 8 || transform_applied {
        ContentSpace::F16Srgb
    } else {
        ContentSpace::Srgb
    }
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
    /// The user display-effect chain (#185, ADR 0006 D7): a chain change
    /// re-keys the output the same way an intent flip does — the toggle
    /// must show as a new `output_gen` on #126's dump channel. The term
    /// is the LIVE chain (what the gpu segment actually runs), never the
    /// config key: a D9 build-failure drop is an output decision change
    /// too, and the config key is persistence only. On WARP the term
    /// normalizes to empty inside [`fingerprint_for`] — the D5 exclusion
    /// means the chain cannot change the output there, so two chain
    /// states with identical output must fingerprint identically.
    pub(crate) chain: EffectChain,
}

/// The ACM diagnostic from `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` —
/// a label for the sRGB content rows, the AC arm's judge for the wide
/// ones (#127 D3 as revised by ADR 0004 D4). Per wingdi.h 26100
/// (probe P1's SDK transcription; the docs page is gone): bit 0 =
/// advancedColorSupported, bit 1 = advancedColorEnabled, bit 2 =
/// wideColorEnforced, bit 3 = advancedColorForceDisabled. `Unknown`
/// covers every read failure — #134's P1 probe corrected the old
/// "whole family fails in agent contexts" verdict to a PS-context
/// artifact: in-process the type 9 call answers reliably (rc = 0,
/// byte-stable, flags 0x3 on the probe machine), and any nonzero rc
/// still lands here honestly. #134's ACM read (`display_profile.rs`,
/// the type 9 target read) is what produces Off/On. On the narrow
/// rows it rides `fingerprint_for` as the `ac` term and nothing else;
/// on an `F16P3` row bit 1 IS the arm decision ("does riviv declare
/// itself the interpreter of its own buffer"), outranking the WCS
/// query (P-D's finding: ACM-state getters can answer non-empty, so
/// the query must not veto the arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AcState {
    Off,
    On,
    Unknown,
}

/// The digest of everything that changes the output: which stage
/// runs, which OUTPUT FACE the decision landed on (ADR 0004 D4's
/// surface term, #154), which destination profile it targets (byte
/// hash), the ACM diagnostic, the backend, and the render policy.
/// Equal fingerprints mean interchangeable output resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OutputFingerprint {
    pub(crate) stage: TransformStage,
    pub(crate) profile_hash: u64,
    pub(crate) ac: AcState,
    pub(crate) backend: Backend,
    pub(crate) surface: OutputSurface,
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

/// The wiring fingerprint (#126 established it, #130 swapped the first
/// placeholders for real inputs, #134 swapped the last, #154 widened
/// the table, #156 made the surface term latch-aware): what the output
/// actually went through. The judge — the WCS display-profile query — is
/// REAL here (the modern getter, resolved per output-decision point);
/// the stage term is the EFFECTIVE stage (`desired_output`'s stage
/// clamped by the session latch — a latched degrade switches the narrow
/// segment off, and the flip is a fingerprint transition the tracker
/// mints a new generation for; the wide stages pass the latch, see
/// [`effective_stage`]); the profile bytes are the real destination
/// profile's (`None` = the query carries none — Unknown, NoProfile, or
/// an unreadable file — and the digest is the empty profile's, pinned
/// so an absent term never drifts); and `ac` is the REAL ACM diagnostic
/// off the judged monitor's target (type 9 bit 1 — on the narrow rows a
/// label for the dump reader, on the wide rows the AC arm's judge per
/// ADR 0004 D4; every read failure is `Unknown`). The `content_class`
/// input is the third dimension's axis: the two narrow classes
/// fingerprint IDENTICALLY cell for cell, the wide class adds the
/// surface/stage arms. The surface term is [`clamped_surface`]'s answer
/// — the desired face WITH the AC surface ratchet's clamp (#156): a
/// latched AC arm folds the wide AC cells back to Legacy + the legacy
/// wide stage word, so the fingerprint tracks the resources the session
/// actually keeps. `ac_surface_latched` is that latch; the WIDE EFFECT
/// latch deliberately does NOT enter — it is a transient on the way to a
/// re-derivation, after which the content class turns narrow and the
/// stage words change on their own (a fingerprint term would mint a gen
/// for a state that exists for one paint). The chain term (#185) is the
/// caller's LIVE chain normalized for WARP (see [`OutputPolicy::chain`]).
// The spec's own signature, like `display_arm` above it: each term names
// the input its ratchet or decision owns — collapsing them into a struct
// would hide exactly that mapping.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fingerprint_for(
    backend: Backend,
    query: DisplayProfileQuery,
    degraded_latched: bool,
    profile_bytes: Option<&[u8]>,
    ac: AcState,
    content_class: ContentSpace,
    ac_surface_latched: bool,
    chain: EffectChain,
) -> OutputFingerprint {
    let desired = desired_output(backend, query, ac, content_class);
    let surface = clamped_surface(backend, query, ac, content_class, ac_surface_latched);
    // The stage term: the AC fold-back also rewrites the wide stage word
    // (the scRGB arm cannot run on the legacy face it folded to); the
    // narrow latch clamps the rest through effective_stage as before.
    let stage = if desired.surface == OutputSurface::AcScRgb && surface == OutputSurface::Legacy {
        wide_legacy_stage(query)
    } else {
        effective_stage(desired.stage, degraded_latched)
    };
    // ADR 0006 D5: WARP never runs the chain (#127's hard exclusion,
    // inherited) — the exclusion is a decision-table row, so the
    // fingerprint folds it in here: on WARP every chain state produces
    // the identical output and must fingerprint identically (a WARP
    // toggle mints no generation).
    let chain = if backend == Backend::Warp {
        EffectChain::default()
    } else {
        chain
    };
    OutputFingerprint {
        stage,
        profile_hash: profile_hash(profile_bytes.unwrap_or(&[])),
        ac,
        backend,
        surface,
        policy: OutputPolicy {
            intent: RenderIntent::RelativeColorimetric,
            quality: RenderQuality::Best,
            chain,
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
    ///
    /// `content_class` is a RECORD field only (#154): the transition
    /// semantics stay fingerprint-only — a content-space flip that
    /// leaves the fingerprint unchanged re-identifies to the same gen
    /// with the field updated, because the output resources are not
    /// keyed on the class (the master/tile keys carry the real class
    /// themselves — #134's anti-placeholder rule means the identity
    /// still records what is actually on display, never a stand-in).
    pub(crate) fn identify(
        &mut self,
        fingerprint: OutputFingerprint,
        content_class: ContentSpace,
    ) -> OutputIdentity {
        if self.active != Some(fingerprint) {
            self.next_gen += 1;
            self.active = Some(fingerprint);
        }
        OutputIdentity {
            output_gen: self.next_gen,
            fingerprint,
            content_space: content_class,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RendererKind;
    use crate::tile::TileKey;

    #[test]
    fn arm_master_space_answers_each_graph_arm_s_own_family() {
        // Every arm that builds a graph must imply a space its own check
        // row accepts (gpu.rs build_effect_graph's table): the P3 arms
        // are wide-only, the narrow effect arm narrow-only — a blank
        // draw fed the arm's implication can never meet the _ row.
        for arm in [
            DisplayArm::SrgbToDisplay,
            DisplayArm::P3ToSrgb,
            DisplayArm::P3ToDisplay,
            DisplayArm::P3ToScRgb,
        ] {
            let space = arm_master_space(arm);
            assert_eq!(
                matches!(space, ContentSpace::F16P3),
                matches!(
                    arm,
                    DisplayArm::P3ToSrgb | DisplayArm::P3ToDisplay | DisplayArm::P3ToScRgb
                ),
                "arm {arm:?} implies {space:?}"
            );
        }
        // The graph-less arms' mapping is inert (no consumer reads it)
        // but pinned so a future arm lands consciously.
        assert_eq!(arm_master_space(DisplayArm::Direct), ContentSpace::Srgb);
        assert_eq!(arm_master_space(DisplayArm::WideBlank), ContentSpace::Srgb);
    }

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
            surface: OutputSurface::Legacy,
            policy: OutputPolicy {
                intent: RenderIntent::RelativeColorimetric,
                quality: RenderQuality::Best,
                chain: EffectChain::default(),
            },
        }
    }

    /// The pre-#185 pins' call shape: every table test below answers for
    /// a session with NO user effect chain — the empty chain is exactly
    /// the zero-perturbation posture (D8), so the wrapper pins it once
    /// instead of spelling it at every call site. The chain TERM itself
    /// has its own tests (see the fingerprint-chain block).
    fn fp(
        backend: Backend,
        query: DisplayProfileQuery,
        degraded_latched: bool,
        profile_bytes: Option<&[u8]>,
        ac: AcState,
        content_class: ContentSpace,
        ac_surface_latched: bool,
    ) -> OutputFingerprint {
        fingerprint_for(
            backend,
            query,
            degraded_latched,
            profile_bytes,
            ac,
            content_class,
            ac_surface_latched,
            EffectChain::default(),
        )
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
        let a1 = tracker.identify(a, ContentSpace::Srgb);
        assert_eq!(
            a1.output_gen, 1,
            "the first identity is gen 1 (zero stays sentinel)"
        );
        assert_eq!(tracker.identify(b, ContentSpace::Srgb).output_gen, 2);
        let a2 = tracker.identify(a, ContentSpace::Srgb);
        assert_eq!(
            a2.output_gen, 3,
            "the B -> A change must mint a new gen, not restore gen 1"
        );
        assert_eq!(
            a2.fingerprint, a1.fingerprint,
            "the reuse key survives the round trip — A's cached output resources stay valid"
        );
        assert_eq!(
            tracker.identify(a, ContentSpace::Srgb).output_gen,
            3,
            "re-identifying the unchanged decision is idempotent"
        );
        let c = fingerprint(TransformStage::None, 0xccc);
        assert_eq!(
            tracker.identify(c, ContentSpace::Srgb).output_gen,
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
        let first = tracker.identify(fp, ContentSpace::Srgb).output_gen;
        fp.ac = AcState::On;
        assert_ne!(tracker.identify(fp, ContentSpace::Srgb).output_gen, first);
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
    fn the_real_input_fingerprint_pins_every_cell() {
        // The constructor (see its doc): the judge is real, the stage is
        // the EFFECTIVE one (the latch clamps GpuEffect down to None),
        // the digest is the real bytes' (or the pinned empty one when the
        // query carries none), and since #134 the ACM term is the REAL
        // type 9 read passed through verbatim — every input state lands
        // in the field, so the read's swap is a visible transition, not
        // a drift. Every cell pinned for exactly that.
        let bytes: &[u8] = &[0xab, 0xcd, 0xef];
        for backend in both_backends() {
            for query in all_queries() {
                for latched in [false, true] {
                    for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                        let cell = fp(
                            backend,
                            query,
                            latched,
                            Some(bytes),
                            ac,
                            ContentSpace::Srgb,
                            false,
                        );
                        let expected_stage =
                            effective_stage(desired_stage(backend, query), latched);
                        assert_eq!(
                            cell.stage, expected_stage,
                            "{backend:?}/{query:?}/{latched}"
                        );
                        assert_eq!(cell.surface, OutputSurface::Legacy);
                        assert_eq!(cell.profile_hash, profile_hash(bytes));
                        assert_eq!(cell.ac, ac, "the ac diagnostic passes through verbatim");
                        assert_eq!(cell.backend, backend);
                        assert_eq!(cell.policy.intent, RenderIntent::RelativeColorimetric);
                        assert_eq!(cell.policy.quality, RenderQuality::Best);

                        // No bytes to hash: the pinned empty digest — an
                        // absent term must never drift.
                        let fp_empty = fp(
                            backend,
                            query,
                            latched,
                            None,
                            AcState::Unknown,
                            ContentSpace::Srgb,
                            false,
                        );
                        assert_eq!(fp_empty.profile_hash, profile_hash(b""));
                    }
                }
            }
        }
    }

    #[test]
    fn backend_latch_and_bytes_transitions_mint_new_gens_through_the_real_constructor() {
        // The erratum semantics (Codex P2, PR #128) through the REAL
        // constructor: the live terms are backend, stage (via the latch
        // and the judge), and the profile bytes — each flip is a
        // fingerprint transition, so the dump channel sees a new gen;
        // re-identifying an unchanged decision stays idempotent. The ac
        // term is held constant here (On throughout) because the bare-ac
        // transition gets its own constructor pin below.
        use DisplayProfileQuery::Profile;
        use DisplayProfileSpace::Custom;
        use TransformStage::{GpuEffect, None as NoStage};
        let adobe = b"TPLCD_8BAF_AdobeRGB.icm-bytes".as_slice();
        let mut tracker = OutputTracker::default();
        // hw + custom + unlatched: the effect stage.
        let hw_effect = fp(
            Backend::Hardware,
            Profile(Custom),
            false,
            Some(adobe),
            AcState::On,
            ContentSpace::Srgb,
            false,
        );
        assert_eq!(hw_effect.stage, GpuEffect);
        assert_eq!(
            tracker.identify(hw_effect, ContentSpace::Srgb).output_gen,
            1
        );
        // The session latch flips the effective stage: a transition.
        let hw_latched = fp(
            Backend::Hardware,
            Profile(Custom),
            true,
            Some(adobe),
            AcState::On,
            ContentSpace::Srgb,
            false,
        );
        assert_eq!(hw_latched.stage, NoStage);
        assert_eq!(
            tracker.identify(hw_latched, ContentSpace::Srgb).output_gen,
            2
        );
        // A backend flip on top: another transition.
        let warp = fp(
            Backend::Warp,
            Profile(Custom),
            false,
            Some(adobe),
            AcState::On,
            ContentSpace::Srgb,
            false,
        );
        assert_eq!(warp.stage, NoStage, "the WARP hard exclusion");
        assert_eq!(tracker.identify(warp, ContentSpace::Srgb).output_gen, 3);
        // New profile bytes under the same decision shape: the digest is
        // a fingerprint term, so a profile change mints a new gen too.
        let other = fp(
            Backend::Warp,
            Profile(Custom),
            false,
            Some(b"a-different-display-profile".as_slice()),
            AcState::On,
            ContentSpace::Srgb,
            false,
        );
        assert_eq!(tracker.identify(other, ContentSpace::Srgb).output_gen, 4);
        // An ordinary same-decision re-identify (a same-kind rebuild):
        // idempotent, same gen.
        assert_eq!(tracker.identify(other, ContentSpace::Srgb).output_gen, 4);
    }

    #[test]
    fn a_bare_ac_flip_mints_a_new_generation_through_the_real_constructor() {
        // #134's "the last placeholder is real" pin: with the name-level
        // judge's inputs (backend, query, latch, bytes) all standing
        // still, an ACM toggle alone is a fingerprint transition — the
        // freshness gate feeds exactly this shape, so a bare ac flip
        // reaches the establishment and walks the dump channel's gen
        // (A→B→A style, every transition mints).
        let bytes: &[u8] = &[0xde, 0xad, 0xbe, 0xef];
        let query = DisplayProfileQuery::Profile(DisplayProfileSpace::Custom);
        let mut tracker = OutputTracker::default();
        let off = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::Off,
            ContentSpace::Srgb,
            false,
        );
        assert_eq!(tracker.identify(off, ContentSpace::Srgb).output_gen, 1);
        let on = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::On,
            ContentSpace::Srgb,
            false,
        );
        let on_identity = tracker.identify(on, ContentSpace::Srgb);
        assert_ne!(
            on_identity.fingerprint, off,
            "the ac term is part of the fingerprint, so the flip moves it"
        );
        assert_eq!(on_identity.output_gen, 2);
        // Back to Off: a new gen again (transition semantics, not memo).
        assert_eq!(
            tracker.identify(off, ContentSpace::Srgb).output_gen,
            3,
            "the Off→On→Off round trip walks gens 1, 2, 3"
        );
        // A read failure joining the mix is a third state, also a move.
        let unknown = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::Unknown,
            ContentSpace::Srgb,
            false,
        );
        assert_eq!(tracker.identify(unknown, ContentSpace::Srgb).output_gen, 4);
    }

    #[test]
    fn the_ac_label_never_feeds_the_narrow_rows_stage_decision_in_any_cell() {
        // The "declaration = no declaration" behavior pin (D3), SCOPED
        // by ADR 0004 D4 (#154): on the sRGB content rows the ACM
        // diagnostic is a fingerprint LABEL, never an input of the
        // decision — across the whole (backend, query, latch) space,
        // all three ac states produce the IDENTICAL effective stage
        // (and the identical non-ac fingerprint terms), so the picture
        // a user sees never moves when the ACM bit alone moves. The
        // wide rows are the deliberate exception (the AC-arm contrast
        // is pinned in its own test below).
        let bytes: &[u8] = &[0xab, 0xcd, 0xef];
        for backend in both_backends() {
            for query in all_queries() {
                for latched in [false, true] {
                    let stages: Vec<_> = [AcState::Off, AcState::On, AcState::Unknown]
                        .iter()
                        .map(|&ac| {
                            fp(
                                backend,
                                query,
                                latched,
                                Some(bytes),
                                ac,
                                ContentSpace::Srgb,
                                false,
                            )
                            .stage
                        })
                        .collect();
                    assert!(
                        stages[0] == stages[1] && stages[1] == stages[2],
                        "stage must not move with ac: {backend:?}/{query:?}/{latched} -> {stages:?}"
                    );
                    // And the rest of the fingerprint agrees too: equal
                    // except for the ac term itself.
                    let a = fp(
                        backend,
                        query,
                        latched,
                        Some(bytes),
                        AcState::Off,
                        ContentSpace::Srgb,
                        false,
                    );
                    let b = fp(
                        backend,
                        query,
                        latched,
                        Some(bytes),
                        AcState::On,
                        ContentSpace::Srgb,
                        false,
                    );
                    assert_eq!(a.stage, b.stage);
                    assert_eq!(a.profile_hash, b.profile_hash);
                    assert_eq!(a.backend, b.backend);
                    assert_eq!(a.surface, b.surface);
                    assert_eq!(a.policy, b.policy);
                    assert_ne!(a, b, "only the ac term differs");
                }
            }
        }
    }

    // ---- the third dimension (#154, ADR 0004 D4) ----

    #[test]
    fn narrow_content_reproduces_the_pre_154_table_in_every_cell() {
        // D1's hard constraint: ordinary and F16Srgb content keep the
        // EXACT pre-#154 table — every (backend, query, ac) cell of
        // both narrow classes answers {Legacy, desired_stage(b, q)},
        // the same value `desired_stage` itself gives (whose body this
        // ticket did not touch).
        for content_class in [ContentSpace::Srgb, ContentSpace::F16Srgb] {
            for backend in both_backends() {
                for query in all_queries() {
                    for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                        let out = desired_output(backend, query, ac, content_class);
                        assert_eq!(
                            out,
                            DesiredOutput {
                                surface: OutputSurface::Legacy,
                                stage: desired_stage(backend, query),
                            },
                            "{content_class:?}/{backend:?}/{query:?}/{ac:?} must be the today cell"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_ac_state_gates_the_wide_arm_but_stays_inert_for_narrow_content() {
        // #134's zero-effect pin, continued and SCOPED by ADR 0004 D4:
        // narrow content — all three ac states answer identically
        // (surface and stage, every cell). Wide content on hardware is
        // the contrast the revision creates: On flips the AC arm while
        // Off and Unknown stay legacy — and a failed read (Unknown) is
        // NOT an arm signal, so Off ≡ Unknown there.
        for content_class in [ContentSpace::Srgb, ContentSpace::F16Srgb] {
            for backend in both_backends() {
                for query in all_queries() {
                    let off = desired_output(backend, query, AcState::Off, content_class);
                    let on = desired_output(backend, query, AcState::On, content_class);
                    let unknown = desired_output(backend, query, AcState::Unknown, content_class);
                    assert_eq!(off, on, "narrow rows ignore ac: {backend:?}/{query:?}");
                    assert_eq!(on, unknown, "narrow rows ignore ac: {backend:?}/{query:?}");
                }
            }
        }
        for query in all_queries() {
            let off = desired_output(Backend::Hardware, query, AcState::Off, ContentSpace::F16P3);
            let on = desired_output(Backend::Hardware, query, AcState::On, ContentSpace::F16P3);
            let unknown = desired_output(
                Backend::Hardware,
                query,
                AcState::Unknown,
                ContentSpace::F16P3,
            );
            assert_eq!(
                on.surface,
                OutputSurface::AcScRgb,
                "the AC declaration arm fires on wide content"
            );
            assert_ne!(
                off, on,
                "the bit outranks the judge on wide content (D4): On vs Off differ"
            );
            assert_eq!(
                off, unknown,
                "Off ≡ Unknown: a failed ACM read is not an arm signal"
            );
        }
    }

    #[test]
    fn wide_content_on_hardware_maps_by_the_ac_state_and_the_judge() {
        // The F16P3 × hardware column of D1's matrix, cell by cell: AC
        // on → the scRGB arm whatever the judge answered; AC off or
        // unknown → the legacy face, with the judge deciding WHICH
        // legacy effect (a custom display profile earns the P3→display
        // leg — the domain half's first leg; every other judge answer
        // the constant P3→sRGB pass-through).
        for query in all_queries() {
            let on = desired_output(Backend::Hardware, query, AcState::On, ContentSpace::F16P3);
            assert_eq!(on.surface, OutputSurface::AcScRgb, "{query:?}");
            assert_eq!(on.stage, TransformStage::GpuEffectP3ToScRgb, "{query:?}");
        }
        for ac in [AcState::Off, AcState::Unknown] {
            let custom = desired_output(
                Backend::Hardware,
                DisplayProfileQuery::Profile(DisplayProfileSpace::Custom),
                ac,
                ContentSpace::F16P3,
            );
            assert_eq!(custom.surface, OutputSurface::Legacy);
            assert_eq!(
                custom.stage,
                TransformStage::GpuEffectP3ToDisplay,
                "ac={ac:?}: the judged display profile is the wide pass's destination"
            );
            for query in [
                DisplayProfileQuery::NoProfile,
                DisplayProfileQuery::Unknown,
                DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent),
            ] {
                let out = desired_output(Backend::Hardware, query, ac, ContentSpace::F16P3);
                assert_eq!(out.surface, OutputSurface::Legacy);
                assert_eq!(
                    out.stage,
                    TransformStage::GpuEffectP3ToSrgb,
                    "ac={ac:?}/{query:?}: no display verdict to target → the constant pass-through"
                );
            }
        }
    }

    #[test]
    fn wide_content_on_warp_is_the_unreachable_total_function_completion() {
        // D5/#155: Stage 1's destination selection never mints an F16P3
        // master in a WARP session (the halves would have to display
        // through a pass WARP cannot run), so this cell must be
        // UNREACHABLE — reaching it at runtime means the #155
        // production gate is broken. The value itself is only the
        // total-function completion of the table.
        for query in all_queries() {
            for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                let out = desired_output(Backend::Warp, query, ac, ContentSpace::F16P3);
                assert_eq!(
                    out,
                    DesiredOutput {
                        surface: OutputSurface::Legacy,
                        stage: TransformStage::None,
                    },
                    "warp/{query:?}/{ac:?}: the D5 completion cell"
                );
            }
        }
    }

    #[test]
    fn wide_content_on_hardware_never_lands_on_identity_or_dwm_acm() {
        // The fake-color ban (D6's philosophy applied to the table):
        // P3 halves displayed as sRGB code values recolor every
        // super-sRGB pixel — the ONE outcome the wide rows must never
        // produce. No (query, ac) cell on hardware may answer None or
        // DwmAcm; there is always an effect carrying the container's
        // semantics.
        for query in all_queries() {
            for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                let stage = desired_output(Backend::Hardware, query, ac, ContentSpace::F16P3).stage;
                assert_ne!(
                    stage,
                    TransformStage::None,
                    "wide content must not display uncorrected: {query:?}/{ac:?}"
                );
                assert_ne!(
                    stage,
                    TransformStage::DwmAcm,
                    "wide content must not lean on the OS's sRGB-era management: {query:?}/{ac:?}"
                );
            }
        }
    }

    #[test]
    fn warp_never_selects_any_gpu_effect_variant_anywhere_in_the_widened_space() {
        // Hard exclusion #1, widened to the third dimension (#154): the
        // software path must not grow a GPU-only effect — neither the
        // original GpuEffect nor any of the wide GpuEffectP3To* arms —
        // over the FULL input space (every judge, every ac state, every
        // content class).
        for query in all_queries() {
            for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                for content_class in [
                    ContentSpace::Srgb,
                    ContentSpace::F16Srgb,
                    ContentSpace::F16P3,
                ] {
                    let stage = desired_output(Backend::Warp, query, ac, content_class).stage;
                    assert!(
                        !matches!(
                            stage,
                            TransformStage::GpuEffect
                                | TransformStage::GpuEffectP3ToSrgb
                                | TransformStage::GpuEffectP3ToDisplay
                                | TransformStage::GpuEffectP3ToScRgb
                        ),
                        "warp must not run a gpu effect: {query:?}/{ac:?}/{content_class:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn wide_stages_pass_the_legacy_quality_latch_unchanged() {
        // The latch (#130's quality ratchet) governs only the legacy
        // sRGB segment's effect stages; the wide stages are NOT its
        // jurisdiction — their failure ladder is D6's own SURFACE
        // ratchet (#156's separate latch, not yet landed). An sRGB
        // row's latch must not eat a wide stage, and a wide stage can
        // never set the latch (it never runs before #155/#156).
        for wide in [
            TransformStage::GpuEffectP3ToSrgb,
            TransformStage::GpuEffectP3ToDisplay,
            TransformStage::GpuEffectP3ToScRgb,
        ] {
            assert_eq!(
                effective_stage(wide, true),
                wide,
                "the legacy latch must not clamp {wide:?}"
            );
            assert_eq!(effective_stage(wide, false), wide);
        }
    }

    #[test]
    fn the_fingerprint_surface_is_legacy_in_every_narrow_cell_and_narrow_classes_hash_alike() {
        // The surface term's zero-change pin for today (#154): across
        // the whole (backend, query, latch, ac, bytes) grid, both
        // narrow content classes answer surface = Legacy AND
        // bit-identical fingerprints to each other — today's sessions
        // mint zero new generations from the third dimension's arrival
        // and the smoke132 identity counts stay exact.
        let bytes: &[u8] = &[0xab, 0xcd, 0xef];
        for backend in both_backends() {
            for query in all_queries() {
                for latched in [false, true] {
                    for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                        for with_bytes in [Some(bytes), None] {
                            let srgb = fp(
                                backend,
                                query,
                                latched,
                                with_bytes,
                                ac,
                                ContentSpace::Srgb,
                                false,
                            );
                            let f16 = fp(
                                backend,
                                query,
                                latched,
                                with_bytes,
                                ac,
                                ContentSpace::F16Srgb,
                                false,
                            );
                            assert_eq!(
                                srgb, f16,
                                "the narrow classes fingerprint alike: \
                                 {backend:?}/{query:?}/{latched}/{ac:?}"
                            );
                            assert_eq!(srgb.surface, OutputSurface::Legacy);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_fingerprint_surface_follows_the_wide_matrix() {
        // The wide cells of the same grid (ADR 0004 D4): the surface
        // term answers the matrix — hardware + ac=On → AcScRgb, every
        // other narrow-shaped cell Legacy — and the stage term is the
        // effective wide stage (the legacy latch passes it through, per
        // the wide-stages pin above; the surface term never sees the
        // latch at all).
        let bytes: &[u8] = &[0xab, 0xcd, 0xef];
        for query in all_queries() {
            for latched in [false, true] {
                let on = fp(
                    Backend::Hardware,
                    query,
                    latched,
                    Some(bytes),
                    AcState::On,
                    ContentSpace::F16P3,
                    false,
                );
                assert_eq!(on.surface, OutputSurface::AcScRgb, "{query:?}/{latched}");
                assert_eq!(
                    on.stage,
                    TransformStage::GpuEffectP3ToScRgb,
                    "{query:?}/{latched}"
                );
                for ac in [AcState::Off, AcState::Unknown] {
                    let fp = fp(
                        Backend::Hardware,
                        query,
                        latched,
                        Some(bytes),
                        ac,
                        ContentSpace::F16P3,
                        false,
                    );
                    assert_eq!(
                        fp.surface,
                        OutputSurface::Legacy,
                        "{query:?}/{latched}/{ac:?}"
                    );
                    let expected = match query {
                        DisplayProfileQuery::Profile(DisplayProfileSpace::Custom) => {
                            TransformStage::GpuEffectP3ToDisplay
                        }
                        _ => TransformStage::GpuEffectP3ToSrgb,
                    };
                    assert_eq!(fp.stage, expected, "{query:?}/{latched}/{ac:?}");
                }
            }
        }
    }

    #[test]
    fn an_ac_surface_flip_on_wide_content_mints_a_new_generation() {
        // The accepted churn (ADR 0004 已裁项 4, the #132 precedent):
        // switching the display content between the narrow and wide
        // classes on an AC machine flips the fingerprint's surface
        // term, and a fingerprint TRANSITION mints — A (F16Srgb) -> B
        // (F16P3) -> A walks three gens, never memoizing an old one.
        let bytes: &[u8] = &[0xde, 0xad];
        let query = DisplayProfileQuery::Profile(DisplayProfileSpace::Custom);
        let narrow = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::On,
            ContentSpace::F16Srgb,
            false,
        );
        let wide = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::On,
            ContentSpace::F16P3,
            false,
        );
        assert_eq!(narrow.surface, OutputSurface::Legacy);
        assert_eq!(wide.surface, OutputSurface::AcScRgb);
        let mut tracker = OutputTracker::default();
        assert_eq!(
            tracker.identify(narrow, ContentSpace::F16Srgb).output_gen,
            1
        );
        assert_eq!(
            tracker.identify(wide, ContentSpace::F16P3).output_gen,
            2,
            "the surface flip is a fingerprint transition"
        );
        assert_eq!(
            tracker.identify(narrow, ContentSpace::F16Srgb).output_gen,
            3,
            "the A -> B -> A round trip walks 1, 2, 3 (transition semantics, not a memo)"
        );
    }

    #[test]
    fn the_identity_carries_the_real_content_space_and_a_class_flip_alone_keeps_the_gen() {
        // #134's anti-placeholder rule, extended to the identity's
        // class field (#154): identify records the display content's
        // REAL class; the transition semantics stay fingerprint-only —
        // a class flip that leaves the fingerprint unchanged (any
        // narrow pair here) is the same gen with the field updated,
        // because the output resources are not keyed on the class (the
        // master/tile keys carry the real one themselves).
        let mut tracker = OutputTracker::default();
        let fp = fingerprint(TransformStage::None, 0x123);
        let first = tracker.identify(fp, ContentSpace::F16Srgb);
        assert_eq!(first.content_space, ContentSpace::F16Srgb);
        assert_eq!(first.output_gen, 1);
        let same_fp = tracker.identify(fp, ContentSpace::Srgb);
        assert_eq!(
            same_fp.output_gen, 1,
            "fingerprint unchanged → the class record alone is not a transition"
        );
        assert_eq!(
            same_fp.content_space,
            ContentSpace::Srgb,
            "the field updated"
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
                let identity = tracker.identify(fingerprint(stage, 0), ContentSpace::Srgb);
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
        // LevelCache, upload keys, tiles — is display-independent.
        // #141 extended TileKey with the master's content space (ADR
        // 0003 D1: a master-side property, part of the key so a
        // same-session 8-bit/FP16 pair never reads each other's
        // bitmaps — the #127 "output-blind" ruling only ever excluded
        // the OUTPUT identity, which a tile must still never see). The
        // exhaustive struct literal below now pins TileKey to exactly
        // {frame_gen, content_space, level, tx, ty}: adding an
        // output-identity field stops compiling (the mistake R7
        // originally feared: invalidating tiles on a profile change),
        // while the space field stays mandatory. Two different output
        // identities below, one untouched tile key.
        let mut tracker = OutputTracker::default();
        let _out1 = tracker.identify(fingerprint(TransformStage::None, 1), ContentSpace::Srgb);
        let _out2 = tracker.identify(
            fingerprint(TransformStage::GpuEffect, 2),
            ContentSpace::Srgb,
        );
        let key = TileKey {
            frame_gen: 7,
            content_space: ContentSpace::F16Srgb,
            level: 2,
            tx: 3,
            ty: 4,
        };
        assert_eq!(
            key,
            TileKey {
                frame_gen: 7,
                content_space: ContentSpace::F16Srgb,
                level: 2,
                tx: 3,
                ty: 4
            }
        );
    }

    // ---- L1 master gating (#140, ADR 0003 D1) ----

    #[test]
    fn the_untagged_8bit_majority_stays_in_the_srgb_master() {
        // The gate's zero-change arm: a plain 8-bit source with no
        // applied transform gains nothing from f16 (its values ARE
        // 8-bit) and must not pay the ×2 cache footprint.
        assert_eq!(master_content_space(8, false), ContentSpace::Srgb);
    }

    #[test]
    fn deep_sources_and_transformed_frames_earn_the_f16_master() {
        // Both gate arms of ADR 0003 D1: a >8-bit source (PNG16's old
        // `into_rgba8()` truncation had precision to keep) and an
        // actually-applied Stage-1 transform (the 16-bit CMM output
        // an 8-bit master would clip) — either one alone suffices.
        assert_eq!(master_content_space(16, false), ContentSpace::F16Srgb);
        assert_eq!(master_content_space(8, true), ContentSpace::F16Srgb);
        assert_eq!(master_content_space(16, true), ContentSpace::F16Srgb);
    }

    // ---- the executed arm (#156, ADR 0004 D3/D6) ----

    /// The FULL wide-arm cell space: every (judge, ac, latch triple,
    /// stack face) combination the draw wiring can present.
    fn wide_arm_cells() -> impl Iterator<
        Item = (
            DisplayProfileQuery,
            AcState,
            bool,
            bool,
            bool,
            OutputSurface,
        ),
    > {
        all_queries()
            .into_iter()
            .flat_map(|query| {
                [AcState::Off, AcState::On, AcState::Unknown]
                    .into_iter()
                    .map(move |ac| {
                        (
                            query,
                            ac,
                            [false, true],
                            [false, true],
                            [false, true],
                            [OutputSurface::Legacy, OutputSurface::AcScRgb],
                        )
                    })
            })
            .flat_map(|(query, ac, nl, al, wl, faces)| {
                nl.into_iter().flat_map(move |narrow_latched| {
                    al.into_iter().flat_map(move |ac_surface_latched| {
                        wl.into_iter().flat_map(move |wide_effect_latched| {
                            faces.into_iter().map(move |stack_face| {
                                (
                                    query,
                                    ac,
                                    narrow_latched,
                                    ac_surface_latched,
                                    wide_effect_latched,
                                    stack_face,
                                )
                            })
                        })
                    })
                })
            })
    }

    #[test]
    fn narrow_rows_answer_the_pre_156_draw_shape_in_every_cell() {
        // The narrow classes' zero-change pin, on the DRAW arm: across
        // the whole (backend, query, ac, narrow-latch, ac-latch,
        // wide-latch, face) grid, `SrgbToDisplay` fires exactly where
        // the pre-#156 wiring synced the two-phase pass
        // (effective_stage(desired_stage(b, q), latched) == GpuEffect),
        // `Direct` everywhere else — and NOTHING else is ever answered.
        for content_class in [ContentSpace::Srgb, ContentSpace::F16Srgb] {
            for backend in both_backends() {
                for query in all_queries() {
                    for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                        for narrow_latched in [false, true] {
                            for ac_surface_latched in [false, true] {
                                for wide_effect_latched in [false, true] {
                                    for stack_face in
                                        [OutputSurface::Legacy, OutputSurface::AcScRgb]
                                    {
                                        let arm = display_arm(
                                            backend,
                                            query,
                                            ac,
                                            content_class,
                                            narrow_latched,
                                            ac_surface_latched,
                                            wide_effect_latched,
                                            stack_face,
                                        );
                                        let expected = match effective_stage(
                                            desired_stage(backend, query),
                                            narrow_latched,
                                        ) {
                                            TransformStage::GpuEffect => DisplayArm::SrgbToDisplay,
                                            _ => DisplayArm::Direct,
                                        };
                                        assert_eq!(
                                            arm, expected,
                                            "{content_class:?}/{backend:?}/{query:?}/{ac:?}/\
                                             {narrow_latched}/{ac_surface_latched}/\
                                             {wide_effect_latched}/{stack_face:?}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wide_rows_on_warp_always_answer_wide_blank() {
        // The D5 completion cell's draw defense: WARP cannot run a wide
        // effect, so every wide WARP cell (whole latch grid, both faces)
        // blanks — never a direct draw of P3 halves.
        for (query, ac, narrow_latched, ac_surface_latched, wide_effect_latched, stack_face) in
            wide_arm_cells()
        {
            let arm = display_arm(
                Backend::Warp,
                query,
                ac,
                ContentSpace::F16P3,
                narrow_latched,
                ac_surface_latched,
                wide_effect_latched,
                stack_face,
            );
            assert_eq!(
                arm,
                DisplayArm::WideBlank,
                "warp/{query:?}/{ac:?}/{narrow_latched}/{ac_surface_latched}/\
                 {wide_effect_latched}/{stack_face:?}"
            );
        }
    }

    #[test]
    fn wide_rows_on_hardware_map_by_the_latches_and_the_live_face() {
        // The wide hardware grid, cell by cell, against the D6 layering:
        // (1) a latched WIDE EFFECT blanks the frame (the re-derivation
        // is coming; nothing may draw); (2) the latched AC surface folds
        // the AC cell onto the legacy wide stage; (3) an unlatched AC
        // cell runs the scRGB arm only on the AC face — on a stale Legacy
        // face the legacy wide stage keeps the colors CORRECT; (4) the
        // judge-off cells run the legacy wide stages regardless of face.
        for (query, ac, narrow_latched, ac_surface_latched, wide_effect_latched, stack_face) in
            wide_arm_cells()
        {
            let arm = display_arm(
                Backend::Hardware,
                query,
                ac,
                ContentSpace::F16P3,
                narrow_latched,
                ac_surface_latched,
                wide_effect_latched,
                stack_face,
            );
            let label = format!(
                "hw/{query:?}/{ac:?}/{narrow_latched}/{ac_surface_latched}/\
                 {wide_effect_latched}/{stack_face:?}"
            );
            if wide_effect_latched {
                assert_eq!(arm, DisplayArm::WideBlank, "{label}");
                continue;
            }
            let legacy_wide = match query {
                DisplayProfileQuery::Profile(DisplayProfileSpace::Custom) => {
                    DisplayArm::P3ToDisplay
                }
                _ => DisplayArm::P3ToSrgb,
            };
            if ac == AcState::On {
                if ac_surface_latched {
                    assert_eq!(arm, legacy_wide, "{label}");
                } else {
                    match stack_face {
                        OutputSurface::AcScRgb => {
                            assert_eq!(arm, DisplayArm::P3ToScRgb, "{label}")
                        }
                        OutputSurface::Legacy => assert_eq!(arm, legacy_wide, "{label}"),
                    }
                }
            } else {
                // Off ≡ Unknown: the legacy face's wide stages, whatever
                // face the stack physically shows (the face reconciliation
                // owns the mismatch, the colors must not).
                assert_eq!(arm, legacy_wide, "{label}");
            }
        }
    }

    #[test]
    fn a_wide_row_never_answers_a_direct_or_narrow_arm() {
        // The fake-color ban, on the executed arm: over the ENTIRE wide
        // input space (both backends, every judge, every ac, every latch
        // triple, both faces) the answer is never Direct and never
        // SrgbToDisplay — P3 halves only ever travel through a P3-mapping
        // effect or a blank.
        for backend in both_backends() {
            for (query, ac, narrow_latched, ac_surface_latched, wide_effect_latched, stack_face) in
                wide_arm_cells()
            {
                let arm = display_arm(
                    backend,
                    query,
                    ac,
                    ContentSpace::F16P3,
                    narrow_latched,
                    ac_surface_latched,
                    wide_effect_latched,
                    stack_face,
                );
                assert_ne!(arm, DisplayArm::Direct, "{backend:?}/{query:?}/{ac:?}");
                assert_ne!(
                    arm,
                    DisplayArm::SrgbToDisplay,
                    "{backend:?}/{query:?}/{ac:?}"
                );
            }
        }
    }

    #[test]
    fn wide_blank_is_reachable_only_through_the_latched_wide_effect_or_warp() {
        // The blank's reachability pin: on hardware a wide cell answers
        // WideBlank exactly when the wide-effect latch is set — with the
        // latch off, every hardware cell runs an effect (the screen shows
        // something, never a blank from the decision layer).
        for (query, ac, narrow_latched, ac_surface_latched, wide_effect_latched, stack_face) in
            wide_arm_cells()
        {
            let arm = display_arm(
                Backend::Hardware,
                query,
                ac,
                ContentSpace::F16P3,
                narrow_latched,
                ac_surface_latched,
                wide_effect_latched,
                stack_face,
            );
            assert_eq!(
                arm == DisplayArm::WideBlank,
                wide_effect_latched,
                "hw/{query:?}/{ac:?}/{narrow_latched}/{ac_surface_latched}/\
                 {wide_effect_latched}/{stack_face:?}"
            );
        }
    }

    #[test]
    fn the_clamped_surface_folds_the_ac_face_back_only_when_latched() {
        // The one helper three consumers share (fingerprint surface term,
        // display_arm's face answer, the window wiring's reconciliation
        // target): the desired face stands until the AC latch fires, then
        // the AC cells fold to Legacy — and ONLY the AC cells (the
        // already-legacy cells cannot move, in any cell of the grid).
        for content_class in [
            ContentSpace::Srgb,
            ContentSpace::F16Srgb,
            ContentSpace::F16P3,
        ] {
            for backend in both_backends() {
                for query in all_queries() {
                    for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                        let desired = desired_output(backend, query, ac, content_class).surface;
                        let clamped = clamped_surface(backend, query, ac, content_class, true);
                        if desired == OutputSurface::AcScRgb {
                            assert_eq!(clamped, OutputSurface::Legacy);
                        } else {
                            assert_eq!(clamped, desired, "a legacy cell must not move");
                        }
                        // Unlatched: the raw desired face.
                        assert_eq!(
                            clamped_surface(backend, query, ac, content_class, false),
                            desired
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_ac_surface_latch_folds_the_wide_fingerprint_back_to_the_legacy_wide_stage() {
        // The #156 fingerprint pin: on the wide AC cell, latching the AC
        // surface is a fingerprint TRANSITION — surface AcScRgb→Legacy AND
        // stage p3_to_scrgb→the judge's legacy wide stage word — so the
        // dump channel's gen moves exactly when the session's output
        // resources change shape. The narrow latch alone must NOT touch
        // the wide cell (the wide stages pass it), and unlatched cells
        // reproduce the #154 matrix exactly.
        let bytes: &[u8] = &[0xab, 0xcd, 0xef];
        let query = DisplayProfileQuery::Profile(DisplayProfileSpace::Custom);
        let open = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::On,
            ContentSpace::F16P3,
            false,
        );
        assert_eq!(open.surface, OutputSurface::AcScRgb);
        assert_eq!(open.stage, TransformStage::GpuEffectP3ToScRgb);
        let folded = fp(
            Backend::Hardware,
            query,
            false,
            Some(bytes),
            AcState::On,
            ContentSpace::F16P3,
            true,
        );
        assert_eq!(folded.surface, OutputSurface::Legacy);
        assert_eq!(
            folded.stage,
            TransformStage::GpuEffectP3ToDisplay,
            "the custom judge keeps its legacy wide stage word through the fold"
        );
        // The pass-through judge folds to the constant wide pass-through.
        for q in [
            DisplayProfileQuery::NoProfile,
            DisplayProfileQuery::Unknown,
            DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent),
        ] {
            let folded = fp(
                Backend::Hardware,
                q,
                false,
                Some(bytes),
                AcState::On,
                ContentSpace::F16P3,
                true,
            );
            assert_eq!(folded.surface, OutputSurface::Legacy, "{q:?}");
            assert_eq!(folded.stage, TransformStage::GpuEffectP3ToSrgb, "{q:?}");
        }
        // The narrow latch does not clamp a wide stage (unlatched AC arm).
        let latched_narrow = fp(
            Backend::Hardware,
            query,
            true,
            Some(bytes),
            AcState::On,
            ContentSpace::F16P3,
            false,
        );
        assert_eq!(latched_narrow.stage, TransformStage::GpuEffectP3ToScRgb);
        assert_eq!(latched_narrow.surface, OutputSurface::AcScRgb);
    }

    #[test]
    fn the_ac_surface_latch_never_moves_a_narrow_fingerprint() {
        // The narrow rows' zero-disturbance pin for the new term: across
        // the whole narrow grid, both ac_surface_latched values produce
        // IDENTICAL fingerprints — the AC face ratchet is a wide-row
        // concept, and today's sessions mint nothing from its arrival.
        let bytes: &[u8] = &[0xab, 0xcd, 0xef];
        for backend in both_backends() {
            for query in all_queries() {
                for latched in [false, true] {
                    for ac in [AcState::Off, AcState::On, AcState::Unknown] {
                        for with_bytes in [Some(bytes), None] {
                            for content_class in [ContentSpace::Srgb, ContentSpace::F16Srgb] {
                                let off = fp(
                                    backend,
                                    query,
                                    latched,
                                    with_bytes,
                                    ac,
                                    content_class,
                                    false,
                                );
                                let on = fp(
                                    backend,
                                    query,
                                    latched,
                                    with_bytes,
                                    ac,
                                    content_class,
                                    true,
                                );
                                assert_eq!(
                                    off, on,
                                    "{backend:?}/{query:?}/{latched}/{ac:?}/{content_class:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- pass_shape: #183's decision table ----

    fn all_arms() -> [DisplayArm; 6] {
        [
            DisplayArm::Direct,
            DisplayArm::SrgbToDisplay,
            DisplayArm::P3ToSrgb,
            DisplayArm::P3ToDisplay,
            DisplayArm::P3ToScRgb,
            DisplayArm::WideBlank,
        ]
    }

    /// The zero-perturbation contract, cell by cell: with the EMPTY chain
    /// every (backend, arm) pair answers exactly the pre-#183 dispatch
    /// shape (Direct/WideBlank draw direct, the color arms stay
    /// two-stage).
    #[test]
    fn empty_chain_reproduces_the_pre_effect_chain_dispatch() {
        for backend in both_backends() {
            for arm in all_arms() {
                let expected = match arm {
                    DisplayArm::Direct | DisplayArm::WideBlank => PassShape::Direct,
                    _ => PassShape::TwoStage,
                };
                assert_eq!(
                    pass_shape(backend, arm, EffectChain::default()),
                    expected,
                    "{backend:?}/{arm:?}"
                );
            }
        }
    }

    /// The one NEW shape: a user chain on the hardware Direct arm forces
    /// the two-stage form (scene → intermediate → chain → target, no
    /// ColorManagement — ADR 0006's CM-less graph form). Either stage
    /// alone suffices (#191): a wb-only chain is as much a chain as a
    /// sharpen-only one.
    #[test]
    fn chain_forces_two_stage_on_the_hardware_direct_arm() {
        for sharpen in 1..=10u8 {
            assert_eq!(
                pass_shape(
                    Backend::Hardware,
                    DisplayArm::Direct,
                    EffectChain {
                        sharpen,
                        ..EffectChain::default()
                    }
                ),
                PassShape::TwoStage,
                "sharpen={sharpen}"
            );
        }
        for white_balance in -10..=10i8 {
            if white_balance == 0 {
                continue;
            }
            assert_eq!(
                pass_shape(
                    Backend::Hardware,
                    DisplayArm::Direct,
                    EffectChain {
                        white_balance,
                        ..EffectChain::default()
                    }
                ),
                PassShape::TwoStage,
                "white_balance={white_balance}"
            );
        }
        for contrast in -10..=10i8 {
            if contrast == 0 {
                continue;
            }
            assert_eq!(
                pass_shape(
                    Backend::Hardware,
                    DisplayArm::Direct,
                    EffectChain {
                        contrast,
                        ..EffectChain::default()
                    }
                ),
                PassShape::TwoStage,
                "contrast={contrast}"
            );
        }
    }

    /// O3=B's executable cell: WARP never runs the chain — #127's hard
    /// exclusion inherited, so a non-empty chain on a WARP session's
    /// Direct arm stays a direct pass (the effect is dropped, the content
    /// still draws).
    #[test]
    fn warp_inherits_the_127_exclusion_the_chain_never_runs() {
        assert_eq!(
            pass_shape(
                Backend::Warp,
                DisplayArm::Direct,
                EffectChain {
                    sharpen: 10,
                    ..EffectChain::default()
                }
            ),
            PassShape::Direct
        );
    }

    /// The color arms own their two-stage shape (the ColorManagement
    /// effect needs the intermediate); the chain rides INSIDE their graph
    /// and never changes the shape, on either backend.
    #[test]
    fn color_arms_stay_two_stage_whatever_the_chain() {
        for backend in both_backends() {
            for sharpen in [0u8, 5, 10] {
                for arm in [
                    DisplayArm::SrgbToDisplay,
                    DisplayArm::P3ToSrgb,
                    DisplayArm::P3ToDisplay,
                    DisplayArm::P3ToScRgb,
                ] {
                    assert_eq!(
                        pass_shape(
                            backend,
                            arm,
                            EffectChain {
                                sharpen,
                                ..EffectChain::default()
                            }
                        ),
                        PassShape::TwoStage,
                        "{backend:?}/{arm:?}/sharpen={sharpen}"
                    );
                }
            }
        }
    }

    /// WideBlank is blank by contract — a constant field convolved is
    /// itself, and no content may reach the target in any shape (spike
    /// s-d2d-effects, hook evidence 5).
    #[test]
    fn wide_blank_stays_direct_whatever_the_chain() {
        for backend in both_backends() {
            assert_eq!(
                pass_shape(
                    backend,
                    DisplayArm::WideBlank,
                    EffectChain {
                        sharpen: 10,
                        ..EffectChain::default()
                    }
                ),
                PassShape::Direct
            );
        }
    }

    // ---- the fingerprint's chain term (#185, ADR 0006 D7/D5) ----

    /// The fixed-args cell every chain-term test starts from: hardware,
    /// a custom-profile judge (the SrgbToDisplay row), a narrow class.
    fn chain_term_fp(backend: Backend, chain: EffectChain) -> OutputFingerprint {
        fingerprint_for(
            backend,
            DisplayProfileQuery::Profile(DisplayProfileSpace::Custom),
            false,
            Some(b"profile-bytes"),
            AcState::Off,
            ContentSpace::Srgb,
            false,
            chain,
        )
    }

    /// D7: the chain is a fingerprint policy term — a chain change is an
    /// output decision change and must mint a new `output_gen` through
    /// the tracker (the toggle's observable on #126's dump channel).
    #[test]
    fn a_chain_change_is_a_fingerprint_transition_the_tracker_mints() {
        let off = chain_term_fp(Backend::Hardware, EffectChain::default());
        let on = chain_term_fp(
            Backend::Hardware,
            EffectChain {
                sharpen: 3,
                ..EffectChain::default()
            },
        );
        assert_ne!(off, on, "the chain term must ride the policy");
        // A->B->A walks gens 1, 2, 3 — the round trip is a change signal
        // too (OutputTracker's own contract, restated for the chain arm).
        let mut tracker = OutputTracker::default();
        let g1 = tracker.identify(off, ContentSpace::Srgb).output_gen;
        let g2 = tracker.identify(on, ContentSpace::Srgb).output_gen;
        let g3 = tracker.identify(off, ContentSpace::Srgb).output_gen;
        assert_eq!((g1, g2, g3), (1, 2, 3));
    }

    /// D5: WARP never runs the chain, so the term normalizes to empty —
    /// two chain states with byte-identical output fingerprint
    /// identically, and a WARP toggle mints no generation.
    #[test]
    fn warp_normalizes_the_chain_term_the_exclusion_is_a_table_row() {
        let off = chain_term_fp(Backend::Warp, EffectChain::default());
        let on = chain_term_fp(
            Backend::Warp,
            EffectChain {
                sharpen: 10,
                ..EffectChain::default()
            },
        );
        assert_eq!(
            off, on,
            "a WARP session's chain cannot change the output, the fingerprints must agree"
        );
        assert_eq!(off.policy.chain, EffectChain::default());
    }

    /// Every chain level is its own identity on hardware (the combo's 11
    /// values must not collide).
    #[test]
    fn every_sharpen_level_fingerprints_apart_on_hardware() {
        let fps: Vec<_> = (0..=10u8)
            .map(|s| {
                chain_term_fp(
                    Backend::Hardware,
                    EffectChain {
                        sharpen: s,
                        ..EffectChain::default()
                    },
                )
            })
            .collect();
        for i in 0..fps.len() {
            for j in i + 1..fps.len() {
                assert_ne!(fps[i], fps[j], "sharpen {i} collides with sharpen {j}");
            }
        }
    }

    /// #191's mirror: the wb combo's 21 values are 21 identities, and a
    /// wb move never collides with a sharpen move (the two stages are
    /// separate terms, not one packed number).
    #[test]
    fn every_white_balance_level_fingerprints_apart_on_hardware() {
        let wb_fps: Vec<_> = (-10..=10i8)
            .map(|w| {
                chain_term_fp(
                    Backend::Hardware,
                    EffectChain {
                        white_balance: w,
                        ..EffectChain::default()
                    },
                )
            })
            .collect();
        for i in 0..wb_fps.len() {
            for j in i + 1..wb_fps.len() {
                assert_ne!(wb_fps[i], wb_fps[j], "wb {i} collides with wb {j}");
            }
        }
        let sharpen_only = chain_term_fp(
            Backend::Hardware,
            EffectChain {
                sharpen: 3,
                ..EffectChain::default()
            },
        );
        for (idx, fp) in wb_fps.iter().enumerate() {
            // wb = 0 there is the default chain — skip that one cell.
            if idx != 10 {
                assert_ne!(*fp, sharpen_only, "sharpen 3 collides with wb {}", idx - 10);
            }
        }
    }

    /// #193's mirror: the contrast combo's 21 values are 21 identities,
    /// and a contrast move never collides with a sharpen or wb move.
    #[test]
    fn every_contrast_level_fingerprints_apart_on_hardware() {
        let c_fps: Vec<_> = (-10..=10i8)
            .map(|c| {
                chain_term_fp(
                    Backend::Hardware,
                    EffectChain {
                        contrast: c,
                        ..EffectChain::default()
                    },
                )
            })
            .collect();
        for i in 0..c_fps.len() {
            for j in i + 1..c_fps.len() {
                assert_ne!(
                    c_fps[i], c_fps[j],
                    "contrast {i} collides with contrast {j}"
                );
            }
        }
        let sharpen_only = chain_term_fp(
            Backend::Hardware,
            EffectChain {
                sharpen: 3,
                ..EffectChain::default()
            },
        );
        let wb_only = chain_term_fp(
            Backend::Hardware,
            EffectChain {
                white_balance: 3,
                ..EffectChain::default()
            },
        );
        for (idx, fp) in c_fps.iter().enumerate() {
            // contrast = 0 there is the default chain — skip that one cell.
            // The message's level is signed (idx as i32 - 10): a plain
            // `idx - 10` would underflow the usize on the negative half.
            if idx != 10 {
                assert_ne!(
                    *fp,
                    sharpen_only,
                    "sharpen 3 collides with contrast {}",
                    idx as i32 - 10
                );
                assert_ne!(
                    *fp,
                    wb_only,
                    "wb 3 collides with contrast {}",
                    idx as i32 - 10
                );
            }
        }
    }

    // ---- the toggles and the seed (#185, #191) ----

    /// The Sharpen toggle's pure core: on flips off, off re-arms from the
    /// config key, a zero config falls to SHARPEN_TOGGLE_ON, and a
    /// hand-edited out-of-domain key clamps at the edge (the raw int
    /// stays in the ini like every other int key).
    #[test]
    fn toggle_sharpen_flips_off_rearms_from_config_and_clamps() {
        let off = EffectChain::default();
        let on3 = EffectChain {
            sharpen: 3,
            ..EffectChain::default()
        };
        // On -> off, whatever the config says.
        assert_eq!(on3.toggle_sharpen(7).sharpen, 0);
        assert_eq!(
            EffectChain {
                sharpen: 10,
                ..EffectChain::default()
            }
            .toggle_sharpen(10)
            .sharpen,
            0
        );
        // Off -> the config's standing level.
        assert_eq!(off.toggle_sharpen(7).sharpen, 7);
        assert_eq!(off.toggle_sharpen(1).sharpen, 1);
        // Off with no standing level -> the documented default.
        assert_eq!(off.toggle_sharpen(0).sharpen, SHARPEN_TOGGLE_ON);
        assert_eq!(off.toggle_sharpen(-4).sharpen, SHARPEN_TOGGLE_ON);
        // Out-of-domain configs clamp at 10.
        assert_eq!(off.toggle_sharpen(999).sharpen, 10);
    }

    /// #191: the White Balance toggle mirrors the sharpen core with the
    /// one SIGNED-domain difference — a negative config is a real
    /// standing level (cooling) and re-arms as-is; only exactly 0 falls
    /// to WHITE_BALANCE_TOGGLE_ON.
    #[test]
    fn toggle_white_balance_flips_rearms_negatives_and_clamps() {
        let off = EffectChain::default();
        let warm = EffectChain {
            white_balance: 3,
            ..EffectChain::default()
        };
        let cool = EffectChain {
            white_balance: -7,
            ..EffectChain::default()
        };
        // On -> off, whatever the config says.
        assert_eq!(warm.toggle_white_balance(5).white_balance, 0);
        assert_eq!(cool.toggle_white_balance(-7).white_balance, 0);
        // Off -> the config's standing level, negative included.
        assert_eq!(off.toggle_white_balance(5).white_balance, 5);
        assert_eq!(off.toggle_white_balance(-7).white_balance, -7);
        // Off with no standing level -> the documented warm-side default.
        assert_eq!(
            off.toggle_white_balance(0).white_balance,
            WHITE_BALANCE_TOGGLE_ON
        );
        // Out-of-domain configs clamp at the edges.
        assert_eq!(off.toggle_white_balance(999).white_balance, 10);
        assert_eq!(off.toggle_white_balance(-999).white_balance, -10);
    }

    /// #193: the Contrast toggle mirrors the wb core verbatim — a
    /// negative config is a real standing level (softening) and re-arms
    /// as-is; only exactly 0 falls to CONTRAST_TOGGLE_ON.
    #[test]
    fn toggle_contrast_flips_rearms_negatives_and_clamps() {
        let off = EffectChain::default();
        let punchy = EffectChain {
            contrast: 3,
            ..EffectChain::default()
        };
        let soft = EffectChain {
            contrast: -7,
            ..EffectChain::default()
        };
        // On -> off, whatever the config says.
        assert_eq!(punchy.toggle_contrast(5).contrast, 0);
        assert_eq!(soft.toggle_contrast(-7).contrast, 0);
        // Off -> the config's standing level, negative included.
        assert_eq!(off.toggle_contrast(5).contrast, 5);
        assert_eq!(off.toggle_contrast(-7).contrast, -7);
        // Off with no standing level -> the documented punchy-side default.
        assert_eq!(off.toggle_contrast(0).contrast, CONTRAST_TOGGLE_ON);
        // Out-of-domain configs clamp at the edges.
        assert_eq!(off.toggle_contrast(999).contrast, 10);
        assert_eq!(off.toggle_contrast(-999).contrast, -10);
    }

    /// #191/#193's per-stage ownership: each toggle flips ITS stage and
    /// leaves the others exactly where they stood — the three View rows
    /// are independent switches, and a stage-off chain with either other
    /// stage running is not empty.
    #[test]
    fn each_toggle_flips_only_its_own_stage() {
        let all = EffectChain {
            sharpen: 6,
            white_balance: -4,
            contrast: 5,
        };
        let sharpen_off = all.toggle_sharpen(6);
        assert_eq!(sharpen_off.sharpen, 0);
        assert_eq!(
            sharpen_off.white_balance, -4,
            "wb survives the sharpen toggle"
        );
        assert_eq!(
            sharpen_off.contrast, 5,
            "contrast survives the sharpen toggle"
        );
        assert!(!sharpen_off.is_empty(), "a wb+contrast chain still runs");
        let wb_off = all.toggle_white_balance(-4);
        assert_eq!(wb_off.white_balance, 0);
        assert_eq!(wb_off.sharpen, 6, "sharpen survives the wb toggle");
        assert_eq!(wb_off.contrast, 5, "contrast survives the wb toggle");
        assert!(!wb_off.is_empty(), "a sharpen+contrast chain still runs");
        let contrast_off = all.toggle_contrast(5);
        assert_eq!(contrast_off.contrast, 0);
        assert_eq!(
            contrast_off.sharpen, 6,
            "sharpen survives the contrast toggle"
        );
        assert_eq!(
            contrast_off.white_balance, -4,
            "wb survives the contrast toggle"
        );
        assert!(!contrast_off.is_empty(), "a sharpen+wb chain still runs");
        let two_off = contrast_off.toggle_white_balance(-4);
        assert!(!two_off.is_empty(), "the last standing stage still runs");
        assert!(two_off.toggle_sharpen(6).is_empty());
    }

    /// The stack-seed clamp: the config keys are the ini's raw ints, the
    /// chain's domains are the docs enum pages (sharpen 0..=10, white
    /// balance and contrast −10..=10 with both signed edges).
    #[test]
    fn from_config_clamps_into_the_docs_domain() {
        assert_eq!(EffectChain::from_config(0, 0, 0).sharpen, 0);
        assert_eq!(EffectChain::from_config(5, 0, 0).sharpen, 5);
        assert_eq!(EffectChain::from_config(10, 0, 0).sharpen, 10);
        assert_eq!(EffectChain::from_config(300, 0, 0).sharpen, 10);
        assert_eq!(EffectChain::from_config(-1, 0, 0).sharpen, 0);
        assert_eq!(EffectChain::from_config(0, 0, 0).white_balance, 0);
        assert_eq!(EffectChain::from_config(0, 7, 0).white_balance, 7);
        assert_eq!(EffectChain::from_config(0, -7, 0).white_balance, -7);
        assert_eq!(EffectChain::from_config(0, 10, 0).white_balance, 10);
        assert_eq!(EffectChain::from_config(0, -10, 0).white_balance, -10);
        assert_eq!(EffectChain::from_config(0, 300, 0).white_balance, 10);
        assert_eq!(EffectChain::from_config(0, -300, 0).white_balance, -10);
        assert_eq!(EffectChain::from_config(0, 0, 0).contrast, 0);
        assert_eq!(EffectChain::from_config(0, 0, 7).contrast, 7);
        assert_eq!(EffectChain::from_config(0, 0, -7).contrast, -7);
        assert_eq!(EffectChain::from_config(0, 0, 10).contrast, 10);
        assert_eq!(EffectChain::from_config(0, 0, -10).contrast, -10);
        assert_eq!(EffectChain::from_config(0, 0, 300).contrast, 10);
        assert_eq!(EffectChain::from_config(0, 0, -300).contrast, -10);
    }
}
