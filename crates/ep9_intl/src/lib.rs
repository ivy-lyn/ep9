//! Intl (ECMA-402 on ICU4X; INTL-I1-D1, layout B): the glue from the builtin ABI (H20,
//! `ep9_vm::Cx`) to the ICU4X 2.4 components. The crate has no `unsafe` and reaches the heap
//! only through `Cx`.
//!
//! ```text
//! ep9_builtins::MODULES = [.., ep9_temporal::MODULES.., iterator, ep9_intl::MODULES..]
//! ep9_intl::MODULES (blocks, append-only; the lazy group of the global key "Intl", INTL-I1-D8)
//!   29 intl      %Intl%                                  intl.rs     (W1 CORE)
//!   30 locale    %Intl.Locale%, %Intl.Locale.prototype%  locale.rs   (W1 LOCALE)
//!   31 collator  %Intl.Collator%, its prototype          collator.rs (W1 COLLATOR)
//! support (W1 CORE):
//!   tag        IsStructurallyValidLanguageTag, CanonicalizeUnicodeLocaleId,
//!              CanonicalizeLocaleList (ECMA-402 §6.2, §9.2.1)
//!   available  the available locales per service (a static sorted list, OnceLock) and the
//!              supportedValuesOf lists
//!   resolve    LookupMatcher (for "lookup" and "best fit"), ResolveLocale, the Unicode
//!              extension keys, SupportedLocales (§9.2)
//!   options    GetOptionsObject, CoerceOptionsToObject, GetOption, GetBooleanOrStringNumberFormatOption,
//!              DefaultNumberOption, GetNumberOption (§9.2.10-§9.2.15)
//!   digits     SetNumberFormatDigitOptions and the digit option record (§16.5.3)
//!   cache      the thread_local cache of ICU4X formatters (Rc; never in a slot, rule 8)
//!   locale_info  Locale info (W1 LOCALE: getCalendars .. getWeekInfo)
//! ```
//!
//! Rules of the glue (design INTL-I1 §4.5): slots hold Values only (strings and packed int32
//! option fields), so no ICU4X object is in a slot and the GC needs no change; a constructor
//! checks its options and tags at once and makes its ICU4X object at the first use, through the
//! `cache` module; no `Local` in a Rust container; `Err` only from a `cx.throw_*` call.
#![forbid(unsafe_code)]

pub mod available;
pub mod cache;
pub(crate) mod cldr;
pub mod collator;
pub mod digits;
pub mod intl;
pub mod locale;
pub mod locale_info;
pub mod options;
pub mod resolve;
pub mod tag;

use ep9_vm::BuiltinModule;

/// The Intl modules, in block order; `ep9_builtins::MODULES` lists them after its own.
pub const MODULES: &[BuiltinModule] = &[intl::MODULE, locale::MODULE, collator::MODULE];

/// Helpers of the unit tests.
#[cfg(test)]
pub(crate) mod testing {
    /// How the cost of `work` grows from the size `small` to the size `big`: the wall time of
    /// one `work(big)` over the wall time of `big / small` runs of `work(small)`, each the best
    /// of 3, the two in turn. Under a linear cost the two take the same time, so the host
    /// speed, the build and a change of the host load hit both alike: about 1, and about
    /// `big / small` for a cost of n^2 (the form of `growth` in `tests/common/mod.rs`; an
    /// absolute wall-time bound failed on a slow CI runner, M6-ORCH-226).
    pub fn growth(small: usize, big: usize, mut work: impl FnMut(usize)) -> f64 {
        let reps = big / small;
        let (mut s, mut b) = (std::time::Duration::MAX, std::time::Duration::MAX);
        for _ in 0..3 {
            let t = std::time::Instant::now();
            for _ in 0..reps {
                work(small);
            }
            s = s.min(t.elapsed());
            let t = std::time::Instant::now();
            work(big);
            b = b.min(t.elapsed());
        }
        let r = b.as_secs_f64() / s.as_secs_f64().max(1e-6);
        eprintln!("growth {small} x {reps} -> {big}: best {s:?} -> {b:?}: {r:.2}");
        r
    }

    /// The bound on [`growth`] for 4 times the size: twice the linear ratio (1) and half the
    /// ratio of a cost of n^2 (4).
    pub const LINEAR: f64 = 2.0;
}
