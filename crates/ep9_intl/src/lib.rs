//! Intl (ECMA-402 on ICU4X; INTL-I1-D1, layout B): the glue from the builtin ABI (H20,
//! `ep9_vm::Cx`) to the ICU4X 2.3 components. The crate has no `unsafe` and reaches the heap
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
