//! Builtin tables (H20 "Builtin tables"): the static definitions that realm init walks, the
//! ids that name builtins and intrinsics, and the per-isolate builtin state.
//!
//! ```text
//! BuiltinTable { modules }                  VM table (natives.rs), then IsolateConfig::builtins
//!   BuiltinModule { block, defs, intrinsics, props }
//!     defs[i]: BuiltinDef { name, length, kind, f }   id = BuiltinId(block, i)
//!     intrinsics: IntrinsicDef { id, value }          %Number% = defs[0], %Math% = a new object
//!     props: ObjectProps { target, [PropDef] }        %Number.prototype%.toString = defs[3]
//!
//! Isolate::new: check (once per table) -> prepare (dense FunctionInfo arrays, root ranges)
//!   -> realm::init (core intrinsics, symbols) -> install: 1 functions, 2 intrinsics, 3 props
//! ```
//!
//! | Rule | Why |
//! |---|---|
//! | A `BuiltinId` is block (7 bits) + index (9 bits); a module owns one block, and its ids are append-only (goldens `tests/golden/*ids*.txt`) | parallel agents never shift each other's ids; H13 index lists, H35 link cells (W4-BUILTIN-IDS) |
//! | Each definition has one function object per realm, made in table order | identity: `Builtin(i)` twice is one object; a fixed heap layout |
//! | Realm init walks the tables in a fixed order: VM table, then the embedder's; inside, module order, then entry order | two isolates have the same property order |
//! | A table is checked before use: blocks, indices, kinds, intrinsics, aliases and duplicate keys | a table error is an `IsolateError`, never a half-built realm |
//! | A constructor a table makes gets `prototype` {} as its first table key; `%Proxy%` gets none | §20 (`Number.prototype` and the others); a key before it would make the lazy `prototype` (`props::own_add`); §28.2.2 (`exotic::is_lazy_prototype` makes none for `%Proxy%`) |
//!
//! Core intrinsics (the prototypes, `%Iterator.prototype%`, `%ArrayIteratorPrototype%` and the
//! global object) come from `realm::init`; tables add properties to them and make the others
//! (constructors, `%Math%`, and `%StringIteratorPrototype%` from the VM table). An intrinsic
//! lives in a runtime root, in `symbol.rs` (`%Symbol.prototype%`), or in a builtin state slot
//! (`IntrinsicId::slot`: ids appended after the runtime roots were fixed).

use core::cell::{Cell, RefCell};
use core::fmt::Write as _;
use core::ptr::NonNull;
use std::collections::HashSet;
use std::sync::OnceLock;

use ep9_heap::{RootStrength, Slot};
use ep9_value::Value;

use crate::abi::HostFn;
use crate::atoms::{self, Atom};
use crate::conv::num;
use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED};
use crate::kinds::{is_object, is_symbol};
use crate::object::{self, attr, own_define, own_get};
use crate::runtime::{Root, Runtime, val};
use crate::symbol;
use crate::unit::FunctionInfo;

// ---- ids --------------------------------------------------------------------------------------

/// The id of a builtin function: block (7 bits) and index in the block (9 bits). A module owns
/// one block; its ids are the positions of its definitions, append-only (H13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuiltinId(u16);

impl BuiltinId {
    /// Bits of the block number.
    pub const BLOCK_BITS: u32 = 7;
    /// Bits of the index in a block.
    pub const INDEX_BITS: u32 = 9;
    /// Blocks: 0 is the VM's own table (`natives.rs`).
    pub const BLOCKS: usize = 1 << Self::BLOCK_BITS;
    /// Definitions per block.
    pub const PER_BLOCK: usize = 1 << Self::INDEX_BITS;

    /// The id of definition `index` of block `block`. Panics (at compile time in a `const`)
    /// if either is out of range.
    pub const fn new(block: u8, index: u16) -> BuiltinId {
        assert!(
            (block as usize) < Self::BLOCKS,
            "builtin block out of range"
        );
        assert!(
            (index as usize) < Self::PER_BLOCK,
            "builtin index out of range"
        );
        BuiltinId(((block as u16) << Self::INDEX_BITS) | index)
    }

    pub const fn block(self) -> u8 {
        (self.0 >> Self::INDEX_BITS) as u8
    }

    pub const fn index(self) -> u16 {
        self.0 & (Self::PER_BLOCK as u16 - 1)
    }

    /// The 16-bit encoding (the H13 index-list value).
    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl core::fmt::Display for BuiltinId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#06x} {}.{}", self.0, self.block(), self.index())
    }
}

/// The intrinsic objects of a realm that builtins and tables name (§6.1.7.4), with stable ids
/// (append-only; the VM golden lists them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum IntrinsicId {
    ObjectPrototype,
    FunctionPrototype,
    ArrayPrototype,
    StringPrototype,
    NumberPrototype,
    BooleanPrototype,
    ErrorPrototype,
    TypeErrorPrototype,
    RangeErrorPrototype,
    ReferenceErrorPrototype,
    SyntaxErrorPrototype,
    EvalErrorPrototype,
    UriErrorPrototype,
    ArrayIteratorPrototype,
    /// `%Symbol.prototype%`: kept by `symbol.rs` (P2-SYM), not by a runtime root.
    SymbolPrototype,
    /// `%Iterator.prototype%`, the prototype of `%ArrayIteratorPrototype%`.
    IteratorPrototype,
    /// The realm's global object (a table target for global properties).
    GlobalObject,
    Object,
    Function,
    Array,
    Number,
    Boolean,
    String,
    Symbol,
    Error,
    TypeError,
    RangeError,
    ReferenceError,
    SyntaxError,
    EvalError,
    UriError,
    Math,
    /// `%StringIteratorPrototype%` (§22.1.5.2), made by the VM table. Its root is a slot of
    /// the isolate's builtin state, not a runtime root.
    StringIteratorPrototype,
    // ---- M2-P0 (generators; async functions for month 3; eval): VM table, state slots ----
    /// `%GeneratorFunction%` (§27.3.1).
    GeneratorFunction,
    /// `%GeneratorFunction.prototype%` (§27.3.3).
    GeneratorFunctionPrototype,
    /// `%GeneratorPrototype%` (§27.5.1, `%GeneratorFunction.prototype.prototype%`).
    GeneratorPrototype,
    /// `%AsyncFunction%` (§27.7.1).
    AsyncFunction,
    /// `%AsyncFunction.prototype%` (§27.7.3).
    AsyncFunctionPrototype,
    /// `%eval%` (§19.2.1), also the global `eval` (P3-EVAL-VM).
    Eval,
    // ---- M2-P4 (Promise, RT-9): made by `ep9_builtins` (promise.rs), state slots ----
    /// `%Promise%` (§27.2.3).
    Promise,
    /// `%Promise.prototype%` (§27.2.5).
    PromisePrototype,
    /// `%AggregateError%` (§20.5.7.1).
    AggregateError,
    /// `%AggregateError.prototype%` (§20.5.7.3).
    AggregateErrorPrototype,
    // ---- M2-P5 (Reflect): made by `ep9_builtins` (reflect.rs), a state slot ----
    /// `%Reflect%` (§28.1).
    Reflect,
    // ---- M2-P5 (VM-SMALL): made by the VM table (natives.rs), a state slot ----
    /// `%Array.prototype%[@@unscopables]` (§23.1.3.41): a null-prototype object.
    ArrayUnscopables,
    // ---- M3-P0 (P0-D3): declared for the month-3 builders, made by none yet (an id that
    // nothing makes is undefined, `intrinsic`); state slots 13-31 ----
    /// `%AsyncGeneratorFunction%` (§27.4.1; made by M3-P2 ASYNCGEN).
    AsyncGeneratorFunction,
    /// `%AsyncGeneratorFunction.prototype%` (§27.4.3; made by M3-P2 ASYNCGEN).
    AsyncGeneratorFunctionPrototype,
    /// `%AsyncGeneratorPrototype%` (§27.6.1; made by M3-P2 ASYNCGEN).
    AsyncGeneratorPrototype,
    /// `%AsyncIteratorPrototype%` (§27.1.3; made by M3-P2 ASYNCGEN).
    AsyncIteratorPrototype,
    /// `%AsyncFromSyncIteratorPrototype%` (§27.1.6.2; made by M3-P2 ASYNCGEN).
    AsyncFromSyncIteratorPrototype,
    /// `%Date%` (§21.4.2; made by M3-P1 B-DATE).
    Date,
    /// `%Date.prototype%` (§21.4.4; made by M3-P1 B-DATE).
    DatePrototype,
    /// `%Map%` (§24.1.1; made by M3-P1 B-COLLECTIONS).
    Map,
    /// `%Map.prototype%` (§24.1.3; made by M3-P1 B-COLLECTIONS).
    MapPrototype,
    /// `%Set%` (§24.2.2; made by M3-P1 B-COLLECTIONS).
    Set,
    /// `%Set.prototype%` (§24.2.4; made by M3-P1 B-COLLECTIONS).
    SetPrototype,
    /// `%WeakMap%` (§24.3.1; made by M3-P1 B-COLLECTIONS).
    WeakMap,
    /// `%WeakMap.prototype%` (§24.3.3; made by M3-P1 B-COLLECTIONS).
    WeakMapPrototype,
    /// `%WeakSet%` (§24.4.1; made by M3-P1 B-COLLECTIONS).
    WeakSet,
    /// `%WeakSet.prototype%` (§24.4.3; made by M3-P1 B-COLLECTIONS).
    WeakSetPrototype,
    /// `%MapIteratorPrototype%` (§24.1.5.2; made by M3-P1 B-COLLECTIONS).
    MapIteratorPrototype,
    /// `%SetIteratorPrototype%` (§24.2.6.2; made by M3-P1 B-COLLECTIONS).
    SetIteratorPrototype,
    /// `%JSON%` (§25.5; made by M3-P1 B-JSON).
    Json,
    /// `%ThrowTypeError%` (§10.2.4.1; made by M3-P2 VM-ARGS).
    ThrowTypeError,
    // ---- M4-P0 (plan v9 §2.5 task 3, rule 18): declared for the month-4 builders, made by
    // none yet; state slots 32-39 ----
    /// `%Proxy%` (§28.2.1; made by M4-P4 PROXY1).
    Proxy,
    /// `%RegExp%` (§22.2.4; made by M4-P3 REGEXP1).
    RegExp,
    /// `%RegExp.prototype%` (§22.2.6; made by M4-P2 RX0A, an ordinary object first; V9-A9).
    RegExpPrototype,
    /// `%RegExpStringIteratorPrototype%` (§22.2.9.2; made by M4-P3 REGEXP1).
    RegExpStringIteratorPrototype,
    /// `%WeakRef%` (§26.1.1; made by M4-P4 WEAKREF).
    WeakRef,
    /// `%WeakRef.prototype%` (§26.1.3; made by M4-P4 WEAKREF).
    WeakRefPrototype,
    /// `%FinalizationRegistry%` (§26.2.1; made by M4-P4 WEAKREF).
    FinalizationRegistry,
    /// `%FinalizationRegistry.prototype%` (§26.2.3; made by M4-P4 WEAKREF).
    FinalizationRegistryPrototype,
    // ---- M6-P0 (OBJECT_LAYOUT 6; plan v11 V11-D21, rule 18): declared for M6-P2 FUSE-PI ----
    /// The realm's static fuse block: a FUSE cell (kind 83), not a spec intrinsic and not an
    /// object; no table makes it (`check_tables` refuses one). Held in the realm root
    /// `Root::RealmFuses`; made by `fuse::new_realm_block` at realm init (M6-P2 FUSE-PI).
    RealmFuses,
    // ---- spec-temporal B4 (BigInt; H20 part 2, H13 "index ids (append-only)") ----
    /// `%BigInt%` (§21.2.1; made by `ep9_builtins` bigint.rs, block 18).
    BigInt,
    /// `%BigInt.prototype%` (§21.2.3, an ordinary object; made by `ep9_builtins` bigint.rs).
    /// The prototype of a BigInt's ToObject and of a property access on a BigInt.
    BigIntPrototype,
    // ---- spec-temporal T1 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal%` (§1 the Temporal namespace object; made by `ep9_temporal` namespace.rs, block 19).
    Temporal,
    /// `%Temporal.Now%` (the Temporal.Now namespace object; made by `ep9_temporal` namespace.rs).
    TemporalNow,
    // ---- spec-temporal T2 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.Duration%` (§7.1; made by `ep9_temporal` duration.rs, block 20).
    TemporalDuration,
    /// `%Temporal.Duration.prototype%` (§7.3, an ordinary object; made by `ep9_temporal` duration.rs).
    TemporalDurationPrototype,
    // ---- spec-temporal T3 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.Instant%` (§8.1; made by `ep9_temporal` instant.rs, block 21).
    TemporalInstant,
    /// `%Temporal.Instant.prototype%` (§8.3, an ordinary object; made by `ep9_temporal` instant.rs).
    TemporalInstantPrototype,
    // ---- spec-temporal T4 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.PlainTime%` (§4.1; made by `ep9_temporal` plain_time.rs, block 22).
    TemporalPlainTime,
    /// `%Temporal.PlainTime.prototype%` (§4.3, an ordinary object; made by `ep9_temporal` plain_time.rs).
    TemporalPlainTimePrototype,
    // ---- spec-temporal T5 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.PlainDate%` (§3.1; made by `ep9_temporal` plain_date.rs, block 23).
    TemporalPlainDate,
    /// `%Temporal.PlainDate.prototype%` (§3.3, an ordinary object; made by `ep9_temporal` plain_date.rs).
    TemporalPlainDatePrototype,
    // ---- spec-temporal T6 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.PlainDateTime%` (§5.1; made by `ep9_temporal` plain_date_time.rs, block 24).
    TemporalPlainDateTime,
    /// `%Temporal.PlainDateTime.prototype%` (§5.3, an ordinary object; made by `ep9_temporal` plain_date_time.rs).
    TemporalPlainDateTimePrototype,
    // ---- spec-temporal T7 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.PlainYearMonth%` (§9.1; made by `ep9_temporal` plain_year_month.rs, block 25).
    TemporalPlainYearMonth,
    /// `%Temporal.PlainYearMonth.prototype%` (§9.3, an ordinary object; made by `ep9_temporal` plain_year_month.rs).
    TemporalPlainYearMonthPrototype,
    /// `%Temporal.PlainMonthDay%` (§10.1; made by `ep9_temporal` plain_month_day.rs, block 26).
    TemporalPlainMonthDay,
    /// `%Temporal.PlainMonthDay.prototype%` (§10.3, an ordinary object; made by `ep9_temporal` plain_month_day.rs).
    TemporalPlainMonthDayPrototype,
    // ---- spec-temporal T8 (Temporal; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Temporal.ZonedDateTime%` (§6.1; made by `ep9_temporal` zoned_date_time.rs, block 27).
    TemporalZonedDateTime,
    /// `%Temporal.ZonedDateTime.prototype%` (§6.3, an ordinary object; made by `ep9_temporal` zoned_date_time.rs).
    TemporalZonedDateTimePrototype,
    // ---- M7-P0 CALLHOST: made by no table; each builtin records itself (abi/forward.rs) ----
    /// `%Function.prototype.apply%` (§20.2.3.1; `Cx::forward_apply`).
    FunctionPrototypeApply,
    /// `%Function.prototype.call%` (§20.2.3.3; `Cx::forward_call`).
    FunctionPrototypeCall,
    // ---- M7-P1 SPEC-ITER (iterator helpers; H20 part 2, H13 "index ids (append-only)") ----
    /// `%Iterator%` (§27.1.3.1; made by `ep9_builtins` iterator.rs, block 28).
    Iterator,
    /// `%IteratorHelperPrototype%` (§27.1.2.1; made by `ep9_builtins` iterator.rs).
    IteratorHelperPrototype,
    /// `%WrapForValidIteratorPrototype%` (§27.1.3.2.1.1; made by `ep9_builtins` iterator.rs).
    WrapForValidIteratorPrototype,
}

impl IntrinsicId {
    /// Every id, in id order.
    pub const ALL: [IntrinsicId; 98] = [
        IntrinsicId::ObjectPrototype,
        IntrinsicId::FunctionPrototype,
        IntrinsicId::ArrayPrototype,
        IntrinsicId::StringPrototype,
        IntrinsicId::NumberPrototype,
        IntrinsicId::BooleanPrototype,
        IntrinsicId::ErrorPrototype,
        IntrinsicId::TypeErrorPrototype,
        IntrinsicId::RangeErrorPrototype,
        IntrinsicId::ReferenceErrorPrototype,
        IntrinsicId::SyntaxErrorPrototype,
        IntrinsicId::EvalErrorPrototype,
        IntrinsicId::UriErrorPrototype,
        IntrinsicId::ArrayIteratorPrototype,
        IntrinsicId::SymbolPrototype,
        IntrinsicId::IteratorPrototype,
        IntrinsicId::GlobalObject,
        IntrinsicId::Object,
        IntrinsicId::Function,
        IntrinsicId::Array,
        IntrinsicId::Number,
        IntrinsicId::Boolean,
        IntrinsicId::String,
        IntrinsicId::Symbol,
        IntrinsicId::Error,
        IntrinsicId::TypeError,
        IntrinsicId::RangeError,
        IntrinsicId::ReferenceError,
        IntrinsicId::SyntaxError,
        IntrinsicId::EvalError,
        IntrinsicId::UriError,
        IntrinsicId::Math,
        IntrinsicId::StringIteratorPrototype,
        IntrinsicId::GeneratorFunction,
        IntrinsicId::GeneratorFunctionPrototype,
        IntrinsicId::GeneratorPrototype,
        IntrinsicId::AsyncFunction,
        IntrinsicId::AsyncFunctionPrototype,
        IntrinsicId::Eval,
        IntrinsicId::Promise,
        IntrinsicId::PromisePrototype,
        IntrinsicId::AggregateError,
        IntrinsicId::AggregateErrorPrototype,
        IntrinsicId::Reflect,
        IntrinsicId::ArrayUnscopables,
        IntrinsicId::AsyncGeneratorFunction,
        IntrinsicId::AsyncGeneratorFunctionPrototype,
        IntrinsicId::AsyncGeneratorPrototype,
        IntrinsicId::AsyncIteratorPrototype,
        IntrinsicId::AsyncFromSyncIteratorPrototype,
        IntrinsicId::Date,
        IntrinsicId::DatePrototype,
        IntrinsicId::Map,
        IntrinsicId::MapPrototype,
        IntrinsicId::Set,
        IntrinsicId::SetPrototype,
        IntrinsicId::WeakMap,
        IntrinsicId::WeakMapPrototype,
        IntrinsicId::WeakSet,
        IntrinsicId::WeakSetPrototype,
        IntrinsicId::MapIteratorPrototype,
        IntrinsicId::SetIteratorPrototype,
        IntrinsicId::Json,
        IntrinsicId::ThrowTypeError,
        IntrinsicId::Proxy,
        IntrinsicId::RegExp,
        IntrinsicId::RegExpPrototype,
        IntrinsicId::RegExpStringIteratorPrototype,
        IntrinsicId::WeakRef,
        IntrinsicId::WeakRefPrototype,
        IntrinsicId::FinalizationRegistry,
        IntrinsicId::FinalizationRegistryPrototype,
        IntrinsicId::RealmFuses,
        IntrinsicId::BigInt,
        IntrinsicId::BigIntPrototype,
        IntrinsicId::Temporal,
        IntrinsicId::TemporalNow,
        IntrinsicId::TemporalDuration,
        IntrinsicId::TemporalDurationPrototype,
        IntrinsicId::TemporalInstant,
        IntrinsicId::TemporalInstantPrototype,
        IntrinsicId::TemporalPlainTime,
        IntrinsicId::TemporalPlainTimePrototype,
        IntrinsicId::TemporalPlainDate,
        IntrinsicId::TemporalPlainDatePrototype,
        IntrinsicId::TemporalPlainDateTime,
        IntrinsicId::TemporalPlainDateTimePrototype,
        IntrinsicId::TemporalPlainYearMonth,
        IntrinsicId::TemporalPlainYearMonthPrototype,
        IntrinsicId::TemporalPlainMonthDay,
        IntrinsicId::TemporalPlainMonthDayPrototype,
        IntrinsicId::TemporalZonedDateTime,
        IntrinsicId::TemporalZonedDateTimePrototype,
        IntrinsicId::FunctionPrototypeApply,
        IntrinsicId::FunctionPrototypeCall,
        IntrinsicId::Iterator,
        IntrinsicId::IteratorHelperPrototype,
        IntrinsicId::WrapForValidIteratorPrototype,
    ];

    /// The spec name (`%Object.prototype%`); `globalThis` for the global object; `[[RealmFuses]]`
    /// for the VM-internal fuse block.
    pub const fn name(self) -> &'static str {
        match self {
            IntrinsicId::ObjectPrototype => "%Object.prototype%",
            IntrinsicId::FunctionPrototype => "%Function.prototype%",
            IntrinsicId::ArrayPrototype => "%Array.prototype%",
            IntrinsicId::StringPrototype => "%String.prototype%",
            IntrinsicId::NumberPrototype => "%Number.prototype%",
            IntrinsicId::BooleanPrototype => "%Boolean.prototype%",
            IntrinsicId::ErrorPrototype => "%Error.prototype%",
            IntrinsicId::TypeErrorPrototype => "%TypeError.prototype%",
            IntrinsicId::RangeErrorPrototype => "%RangeError.prototype%",
            IntrinsicId::ReferenceErrorPrototype => "%ReferenceError.prototype%",
            IntrinsicId::SyntaxErrorPrototype => "%SyntaxError.prototype%",
            IntrinsicId::EvalErrorPrototype => "%EvalError.prototype%",
            IntrinsicId::UriErrorPrototype => "%URIError.prototype%",
            IntrinsicId::ArrayIteratorPrototype => "%ArrayIteratorPrototype%",
            IntrinsicId::SymbolPrototype => "%Symbol.prototype%",
            IntrinsicId::IteratorPrototype => "%Iterator.prototype%",
            IntrinsicId::GlobalObject => "globalThis",
            IntrinsicId::Object => "%Object%",
            IntrinsicId::Function => "%Function%",
            IntrinsicId::Array => "%Array%",
            IntrinsicId::Number => "%Number%",
            IntrinsicId::Boolean => "%Boolean%",
            IntrinsicId::String => "%String%",
            IntrinsicId::Symbol => "%Symbol%",
            IntrinsicId::Error => "%Error%",
            IntrinsicId::TypeError => "%TypeError%",
            IntrinsicId::RangeError => "%RangeError%",
            IntrinsicId::ReferenceError => "%ReferenceError%",
            IntrinsicId::SyntaxError => "%SyntaxError%",
            IntrinsicId::EvalError => "%EvalError%",
            IntrinsicId::UriError => "%URIError%",
            IntrinsicId::Math => "%Math%",
            IntrinsicId::StringIteratorPrototype => "%StringIteratorPrototype%",
            IntrinsicId::GeneratorFunction => "%GeneratorFunction%",
            IntrinsicId::GeneratorFunctionPrototype => "%GeneratorFunction.prototype%",
            IntrinsicId::GeneratorPrototype => "%GeneratorPrototype%",
            IntrinsicId::AsyncFunction => "%AsyncFunction%",
            IntrinsicId::AsyncFunctionPrototype => "%AsyncFunction.prototype%",
            IntrinsicId::Eval => "%eval%",
            IntrinsicId::Promise => "%Promise%",
            IntrinsicId::PromisePrototype => "%Promise.prototype%",
            IntrinsicId::AggregateError => "%AggregateError%",
            IntrinsicId::AggregateErrorPrototype => "%AggregateError.prototype%",
            IntrinsicId::Reflect => "%Reflect%",
            IntrinsicId::ArrayUnscopables => "%Array.prototype%[@@unscopables]",
            IntrinsicId::AsyncGeneratorFunction => "%AsyncGeneratorFunction%",
            IntrinsicId::AsyncGeneratorFunctionPrototype => "%AsyncGeneratorFunction.prototype%",
            IntrinsicId::AsyncGeneratorPrototype => "%AsyncGeneratorPrototype%",
            IntrinsicId::AsyncIteratorPrototype => "%AsyncIteratorPrototype%",
            IntrinsicId::AsyncFromSyncIteratorPrototype => "%AsyncFromSyncIteratorPrototype%",
            IntrinsicId::Date => "%Date%",
            IntrinsicId::DatePrototype => "%Date.prototype%",
            IntrinsicId::Map => "%Map%",
            IntrinsicId::MapPrototype => "%Map.prototype%",
            IntrinsicId::Set => "%Set%",
            IntrinsicId::SetPrototype => "%Set.prototype%",
            IntrinsicId::WeakMap => "%WeakMap%",
            IntrinsicId::WeakMapPrototype => "%WeakMap.prototype%",
            IntrinsicId::WeakSet => "%WeakSet%",
            IntrinsicId::WeakSetPrototype => "%WeakSet.prototype%",
            IntrinsicId::MapIteratorPrototype => "%MapIteratorPrototype%",
            IntrinsicId::SetIteratorPrototype => "%SetIteratorPrototype%",
            IntrinsicId::Json => "%JSON%",
            IntrinsicId::ThrowTypeError => "%ThrowTypeError%",
            IntrinsicId::Proxy => "%Proxy%",
            IntrinsicId::RegExp => "%RegExp%",
            IntrinsicId::RegExpPrototype => "%RegExp.prototype%",
            IntrinsicId::RegExpStringIteratorPrototype => "%RegExpStringIteratorPrototype%",
            IntrinsicId::WeakRef => "%WeakRef%",
            IntrinsicId::WeakRefPrototype => "%WeakRef.prototype%",
            IntrinsicId::FinalizationRegistry => "%FinalizationRegistry%",
            IntrinsicId::FinalizationRegistryPrototype => "%FinalizationRegistry.prototype%",
            IntrinsicId::RealmFuses => "[[RealmFuses]]",
            IntrinsicId::BigInt => "%BigInt%",
            IntrinsicId::BigIntPrototype => "%BigInt.prototype%",
            IntrinsicId::Temporal => "%Temporal%",
            IntrinsicId::TemporalNow => "%Temporal.Now%",
            IntrinsicId::TemporalDuration => "%Temporal.Duration%",
            IntrinsicId::TemporalDurationPrototype => "%Temporal.Duration.prototype%",
            IntrinsicId::TemporalInstant => "%Temporal.Instant%",
            IntrinsicId::TemporalInstantPrototype => "%Temporal.Instant.prototype%",
            IntrinsicId::TemporalPlainTime => "%Temporal.PlainTime%",
            IntrinsicId::TemporalPlainTimePrototype => "%Temporal.PlainTime.prototype%",
            IntrinsicId::TemporalPlainDate => "%Temporal.PlainDate%",
            IntrinsicId::TemporalPlainDatePrototype => "%Temporal.PlainDate.prototype%",
            IntrinsicId::TemporalPlainDateTime => "%Temporal.PlainDateTime%",
            IntrinsicId::TemporalPlainDateTimePrototype => "%Temporal.PlainDateTime.prototype%",
            IntrinsicId::TemporalPlainYearMonth => "%Temporal.PlainYearMonth%",
            IntrinsicId::TemporalPlainYearMonthPrototype => "%Temporal.PlainYearMonth.prototype%",
            IntrinsicId::TemporalPlainMonthDay => "%Temporal.PlainMonthDay%",
            IntrinsicId::TemporalPlainMonthDayPrototype => "%Temporal.PlainMonthDay.prototype%",
            IntrinsicId::TemporalZonedDateTime => "%Temporal.ZonedDateTime%",
            IntrinsicId::TemporalZonedDateTimePrototype => "%Temporal.ZonedDateTime.prototype%",
            IntrinsicId::FunctionPrototypeApply => "%Function.prototype.apply%",
            IntrinsicId::FunctionPrototypeCall => "%Function.prototype.call%",
            IntrinsicId::Iterator => "%Iterator%",
            IntrinsicId::IteratorHelperPrototype => "%IteratorHelperPrototype%",
            IntrinsicId::WrapForValidIteratorPrototype => "%WrapForValidIteratorPrototype%",
        }
    }

    /// The runtime root that holds this intrinsic; `None` for `%Symbol.prototype%`, which
    /// `symbol.rs` keeps, and for the intrinsics in builtin state slots ([`Self::slot`]).
    pub(crate) const fn root(self) -> Option<Root> {
        Some(match self {
            IntrinsicId::ObjectPrototype => Root::ObjectPrototype,
            IntrinsicId::FunctionPrototype => Root::FunctionPrototype,
            IntrinsicId::ArrayPrototype => Root::ArrayPrototype,
            IntrinsicId::StringPrototype => Root::StringPrototype,
            IntrinsicId::NumberPrototype => Root::NumberPrototype,
            IntrinsicId::BooleanPrototype => Root::BooleanPrototype,
            IntrinsicId::ErrorPrototype => Root::ErrorPrototype,
            IntrinsicId::TypeErrorPrototype => Root::TypeErrorPrototype,
            IntrinsicId::RangeErrorPrototype => Root::RangeErrorPrototype,
            IntrinsicId::ReferenceErrorPrototype => Root::ReferenceErrorPrototype,
            IntrinsicId::SyntaxErrorPrototype => Root::SyntaxErrorPrototype,
            IntrinsicId::EvalErrorPrototype => Root::EvalErrorPrototype,
            IntrinsicId::UriErrorPrototype => Root::UriErrorPrototype,
            IntrinsicId::ArrayIteratorPrototype => Root::ArrayIteratorPrototype,
            IntrinsicId::SymbolPrototype => return None,
            IntrinsicId::IteratorPrototype => Root::IteratorPrototype,
            IntrinsicId::GlobalObject => Root::GlobalObject,
            IntrinsicId::Object => Root::ObjectConstructor,
            IntrinsicId::Function => Root::FunctionConstructor,
            IntrinsicId::Array => Root::ArrayConstructor,
            IntrinsicId::Number => Root::NumberConstructor,
            IntrinsicId::Boolean => Root::BooleanConstructor,
            IntrinsicId::String => Root::StringConstructor,
            IntrinsicId::Symbol => Root::SymbolConstructor,
            IntrinsicId::Error => Root::ErrorConstructor,
            IntrinsicId::TypeError => Root::TypeErrorConstructor,
            IntrinsicId::RangeError => Root::RangeErrorConstructor,
            IntrinsicId::ReferenceError => Root::ReferenceErrorConstructor,
            IntrinsicId::SyntaxError => Root::SyntaxErrorConstructor,
            IntrinsicId::EvalError => Root::EvalErrorConstructor,
            IntrinsicId::UriError => Root::UriErrorConstructor,
            IntrinsicId::Math => Root::Math,
            IntrinsicId::RealmFuses => Root::RealmFuses,
            IntrinsicId::StringIteratorPrototype
            | IntrinsicId::GeneratorFunction
            | IntrinsicId::GeneratorFunctionPrototype
            | IntrinsicId::GeneratorPrototype
            | IntrinsicId::AsyncFunction
            | IntrinsicId::AsyncFunctionPrototype
            | IntrinsicId::Eval
            | IntrinsicId::Promise
            | IntrinsicId::PromisePrototype
            | IntrinsicId::AggregateError
            | IntrinsicId::AggregateErrorPrototype
            | IntrinsicId::Reflect
            | IntrinsicId::ArrayUnscopables
            | IntrinsicId::AsyncGeneratorFunction
            | IntrinsicId::AsyncGeneratorFunctionPrototype
            | IntrinsicId::AsyncGeneratorPrototype
            | IntrinsicId::AsyncIteratorPrototype
            | IntrinsicId::AsyncFromSyncIteratorPrototype
            | IntrinsicId::Date
            | IntrinsicId::DatePrototype
            | IntrinsicId::Map
            | IntrinsicId::MapPrototype
            | IntrinsicId::Set
            | IntrinsicId::SetPrototype
            | IntrinsicId::WeakMap
            | IntrinsicId::WeakMapPrototype
            | IntrinsicId::WeakSet
            | IntrinsicId::WeakSetPrototype
            | IntrinsicId::MapIteratorPrototype
            | IntrinsicId::SetIteratorPrototype
            | IntrinsicId::Json
            | IntrinsicId::ThrowTypeError
            | IntrinsicId::Proxy
            | IntrinsicId::RegExp
            | IntrinsicId::RegExpPrototype
            | IntrinsicId::RegExpStringIteratorPrototype
            | IntrinsicId::WeakRef
            | IntrinsicId::WeakRefPrototype
            | IntrinsicId::FinalizationRegistry
            | IntrinsicId::FinalizationRegistryPrototype
            | IntrinsicId::BigInt
            | IntrinsicId::BigIntPrototype
            | IntrinsicId::Temporal
            | IntrinsicId::TemporalNow
            | IntrinsicId::TemporalDuration
            | IntrinsicId::TemporalDurationPrototype
            | IntrinsicId::TemporalInstant
            | IntrinsicId::TemporalInstantPrototype
            | IntrinsicId::TemporalPlainTime
            | IntrinsicId::TemporalPlainTimePrototype
            | IntrinsicId::TemporalPlainDate
            | IntrinsicId::TemporalPlainDatePrototype
            | IntrinsicId::TemporalPlainDateTime
            | IntrinsicId::TemporalPlainDateTimePrototype
            | IntrinsicId::TemporalPlainYearMonth
            | IntrinsicId::TemporalPlainYearMonthPrototype
            | IntrinsicId::TemporalPlainMonthDay
            | IntrinsicId::TemporalPlainMonthDayPrototype
            | IntrinsicId::TemporalZonedDateTime
            | IntrinsicId::TemporalZonedDateTimePrototype
            | IntrinsicId::FunctionPrototypeApply
            | IntrinsicId::FunctionPrototypeCall
            | IntrinsicId::Iterator
            | IntrinsicId::IteratorHelperPrototype
            | IntrinsicId::WrapForValidIteratorPrototype => return None,
        })
    }

    /// The builtin state slot ([`BuiltinState`]) that holds this intrinsic: intrinsics added
    /// after the runtime roots were fixed (runtime.rs is not the table owner's file).
    const fn slot(self) -> Option<usize> {
        match self {
            IntrinsicId::StringIteratorPrototype => Some(0),
            IntrinsicId::GeneratorFunction => Some(1),
            IntrinsicId::GeneratorFunctionPrototype => Some(2),
            IntrinsicId::GeneratorPrototype => Some(3),
            IntrinsicId::AsyncFunction => Some(4),
            IntrinsicId::AsyncFunctionPrototype => Some(5),
            IntrinsicId::Eval => Some(6),
            IntrinsicId::Promise => Some(7),
            IntrinsicId::PromisePrototype => Some(8),
            IntrinsicId::AggregateError => Some(9),
            IntrinsicId::AggregateErrorPrototype => Some(10),
            IntrinsicId::Reflect => Some(11),
            IntrinsicId::ArrayUnscopables => Some(12),
            IntrinsicId::AsyncGeneratorFunction => Some(13),
            IntrinsicId::AsyncGeneratorFunctionPrototype => Some(14),
            IntrinsicId::AsyncGeneratorPrototype => Some(15),
            IntrinsicId::AsyncIteratorPrototype => Some(16),
            IntrinsicId::AsyncFromSyncIteratorPrototype => Some(17),
            IntrinsicId::Date => Some(18),
            IntrinsicId::DatePrototype => Some(19),
            IntrinsicId::Map => Some(20),
            IntrinsicId::MapPrototype => Some(21),
            IntrinsicId::Set => Some(22),
            IntrinsicId::SetPrototype => Some(23),
            IntrinsicId::WeakMap => Some(24),
            IntrinsicId::WeakMapPrototype => Some(25),
            IntrinsicId::WeakSet => Some(26),
            IntrinsicId::WeakSetPrototype => Some(27),
            IntrinsicId::MapIteratorPrototype => Some(28),
            IntrinsicId::SetIteratorPrototype => Some(29),
            IntrinsicId::Json => Some(30),
            IntrinsicId::ThrowTypeError => Some(31),
            IntrinsicId::Proxy => Some(32),
            IntrinsicId::RegExp => Some(33),
            IntrinsicId::RegExpPrototype => Some(34),
            IntrinsicId::RegExpStringIteratorPrototype => Some(35),
            IntrinsicId::WeakRef => Some(36),
            IntrinsicId::WeakRefPrototype => Some(37),
            IntrinsicId::FinalizationRegistry => Some(38),
            IntrinsicId::FinalizationRegistryPrototype => Some(39),
            IntrinsicId::BigInt => Some(40),
            IntrinsicId::BigIntPrototype => Some(41),
            IntrinsicId::Temporal => Some(42),
            IntrinsicId::TemporalNow => Some(43),
            IntrinsicId::TemporalDuration => Some(44),
            IntrinsicId::TemporalDurationPrototype => Some(45),
            IntrinsicId::TemporalInstant => Some(46),
            IntrinsicId::TemporalInstantPrototype => Some(47),
            IntrinsicId::TemporalPlainTime => Some(48),
            IntrinsicId::TemporalPlainTimePrototype => Some(49),
            IntrinsicId::TemporalPlainDate => Some(50),
            IntrinsicId::TemporalPlainDatePrototype => Some(51),
            IntrinsicId::TemporalPlainDateTime => Some(52),
            IntrinsicId::TemporalPlainDateTimePrototype => Some(53),
            IntrinsicId::TemporalPlainYearMonth => Some(54),
            IntrinsicId::TemporalPlainYearMonthPrototype => Some(55),
            IntrinsicId::TemporalPlainMonthDay => Some(56),
            IntrinsicId::TemporalPlainMonthDayPrototype => Some(57),
            IntrinsicId::TemporalZonedDateTime => Some(58),
            IntrinsicId::TemporalZonedDateTimePrototype => Some(59),
            IntrinsicId::FunctionPrototypeApply => Some(60),
            IntrinsicId::FunctionPrototypeCall => Some(61),
            IntrinsicId::Iterator => Some(62),
            IntrinsicId::IteratorHelperPrototype => Some(63),
            IntrinsicId::WrapForValidIteratorPrototype => Some(64),
            _ => None,
        }
    }

    /// Made by `realm::init` (and `symbol::install`), not by a table: tables may target these
    /// but not create them.
    pub const fn is_core(self) -> bool {
        (self as u16) <= IntrinsicId::GlobalObject as u16
    }
}

/// The intrinsic `id` of the current realm, a raw read: an object, or undefined if no table made
/// it, until symbols exist (`%Symbol.prototype%`) or until its lazy group installs (INTL-I1-D8).
pub(crate) fn intrinsic(rt: &Runtime, id: IntrinsicId) -> u64 {
    match (id.root(), id.slot()) {
        (Some(r), _) => rt.root(r),
        (None, Some(i)) => rt.builtins.intrinsics[i].bits(),
        // %Symbol.prototype% lives in symbol.rs.
        (None, None) => symbol::prototype(rt),
    }
}

/// Makes `v` the intrinsic `id` of the current realm (a table made it), in its root or slot
/// and in the intrinsics list of the current REALM record, so the record stays a copy of the
/// roots (`realm::switch` reads it back). Not a GC point.
fn set_intrinsic(rt: &Runtime, id: IntrinsicId, v: u64) {
    match (id.root(), id.slot()) {
        (Some(_), _) | (None, Some(_)) => load_intrinsic(rt, id, v),
        (None, None) => unreachable!("{} is not made by a table", id.name()),
    }
    crate::realm::record_intrinsic(rt, id, v);
}

/// Writes `v` into the root, the builtin state slot or the symbol state that holds the
/// intrinsic `id` of the current realm. Realm switch and realm creation (`realm.rs`) only:
/// the caller keeps the REALM record in step. Not a GC point.
pub(crate) fn load_intrinsic(rt: &Runtime, id: IntrinsicId, v: u64) {
    match (id.root(), id.slot()) {
        (Some(r), _) => rt.set_root(r, v),
        // E3: a store into a root slot takes no barrier.
        (None, Some(i)) => rt.builtins.intrinsics[i].init(val(v)),
        (None, None) => symbol::set_prototype(rt, v),
    }
}

// ---- table types ------------------------------------------------------------------------------

/// What a builtin function is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinKind {
    /// A function without `[[Construct]]` (most builtins).
    Method,
    /// A getter of an accessor property; its `name` is `"get <name>"`.
    Getter,
    /// A setter of an accessor property; its `name` is `"set <name>"`.
    Setter,
    /// A function with `[[Construct]]`.
    Constructor,
}

impl BuiltinKind {
    const fn text(self) -> &'static str {
        match self {
            BuiltinKind::Method => "method",
            BuiltinKind::Getter => "getter",
            BuiltinKind::Setter => "setter",
            BuiltinKind::Constructor => "constructor",
        }
    }
}

/// One builtin function: its `name` and `length` properties (§10.2.9, §10.2.10), its kind and
/// its host function.
#[derive(Clone, Copy)]
pub struct BuiltinDef {
    pub name: &'static str,
    pub length: u32,
    pub kind: BuiltinKind,
    pub f: HostFn,
}

impl BuiltinDef {
    pub const fn method(name: &'static str, length: u32, f: HostFn) -> BuiltinDef {
        BuiltinDef {
            name,
            length,
            kind: BuiltinKind::Method,
            f,
        }
    }

    pub const fn constructor(name: &'static str, length: u32, f: HostFn) -> BuiltinDef {
        BuiltinDef {
            name,
            length,
            kind: BuiltinKind::Constructor,
            f,
        }
    }

    /// The getter of the accessor `name` (`length` 0).
    pub const fn getter(name: &'static str, f: HostFn) -> BuiltinDef {
        BuiltinDef {
            name,
            length: 0,
            kind: BuiltinKind::Getter,
            f,
        }
    }

    /// The setter of the accessor `name` (`length` 1).
    pub const fn setter(name: &'static str, f: HostFn) -> BuiltinDef {
        BuiltinDef {
            name,
            length: 1,
            kind: BuiltinKind::Setter,
            f,
        }
    }

    /// The function's `name` property: "get x" / "set x" for accessors.
    fn function_name(&self) -> String {
        match self.kind {
            BuiltinKind::Getter => format!("get {}", self.name),
            BuiltinKind::Setter => format!("set {}", self.name),
            _ => self.name.to_string(),
        }
    }
}

impl core::fmt::Debug for BuiltinDef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuiltinDef")
            .field("name", &self.name)
            .field("length", &self.length)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// A well-known symbol (§6.1.5.1 table 1, plus `dispose` and `asyncDispose`), for table keys
/// and values. `symbol.rs` makes the symbols; this names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WellKnownSymbol(pub(crate) symbol::WellKnownSymbol);

impl WellKnownSymbol {
    pub const ASYNC_ITERATOR: Self = Self(symbol::WellKnownSymbol::AsyncIterator);
    pub const HAS_INSTANCE: Self = Self(symbol::WellKnownSymbol::HasInstance);
    pub const IS_CONCAT_SPREADABLE: Self = Self(symbol::WellKnownSymbol::IsConcatSpreadable);
    pub const ITERATOR: Self = Self(symbol::WellKnownSymbol::Iterator);
    pub const MATCH: Self = Self(symbol::WellKnownSymbol::Match);
    pub const MATCH_ALL: Self = Self(symbol::WellKnownSymbol::MatchAll);
    pub const REPLACE: Self = Self(symbol::WellKnownSymbol::Replace);
    pub const SEARCH: Self = Self(symbol::WellKnownSymbol::Search);
    pub const SPECIES: Self = Self(symbol::WellKnownSymbol::Species);
    pub const SPLIT: Self = Self(symbol::WellKnownSymbol::Split);
    pub const TO_PRIMITIVE: Self = Self(symbol::WellKnownSymbol::ToPrimitive);
    pub const TO_STRING_TAG: Self = Self(symbol::WellKnownSymbol::ToStringTag);
    pub const UNSCOPABLES: Self = Self(symbol::WellKnownSymbol::Unscopables);
    pub const DISPOSE: Self = Self(symbol::WellKnownSymbol::Dispose);
    pub const ASYNC_DISPOSE: Self = Self(symbol::WellKnownSymbol::AsyncDispose);

    /// The `Symbol` property name (`iterator` for `Symbol.iterator`).
    pub const fn name(self) -> &'static str {
        self.0.name()
    }
}

/// A property key in a table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropKey {
    /// A name of the `atoms!` list (no lookup at init).
    Atom(Atom),
    /// Any other name, interned at init.
    Str(&'static str),
    /// A well-known symbol.
    Symbol(WellKnownSymbol),
}

impl PropKey {
    /// The key as the spec writes it: `toString`, `[Symbol.iterator]`.
    pub fn text(&self) -> String {
        match *self {
            PropKey::Atom(a) => a.text().to_string(),
            PropKey::Str(s) => s.to_string(),
            PropKey::Symbol(w) => format!("[Symbol.{}]", w.name()),
        }
    }

    /// The same key? (`Atom(Length)` and `Str("length")` are one key.) No allocation:
    /// `check_tables` calls it for each pair of keys, at each process start.
    fn same(&self, other: &PropKey) -> bool {
        match (self.string_text(), other.string_text()) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self == other,
            _ => false,
        }
    }

    /// The text of a string key; `None` for a symbol.
    fn string_text(&self) -> Option<&'static str> {
        match *self {
            PropKey::Atom(a) => Some(a.text()),
            PropKey::Str(s) => Some(s),
            PropKey::Symbol(_) => None,
        }
    }
}

/// A constant property value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConstValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    /// A string (interned at init).
    Str(&'static str),
}

/// The value of a table property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropValue {
    /// The function object of definition `i` of this module (`Method` or `Constructor`).
    Builtin(u16),
    /// An intrinsic of the realm.
    Intrinsic(IntrinsicId),
    Const(ConstValue),
    /// A well-known symbol (`Symbol.iterator`).
    Symbol(WellKnownSymbol),
    /// The value of another key of the same object, defined earlier in walk order (one
    /// function object under two names: `Array.prototype[@@iterator]` is `values`).
    Alias(PropKey),
    /// An accessor property: definitions `get` (a `Getter`) and `set` (a `Setter`) of this
    /// module. Needs accessor properties (P3-ACC); before that, install fails.
    Accessor {
        get: Option<u16>,
        set: Option<u16>,
    },
}

/// Property attributes. For an accessor, `writable` is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropAttrs {
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl PropAttrs {
    /// {w, c}: the default of builtin functions and other data properties (ECMA-262 §18).
    pub const METHOD: PropAttrs = PropAttrs {
        writable: true,
        enumerable: false,
        configurable: true,
    };
    /// None: value properties such as `NaN`, `Math.PI` and a constructor's `prototype`.
    pub const NONE: PropAttrs = PropAttrs {
        writable: false,
        enumerable: false,
        configurable: false,
    };
    /// {c}: `@@toStringTag` values and accessors (§18: {e: false, c: true}).
    pub const CONFIGURABLE: PropAttrs = PropAttrs {
        writable: false,
        enumerable: false,
        configurable: true,
    };
    /// {w, e, c}: an ordinary assignment.
    pub const ALL: PropAttrs = PropAttrs {
        writable: true,
        enumerable: true,
        configurable: true,
    };

    fn bits(self) -> i32 {
        (if self.writable { attr::WRITABLE } else { 0 })
            | (if self.enumerable { attr::ENUMERABLE } else { 0 })
            | (if self.configurable {
                attr::CONFIGURABLE
            } else {
                0
            })
    }

    fn text(self) -> String {
        let mut s = String::new();
        for (on, c) in [
            (self.writable, 'w'),
            (self.enumerable, 'e'),
            (self.configurable, 'c'),
        ] {
            s.push(if on { c } else { '-' });
        }
        s
    }
}

/// One property of an object in a table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropDef {
    pub key: PropKey,
    pub value: PropValue,
    pub attrs: PropAttrs,
}

impl PropDef {
    /// `key` = the function of definition `def`, {w, c}.
    pub const fn method(key: PropKey, def: u16) -> PropDef {
        PropDef {
            key,
            value: PropValue::Builtin(def),
            attrs: PropAttrs::METHOD,
        }
    }

    /// `key` = the intrinsic `id` with `attrs`.
    pub const fn intrinsic(key: PropKey, id: IntrinsicId, attrs: PropAttrs) -> PropDef {
        PropDef {
            key,
            value: PropValue::Intrinsic(id),
            attrs,
        }
    }

    /// `key` = a constant with `attrs`.
    pub const fn constant(key: PropKey, c: ConstValue, attrs: PropAttrs) -> PropDef {
        PropDef {
            key,
            value: PropValue::Const(c),
            attrs,
        }
    }

    /// `key` = the value of `of` on the same object, with `attrs`.
    pub const fn alias(key: PropKey, of: PropKey, attrs: PropAttrs) -> PropDef {
        PropDef {
            key,
            value: PropValue::Alias(of),
            attrs,
        }
    }

    /// An accessor `key` with a getter and/or a setter of this module, {c}.
    pub const fn accessor(key: PropKey, get: Option<u16>, set: Option<u16>) -> PropDef {
        PropDef {
            key,
            value: PropValue::Accessor { get, set },
            attrs: PropAttrs::CONFIGURABLE,
        }
    }
}

/// The properties a module gives one object.
#[derive(Clone, Copy, Debug)]
pub struct ObjectProps {
    pub target: IntrinsicId,
    pub props: &'static [PropDef],
}

/// How a table makes an intrinsic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicValue {
    /// The function object of definition `i` of this module (a constructor such as
    /// `%Number%`).
    Builtin(u16),
    /// A new ordinary object with this prototype (`None`: null), such as `%Math%`.
    Object { proto: Option<IntrinsicId> },
}

/// An intrinsic that a module makes.
#[derive(Clone, Copy, Debug)]
pub struct IntrinsicDef {
    pub id: IntrinsicId,
    pub value: IntrinsicValue,
}

/// One module of a table: a block of builtin ids, the intrinsics it makes and the properties
/// it adds.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinModule {
    pub name: &'static str,
    /// Below 128; unique among the installed modules.
    pub block: u8,
    /// Definition `i` has the id `BuiltinId::new(block, i)`.
    pub defs: &'static [BuiltinDef],
    pub intrinsics: &'static [IntrinsicDef],
    pub props: &'static [ObjectProps],
}

impl BuiltinModule {
    /// An empty module.
    pub const fn empty(name: &'static str, block: u8) -> BuiltinModule {
        BuiltinModule {
            name,
            block,
            defs: &[],
            intrinsics: &[],
            props: &[],
        }
    }

    /// The id of definition `index`.
    pub const fn id(&self, index: u16) -> BuiltinId {
        BuiltinId::new(self.block, index)
    }

    /// The id golden of this module: one line per definition (id, kind, length, name, where
    /// it is installed). An id change (a reorder, a removal) changes this text.
    pub fn ids_text(&self) -> String {
        let mut s = format!(
            "# ep9 builtin ids: module {}, block {}. Append-only (H20, W4-BUILTIN-IDS): never\n\
             # reorder, remove or reuse a line; `EP9_BLESS=1` rewrites this file after a review.\n\
             # id     block.index  kind         length  name  (installed as)\n",
            self.name, self.block
        );
        for (i, d) in self.defs.iter().enumerate() {
            let id = self.id(i as u16);
            let _ = writeln!(
                s,
                "{:#06x}   {:<11}  {:<11}  {:<6}  {:?}  ({})",
                id.bits(),
                format!("{}.{}", id.block(), id.index()),
                d.kind.text(),
                d.length,
                d.name,
                self.installed_as(i as u16)
            );
        }
        s
    }

    /// Where definition `i` first appears: an intrinsic it makes, or `target.key`.
    fn installed_as(&self, i: u16) -> String {
        if let Some(d) = self
            .intrinsics
            .iter()
            .find(|d| d.value == IntrinsicValue::Builtin(i))
        {
            return d.id.name().to_string();
        }
        for op in self.props {
            for p in op.props {
                let hit = match p.value {
                    PropValue::Builtin(j) => j == i,
                    PropValue::Accessor { get, set } => get == Some(i) || set == Some(i),
                    _ => false,
                };
                if hit {
                    return format!("{}.{} {}", op.target.name(), p.key.text(), p.attrs.text());
                }
            }
        }
        "not installed".to_string()
    }
}

/// A builtin table: the modules an embedder installs into every realm
/// (`IsolateConfig::builtins`; `ep9_builtins::TABLE` is the engine's).
pub struct BuiltinTable {
    pub name: &'static str,
    pub modules: &'static [BuiltinModule],
    /// The result of [`check`] for this table after the VM table (once per process).
    checked: OnceLock<Result<(), String>>,
    /// The lazy namespace groups of the VM table followed by this table ([`LazyGroups`];
    /// once per process, after [`check`]).
    lazy: OnceLock<LazyGroups>,
}

impl BuiltinTable {
    pub const fn new(name: &'static str, modules: &'static [BuiltinModule]) -> BuiltinTable {
        BuiltinTable {
            name,
            modules,
            checked: OnceLock::new(),
            lazy: OnceLock::new(),
        }
    }
}

impl core::fmt::Debug for BuiltinTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuiltinTable")
            .field("name", &self.name)
            .field("modules", &self.modules.len())
            .finish_non_exhaustive()
    }
}

// ---- checks -------------------------------------------------------------------------------------

/// Checks the VM table followed by `embedder` (once per process and table): see
/// [`check_tables`].
pub(crate) fn check(embedder: Option<&'static BuiltinTable>) -> Result<(), String> {
    let vm = &crate::natives::VM_TABLE;
    match embedder {
        None => vm.checked.get_or_init(|| check_tables(&[vm])).clone(),
        Some(t) => t.checked.get_or_init(|| check_tables(&[vm, t])).clone(),
    }
}

/// The table rules: blocks below 128 and unique; at most 512 definitions per block; each
/// intrinsic made once (never the VM-internal `IntrinsicId::RealmFuses`), and a core one only
/// named (by the VM table, as the builtin realm init makes it from); the prototype of a new
/// intrinsic object made before it (walk order); every index in range and of the right kind;
/// every named intrinsic made by realm init or a module; every alias names a data key (not an
/// accessor) defined earlier on the same object; no key twice on one object; the first key of
/// a constructor a module makes is `prototype`, a data property with attributes {} (§20:
/// `Number.prototype` and others), and every such constructor but `%Proxy%` (§28.2.2) has one.
pub fn check_tables(tables: &[&BuiltinTable]) -> Result<(), String> {
    let modules = || tables.iter().flat_map(|t| t.modules.iter());
    let mut blocks = HashSet::new();
    let mut made: HashSet<IntrinsicId> = IntrinsicId::ALL
        .into_iter()
        .filter(|id| id.is_core())
        .collect();
    // The constructors the modules make, in walk order, with the module that makes each.
    let mut constructors: Vec<(IntrinsicId, &'static str)> = Vec::new();
    for m in modules() {
        let at = |what: String| format!("builtin table module `{}`: {what}", m.name);
        if usize::from(m.block) >= BuiltinId::BLOCKS || !blocks.insert(m.block) {
            return Err(at(format!(
                "block {} is out of range or used twice",
                m.block
            )));
        }
        if m.defs.len() > BuiltinId::PER_BLOCK {
            return Err(at(format!("{} definitions (max 512)", m.defs.len())));
        }
        for d in m.intrinsics {
            // Pass 2 makes intrinsics in walk order: a prototype must exist by then.
            if let IntrinsicValue::Object { proto: Some(p) } = d.value
                && !made.contains(&p)
            {
                return Err(at(format!(
                    "{} has the prototype {}, which is not made before it",
                    d.id.name(),
                    p.name()
                )));
            }
            if d.id == IntrinsicId::RealmFuses {
                return Err(at(format!(
                    "{} is VM-internal: no table makes it",
                    d.id.name()
                )));
            }
            let core_builtin = d.id.is_core()
                && m.block == crate::natives::VM_BLOCK
                && matches!(d.value, IntrinsicValue::Builtin(_));
            if !core_builtin && (d.id.is_core() || !made.insert(d.id)) {
                return Err(at(format!(
                    "{} is a core intrinsic or made twice",
                    d.id.name()
                )));
            }
            match d.value {
                IntrinsicValue::Builtin(i) => {
                    data_def(m, i).map_err(at)?;
                    let ctor = m.defs[usize::from(i)].kind == BuiltinKind::Constructor;
                    if ctor && d.id != IntrinsicId::Proxy {
                        constructors.push((d.id, m.name));
                    }
                }
                IntrinsicValue::Object { .. } => {}
            }
        }
    }
    let prototype = PropKey::Atom(Atom::Prototype);
    // Defined keys with their values, one list per target (index `IntrinsicId as usize`) in
    // walk order, for aliases and duplicates. A check scans its own target's list only, so
    // this stays linear in the number of keys (it runs at each process start).
    let mut defined: Vec<Vec<(PropKey, PropValue)>> = vec![Vec::new(); IntrinsicId::ALL.len()];
    for m in modules() {
        let at = |what: String| format!("builtin table module `{}`: {what}", m.name);
        for op in m.props {
            if !made.contains(&op.target) {
                return Err(at(format!("target {} is not made", op.target.name())));
            }
            let on_target = &mut defined[op.target as usize];
            for p in op.props {
                let here =
                    |what: &str| at(format!("{}.{}: {what}", op.target.name(), p.key.text()));
                if on_target.iter().any(|(k, _)| k.same(&p.key)) {
                    return Err(here("defined twice"));
                }
                // Keys go in in walk order; a new key on a function first makes its lazy `prototype`
                // (`props::own_add`): a constructor but %Proxy% (§28.2.2) has `prototype` {} first.
                let is_prototype = p.key.same(&prototype)
                    && p.attrs == PropAttrs::NONE
                    && !matches!(p.value, PropValue::Accessor { .. });
                if !is_prototype
                    && constructors.iter().any(|(c, _)| *c == op.target)
                    && !on_target.iter().any(|(k, _)| k.same(&prototype))
                {
                    return Err(here(
                        "the first key of a constructor is `prototype`, a data property with \
                         attributes {}",
                    ));
                }
                match p.value {
                    PropValue::Builtin(i) => {
                        data_def(m, i).map_err(|e| here(&e))?;
                    }
                    PropValue::Intrinsic(id) => {
                        if !made.contains(&id) {
                            return Err(here(&format!("{} is not made", id.name())));
                        }
                    }
                    PropValue::Alias(k) => {
                        let of = on_target.iter().find(|(d, _)| d.same(&k));
                        match of {
                            None => {
                                return Err(here(&format!(
                                    "alias of {}, not defined before",
                                    k.text()
                                )));
                            }
                            // An alias copies a data value (one function object under two
                            // names, as `Array.prototype[@@iterator]`); no spec alias is an
                            // accessor, and install would copy the internal pair.
                            Some((_, PropValue::Accessor { .. })) => {
                                return Err(here(&format!(
                                    "alias of {}, an accessor (an alias needs a data property)",
                                    k.text()
                                )));
                            }
                            Some(_) => {}
                        }
                    }
                    PropValue::Accessor { get, set } => {
                        if get.is_none() && set.is_none() {
                            return Err(here("an accessor without a getter or a setter"));
                        }
                        for (i, kind) in [(get, BuiltinKind::Getter), (set, BuiltinKind::Setter)] {
                            if let Some(i) = i
                                && m.defs.get(usize::from(i)).map(|d| d.kind) != Some(kind)
                            {
                                return Err(here(&format!("definition {i} is not a {kind:?}")));
                            }
                        }
                    }
                    PropValue::Const(_) | PropValue::Symbol(_) => {}
                }
                on_target.push((p.key, p.value));
            }
        }
    }
    for (c, module) in &constructors {
        if !defined[*c as usize].iter().any(|(k, _)| k.same(&prototype)) {
            return Err(format!(
                "builtin table module `{module}`: the constructor {} has no `prototype`",
                c.name()
            ));
        }
    }
    Ok(())
}

/// Definition `i` of `m` is a function value (a `Method` or a `Constructor`).
fn data_def(m: &BuiltinModule, i: u16) -> Result<(), String> {
    match m.defs.get(usize::from(i)).map(|d| d.kind) {
        Some(BuiltinKind::Method | BuiltinKind::Constructor) => Ok(()),
        Some(k) => Err(format!("definition {i} is a {k:?}, not a function value")),
        None => Err(format!("definition {i} does not exist")),
    }
}

// ---- per-isolate state ------------------------------------------------------------------------

/// The number of intrinsics held in builtin state slots ([`IntrinsicId::slot`]).
const INTRINSIC_SLOTS: usize = 65;

/// Per-isolate builtin state (`Runtime::builtins`): one entry per installed module, and the
/// intrinsics without a runtime root.
///
/// Realms (H22-Q3): the FunctionInfos are per isolate (every realm's function object of a
/// definition points at the same one). The intrinsic slots hold the current realm's values
/// (`realm::switch` loads them from the REALM record). The function slots of the modules hold
/// the first realm's function objects; a later realm's install makes its own in the install
/// list ([`begin_realm_install`]), and after its install they are reachable only through
/// that realm's objects. So [`existing_function`] knows the function objects of the first
/// realm only. A lazy group ([`LazyGroups`]) installs into a realm at its first use: its
/// function objects go into the module slots for the first realm, else into a new install
/// list, as a later realm's install does.
pub(crate) struct BuiltinState {
    /// The embedder's table (`IsolateConfig::builtins`).
    table: Cell<Option<&'static BuiltinTable>>,
    /// The lazy namespace groups of the isolate's tables (set by [`prepare`]).
    lazy: Cell<Option<&'static LazyGroups>>,
    /// The installed modules in walk order (the VM table first).
    modules: RefCell<Vec<ModuleState>>,
    /// The number of definitions of all modules (the length of an install list).
    definitions: Cell<usize>,
    /// The intrinsics of [`IntrinsicId::slot`], undefined until made: a strong root range
    /// that [`prepare`] registers (the heap is the Runtime's first field, so it drops with the
    /// range before this box).
    intrinsics: Box<[Slot]>,
    /// One slot: the install list of a realm that is being installed after the first (an
    /// ELEMENTS cell, one entry per definition in walk order), or undefined. A strong root
    /// range that [`prepare`] registers.
    install_list: Box<[Slot]>,
}

impl Default for BuiltinState {
    fn default() -> Self {
        BuiltinState {
            table: Cell::new(None),
            lazy: Cell::new(None),
            modules: RefCell::new(Vec::new()),
            definitions: Cell::new(0),
            intrinsics: (0..INTRINSIC_SLOTS)
                .map(|_| Slot::new(Value::UNDEFINED))
                .collect(),
            install_list: Box::new([Slot::new(Value::UNDEFINED)]),
        }
    }
}

struct ModuleState {
    module: &'static BuiltinModule,
    /// The index of the module's first definition in an install list.
    first: usize,
    /// One FunctionInfo per definition (dense; frames and function cells point into it). The
    /// box of a lazy group's module is empty until the group's first install.
    infos: Box<[FunctionInfo]>,
    /// The function object of each definition in the first realm, undefined until made: a
    /// strong root range (the heap is the Runtime's first field, so it drops with the range
    /// before this box).
    functions: Box<[Slot]>,
}

impl BuiltinState {
    /// The embedder's table, if one is installed.
    pub(crate) fn table(&self) -> Option<&'static BuiltinTable> {
        self.table.get()
    }
}

/// The tables of an isolate in walk order.
fn tables(embedder: Option<&'static BuiltinTable>) -> impl Iterator<Item = &'static BuiltinTable> {
    core::iter::once(&crate::natives::VM_TABLE).chain(embedder)
}

/// Builds the dense FunctionInfo array and registers the function root range of every module
/// (VM table first) and the intrinsic slots, and finds the lazy groups of the tables
/// ([`LazyGroups`], once per table). No GC allocation. Runs once, before `realm::init`.
pub(crate) fn prepare(rt: &Runtime, embedder: Option<&'static BuiltinTable>) {
    rt.builtins.table.set(embedder);
    rt.builtins.lazy.set(Some(lazy_groups(embedder)));
    let mut mods = rt.builtins.modules.borrow_mut();
    debug_assert!(mods.is_empty(), "builtins::prepare runs once");
    // SAFETY: the slots live in a box that the Runtime owns (`builtins`); the heap is the
    // Runtime's first field, so it drops (with this range) before the box. They hold
    // `undefined` now and intrinsic objects later (never 0).
    unsafe {
        rt.heap.add_root_range(
            NonNull::from(&rt.builtins.intrinsics[..]),
            RootStrength::Strong,
        );
        rt.heap.add_root_range(
            NonNull::from(&rt.builtins.install_list[..]),
            RootStrength::Strong,
        );
    }
    let lazy = lazy_of(rt);
    let mut first = 0;
    for (mi, module) in tables(embedder).flat_map(|t| t.modules.iter()).enumerate() {
        // A lazy group's FunctionInfos come at its first install ([`install_group`]).
        let infos = if lazy.module.get(mi).copied().flatten().is_some() {
            Box::default()
        } else {
            function_infos(module)
        };
        let functions: Box<[Slot]> = module
            .defs
            .iter()
            .map(|_| Slot::new(Value::UNDEFINED))
            .collect();
        // SAFETY: the slots live in a box that the Runtime owns (`builtins`); the heap is the
        // Runtime's first field, so it drops (with this range) before the box. They hold
        // `undefined` now and function objects later (never 0).
        unsafe {
            rt.heap
                .add_root_range(NonNull::from(&functions[..]), RootStrength::Strong);
        }
        mods.push(ModuleState {
            module,
            first,
            infos,
            functions,
        });
        first += module.defs.len();
    }
    rt.builtins.definitions.set(first);
}

/// One FunctionInfo per definition of `module`.
fn function_infos(module: &BuiltinModule) -> Box<[FunctionInfo]> {
    module
        .defs
        .iter()
        .map(|d| {
            let constructor = d.kind == BuiltinKind::Constructor;
            FunctionInfo::host(d.f, &d.function_name(), d.length, constructor)
        })
        .collect()
}

/// Starts the install of a realm after the first: a new install list (one undefined entry per
/// definition), so [`function_object`] makes this realm's own function objects. GC point.
pub(crate) fn begin_realm_install(rt: &Runtime) {
    let n = rt.builtins.definitions.get();
    let list = object::new_list(rt, n, UNDEFINED);
    // E3: a store into a root slot takes no barrier.
    rt.builtins.install_list[0].init(val(list));
}

/// Ends the install that [`begin_realm_install`] started: the list goes; the realm's function
/// objects stay reachable through its objects. Not a GC point.
pub(crate) fn end_realm_install(rt: &Runtime) {
    rt.builtins.install_list[0].init(Value::UNDEFINED);
}

/// Makes every intrinsic slot undefined (a new realm's init and install fill them). Not a GC
/// point.
pub(crate) fn clear_intrinsic_slots(rt: &Runtime) {
    for s in rt.builtins.intrinsics.iter() {
        s.init(Value::UNDEFINED);
    }
}

/// Where the function object of a builtin lives while its realm installs: a module slot (the
/// first realm) or an entry of the install list (a later realm).
#[derive(Clone, Copy)]
enum Home {
    Slot(usize, usize),
    List(u64, usize),
}

/// (home, FunctionInfo, function bits) of builtin `id` in the realm being installed, or in
/// the first realm when no install list is set.
fn lookup(rt: &Runtime, id: BuiltinId) -> Option<(Home, *const FunctionInfo, u64)> {
    let mods = rt.builtins.modules.borrow();
    let (m, st) = mods
        .iter()
        .enumerate()
        .find(|(_, s)| s.module.block == id.block())?;
    let i = usize::from(id.index());
    let info = st.infos.get(i)?;
    let list = rt.builtins.install_list[0].bits();
    let (home, f) = if list == UNDEFINED {
        (Home::Slot(m, i), st.functions[i].bits())
    } else {
        let at = st.first + i;
        (Home::List(list, at), object::list_get(list, at))
    };
    Some((home, core::ptr::from_ref(info), f))
}

/// The function object of builtin `id` in the realm being installed (the current realm), made
/// on first use (a GC point then). Panics if no installed module has `id`.
pub(crate) fn function_object(rt: &Runtime, id: BuiltinId) -> u64 {
    let (home, info, f) = lookup(rt, id).unwrap_or_else(|| panic!("no builtin {id}"));
    if f != UNDEFINED {
        return f;
    }
    // GC point; no borrow is held (W4-GC-RULES). The install list is a root.
    let f = object::new_function(rt, info, UNDEFINED);
    match home {
        // E3: a store into a root slot takes no barrier.
        Home::Slot(m, i) => rt.builtins.modules.borrow()[m].functions[i].init(val(f)),
        Home::List(list, at) => object::list_set(rt, list, at, f),
    }
    f
}

/// The function object of builtin `id` if it was made and belongs to the current realm (no
/// allocation): the "is this still the intrinsic?" test of the fast paths
/// (`interp/slow/iter.rs`, `property.rs`, `natives.rs`). It knows the first realm's function
/// objects only ([`BuiltinState`]): in a later realm it gives `None`, so those fast paths
/// take their spec steps there, and a function of another realm never passes the test.
pub(crate) fn existing_function(rt: &Runtime, id: BuiltinId) -> Option<u64> {
    lookup(rt, id)
        .map(|(_, _, f)| f)
        .filter(|&f| f != UNDEFINED && object::function_realm(f) == rt.root(Root::Realm))
}

/// Installs the tables that [`prepare`] registered into the current realm: (1) the function
/// object of every definition, (2) the intrinsics the modules make, then the `[[Prototype]]`
/// of %GeneratorFunction%, %AsyncFunction% and %AsyncGeneratorFunction% (%Function%) and of
/// the native errors (%Error%), (3) the properties. Walk order: VM table, then the embedder's;
/// modules, then entries, in order. The modules of a lazy group ([`LazyGroups`]) are left
/// out: only the group's global key gets its place, with the reserved value HOLE
/// ([`install_group`] installs the group at its first use). Runs after `realm::init`. GC
/// points.
pub(crate) fn install(rt: &Runtime) -> Result<(), String> {
    install_pass(rt, Pass::Eager)
}

/// What one install pass covers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Realm init: every module that is in no lazy group, and the reserved value of each
    /// lazy key.
    Eager,
    /// Lazy group `g` into the current realm (the realm roots are that realm's).
    Group(u8),
}

/// The three install steps of [`install`] for the modules of `pass`. GC points: every new
/// cell goes into a root (a module slot, the install list, an intrinsic slot or root) or a
/// property of an object that one holds before the next allocation.
fn install_pass(rt: &Runtime, pass: Pass) -> Result<(), String> {
    let embedder = rt.builtins.table();
    let lazy = lazy_of(rt);
    let modules: Vec<(&'static BuiltinModule, Option<u8>)> = tables(embedder)
        .flat_map(|t| t.modules.iter())
        .enumerate()
        .map(|(i, m)| (m, lazy.module.get(i).copied().flatten()))
        .collect();
    let wanted = |g: Option<u8>| match pass {
        Pass::Eager => g.is_none(),
        Pass::Group(x) => g == Some(x),
    };
    for &(m, g) in &modules {
        if wanted(g) {
            for i in 0..m.defs.len() {
                function_object(rt, m.id(i as u16));
            }
        }
    }
    for &(m, _) in modules.iter().filter(|(_, g)| wanted(*g)) {
        for d in m.intrinsics {
            let v = match d.value {
                IntrinsicValue::Builtin(i) => function_object(rt, m.id(i)),
                IntrinsicValue::Object { proto } => {
                    let p = proto.map_or(NULL, |p| intrinsic(rt, p));
                    if p != NULL && !is_object(p) {
                        return Err(format!(
                            "builtin table module `{}`: the prototype of {} is not installed",
                            m.name,
                            d.id.name()
                        ));
                    }
                    object::new_object(rt, p)
                }
            };
            if d.id.is_core() {
                // Realm init made it from this builtin (checked: VM table only).
                if intrinsic(rt, d.id) != v {
                    return Err(format!(
                        "builtin table module `{}`: realm init did not make {} from its builtin",
                        m.name,
                        d.id.name()
                    ));
                }
            } else {
                set_intrinsic(rt, d.id, v);
            }
        }
    }
    if pass == Pass::Eager {
        fixed_prototypes(rt)?;
    }
    for &(m, g) in &modules {
        let key_of = |g: u8| lazy.groups[usize::from(g)].key;
        for op in m.props {
            let lazy_key = |p: &PropDef| {
                g.filter(|&g| {
                    op.target == IntrinsicId::GlobalObject && p.key.string_text() == Some(key_of(g))
                })
            };
            if !wanted(g) {
                // A lazy module at realm init: only its global key, with the reserved value.
                if pass == Pass::Eager {
                    for p in op.props.iter().filter(|p| lazy_key(p).is_some()) {
                        define_reserved(rt, m, p)?;
                    }
                }
                continue;
            }
            let target = intrinsic(rt, op.target);
            if !is_object(target) {
                return Err(format!(
                    "builtin table module `{}`: {} is not installed",
                    m.name,
                    op.target.name()
                ));
            }
            for p in op.props {
                if lazy_key(p).is_some() {
                    store_reserved(rt, m, target, p)?;
                } else {
                    define(rt, m, target, op.target, p)?;
                }
            }
        }
    }
    Ok(())
}

/// Step (2) of the eager install, after the intrinsics: §27.3.2, §27.7.2, §27.4.2 (the
/// `[[Prototype]]` of %GeneratorFunction%, %AsyncFunction% and %AsyncGeneratorFunction% is
/// %Function%) and §20.5.6.2 (the `[[Prototype]]` of each NativeError constructor is %Error%).
/// A table gives a builtin's function object %Function.prototype% only. %AggregateError%
/// (§20.5.7.2) gets it in `realm.rs`, after the embedder's table made it. GC points
/// (dictionary mode, OM-10): the intrinsics are roots.
fn fixed_prototypes(rt: &Runtime) -> Result<(), String> {
    let function = intrinsic(rt, IntrinsicId::Function);
    for id in [
        IntrinsicId::GeneratorFunction,
        IntrinsicId::AsyncFunction,
        IntrinsicId::AsyncGeneratorFunction,
    ] {
        let c = intrinsic(rt, id);
        if !is_object(function)
            || !is_object(c)
            || !crate::property::set_prototype_of(rt, c, function)
        {
            return Err(format!("could not set the [[Prototype]] of {}", id.name()));
        }
    }
    let error = intrinsic(rt, IntrinsicId::Error);
    for id in [
        IntrinsicId::EvalError,
        IntrinsicId::RangeError,
        IntrinsicId::ReferenceError,
        IntrinsicId::SyntaxError,
        IntrinsicId::TypeError,
        IntrinsicId::UriError,
    ] {
        let c = intrinsic(rt, id);
        if !is_object(error) || !is_object(c) || !crate::property::set_prototype_of(rt, c, error) {
            return Err(format!("could not set the [[Prototype]] of {}", id.name()));
        }
    }
    Ok(())
}

/// The key bits of `k` (an atom or a symbol). GC point (interning).
fn key_bits(rt: &Runtime, k: PropKey) -> Result<u64, String> {
    Ok(match k {
        PropKey::Atom(a) => rt.atom(a),
        PropKey::Str(s) => atoms::intern_str(rt, s),
        PropKey::Symbol(w) => well_known(rt, w)?,
    })
}

fn well_known(rt: &Runtime, w: WellKnownSymbol) -> Result<u64, String> {
    let s = symbol::well_known(rt, w.0);
    if is_symbol(s) {
        Ok(s)
    } else {
        Err(format!(
            "Symbol.{} does not exist yet (symbols, P2-SYM)",
            w.name()
        ))
    }
}

/// Defines one table property on `target`. An accessor goes straight onto the object (a new
/// ACCESSOR_PAIR, as `property::define_own_property` makes for a new key of an ordinary
/// object), so an install needs only `&Runtime` and runs no JS. GC points: `target`, the key
/// and the value live in Rust locals (conservative scan) and the intrinsics are roots.
fn define(
    rt: &Runtime,
    m: &BuiltinModule,
    target: u64,
    id: IntrinsicId,
    p: &PropDef,
) -> Result<(), String> {
    let at = |what: &str| {
        format!(
            "builtin table module `{}`: {}.{}: {what}",
            m.name,
            id.name(),
            p.key.text()
        )
    };
    let key = key_bits(rt, p.key).map_err(|e| at(&e))?;
    // The checks see table keys only; realm init and `object::new_function` (a function's
    // `length` and `name`) make others. A second definition is an error in every build.
    if own_get(target, key).is_some() {
        return Err(at(
            "the object has this key already (from realm init or the function)",
        ));
    }
    let v = match p.value {
        PropValue::Builtin(i) => function_object(rt, m.id(i)),
        PropValue::Intrinsic(i) => {
            let v = intrinsic(rt, i);
            if !is_object(v) {
                return Err(at(&format!("{} is not installed", i.name())));
            }
            v
        }
        PropValue::Const(c) => match c {
            ConstValue::Undefined => UNDEFINED,
            ConstValue::Null => NULL,
            ConstValue::Bool(b) => {
                if b {
                    TRUE
                } else {
                    FALSE
                }
            }
            ConstValue::Number(d) => num(d),
            ConstValue::Str(s) => atoms::intern_str(rt, s),
        },
        PropValue::Symbol(w) => well_known(rt, w).map_err(|e| at(&e))?,
        PropValue::Alias(k) => {
            let k = key_bits(rt, k).map_err(|e| at(&e))?;
            match own_get(target, k) {
                None => return Err(at("the aliased key is missing")),
                // `check_tables` refuses this: the slot holds the internal ACCESSOR_PAIR cell.
                Some((_, a)) if a & attr::ACCESSOR != 0 => {
                    return Err(at("the aliased key is an accessor"));
                }
                Some((v, _)) => v,
            }
        }
        PropValue::Accessor { get, set } => {
            // The ordinary step for a new key (`property::add_accessor`): a table target is
            // an ordinary object or a function, never an Array or a routed kind.
            if crate::exotic::is_routed(crate::kinds::cell_kind(target)) || object::is_array(target)
            {
                return Err(at("an accessor on an exotic object"));
            }
            let get = get.map_or(UNDEFINED, |i| function_object(rt, m.id(i)));
            let set = set.map_or(UNDEFINED, |i| function_object(rt, m.id(i)));
            // GC point: `target`, `key`, `get` and `set` live in Rust locals.
            let pair = crate::accessor::new_pair(rt, get, set);
            let ec = p.attrs.bits() & (attr::ENUMERABLE | attr::CONFIGURABLE);
            own_define(rt, target, key, pair, attr::ACCESSOR | ec);
            return Ok(());
        }
    };
    own_define(rt, target, key, v, p.attrs.bits());
    Ok(())
}

/// Realm init for the global key `p` of a lazy group: the key gets its table place and
/// attributes with the reserved value HOLE (a VM-internal immediate that is no JS value,
/// H06). Its field goes to rep ANY and not const (`field_rep::widen`), so the later install
/// store ([`store_reserved`]) moves no shape and pops no fuse, and no compiled fact holds
/// HOLE. GC point (the key, the property).
fn define_reserved(rt: &Runtime, m: &BuiltinModule, p: &PropDef) -> Result<(), String> {
    let global = intrinsic(rt, IntrinsicId::GlobalObject);
    let key = key_bits(rt, p.key)?;
    if own_get(global, key).is_some() {
        return Err(format!(
            "builtin table module `{}`: globalThis.{}: the object has this key already",
            m.name,
            p.key.text()
        ));
    }
    own_define(rt, global, key, HOLE, p.attrs.bits());
    let s = crate::shape::shape_of(global);
    if !crate::shape::is_dictionary(s)
        && let Some(f) = crate::shape::lookup(rt, s, key)
    {
        crate::field_rep::widen(rt, s, f.index);
    }
    Ok(())
}

/// The group install for the global key `p` on `global`: stores the key's intrinsic only
/// while the property is still a data property that holds HOLE. A write, a define or a
/// delete before the install replaced the reserved value, and the install keeps that result
/// (the result of an eager install followed by the same operation). Not a GC point.
fn store_reserved(rt: &Runtime, m: &BuiltinModule, global: u64, p: &PropDef) -> Result<(), String> {
    let PropValue::Intrinsic(id) = p.value else {
        unreachable!("a lazy key names its group's intrinsic (lazy_groups)");
    };
    let v = intrinsic(rt, id);
    if !is_object(v) {
        return Err(format!(
            "builtin table module `{}`: {} is not installed",
            m.name,
            id.name()
        ));
    }
    // The key is an atom already: realm init interned it for `define_reserved`.
    let key = key_bits(rt, p.key)?;
    if matches!(own_get(global, key), Some((HOLE, a)) if a & attr::ACCESSOR == 0) {
        object::own_replace(rt, global, key, v);
    }
    Ok(())
}

// ---- lazy namespace groups (INTL-I1-D8; M7-P1 LAZYNS) -----------------------------------------

/// The global keys whose namespace installs into a realm at its first use, not at realm init.
/// A key that no module puts on %GlobalObject% does nothing (Intl before INTL wave 1).
const LAZY_KEYS: [&str; 2] = ["Temporal", "Intl"];

/// The intrinsics whose `[[Prototype]]` the eager install sets after its step (2)
/// ([`fixed_prototypes`], `realm.rs` for %AggregateError%): never in a lazy group.
const FIXED_PROTOTYPES: [IntrinsicId; 12] = [
    IntrinsicId::Function,
    IntrinsicId::Error,
    IntrinsicId::GeneratorFunction,
    IntrinsicId::AsyncFunction,
    IntrinsicId::AsyncGeneratorFunction,
    IntrinsicId::EvalError,
    IntrinsicId::RangeError,
    IntrinsicId::ReferenceError,
    IntrinsicId::SyntaxError,
    IntrinsicId::TypeError,
    IntrinsicId::UriError,
    IntrinsicId::AggregateError,
];

/// One lazy group: the modules that one lazy key brings.
struct LazyGroup {
    /// The global key (an entry of [`LAZY_KEYS`]).
    key: &'static str,
    /// The first intrinsic the group makes in walk order (the key's value): the group is
    /// installed in a realm when this intrinsic is an object there (no REALM field and no
    /// root records it).
    first: IntrinsicId,
}

/// The lazy namespace groups of an isolate's tables (the VM table, then the embedder's),
/// computed once per table from the table data (INTL-I1-D8): for each key of [`LAZY_KEYS`],
/// the module that puts the key on %GlobalObject% as one of its own intrinsics, then (to a
/// fixed point) every module whose properties target only group intrinsics or its own
/// intrinsics, and at least one group intrinsic. A key is lazy only if no module outside its
/// group names a group intrinsic (as a target, a value or a prototype), no group module is
/// the VM's own, and no group intrinsic is core or in [`FIXED_PROTOTYPES`]; else its
/// modules stay eager. The map holds no `Value` (rule 8).
pub(crate) struct LazyGroups {
    groups: Vec<LazyGroup>,
    /// The group of each module in walk order; `None`: eager.
    module: Vec<Option<u8>>,
    /// The group of each intrinsic id (index `IntrinsicId as usize`); `None`: eager.
    id: [Option<u8>; IntrinsicId::ALL.len()],
}

impl LazyGroups {
    const NONE: LazyGroups = LazyGroups {
        groups: Vec::new(),
        module: Vec::new(),
        id: [None; IntrinsicId::ALL.len()],
    };

    fn compute(modules: &[&'static BuiltinModule]) -> LazyGroups {
        // A set of intrinsic ids: one flag per id (index `IntrinsicId as usize`). It runs at
        // each process start, so no hashing.
        type Ids = [bool; IntrinsicId::ALL.len()];
        let mut out = LazyGroups::NONE;
        out.module = vec![None; modules.len()];
        let names = |m: &BuiltinModule, ids: &Ids| {
            m.intrinsics.iter().any(
                |d| matches!(d.value, IntrinsicValue::Object { proto: Some(p) } if ids[p as usize]),
            ) || m.props.iter().any(|op| {
                ids[op.target as usize]
                    || op
                        .props
                        .iter()
                        .any(|p| matches!(p.value, PropValue::Intrinsic(i) if ids[i as usize]))
            })
        };
        let makes = |m: &BuiltinModule, id: IntrinsicId| m.intrinsics.iter().any(|d| d.id == id);
        for key in LAZY_KEYS {
            let is_key = |p: &PropDef| p.key.string_text() == Some(key);
            let found = modules.iter().enumerate().find_map(|(i, m)| {
                let p = m
                    .props
                    .iter()
                    .filter(|op| op.target == IntrinsicId::GlobalObject)
                    .flat_map(|op| op.props)
                    .find(|p| is_key(p))?;
                match p.value {
                    PropValue::Intrinsic(id) if makes(m, id) => Some((i, id)),
                    _ => None,
                }
            });
            let Some((km, first)) = found else { continue };
            if out.module[km].is_some() || out.groups.len() >= usize::from(u8::MAX) {
                continue;
            }
            let mut ids: Ids = [false; IntrinsicId::ALL.len()];
            for d in modules[km].intrinsics {
                ids[d.id as usize] = true;
            }
            // The key module: its only %GlobalObject% property is the key; its other
            // properties target its own intrinsics.
            let key_module_ok = modules[km].props.iter().all(|op| {
                if op.target == IntrinsicId::GlobalObject {
                    op.props.len() == 1 && is_key(&op.props[0])
                } else {
                    ids[op.target as usize]
                }
            });
            if !key_module_ok || modules[km].intrinsics.first().map(|d| d.id) != Some(first) {
                continue;
            }
            let mut member = vec![false; modules.len()];
            member[km] = true;
            loop {
                let mut grew = false;
                for (i, m) in modules.iter().enumerate() {
                    if member[i] || out.module[i].is_some() || m.props.is_empty() {
                        continue;
                    }
                    if m.props.iter().any(|op| ids[op.target as usize])
                        && m.props
                            .iter()
                            .all(|op| ids[op.target as usize] || makes(m, op.target))
                    {
                        member[i] = true;
                        grew = true;
                        for d in m.intrinsics {
                            ids[d.id as usize] = true;
                        }
                    }
                }
                if !grew {
                    break;
                }
            }
            let leaks = modules
                .iter()
                .enumerate()
                .any(|(i, m)| !member[i] && names(m, &ids));
            let pinned = modules
                .iter()
                .enumerate()
                .any(|(i, m)| member[i] && m.block == crate::natives::VM_BLOCK)
                || IntrinsicId::ALL
                    .into_iter()
                    .any(|id| ids[id as usize] && (id.is_core() || FIXED_PROTOTYPES.contains(&id)));
            // The first intrinsic in walk order is the key's value (the installed mark).
            if leaks || pinned || member.iter().position(|&b| b) != Some(km) {
                continue;
            }
            let g = out.groups.len() as u8;
            out.groups.push(LazyGroup { key, first });
            for (i, m) in modules.iter().enumerate() {
                if member[i] {
                    out.module[i] = Some(g);
                    for d in m.intrinsics {
                        out.id[d.id as usize] = Some(g);
                    }
                }
            }
        }
        out
    }
}

/// The lazy groups of the VM table followed by `embedder` (once per process and table).
fn lazy_groups(embedder: Option<&'static BuiltinTable>) -> &'static LazyGroups {
    let t = embedder.unwrap_or(&crate::natives::VM_TABLE);
    t.lazy.get_or_init(|| {
        let modules: Vec<&'static BuiltinModule> =
            tables(embedder).flat_map(|t| t.modules.iter()).collect();
        LazyGroups::compute(&modules)
    })
}

/// The lazy groups of the isolate (none before [`prepare`]).
fn lazy_of(rt: &Runtime) -> &'static LazyGroups {
    static NONE: LazyGroups = LazyGroups::NONE;
    rt.builtins.lazy.get().unwrap_or(&NONE)
}

/// The lazy group of the intrinsic `id`, if it has one. Not a GC point.
pub(crate) fn lazy_group(rt: &Runtime, id: IntrinsicId) -> Option<u8> {
    lazy_of(rt).id[id as usize]
}

/// The lazy group whose global key is the property key `key`, if one is. Not a GC point.
pub(crate) fn lazy_group_of_key(rt: &Runtime, key: u64) -> Option<u8> {
    let groups = &lazy_of(rt).groups;
    if groups.is_empty() || !crate::kinds::is_string(key) {
        return None;
    }
    let text = crate::string::to_rust_string(key);
    groups.iter().position(|g| g.key == text).map(|g| g as u8)
}

/// Installs lazy group `g` into `realm` (current or not) unless it is installed there: its
/// function objects, intrinsics and properties, in table order (the three steps of
/// [`install`]), and the group's global key while that still holds HOLE. For a realm that is
/// not current, only the realm roots switch to it and back (no JS runs, so `Cx::realm` does
/// not change). The first realm's function objects go into the module slots, a later
/// realm's into a new install list (as its own install did). Panics if the install fails
/// (`check_tables` passed for the table, and the eager install of the same table did not
/// fail). GC point.
pub(crate) fn install_group(rt: &Runtime, realm: u64, g: u8) {
    let group = &lazy_of(rt).groups[usize::from(g)];
    if is_object(crate::realm::intrinsic_in(rt, realm, group.first)) {
        return;
    }
    // The group's FunctionInfos, once per isolate (no function object points into the empty
    // box that this replaces). Not a GC point.
    for (st, lg) in rt
        .builtins
        .modules
        .borrow_mut()
        .iter_mut()
        .zip(&lazy_of(rt).module)
    {
        if *lg == Some(g) && st.infos.is_empty() {
            st.infos = function_infos(st.module);
        }
    }
    // GC points below: `current` and `saved` live in Rust locals (conservative scan), so the
    // current realm (through its record) and an install list in use stay live.
    let current = rt.root(Root::Realm);
    let other = realm != current;
    if other {
        load_realm_roots(rt, realm);
    }
    let saved = rt.builtins.install_list[0].bits();
    if is_first_realm(rt, realm) {
        // E3: a store into a root slot takes no barrier.
        rt.builtins.install_list[0].init(Value::UNDEFINED);
    } else {
        begin_realm_install(rt);
    }
    let r = install_pass(rt, Pass::Group(g));
    rt.builtins.install_list[0].init(val(saved));
    if other {
        load_realm_roots(rt, current);
    }
    if let Err(e) = r {
        panic!(
            "ep9 internal error: the lazy {} namespace install failed: {e}",
            group.key
        );
    }
}

/// Whether `realm` is the first realm of the isolate: the one whose function objects the
/// module slots hold (its %Function.prototype%, VM builtin 0.0, is in the slot). Not a GC
/// point.
fn is_first_realm(rt: &Runtime, realm: u64) -> bool {
    let mods = rt.builtins.modules.borrow();
    let f = mods[0].functions[usize::from(crate::natives::vm::FUNCTION_PROTOTYPE)].bits();
    f != UNDEFINED && object::function_realm(f) == realm
}

/// The root part of `realm::switch`: writes `realm`'s intrinsics, lexical record and record
/// into the current-realm roots, the intrinsic slots and `%Symbol.prototype%`, and empties
/// the function shape caches (`shape::clear_realm_caches`). `Cx::realm` and
/// `Cx::global_this` stay: no JS runs before the switch back. Not a GC point.
fn load_realm_roots(rt: &Runtime, realm: u64) {
    use crate::object::realmf;
    let list = object::record_get(realm, realmf::INTRINSICS);
    for id in IntrinsicId::ALL {
        load_intrinsic(rt, id, object::list_get(list, id as usize));
    }
    rt.set_root(
        Root::Lexical,
        object::record_get(realm, realmf::GLOBAL_LEXICAL),
    );
    rt.set_root(Root::Realm, realm);
    crate::shape::clear_realm_caches(rt);
}

/// Whether the function object `f` was made from a definition of an installed builtin table
/// (in any realm: every realm's function objects share the definition's FunctionInfo), not by
/// `Isolate::register_host_function` or a closure. Not a GC point.
pub(crate) fn is_table_function(rt: &Runtime, f: u64) -> bool {
    let info = core::ptr::from_ref(object::function_info(f));
    rt.builtins
        .modules
        .borrow()
        .iter()
        .any(|st| st.infos.as_ptr_range().contains(&info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intrinsic_ids_are_dense_and_map_to_distinct_roots() {
        let mut roots = Vec::new();
        for (i, id) in IntrinsicId::ALL.iter().enumerate() {
            assert_eq!(*id as usize, i);
            if let Some(r) = id.root() {
                roots.push(r as u8);
            }
        }
        let n = roots.len();
        roots.sort_unstable();
        roots.dedup();
        assert_eq!(roots.len(), n);
        // %Symbol.prototype% lives in symbol.rs; the others without a root have one slot each.
        let mut slots: Vec<usize> = IntrinsicId::ALL.iter().filter_map(|id| id.slot()).collect();
        for id in IntrinsicId::ALL {
            assert!(id.root().is_none() || id.slot().is_none(), "{}", id.name());
            assert!(!id.is_core() || id.slot().is_none(), "{}", id.name());
        }
        assert_eq!(n + slots.len(), IntrinsicId::ALL.len() - 1);
        slots.sort_unstable();
        assert_eq!(slots, (0..INTRINSIC_SLOTS).collect::<Vec<_>>());
        assert!(
            roots
                .iter()
                .all(|&r| usize::from(r) < crate::runtime::ROOT_COUNT)
        );
    }

    #[test]
    fn builtin_ids_pack_block_and_index() {
        let id = BuiltinId::new(5, 300);
        assert_eq!((id.block(), id.index()), (5, 300));
        assert_eq!(id.bits(), (5 << 9) | 300);
        assert_eq!(BuiltinId::new(127, 511).bits(), u16::MAX);
        assert_eq!(id.to_string(), "0x0b2c 5.300");
    }

    #[test]
    fn well_known_symbols_mirror_symbol_rs() {
        let ours = [
            WellKnownSymbol::ASYNC_ITERATOR,
            WellKnownSymbol::HAS_INSTANCE,
            WellKnownSymbol::IS_CONCAT_SPREADABLE,
            WellKnownSymbol::ITERATOR,
            WellKnownSymbol::MATCH,
            WellKnownSymbol::MATCH_ALL,
            WellKnownSymbol::REPLACE,
            WellKnownSymbol::SEARCH,
            WellKnownSymbol::SPECIES,
            WellKnownSymbol::SPLIT,
            WellKnownSymbol::TO_PRIMITIVE,
            WellKnownSymbol::TO_STRING_TAG,
            WellKnownSymbol::UNSCOPABLES,
            WellKnownSymbol::DISPOSE,
            WellKnownSymbol::ASYNC_DISPOSE,
        ];
        assert_eq!(ours.len(), symbol::WellKnownSymbol::ALL.len());
        for (a, b) in ours.iter().zip(symbol::WellKnownSymbol::ALL) {
            assert_eq!(a.0, b);
        }
    }

    #[test]
    fn keys_compare_by_text_and_symbol() {
        assert!(PropKey::Atom(Atom::Length).same(&PropKey::Str("length")));
        assert!(!PropKey::Str("iterator").same(&PropKey::Symbol(WellKnownSymbol::ITERATOR)));
        assert!(!PropKey::Symbol(WellKnownSymbol::ITERATOR).same(&PropKey::Str("iterator")));
        assert!(!PropKey::Atom(Atom::Length).same(&PropKey::Str("name")));
        assert!(
            PropKey::Symbol(WellKnownSymbol::ITERATOR)
                .same(&PropKey::Symbol(WellKnownSymbol::ITERATOR))
        );
        assert!(
            !PropKey::Symbol(WellKnownSymbol::ITERATOR)
                .same(&PropKey::Symbol(WellKnownSymbol::ASYNC_ITERATOR))
        );
        assert_eq!(
            PropKey::Symbol(WellKnownSymbol::ITERATOR).text(),
            "[Symbol.iterator]"
        );
        assert_eq!(PropAttrs::METHOD.text(), "w-c");
        assert_eq!(PropAttrs::METHOD.bits(), attr::HIDDEN);
        assert_eq!(PropAttrs::ALL.bits(), attr::DEFAULT);
    }
}
