//! Execution options (C-23; doc 13 §1.2, MODES-D7, MODES-D11; M5-P0 P0-VM5): the preset an
//! isolate runs and its type-feedback mode, given to [`Isolate::new_with`]. Also
//! [`build_info`], the features and the default preset of this build (MC-w17-5), which every
//! result JSON copies so a reading names the build it measured.
//!
//! ```text
//! ExecOptions::jit()      preset jit      feedback full       needs the `jit` feature (M-R6)
//! ExecOptions::jitless()  preset jitless  feedback ics_only   every build
//!   .feedback(Feedback::FULL | Feedback::ICS_ONLY)            the setter; works with both presets
//! Isolate::new(config)  =  Isolate::new_with(config, ExecOptions::default())
//! ExecOptions::default() = jit() with the `jit` feature, else jitless()
//! ```
//!
//! | Rule | Where |
//! |---|---|
//! | `IsolateConfig` is a Frozen H20 core line, so the options are a second argument, not a field (doc 13 §1.2) | [`Isolate::new_with`] |
//! | Opaque: no public field, so a later setter or preset is a new H20 golden line, never a changed one | [`ExecOptions`], [`Preset`], [`Feedback`] (associated consts) |
//! | A preset the build cannot run is an error, never a silent downgrade; it reuses `IsolateError::Config` (M-R6) | [`Isolate::new_with`] |
//! | The jitless preset defaults to `ics_only`; the jit preset to `full` (MODES-D11) | [`ExecOptions::jitless`], [`ExecOptions::jit`] |
//!
//! M5-P0 stores the options (`Runtime::exec`); nothing reads them yet. The feedback mode takes
//! effect in M5-P3 (PROFILE6: the `ics_only` T0 handler table, chosen per `Cx`); until then
//! both modes run the same T0 table. `ics_only` keeps every IC and every object-model input
//! (allocation-site element-kind feedback included) and drops only the `profile.rs` record
//! hooks (MODES-D19).
//!
//! [`Isolate::new_with`]: crate::Isolate::new_with

/// An execution preset (doc 13 §1.2). Opaque: a new preset (hybrid) is a new associated
/// const.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Preset(u8);

impl Preset {
    /// The JIT preset: runtime code generation allowed (needs the `jit` feature).
    pub const JIT: Preset = Preset(0);
    /// The jitless preset: no runtime code generation (every build).
    pub const JITLESS: Preset = Preset(1);

    /// The preset name (`jit`, `jitless`), as result JSON files write it.
    pub const fn name(self) -> &'static str {
        match self.0 {
            0 => "jit",
            _ => "jitless",
        }
    }

    /// The preset named `name` ([`Preset::name`]), if there is one.
    pub fn from_name(name: &str) -> Option<Preset> {
        match name {
            "jit" => Some(Preset::JIT),
            "jitless" => Some(Preset::JITLESS),
            _ => None,
        }
    }

    /// True if this build can run the preset: the jit preset needs the `jit` feature (M-R6,
    /// doc 13:77); the jitless preset runs in every build (M-R3).
    pub const fn available(self) -> bool {
        self.0 != Preset::JIT.0 || cfg!(feature = "jit")
    }
}

/// The type-feedback mode (MODES-D11, MODES-D19). Opaque, as [`Preset`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Feedback(u8);

impl Feedback {
    /// T0 runs the `profile.rs` record hooks.
    pub const FULL: Feedback = Feedback(0);
    /// T0 keeps its ICs and every object-model input, and runs no `profile.rs` record hook.
    /// Stored from M5-P0; effective from M5-P3 (PROFILE6).
    pub const ICS_ONLY: Feedback = Feedback(1);

    /// The mode name (`full`, `ics_only`), as result JSON files write it.
    pub const fn name(self) -> &'static str {
        match self.0 {
            0 => "full",
            _ => "ics_only",
        }
    }

    /// The mode named `name` ([`Feedback::name`]), if there is one.
    pub fn from_name(name: &str) -> Option<Feedback> {
        match name {
            "full" => Some(Feedback::FULL),
            "ics_only" => Some(Feedback::ICS_ONLY),
            _ => None,
        }
    }
}

/// How an isolate executes (C-23): a preset and a type-feedback mode. Opaque: made by a
/// preset constructor, changed by setters. Later fields (the policy of doc 13 §1.2, the fuzz
/// hooks) come with new setters, never public fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecOptions {
    preset: Preset,
    feedback: Feedback,
}

impl ExecOptions {
    /// The JIT preset with full feedback. [`Isolate::new_with`](crate::Isolate::new_with)
    /// refuses it in a build without the `jit` feature.
    pub const fn jit() -> ExecOptions {
        ExecOptions {
            preset: Preset::JIT,
            feedback: Feedback::FULL,
        }
    }

    /// The jitless preset with `ics_only` feedback (its default, MODES-D11).
    pub const fn jitless() -> ExecOptions {
        ExecOptions {
            preset: Preset::JITLESS,
            feedback: Feedback::ICS_ONLY,
        }
    }

    /// The default options of this build: [`ExecOptions::jit`] with the `jit` feature, else
    /// [`ExecOptions::jitless`]. [`Isolate::new`](crate::Isolate::new) uses them.
    pub const fn build_default() -> ExecOptions {
        if cfg!(feature = "jit") {
            ExecOptions::jit()
        } else {
            ExecOptions::jitless()
        }
    }

    /// The same options with the feedback mode `f` (either preset takes either mode).
    #[must_use]
    pub const fn feedback(self, f: Feedback) -> ExecOptions {
        ExecOptions {
            feedback: f,
            ..self
        }
    }

    /// The preset.
    pub const fn preset(&self) -> Preset {
        self.preset
    }

    /// The feedback mode.
    pub const fn feedback_mode(&self) -> Feedback {
        self.feedback
    }
}

impl Default for ExecOptions {
    fn default() -> Self {
        ExecOptions::build_default()
    }
}

/// What this build of ep9_vm is (MC-w17-5): its features and the preset of
/// [`ExecOptions::default`]. Result JSON files write both (test262, bench).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// The ep9_vm features on in this build, out of `jit`, `become`, `spike-native` and
    /// `ic-stats`, in that order.
    pub features: &'static [&'static str],
    /// The name of the default preset ([`Preset::name`]).
    pub preset: &'static str,
}

/// The features on in this build (the order of [`BuildInfo::features`]).
const FEATURES: &[&str] = &[
    #[cfg(feature = "jit")]
    "jit",
    #[cfg(feature = "become")]
    "become",
    #[cfg(feature = "spike-native")]
    "spike-native",
    #[cfg(feature = "ic-stats")]
    "ic-stats",
];

/// The [`BuildInfo`] of this build.
pub fn build_info() -> BuildInfo {
    BuildInfo {
        features: FEATURES,
        preset: ExecOptions::build_default().preset().name(),
    }
}

/// `features jit,become; preset jit` (`features none` when no feature is on): the second line
/// of `ep9 --version`.
impl std::fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let features = if self.features.is_empty() {
            "none".to_string()
        } else {
            self.features.join(",")
        };
        write!(f, "features {features}; preset {}", self.preset)
    }
}
