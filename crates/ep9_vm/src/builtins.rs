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
//! | A constructor a table makes gets `prototype` {} as its first table key | §20 (`Number.prototype` and the others); a key before it would make the lazy `prototype` (`props::own_add`) |
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
use crate::conv::{self, num};
use crate::descriptor::Desc;
use crate::frame::{FALSE, NULL, TRUE, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::{is_error, is_object, is_symbol};
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
    /// `%eval%` (§19.2.1). Made, but not a global binding until P3-EVAL-VM (C19).
    Eval,
}

impl IntrinsicId {
    /// Every id, in id order.
    pub const ALL: [IntrinsicId; 39] = [
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
    ];

    /// The spec name (`%Object.prototype%`); `globalThis` for the global object.
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
            IntrinsicId::StringIteratorPrototype
            | IntrinsicId::GeneratorFunction
            | IntrinsicId::GeneratorFunctionPrototype
            | IntrinsicId::GeneratorPrototype
            | IntrinsicId::AsyncFunction
            | IntrinsicId::AsyncFunctionPrototype
            | IntrinsicId::Eval => return None,
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
            _ => None,
        }
    }

    /// Made by `realm::init` (and `symbol::install`), not by a table: tables may target these
    /// but not create them.
    pub const fn is_core(self) -> bool {
        (self as u16) <= IntrinsicId::GlobalObject as u16
    }
}

/// The intrinsic `id` of the realm: an object, or undefined if no table made it (or, for
/// `%Symbol.prototype%`, until symbols exist).
pub(crate) fn intrinsic(rt: &Runtime, id: IntrinsicId) -> u64 {
    match (id.root(), id.slot()) {
        (Some(r), _) => rt.root(r),
        (None, Some(i)) => rt.builtins.intrinsics[i].bits(),
        // %Symbol.prototype% lives in symbol.rs.
        (None, None) => symbol::prototype(rt),
    }
}

fn set_intrinsic(rt: &Runtime, id: IntrinsicId, v: u64) {
    match (id.root(), id.slot()) {
        (Some(r), _) => rt.set_root(r, v),
        // E3: a store into a root slot takes no barrier.
        (None, Some(i)) => rt.builtins.intrinsics[i].init(val(v)),
        (None, None) => unreachable!("{} is not made by a table", id.name()),
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

    /// The same key? (`Atom(Length)` and `Str("length")` are one key.)
    fn same(&self, other: &PropKey) -> bool {
        match (self, other) {
            (PropKey::Symbol(a), PropKey::Symbol(b)) => a == b,
            (PropKey::Symbol(_), _) | (_, PropKey::Symbol(_)) => false,
            _ => self.text() == other.text(),
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
}

impl BuiltinTable {
    pub const fn new(name: &'static str, modules: &'static [BuiltinModule]) -> BuiltinTable {
        BuiltinTable {
            name,
            modules,
            checked: OnceLock::new(),
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
/// intrinsic made once, and a core one only named (by the VM table, as the builtin realm init
/// makes it from); the prototype of a new intrinsic object made before it (walk order); every
/// index in range and of the right kind; every named intrinsic made by realm init or a
/// module; every alias names a data key (not an accessor) defined earlier on the same object;
/// no key twice on one object; the first key of a constructor a module makes is `prototype`, a
/// data property with attributes {} (§20: `Number.prototype` and the others), and every such
/// constructor has one.
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
                    if m.defs[usize::from(i)].kind == BuiltinKind::Constructor {
                        constructors.push((d.id, m.name));
                    }
                }
                IntrinsicValue::Object { .. } => {}
            }
        }
    }
    let prototype = PropKey::Atom(Atom::Prototype);
    // Defined keys per target with their values, in walk order (for aliases and duplicates).
    let mut defined: Vec<(IntrinsicId, PropKey, PropValue)> = Vec::new();
    for m in modules() {
        let at = |what: String| format!("builtin table module `{}`: {what}", m.name);
        for op in m.props {
            if !made.contains(&op.target) {
                return Err(at(format!("target {} is not made", op.target.name())));
            }
            for p in op.props {
                let here =
                    |what: &str| at(format!("{}.{}: {what}", op.target.name(), p.key.text()));
                if defined
                    .iter()
                    .any(|(t, k, _)| *t == op.target && k.same(&p.key))
                {
                    return Err(here("defined twice"));
                }
                // Install adds keys in walk order, and adding a key to a function first makes
                // its lazy `prototype` (`props::own_add`), which the table's own `prototype`
                // would then meet. So a constructor gets `prototype` {} before any other key.
                let is_prototype = p.key.same(&prototype)
                    && p.attrs == PropAttrs::NONE
                    && !matches!(p.value, PropValue::Accessor { .. });
                if !is_prototype
                    && constructors.iter().any(|(c, _)| *c == op.target)
                    && !defined
                        .iter()
                        .any(|(t, k, _)| *t == op.target && k.same(&prototype))
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
                        let of = defined
                            .iter()
                            .find(|(t, d, _)| *t == op.target && d.same(&k));
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
                            Some((_, _, PropValue::Accessor { .. })) => {
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
                defined.push((op.target, p.key, p.value));
            }
        }
    }
    for (c, module) in &constructors {
        if !defined.iter().any(|(t, k, _)| t == c && k.same(&prototype)) {
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
const INTRINSIC_SLOTS: usize = 7;

/// Per-isolate builtin state (`Runtime::builtins`): one entry per installed module, and the
/// intrinsics without a runtime root.
pub(crate) struct BuiltinState {
    /// The embedder's table (`IsolateConfig::builtins`).
    table: Cell<Option<&'static BuiltinTable>>,
    /// The installed modules in walk order (the VM table first).
    modules: RefCell<Vec<ModuleState>>,
    /// The intrinsics of [`IntrinsicId::slot`], undefined until made: a strong root range
    /// that [`prepare`] registers (the heap is the Runtime's first field, so it drops with the
    /// range before this box).
    intrinsics: Box<[Slot]>,
}

impl Default for BuiltinState {
    fn default() -> Self {
        BuiltinState {
            table: Cell::new(None),
            modules: RefCell::new(Vec::new()),
            intrinsics: (0..INTRINSIC_SLOTS)
                .map(|_| Slot::new(Value::UNDEFINED))
                .collect(),
        }
    }
}

struct ModuleState {
    module: &'static BuiltinModule,
    /// One FunctionInfo per definition (dense; frames and function cells point into it).
    infos: Box<[FunctionInfo]>,
    /// The function object of each definition, undefined until made: a strong root range
    /// (the heap is the Runtime's first field, so it drops with the range before this box).
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
/// (VM table first) and the intrinsic slots. No GC allocation. Runs once, before
/// `realm::init`.
pub(crate) fn prepare(rt: &Runtime, embedder: Option<&'static BuiltinTable>) {
    rt.builtins.table.set(embedder);
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
    }
    for module in tables(embedder).flat_map(|t| t.modules.iter()) {
        let infos: Box<[FunctionInfo]> = module
            .defs
            .iter()
            .map(|d| {
                let constructor = d.kind == BuiltinKind::Constructor;
                FunctionInfo::host(d.f, &d.function_name(), d.length, constructor)
            })
            .collect();
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
            infos,
            functions,
        });
    }
}

/// (module position, FunctionInfo, current function bits) of builtin `id`.
fn lookup(rt: &Runtime, id: BuiltinId) -> Option<(usize, *const FunctionInfo, u64)> {
    let mods = rt.builtins.modules.borrow();
    let (m, st) = mods
        .iter()
        .enumerate()
        .find(|(_, s)| s.module.block == id.block())?;
    let i = usize::from(id.index());
    let info = st.infos.get(i)?;
    Some((m, core::ptr::from_ref(info), st.functions[i].bits()))
}

/// The function object of builtin `id` in this realm, made on first use (a GC point then).
/// Panics if no installed module has `id`.
pub(crate) fn function_object(rt: &Runtime, id: BuiltinId) -> u64 {
    let (m, info, f) = lookup(rt, id).unwrap_or_else(|| panic!("no builtin {id}"));
    if f != UNDEFINED {
        return f;
    }
    // GC point; no borrow is held (W4-GC-RULES).
    let f = object::new_function(rt, info, UNDEFINED);
    // E3: a store into a root slot takes no barrier.
    rt.builtins.modules.borrow()[m].functions[usize::from(id.index())].init(val(f));
    f
}

/// The function object of builtin `id` if the realm has made it (no allocation): the "is
/// this still the intrinsic?" test of the iteration fast paths (`interp/slow/iter.rs`).
pub(crate) fn existing_function(rt: &Runtime, id: BuiltinId) -> Option<u64> {
    lookup(rt, id)
        .map(|(_, _, f)| f)
        .filter(|&f| f != UNDEFINED)
}

/// Installs the tables that [`prepare`] registered: (1) the function object of every
/// definition, (2) the intrinsics the modules make, then the [[Prototype]] of
/// %GeneratorFunction% and %AsyncFunction%, (3) the properties. Walk order: VM table,
/// then the embedder's; modules, then entries, in order. Runs after `realm::init`.
pub(crate) fn install(cx: &mut Cx) -> Result<(), String> {
    let rt = cx.rt();
    let embedder = rt.builtins.table();
    let modules: Vec<&'static BuiltinModule> =
        tables(embedder).flat_map(|t| t.modules.iter()).collect();
    for m in &modules {
        for i in 0..m.defs.len() {
            function_object(rt, m.id(i as u16));
        }
    }
    for m in &modules {
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
    // §27.3.2, §27.7.2: the [[Prototype]] of %GeneratorFunction% and %AsyncFunction% is
    // %Function%. A table gives a builtin's function object %Function.prototype% only. GC
    // point (dictionary mode, OM-10): the intrinsics are roots.
    let function = intrinsic(rt, IntrinsicId::Function);
    for id in [IntrinsicId::GeneratorFunction, IntrinsicId::AsyncFunction] {
        let c = intrinsic(rt, id);
        if !is_object(function)
            || !is_object(c)
            || !crate::property::set_prototype_of(rt, c, function)
        {
            return Err(format!("could not set the [[Prototype]] of {}", id.name()));
        }
    }
    for m in &modules {
        for op in m.props {
            let target = intrinsic(rt, op.target);
            if !is_object(target) {
                return Err(format!(
                    "builtin table module `{}`: {} is not installed",
                    m.name,
                    op.target.name()
                ));
            }
            for p in op.props {
                define(cx, m, target, op.target, p)?;
            }
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

/// Defines one table property on `target`. GC points: `target`, the key and the value live in
/// Rust locals (conservative scan) and the intrinsics are roots.
fn define(
    cx: &mut Cx,
    m: &BuiltinModule,
    target: u64,
    id: IntrinsicId,
    p: &PropDef,
) -> Result<(), String> {
    let rt = cx.rt();
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
            let desc = Desc {
                get: Some(get.map_or(UNDEFINED, |i| function_object(rt, m.id(i)))),
                set: Some(set.map_or(UNDEFINED, |i| function_object(rt, m.id(i)))),
                enumerable: Some(p.attrs.enumerable),
                configurable: Some(p.attrs.configurable),
                ..Desc::default()
            };
            return match crate::property::define_own_property(cx, target, key, desc) {
                Ok(true) => Ok(()),
                Ok(false) => Err(at("the accessor was rejected")),
                Err(_) => {
                    // `e` lives in a Rust local while its text is read.
                    let e = rt.take_pending();
                    let text = if is_error(e) {
                        crate::error::error_header(cx, e)
                    } else {
                        conv::describe(e)
                    };
                    Err(at(&text))
                }
            };
        }
    };
    own_define(rt, target, key, v, p.attrs.bits());
    Ok(())
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
        assert_eq!(
            PropKey::Symbol(WellKnownSymbol::ITERATOR).text(),
            "[Symbol.iterator]"
        );
        assert_eq!(PropAttrs::METHOD.text(), "w-c");
        assert_eq!(PropAttrs::METHOD.bits(), attr::HIDDEN);
        assert_eq!(PropAttrs::ALL.bits(), attr::DEFAULT);
    }
}
