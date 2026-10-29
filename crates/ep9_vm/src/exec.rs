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

    /// The preset name (`jit`, `jitless`), as result JSON files write it. A value this build
    /// does not know (a later preset) is `unknown`, never a known name (MC-w18-3).
    pub const fn name(self) -> &'static str {
        match self.0 {
            0 => "jit",
            1 => "jitless",
            _ => "unknown",
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
    /// doc 13:77); the jitless preset runs in every build (M-R3); a preset this build does not
    /// know runs in none (an allow-list, MC-w18-3).
    pub const fn available(self) -> bool {
        self.0 == Preset::JITLESS.0 || (self.0 == Preset::JIT.0 && cfg!(feature = "jit"))
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

    /// The mode name (`full`, `ics_only`), as result JSON files write it. A value this build
    /// does not know is `unknown` (MC-w18-3).
    pub const fn name(self) -> &'static str {
        match self.0 {
            0 => "full",
            1 => "ics_only",
            _ => "unknown",
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
    fuzz: FuzzHooks,
}

impl ExecOptions {
    /// The JIT preset with full feedback. [`Isolate::new_with`](crate::Isolate::new_with)
    /// refuses it in a build without the `jit` feature.
    pub const fn jit() -> ExecOptions {
        ExecOptions {
            preset: Preset::JIT,
            feedback: Feedback::FULL,
            fuzz: FuzzHooks::NONE,
        }
    }

    /// The jitless preset with `ics_only` feedback (its default, MODES-D11).
    pub const fn jitless() -> ExecOptions {
        ExecOptions {
            preset: Preset::JITLESS,
            feedback: Feedback::ICS_ONLY,
            fuzz: FuzzHooks::NONE,
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

/// The fuzz-hook seat of an isolate (M5-P1 REPRL2; w16 O7, V10-D27): the place in
/// [`ExecOptions`] where the hooks that a fuzzer host sets for one isolate live.
///
/// ```text
/// REPRL child, per script:  ExecOptions (preset, feedback, fuzz hooks) -> Isolate::new_with
///                                                                         -> Runtime::exec
///                           the isolate drops at the end of the script: its hooks go with it
/// ```
///
/// Why a seat in the options: V8 once leaked a fuzz flag from one REPRL script to the next
/// through a process global (V8 ac1bee768f, MODES-CHECK-w16 O7). ep9 runs one fresh isolate per
/// REPRL script, so a hook stored here cannot reach the next script; a static or a
/// thread-local could (ep9 has per-thread state that outlives an isolate).
///
/// Empty in M5-P1: each hook comes with its feature (fuse pop after FR0 and CONST/REP;
/// tier-up and deopt with T1; PIN), never before (V10-D27). How a hook is added:
/// - a private field here, `Copy` data (the options sit in a `Cell` in the isolate's
///   `Runtime`; no captured state, so per-run state stays in the isolate): a `&'static` table
///   of `fn` pointers with its own `PartialEq` (by address of the table), not a bare `fn`
///   pointer, because `derive(PartialEq)` over a fn-pointer field fails under `-D warnings`
///   (`unpredictable_function_pointer_comparisons`; MODES-CHECK-w18 MC-w18-8);
/// - a setter on [`ExecOptions`] (a new H20 golden line, `embed (not core)`; never a public
///   field), and its reader through `rt.exec.get().fuzz_hooks()`;
/// - a `None` default, so a build or an embedder that sets no hook runs as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FuzzHooks {}

impl FuzzHooks {
    /// No hook set: every preset constructor starts with it.
    pub(crate) const NONE: FuzzHooks = FuzzHooks {};
}

impl ExecOptions {
    /// The fuzz hooks of these options (the seat; none is built yet, V10-D27).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the seat: the first fuzz hook reads it (V10-D27)")
    )]
    pub(crate) const fn fuzz_hooks(&self) -> FuzzHooks {
        self.fuzz
    }
}

#[cfg(test)]
mod fuzz_seat_tests {
    //! The fuzz-hook seat (M5-P1 REPRL2): every preset starts empty, the feedback setter keeps
    //! it, and each isolate reads its own copy (the one `Isolate::new_with` stored).

    use super::{ExecOptions, Feedback, FuzzHooks};
    use crate::{Isolate, IsolateConfig};

    #[test]
    fn every_preset_starts_with_an_empty_seat() {
        for o in [
            ExecOptions::jit(),
            ExecOptions::jitless(),
            ExecOptions::build_default(),
            ExecOptions::default(),
            ExecOptions::jitless().feedback(Feedback::FULL),
        ] {
            assert_eq!(o.fuzz_hooks(), FuzzHooks::NONE, "{o:?}");
        }
        assert_eq!(FuzzHooks::default(), FuzzHooks::NONE);
    }

    #[test]
    fn each_isolate_holds_its_own_options() {
        let a = Isolate::new_with(IsolateConfig::default(), ExecOptions::jitless()).unwrap();
        let b = Isolate::new_with(
            IsolateConfig::default(),
            ExecOptions::jitless().feedback(Feedback::FULL),
        )
        .unwrap();
        assert_eq!(a.rt.exec.get().feedback_mode(), Feedback::ICS_ONLY);
        assert_eq!(b.rt.exec.get().feedback_mode(), Feedback::FULL);
        assert_eq!(a.rt.exec.get().fuzz_hooks(), FuzzHooks::NONE);
        drop(a);
        let c = Isolate::new(IsolateConfig::default()).unwrap();
        assert_eq!(c.rt.exec.get(), ExecOptions::default());
        assert_eq!(b.rt.exec.get().fuzz_hooks(), FuzzHooks::NONE);
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecOptions, Feedback, Preset};
    use crate::{Isolate, IsolateConfig, IsolateError};

    /// MC-w18-3: the names and `available` are an allow-list. A value no build assigns (255,
    /// at the top of the range, never the next planned one) is `unknown`, no build runs it,
    /// and `new_with` refuses it with a text that is not the jit one.
    #[test]
    fn an_unknown_preset_or_mode_is_refused() {
        let (p, f) = (Preset(255), Feedback(255));
        assert_eq!((p.name(), f.name()), ("unknown", "unknown"));
        assert!(!p.available());
        assert_eq!(Preset::from_name(p.name()), None);
        assert_eq!(Feedback::from_name(f.name()), None);
        let exec = ExecOptions {
            preset: p,
            ..ExecOptions::jitless()
        };
        match Isolate::new_with(IsolateConfig::default(), exec) {
            Err(IsolateError::Config(m)) => {
                assert!(
                    m.contains("unknown preset") && !m.contains("jit feature"),
                    "{m}"
                )
            }
            Err(e) => panic!("another error: {e}"),
            Ok(_) => panic!("an unknown preset ran"),
        }
    }
}
