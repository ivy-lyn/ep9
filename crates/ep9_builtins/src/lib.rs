//! ECMA-262 builtins written against the builtin ABI (H20). The crate's engine dependencies are
//! `ep9_vm` and `ep9_math` only (`.github/scripts/dep-guards.sh` check 8), and it has no
//! `unsafe` code. Builtins reach the VM only through `ep9_vm::Cx`: no raw `Value`, no heap cell
//! type (`clippy.toml` `disallowed-types`).
//!
//! ```text
//! TABLE = [object, function, array, number, boolean, string, symbol, math, global,
//!          block 1  2         3      4       5        6       7       8     9
//!          promise, reflect, date, collections, json, proxy, regexp, weakref, bigint,
//!          10       11       12    13           14    15     16      17       18
//!          ep9_temporal::MODULES: temporal, duration, instant, plain_time, plain_date,
//!                                 19        20        21       22          23
//!                                 plain_date_time, plain_year_month, plain_month_day,
//!                                 24               25                26
//!                                 zoned_date_time,
//!                                 27
//!          iterator,
//!          28
//!          ep9_intl::MODULES: intl, locale, collator]
//!                             29    30      31
//!          (M4-P0: 15-17 empty until M4-P4 PROXY1, M4-P2 RX0A / M4-P3 REGEXP1, M4-P4 WEAKREF)
//! each module: BLOCK, DEFS (id = BLOCK.i, append-only), INTRINSICS, PROPS, MODULE
//! IsolateConfig { builtins: Some(&ep9_builtins::TABLE), .. }: realm init installs the VM's
//! table (block 0), then these modules in this order.
//! ```
//!
//! How to add a builtin: `TEMPLATES.md`.
#![forbid(unsafe_code)]

pub mod array;
pub mod boolean;
pub mod collections;
pub mod date;
pub mod function;
pub mod global;
pub mod json;
pub mod math;
pub mod number;
pub mod object;
pub mod promise;
pub mod proxy;
pub mod reflect;
pub mod regexp;
pub mod string;
pub mod symbol;
pub mod weakref;
// spec-temporal B4: block 18.
pub mod bigint;
// M7-P1 SPEC-ITER: block 28.
pub mod iterator;

use ep9_vm::{BuiltinModule, BuiltinTable};

/// The modules of [`TABLE`] in install order (fixed: property order follows it).
pub const MODULES: &[BuiltinModule] = &[
    object::MODULE,
    function::MODULE,
    array::MODULE,
    number::MODULE,
    boolean::MODULE,
    string::MODULE,
    symbol::MODULE,
    math::MODULE,
    global::MODULE,
    promise::MODULE,
    reflect::MODULE,
    date::MODULE,
    collections::MODULE,
    json::MODULE,
    proxy::MODULE,
    regexp::MODULE,
    weakref::MODULE,
    bigint::MODULE,
    // ep9_temporal::MODULES (design SPEC-T-D11), in block order.
    ep9_temporal::namespace::MODULE,
    ep9_temporal::duration::MODULE,
    ep9_temporal::instant::MODULE,
    ep9_temporal::plain_time::MODULE,
    ep9_temporal::plain_date::MODULE,
    ep9_temporal::plain_date_time::MODULE,
    ep9_temporal::plain_year_month::MODULE,
    ep9_temporal::plain_month_day::MODULE,
    ep9_temporal::zoned_date_time::MODULE,
    // M7-P1 SPEC-ITER: Iterator and the iterator helpers.
    iterator::MODULE,
    // ep9_intl::MODULES (INTL-I1-D1, INTL W1), in block order: the lazy group "Intl".
    ep9_intl::intl::MODULE,
    ep9_intl::locale::MODULE,
    ep9_intl::collator::MODULE,
];

/// The engine's builtin table: `IsolateConfig { builtins: Some(&ep9_builtins::TABLE), .. }`.
pub static TABLE: BuiltinTable = BuiltinTable::new("ep9_builtins", MODULES);

/// The code blocks of `TEMPLATES.md` run as doctests, so the guide follows the ABI.
#[cfg(doctest)]
#[doc = include_str!("../TEMPLATES.md")]
pub struct TemplatesMd;
