//! RegExp (§22.2): module `regexp`, block 16. Owners: M4-P2 RX0A (the `%RegExp.prototype%`
//! intrinsic, an ordinary object first; V9-A9), then M4-P3 REGEXP1 (the constructor, the
//! prototype methods and accessors, and `String.prototype.match` / `search`). It reaches
//! `ep9_regexp` only through `Cx` part-2 methods (V9-D15).
//!
//! ```text
//! RegExp(p, f), new RegExp(p, f)   §22.2.4.1: IsRegExp, then Cx::regexp_create (compiles now)
//! %RegExp.prototype%
//!   regexp/proto.rs    exec, test, toString; the accessors flags, source, global ... sticky
//!   regexp/exec.rs     RegExpBuiltinExec (lastIndex steps + Cx::regexp_exec), RegExpExec,
//!                      the match result array, AdvanceStringIndex
//!   regexp/symbols.rs  [Symbol.match], [Symbol.replace], [Symbol.search], [Symbol.split]
//! String.prototype.match, .search  (string/pattern.rs; ids in this block)
//!                      -> RegExpCreate, then Invoke(rx, @@match / @@search)
//! String.prototype.replace, .split (string module) -> @@replace / @@split of a RegExp
//! ```
//!
//! | Intrinsic | State |
//! |---|---|
//! | `%RegExp%` | Made (M4-P3): the constructor, `prototype`, `[Symbol.species]` |
//! | `%RegExp.prototype%` | An ordinary object whose prototype is `%Object.prototype%` (§22.2.6; not a RegExp instance; M4-P2 RX0A) with the methods and accessors above (M4-P3). RegExp literals take it as their prototype (`ep9_vm` regexp.rs) |
//! | `%RegExpStringIteratorPrototype%`, `[Symbol.matchAll]`, `String.prototype.matchAll`, `replaceAll` with a RegExp, `compile`, the legacy static properties (`RegExp.$1`) | Not made (month 5, REGEXP2) |
//!
//! Fast paths (MC-w14-18, the H31 fuses they rely on): while `exec` (looked up on the
//! RegExp, as RegExpExec does) is the builtin `exec` of the current realm (`Cx::is_builtin`),
//! RegExpExec runs RegExpBuiltinExec itself and keeps the match as a `MatchRecord` (Rust
//! memory) instead of a result array that no JS can see; `[Symbol.split]` with
//! `%RegExp%` as the species constructor and that `exec` searches with one non-sticky program
//! instead of one sticky exec per position (RX0 has no single-position attempt). Each test
//! runs per call; a fuse registry comes in month 5.
//!
//! The internal slots of a RegExp object are the `ObjectClass::RegExp` slots of `ep9_vm`
//! (abi/slots.rs: `[[OriginalSource]]` 0, `[[OriginalFlags]]` 1, `[[RegExpRecord]]` 2 = the
//! flag bits, `[[RegExpMatcher]]` 3); this module reads 0-2 (`exec::slot`) and never
//! writes them.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-regexp.txt`.

mod annex_b;
mod exec;
mod iter;
mod proto;
mod symbols;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, JsString, Local, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

pub(crate) use self::exec::{is_regexp_object, slot};
use crate::string::pattern;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 16;

/// Definition indices (ids `BLOCK.i`).
pub(crate) mod def {
    pub const REGEXP: u16 = 0;
    pub const EXEC: u16 = 1;
    pub const TEST: u16 = 2;
    pub const TO_STRING: u16 = 3;
    pub const FLAGS: u16 = 4;
    pub const GLOBAL: u16 = 5;
    pub const IGNORE_CASE: u16 = 6;
    pub const MULTILINE: u16 = 7;
    pub const DOT_ALL: u16 = 8;
    pub const UNICODE: u16 = 9;
    pub const UNICODE_SETS: u16 = 10;
    pub const STICKY: u16 = 11;
    pub const HAS_INDICES: u16 = 12;
    pub const SOURCE: u16 = 13;
    pub const MATCH: u16 = 14;
    pub const REPLACE: u16 = 15;
    pub const SEARCH: u16 = 16;
    pub const SPLIT: u16 = 17;
    pub const SPECIES: u16 = 18;
    pub const STRING_MATCH: u16 = 19;
    pub const STRING_SEARCH: u16 = 20;
    // M5-P2 REGEXP2
    pub const MATCH_ALL: u16 = 21;
    pub const ITERATOR_NEXT: u16 = 22;
    pub const STRING_MATCH_ALL: u16 = 23;
    pub const ESCAPE: u16 = 24;
    pub const COMPILE: u16 = 25;
    pub const GET_INPUT: u16 = 26;
    pub const SET_INPUT: u16 = 27;
    pub const GET_LAST_MATCH: u16 = 28;
    pub const GET_LAST_PAREN: u16 = 29;
    pub const GET_LEFT_CONTEXT: u16 = 30;
    pub const GET_RIGHT_CONTEXT: u16 = 31;
    /// `get RegExp.$1` ... `get RegExp.$9` are 32 ... 40.
    pub const GET_PAREN_1: u16 = 32;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("RegExp", 2, regexp),        // 0
    BuiltinDef::method("exec", 1, proto::exec),          // 1
    BuiltinDef::method("test", 1, proto::test),          // 2
    BuiltinDef::method("toString", 0, proto::to_string), // 3
    BuiltinDef::getter("flags", proto::flags),           // 4
    BuiltinDef::getter("global", proto::global),         // 5
    BuiltinDef::getter("ignoreCase", proto::ignore_case), // 6
    BuiltinDef::getter("multiline", proto::multiline),   // 7
    BuiltinDef::getter("dotAll", proto::dot_all),        // 8
    BuiltinDef::getter("unicode", proto::unicode),       // 9
    BuiltinDef::getter("unicodeSets", proto::unicode_sets), // 10
    BuiltinDef::getter("sticky", proto::sticky),         // 11
    BuiltinDef::getter("hasIndices", proto::has_indices), // 12
    BuiltinDef::getter("source", proto::source),         // 13
    BuiltinDef::method("[Symbol.match]", 1, symbols::match_), // 14
    BuiltinDef::method("[Symbol.replace]", 2, symbols::replace), // 15
    BuiltinDef::method("[Symbol.search]", 1, symbols::search), // 16
    BuiltinDef::method("[Symbol.split]", 2, symbols::split), // 17
    BuiltinDef::getter("[Symbol.species]", species),     // 18
    BuiltinDef::method("match", 1, pattern::match_),     // 19
    BuiltinDef::method("search", 1, pattern::search),    // 20
    BuiltinDef::method("[Symbol.matchAll]", 1, iter::match_all), // 21
    BuiltinDef::method("next", 0, iter::next),           // 22
    BuiltinDef::method("matchAll", 1, pattern::match_all), // 23
    BuiltinDef::method("escape", 1, proto::escape),      // 24
    BuiltinDef::method("compile", 2, annex_b::compile),  // 25
    BuiltinDef::getter("input", annex_b::get_input),     // 26
    BuiltinDef::setter("input", annex_b::set_input),     // 27
    BuiltinDef::getter("lastMatch", annex_b::get_last_match), // 28
    BuiltinDef::getter("lastParen", annex_b::get_last_paren), // 29
    BuiltinDef::getter("leftContext", annex_b::get_left_context), // 30
    BuiltinDef::getter("rightContext", annex_b::get_right_context), // 31
    BuiltinDef::getter("$1", annex_b::get_paren_1),      // 32
    BuiltinDef::getter("$2", annex_b::get_paren_2),      // 33
    BuiltinDef::getter("$3", annex_b::get_paren_3),      // 34
    BuiltinDef::getter("$4", annex_b::get_paren_4),      // 35
    BuiltinDef::getter("$5", annex_b::get_paren_5),      // 36
    BuiltinDef::getter("$6", annex_b::get_paren_6),      // 37
    BuiltinDef::getter("$7", annex_b::get_paren_7),      // 38
    BuiltinDef::getter("$8", annex_b::get_paren_8),      // 39
    BuiltinDef::getter("$9", annex_b::get_paren_9),      // 40
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    // §22.2.6: %RegExp.prototype% is an ordinary object (not a RegExp instance).
    IntrinsicDef {
        id: IntrinsicId::RegExpPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    // §22.2.4: %RegExp%.
    IntrinsicDef {
        id: IntrinsicId::RegExp,
        value: IntrinsicValue::Builtin(def::REGEXP),
    },
    // §22.2.9.2: %RegExpStringIteratorPrototype% (prototype %Iterator.prototype%).
    IntrinsicDef {
        id: IntrinsicId::RegExpStringIteratorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
];

/// A method of this module under the key `name`.
const fn method(name: &'static str, def: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), def)
}

/// A method of this module under a well-known symbol.
const fn symbol_method(sym: WellKnownSymbol, def: u16) -> PropDef {
    PropDef::method(PropKey::Symbol(sym), def)
}

/// An accessor {c} of this module with a getter only.
const fn getter(name: &'static str, def: u16) -> PropDef {
    PropDef::accessor(PropKey::Str(name), Some(def), None)
}

/// A legacy static accessor of `%RegExp%` (Annex B legacy features: {c}, not enumerable)
/// under `name`, getter `get` and setter `set`.
const fn legacy(name: &'static str, get: u16, set: Option<u16>) -> PropDef {
    PropDef::accessor(PropKey::Str(name), Some(get), set)
}

/// The properties this module adds, per object (the order of node v24's own keys).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::RegExp,
        props: &[
            // §22.2.5.1 RegExp.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::RegExpPrototype,
                PropAttrs::NONE,
            ),
            // Annex B legacy features: the static accessors (node v24's order).
            legacy("input", def::GET_INPUT, Some(def::SET_INPUT)),
            legacy("$_", def::GET_INPUT, Some(def::SET_INPUT)),
            legacy("lastMatch", def::GET_LAST_MATCH, None),
            legacy("$&", def::GET_LAST_MATCH, None),
            legacy("lastParen", def::GET_LAST_PAREN, None),
            legacy("$+", def::GET_LAST_PAREN, None),
            legacy("leftContext", def::GET_LEFT_CONTEXT, None),
            legacy("$`", def::GET_LEFT_CONTEXT, None),
            legacy("rightContext", def::GET_RIGHT_CONTEXT, None),
            legacy("$'", def::GET_RIGHT_CONTEXT, None),
            legacy("$1", def::GET_PAREN_1, None),
            legacy("$2", def::GET_PAREN_1 + 1, None),
            legacy("$3", def::GET_PAREN_1 + 2, None),
            legacy("$4", def::GET_PAREN_1 + 3, None),
            legacy("$5", def::GET_PAREN_1 + 4, None),
            legacy("$6", def::GET_PAREN_1 + 5, None),
            legacy("$7", def::GET_PAREN_1 + 6, None),
            legacy("$8", def::GET_PAREN_1 + 7, None),
            legacy("$9", def::GET_PAREN_1 + 8, None),
            // §22.2.5.1 RegExp.escape ( S ) (ES2025).
            method("escape", def::ESCAPE),
            // §22.2.5.2 get RegExp [ %Symbol.species% ] ({c}).
            PropDef::accessor(
                PropKey::Symbol(WellKnownSymbol::SPECIES),
                Some(def::SPECIES),
                None,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::RegExpPrototype,
        props: &[
            // §22.2.6.1 RegExp.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::RegExp,
                PropAttrs::METHOD,
            ),
            method("exec", def::EXEC),
            getter("dotAll", def::DOT_ALL),
            getter("flags", def::FLAGS),
            getter("global", def::GLOBAL),
            getter("hasIndices", def::HAS_INDICES),
            getter("ignoreCase", def::IGNORE_CASE),
            getter("multiline", def::MULTILINE),
            getter("source", def::SOURCE),
            getter("sticky", def::STICKY),
            getter("unicode", def::UNICODE),
            getter("unicodeSets", def::UNICODE_SETS),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), def::TO_STRING),
            method("test", def::TEST),
            // B.2.4.1 RegExp.prototype.compile ( pattern, flags ).
            method("compile", def::COMPILE),
            symbol_method(WellKnownSymbol::MATCH, def::MATCH),
            symbol_method(WellKnownSymbol::MATCH_ALL, def::MATCH_ALL),
            symbol_method(WellKnownSymbol::REPLACE, def::REPLACE),
            symbol_method(WellKnownSymbol::SEARCH, def::SEARCH),
            symbol_method(WellKnownSymbol::SPLIT, def::SPLIT),
        ],
    },
    ObjectProps {
        // §22.1.3.13 String.prototype.match, §22.1.3.22 String.prototype.search (they need
        // RegExpCreate, so they live in this block; the string module keeps its ids).
        target: IntrinsicId::StringPrototype,
        props: &[
            method("match", def::STRING_MATCH),
            method("search", def::STRING_SEARCH),
            // §22.1.3.14 String.prototype.matchAll (M5-P2).
            method("matchAll", def::STRING_MATCH_ALL),
        ],
    },
    ObjectProps {
        // §22.2.9.2 %RegExpStringIteratorPrototype%: next, [%Symbol.toStringTag%].
        target: IntrinsicId::RegExpStringIteratorPrototype,
        props: &[
            method("next", def::ITERATOR_NEXT),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("RegExp String Iterator"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 RegExp ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("RegExp"),
            IntrinsicId::RegExp,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "regexp",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// IsRegExp ( argument ) (§7.2.8).
pub(crate) fn is_reg_exp<'s>(cx: &mut Cx<'s>, argument: Local<'s>) -> JsResult<bool> {
    // 1. If argument is not an Object, return false.
    let Some(o) = argument.as_object() else {
        return Ok(false);
    };
    // 2. Let matcher be ? Get(argument, %Symbol.match%).
    let sym = crate::string::well_known(cx, "match")?;
    let matcher = cx.get(o, sym)?;
    // 3. If matcher is not undefined, return ToBoolean(matcher).
    if !matcher.is_undefined() {
        return Ok(cx.to_boolean(matcher));
    }
    // 4. If argument has a [[RegExpMatcher]] internal slot, return true.
    // 5. Return false.
    Ok(is_regexp_object(cx, o))
}

/// RegExpCreate ( P, F ) (§22.2.3.2) with F undefined or a string: RegExpAlloc(%RegExp%),
/// then RegExpInitialize. Used by String.prototype.match and search.
pub(crate) fn regexp_create<'s>(
    cx: &mut Cx<'s>,
    p: Local<'s>,
    f: &str,
) -> JsResult<Local<'s, ep9_vm::JsObject>> {
    // 1. Let obj be ! RegExpAlloc(%RegExp%). (No JS runs: %RegExp%.prototype is a data
    //    property that cannot change, so the allocation moves after the ToString.)
    let proto = cx.intrinsic(IntrinsicId::RegExpPrototype);
    // 2. Return ? RegExpInitialize(obj, P, F).
    let p = initialize_text(cx, p)?;
    let f = cx.new_string(f)?;
    cx.regexp_create(proto, p, f)
}

/// Steps 1-4 of RegExpInitialize (§22.2.3.3) for one of P and F: the empty String for
/// undefined, else ? ToString.
fn initialize_text<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Local<'s, JsString>> {
    if v.is_undefined() {
        cx.new_string("")
    } else {
        cx.to_string(v)
    }
}

/// RegExp ( pattern, flags ) (§22.2.4.1).
fn regexp<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (pattern, flags) = (args.get(0), args.get(1));
    // 1. Let patternIsRegExp be ? IsRegExp(pattern).
    let pattern_is_regexp = is_reg_exp(cx, pattern)?;
    // 2. If NewTarget is undefined, then
    let new_target = if new_target.is_undefined() {
        //    a. Let newTarget be the active function object.
        let active = args.callee().value();
        //    b. If patternIsRegExp is true and flags is undefined, then
        if pattern_is_regexp && flags.is_undefined() {
            //   i. Let patternConstructor be ? Get(pattern, "constructor").
            let o = pattern
                .as_object()
                .expect("IsRegExp is true only for an Object");
            let pattern_constructor = cx.get(o, "constructor")?;
            //   ii. If SameValue(newTarget, patternConstructor) is true, return pattern.
            if cx.same_value(active, pattern_constructor) {
                return Ok(pattern);
            }
        }
        active
    } else {
        // 3. Else, let newTarget be NewTarget.
        new_target
    };
    // 4. If pattern is an Object and pattern has a [[RegExpMatcher]] internal slot, then
    let (p, f) = if let Some(o) = pattern.as_object()
        && is_regexp_object(cx, o)
    {
        //    a. Let P be pattern.[[OriginalSource]].
        let p = cx.slot(o, ep9_vm::ObjectClass::RegExp, slot::SOURCE)?;
        //    b. If flags is undefined, let F be pattern.[[OriginalFlags]].
        //    c. Else, let F be flags.
        let f = if flags.is_undefined() {
            cx.slot(o, ep9_vm::ObjectClass::RegExp, slot::FLAGS)?
        } else {
            flags
        };
        (p, f)
    } else if pattern_is_regexp {
        // 5. Else if patternIsRegExp is true, then
        let o = pattern
            .as_object()
            .expect("IsRegExp is true only for an Object");
        //    a. Let P be ? Get(pattern, "source").
        let p = cx.get(o, "source")?;
        //    b. If flags is undefined, then
        //       i. Let F be ? Get(pattern, "flags").
        //    c. Else,
        //       i. Let F be flags.
        let f = if flags.is_undefined() {
            cx.get(o, "flags")?
        } else {
            flags
        };
        (p, f)
    } else {
        // 6. Else,
        //    a. Let P be pattern.
        //    b. Let F be flags.
        (pattern, flags)
    };
    // 7. Let O be ? RegExpAlloc(newTarget). (GetPrototypeFromConstructor is its only step
    //    that can run JS; `Cx::regexp_create` does the rest of it.)
    let nt = new_target
        .as_object()
        .expect("NewTarget is a constructor, an Object");
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::RegExpPrototype)?;
    // 8. Return ? RegExpInitialize(O, P, F).
    let p = initialize_text(cx, p)?;
    let f = initialize_text(cx, f)?;
    let o = cx.regexp_create(proto, p, f)?;
    // (RegExpAlloc, Annex B legacy features: [[LegacyFeaturesEnabled]] is true only when
    // newTarget is the current realm's %RegExp%.)
    let this_realm_regexp = cx.intrinsic(IntrinsicId::RegExp);
    if !cx.same_value(new_target, this_realm_regexp) {
        cx.regexp_disable_legacy_features(o);
    }
    Ok(o.value())
}

/// get RegExp [ %Symbol.species% ] (§22.2.5.2).
fn species<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return the this value.
    Ok(this)
}
