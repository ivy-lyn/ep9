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
//!          iterator, disposable,
//!          28        29
//!          ep9_intl::MODULES: intl, locale, collator, plural_rules, number_format,
//!                             30    31      32        33            34
//!                             list_format, segmenter, display_names, relative_time_format,
//!                             35           36         37             38
//!          array_buffer, data_view, typed_array, typed_array_proto, base64, atomics]
//!          38            39         40           41                 42      43
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
// M7-P3 SPEC-ERM: block 29.
pub mod disposable;
// TypedArray TA-2 TA-BUFFERS: blocks 38 and 39.
pub mod array_buffer;
pub mod data_view;
// TypedArray TA-3 TA-CTORS: block 40; TA-4: its modules proto (block 41) and base64 (42).
pub mod typed_array;
// TypedArray TA-4 TA-ATOMICS: block 43.
pub mod atomics;

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
    // M7-P3 SPEC-ERM: SuppressedError, DisposableStack, AsyncDisposableStack.
    disposable::MODULE,
    // ep9_intl::MODULES (INTL-I1-D1, INTL W1), in block order: the lazy group "Intl".
    ep9_intl::intl::MODULE,
    ep9_intl::locale::MODULE,
    ep9_intl::collator::MODULE,
    // INTL W2 (the NF-CORE skeleton; PLURAL, NF-CORE, LIST+CASEMAP, SEGMENTER and DISPLAYNAMES
    // fill them): the lazy group "Intl".
    ep9_intl::plural_rules::MODULE,
    ep9_intl::number_format::MODULE,
    ep9_intl::list_format::MODULE,
    ep9_intl::segmenter::MODULE,
    ep9_intl::display_names::MODULE,
    // INTL W3 (the RTF skeleton; RTF fills it): the lazy group "Intl".
    ep9_intl::relative_time_format::MODULE,
    // TypedArray TA-2 TA-BUFFERS: ArrayBuffer, SharedArrayBuffer, DataView; the lazy group of
    // the keys `ArrayBuffer`, `SharedArrayBuffer` and `DataView` (TA-D14).
    array_buffer::MODULE,
    data_view::MODULE,
    // TypedArray TA-3 TA-CTORS: %TypedArray% and the 12 TypedArray constructors; their 12
    // global keys join the lazy group of `array_buffer` (TA-D14).
    typed_array::MODULE,
    // TypedArray TA-4: the %TypedArray.prototype% methods, the Uint8Array base64 and hex
    // methods (both join the lazy group of `array_buffer`: each entry targets a group
    // intrinsic) and %Atomics% (its global key is a key of the group, TA-D14). Install order
    // sets property order: node's `Uint8Array` keys put `fromBase64` after `BYTES_PER_ELEMENT`.
    typed_array::proto::MODULE,
    typed_array::base64::MODULE,
    atomics::MODULE,
];

/// The engine's builtin table: `IsolateConfig { builtins: Some(&ep9_builtins::TABLE), .. }`.
pub static TABLE: BuiltinTable = BuiltinTable::new("ep9_builtins", MODULES);

/// The code blocks of `TEMPLATES.md` run as doctests, so the guide follows the ABI.
#[cfg(doctest)]
#[doc = include_str!("../TEMPLATES.md")]
pub struct TemplatesMd;
