//! String builtins (§22.1, Annex B.2.2): module `string`, block 6. Targets `%String%` (made
//! here) and `%String.prototype%` (`IntrinsicId::String`, `IntrinsicId::StringPrototype`).
//!
//! ```text
//! String(v)              ToString(v); a symbol gives SymbolDescriptiveString
//!                        (Cx::descriptive_string) when called, a TypeError with new
//! new String(v)          StringCreate: a String wrapper (Cx::new_wrapper) whose prototype
//!                        comes from new.target; `length` and indices are virtual (R6)
//! String.fromCharCode    ToUint16 of each argument, in order
//! String.fromCodePoint   ToNumber of each argument, RangeError past U+10FFFF  (P4)
//! String.raw             the template's `raw` strings with the substitutions   (P4)
//! %String.prototype%     charCodeAt; toString, valueOf: thisStringValue
//!                        (Cx::this_primitive_value)
//!   string/proto.rs      at, charAt, codePointAt, concat, endsWith, includes, indexOf,
//!                        lastIndexOf, isWellFormed, toWellFormed, localeCompare (v0: code
//!                        unit order), padStart, padEnd, repeat, slice, startsWith,
//!                        substring, trim, trimStart (= trimLeft), trimEnd (= trimRight)  (P4)
//!   string/case.rs       toLowerCase, toUpperCase, toLocaleLowerCase, toLocaleUpperCase:
//!                        Rust std full case mapping, Unicode 17 (H26), lone surrogates kept;
//!                        the toLocale forms are the plain ones in v0                     (P4)
//!   string/pattern.rs    split, replace, replaceAll with string patterns; an object
//!                        pattern with @@split / @@replace is called (RegExp is M1)       (P4)
//!   string/annex_b.rs    substr and the 13 HTML methods (anchor ... sup)               (P4)
//! global                 String {w, c}
//! ```
//!
//! Not here: match, matchAll, search (they need RegExp, M1) and normalize (the normalization
//! tables are M1).
//!
//! Strings are read as UTF-16 code units (the private `units` helper) and built in a
//! `Vec<u16>` (no `Value` in a Rust heap list). A loop whose length is bounded by a string's
//! length, that runs no JS and allocates nothing on the JS heap, is plain native work (as a
//! copy) and does not poll. A loop that allocates a JS string per pass (split, replaceAll) or
//! that can run to 2^53 - 1 (String.raw) calls `Cx::poll_interrupt` once per pass
//! (M2-P2-POLL).
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-string.txt`.

mod annex_b;
mod case;
mod pattern;
mod proto;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, IntrinsicValue, JsResult,
    JsString, JsSymbol, Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey,
};

use crate::array::length_of_array_like;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 6;

/// Definition indices (ids `BLOCK.i`) of the P4 builtins.
mod def {
    pub const RAW: u16 = 5;
    pub const FROM_CODE_POINT: u16 = 6;
    pub const AT: u16 = 7;
    pub const CHAR_AT: u16 = 8;
    pub const CODE_POINT_AT: u16 = 9;
    pub const CONCAT: u16 = 10;
    pub const ENDS_WITH: u16 = 11;
    pub const INCLUDES: u16 = 12;
    pub const INDEX_OF: u16 = 13;
    pub const LAST_INDEX_OF: u16 = 14;
    pub const IS_WELL_FORMED: u16 = 15;
    pub const TO_WELL_FORMED: u16 = 16;
    pub const LOCALE_COMPARE: u16 = 17;
    pub const PAD_START: u16 = 18;
    pub const PAD_END: u16 = 19;
    pub const REPEAT: u16 = 20;
    pub const SLICE: u16 = 21;
    pub const SPLIT: u16 = 22;
    pub const STARTS_WITH: u16 = 23;
    pub const SUBSTRING: u16 = 24;
    pub const TO_LOWER_CASE: u16 = 25;
    pub const TO_UPPER_CASE: u16 = 26;
    pub const TO_LOCALE_LOWER_CASE: u16 = 27;
    pub const TO_LOCALE_UPPER_CASE: u16 = 28;
    pub const TRIM: u16 = 29;
    pub const TRIM_START: u16 = 30;
    pub const TRIM_END: u16 = 31;
    pub const REPLACE: u16 = 32;
    pub const REPLACE_ALL: u16 = 33;
    pub const SUBSTR: u16 = 34;
    pub const ANCHOR: u16 = 35;
    pub const BIG: u16 = 36;
    pub const BLINK: u16 = 37;
    pub const BOLD: u16 = 38;
    pub const FIXED: u16 = 39;
    pub const FONTCOLOR: u16 = 40;
    pub const FONTSIZE: u16 = 41;
    pub const ITALICS: u16 = 42;
    pub const LINK: u16 = 43;
    pub const SMALL: u16 = 44;
    pub const STRIKE: u16 = 45;
    pub const SUB: u16 = 46;
    pub const SUP: u16 = 47;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("String", 1, string), // 0
    BuiltinDef::method("fromCharCode", 1, from_char_code), // 1
    BuiltinDef::method("charCodeAt", 1, char_code_at), // 2
    BuiltinDef::method("toString", 0, to_string), // 3
    BuiltinDef::method("valueOf", 0, value_of),   // 4
    BuiltinDef::method("raw", 1, raw),            // 5
    BuiltinDef::method("fromCodePoint", 1, from_code_point), // 6
    BuiltinDef::method("at", 1, proto::at),       // 7
    BuiltinDef::method("charAt", 1, proto::char_at), // 8
    BuiltinDef::method("codePointAt", 1, proto::code_point_at), // 9
    BuiltinDef::method("concat", 1, proto::concat), // 10
    BuiltinDef::method("endsWith", 1, proto::ends_with), // 11
    BuiltinDef::method("includes", 1, proto::includes), // 12
    BuiltinDef::method("indexOf", 1, proto::index_of), // 13
    BuiltinDef::method("lastIndexOf", 1, proto::last_index_of), // 14
    BuiltinDef::method("isWellFormed", 0, proto::is_well_formed), // 15
    BuiltinDef::method("toWellFormed", 0, proto::to_well_formed), // 16
    BuiltinDef::method("localeCompare", 1, proto::locale_compare), // 17
    BuiltinDef::method("padStart", 1, proto::pad_start), // 18
    BuiltinDef::method("padEnd", 1, proto::pad_end), // 19
    BuiltinDef::method("repeat", 1, proto::repeat), // 20
    BuiltinDef::method("slice", 2, proto::slice), // 21
    BuiltinDef::method("split", 2, pattern::split), // 22
    BuiltinDef::method("startsWith", 1, proto::starts_with), // 23
    BuiltinDef::method("substring", 2, proto::substring), // 24
    BuiltinDef::method("toLowerCase", 0, case::to_lower_case), // 25
    BuiltinDef::method("toUpperCase", 0, case::to_upper_case), // 26
    BuiltinDef::method("toLocaleLowerCase", 0, case::to_locale_lower_case), // 27
    BuiltinDef::method("toLocaleUpperCase", 0, case::to_locale_upper_case), // 28
    BuiltinDef::method("trim", 0, proto::trim),   // 29
    BuiltinDef::method("trimStart", 0, proto::trim_start), // 30
    BuiltinDef::method("trimEnd", 0, proto::trim_end), // 31
    BuiltinDef::method("replace", 2, pattern::replace), // 32
    BuiltinDef::method("replaceAll", 2, pattern::replace_all), // 33
    BuiltinDef::method("substr", 2, annex_b::substr), // 34
    BuiltinDef::method("anchor", 1, annex_b::anchor), // 35
    BuiltinDef::method("big", 0, annex_b::big),   // 36
    BuiltinDef::method("blink", 0, annex_b::blink), // 37
    BuiltinDef::method("bold", 0, annex_b::bold), // 38
    BuiltinDef::method("fixed", 0, annex_b::fixed), // 39
    BuiltinDef::method("fontcolor", 1, annex_b::fontcolor), // 40
    BuiltinDef::method("fontsize", 1, annex_b::fontsize), // 41
    BuiltinDef::method("italics", 0, annex_b::italics), // 42
    BuiltinDef::method("link", 1, annex_b::link), // 43
    BuiltinDef::method("small", 0, annex_b::small), // 44
    BuiltinDef::method("strike", 0, annex_b::strike), // 45
    BuiltinDef::method("sub", 0, annex_b::sub),   // 46
    BuiltinDef::method("sup", 0, annex_b::sup),   // 47
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::String,
    value: IntrinsicValue::Builtin(0),
}];

/// A method of this module under the key `name`.
const fn method(name: &'static str, def: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), def)
}

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::String,
        props: &[
            // §22.1.2.3 String.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::StringPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("fromCharCode"), 1),
            method("fromCodePoint", def::FROM_CODE_POINT),
            method("raw", def::RAW),
        ],
    },
    ObjectProps {
        target: IntrinsicId::StringPrototype,
        props: &[
            // §22.1.3.6 String.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::String,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Str("charCodeAt"), 2),
            PropDef::method(PropKey::Atom(Atom::ToStringKey), 3),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 4),
            // §22.1.3, in spec order.
            method("at", def::AT),
            method("charAt", def::CHAR_AT),
            method("codePointAt", def::CODE_POINT_AT),
            method("concat", def::CONCAT),
            method("endsWith", def::ENDS_WITH),
            method("includes", def::INCLUDES),
            method("indexOf", def::INDEX_OF),
            method("isWellFormed", def::IS_WELL_FORMED),
            method("lastIndexOf", def::LAST_INDEX_OF),
            method("localeCompare", def::LOCALE_COMPARE),
            method("padEnd", def::PAD_END),
            method("padStart", def::PAD_START),
            method("repeat", def::REPEAT),
            method("replace", def::REPLACE),
            method("replaceAll", def::REPLACE_ALL),
            method("slice", def::SLICE),
            method("split", def::SPLIT),
            method("startsWith", def::STARTS_WITH),
            method("substring", def::SUBSTRING),
            method("toLocaleLowerCase", def::TO_LOCALE_LOWER_CASE),
            method("toLocaleUpperCase", def::TO_LOCALE_UPPER_CASE),
            method("toLowerCase", def::TO_LOWER_CASE),
            method("toUpperCase", def::TO_UPPER_CASE),
            method("toWellFormed", def::TO_WELL_FORMED),
            method("trim", def::TRIM),
            method("trimEnd", def::TRIM_END),
            method("trimStart", def::TRIM_START),
            // Annex B.2.2: substr, the HTML methods, and trimLeft / trimRight, which are the
            // same function objects as trimStart / trimEnd (B.2.2.15, B.2.2.16).
            method("substr", def::SUBSTR),
            method("anchor", def::ANCHOR),
            method("big", def::BIG),
            method("blink", def::BLINK),
            method("bold", def::BOLD),
            method("fixed", def::FIXED),
            method("fontcolor", def::FONTCOLOR),
            method("fontsize", def::FONTSIZE),
            method("italics", def::ITALICS),
            method("link", def::LINK),
            method("small", def::SMALL),
            method("strike", def::STRIKE),
            method("sub", def::SUB),
            method("sup", def::SUP),
            PropDef::alias(
                PropKey::Str("trimLeft"),
                PropKey::Str("trimStart"),
                PropAttrs::METHOD,
            ),
            PropDef::alias(
                PropKey::Str("trimRight"),
                PropKey::Str("trimEnd"),
                PropAttrs::METHOD,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 String ( . . . ).
        props: &[PropDef::intrinsic(
            PropKey::Str("String"),
            IntrinsicId::String,
            PropAttrs::METHOD,
        )],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "string",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- helpers shared by the submodules ------------------------------------------------------

/// The longest string the VM makes, in code units: `MAX_STRING_LENGTH` of
/// crates/ep9_vm/src/string.rs, which `Cx::new_string_utf16` enforces. The builders here check
/// it before they grow a `Vec<u16>` past it, so a huge `repeat` or `padEnd` throws the same
/// RangeError without allocating first.
const MAX_LENGTH: usize = (1 << 29) - 24;

/// RequireObjectCoercible(this value), then ToString: steps 1-2 of most String.prototype
/// methods. `name` is the method name for the TypeError text (V8's text).
fn this_string<'s>(cx: &mut Cx<'s>, this: Local<'s>, name: &str) -> JsResult<Local<'s, JsString>> {
    if this.is_undefined_or_null() {
        let msg = format!("String.prototype.{name} called on null or undefined");
        return Err(cx.throw_type_error(&msg));
    }
    cx.to_string(this)
}

/// The code units of `s`, copied out (a view cannot live across a GC point, R3).
fn units<'s>(cx: &mut Cx<'s>, s: Local<'s, JsString>) -> JsResult<Vec<u16>> {
    let len = cx.string_len(s);
    units_range(cx, s, 0, len)
}

/// The code units `from..to` of `s` (`from <= to <= length`), copied out: the substring of S
/// from `from` to `to`.
fn units_range<'s>(
    cx: &mut Cx<'s>,
    s: Local<'s, JsString>,
    from: usize,
    to: usize,
) -> JsResult<Vec<u16>> {
    debug_assert!(from <= to && to <= cx.string_len(s));
    let flat = cx.flatten(s)?;
    Ok((from..to)
        .map(|i| cx.code_unit_at(flat, i).expect("i < length"))
        .collect())
}

/// ToString(v) as code units.
fn to_units<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Vec<u16>> {
    let s = cx.to_string(v)?;
    units(cx, s)
}

/// A new string from code units, as a value.
fn new_str<'s>(cx: &mut Cx<'s>, units: &[u16]) -> JsResult<Local<'s>> {
    Ok(cx.new_string_utf16(units)?.value())
}

/// Appends `more` to `out`; RangeError if the result would pass [`MAX_LENGTH`].
fn push_units<'s>(cx: &mut Cx<'s>, out: &mut Vec<u16>, more: &[u16]) -> JsResult<()> {
    if out.len() + more.len() > MAX_LENGTH {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    out.extend_from_slice(more);
    Ok(())
}

/// The index for a relative position already through ToIntegerOrInfinity (an integer or
/// ±∞), clamped between 0 and `len`.
fn clamp(n: f64, len: usize) -> usize {
    if n <= 0.0 {
        0
    } else if n >= len as f64 {
        len
    } else {
        n as usize
    }
}

/// StringIndexOf ( string, searchValue, fromIndex ) (§6.1.4.1), on code units. `None` for
/// "not found" (the spec's -1).
fn string_index_of(string: &[u16], search: &[u16], from: usize) -> Option<usize> {
    // 1. Let len be the length of string.
    let len = string.len();
    // 2. If searchValue is the empty String and fromIndex ≤ len, return fromIndex.
    if search.is_empty() {
        return (from <= len).then_some(from);
    }
    // 3. Let searchLen be the length of searchValue.
    let search_len = search.len();
    // 4. For each integer i such that fromIndex ≤ i ≤ len - searchLen, in ascending order, do
    //    a. Let candidate be the substring of string from i to i + searchLen.
    //    b. If candidate is searchValue, return i.
    // 5. Return not-found.
    if from >= len || search_len > len - from {
        return None;
    }
    let first = search[0];
    (from..=len - search_len).find(|&i| string[i] == first && string[i..i + search_len] == *search)
}

/// The well-known symbol `Symbol.<name>`, read from `%Symbol%` (its own property is
/// non-writable and non-configurable, §20.4.2, so this is always the symbol).
fn well_known<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s, JsSymbol>> {
    let ctor = cx.intrinsic(IntrinsicId::Symbol);
    let v = cx.get(ctor, name)?;
    match v.as_symbol() {
        Some(sym) => Ok(sym),
        None => {
            let msg = format!("Symbol.{name} is not a symbol");
            Err(cx.throw_type_error(&msg))
        }
    }
}

// ---- %String% ------------------------------------------------------------------------------

/// String ( value ) (§22.1.1.1).
fn string<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If value is not present, then
    //    a. Let s be the empty String.
    let s = if args.is_empty() {
        cx.key("").value()
    } else {
        // 2. Else,
        let value = args.get(0);
        //    a. If NewTarget is undefined and value is a Symbol, return
        //       SymbolDescriptiveString(value).
        if new_target.is_undefined()
            && let Some(sym) = value.as_symbol()
        {
            return Ok(cx.descriptive_string(sym)?.value());
        }
        //    b. Let s be ? ToString(value).
        cx.to_string(value)?.value()
    };
    // 3. If NewTarget is undefined, return s.
    let Some(nt) = new_target.as_object() else {
        return Ok(s);
    };
    // 4. Return StringCreate(s, ? GetPrototypeFromConstructor(NewTarget,
    //    "%String.prototype%")).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::StringPrototype)?;
    Ok(cx.new_wrapper(proto, s)?.value())
}

/// String.fromCharCode ( ...codeUnits ) (§22.1.2.1).
fn from_char_code<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let result be the empty String.
    // (code units are plain u16: no list of Locals is needed)
    let mut result = Vec::with_capacity(args.len());
    // 2. For each element next of codeUnits, do
    for next in args.iter() {
        //    a. Let nextCU be the code unit whose numeric value is ℝ(? ToUint16(next)).
        //       (ToUint16 is ToUint32 modulo 2^16: 2^16 divides 2^32.)
        let next_cu = cx.to_uint32(next)? as u16;
        //    b. Set result to the string-concatenation of result and nextCU.
        result.push(next_cu);
    }
    // 3. Return result.
    Ok(cx.new_string_utf16(&result)?.value())
}

/// String.fromCodePoint ( ...codePoints ) (§22.1.2.2).
fn from_code_point<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let result be the empty String.
    let mut result = Vec::with_capacity(args.len());
    // 2. For each element next of codePoints, do
    for next in args.iter() {
        //    a. Let nextCP be ? ToNumber(next).
        let next_cp = cx.to_number(next)?;
        //    b. If nextCP is not an integral Number, throw a RangeError exception.
        //    c. If ℝ(nextCP) < 0 or ℝ(nextCP) > 0x10FFFF, throw a RangeError exception.
        if !(next_cp.is_finite()
            && next_cp.trunc() == next_cp
            && (0.0..=1_114_111.0).contains(&next_cp))
        {
            let text = cx.to_rust_string(Local::number(next_cp))?;
            return Err(cx.throw_range_error(&format!("Invalid code point {text}")));
        }
        //    d. Set result to the string-concatenation of result and
        //       UTF16EncodeCodePoint(ℝ(nextCP)).
        let cp = next_cp as u32;
        match char::from_u32(cp) {
            Some(c) => {
                let mut buf = [0u16; 2];
                result.extend_from_slice(c.encode_utf16(&mut buf));
            }
            // A surrogate code point is one code unit.
            None => result.push(cp as u16),
        }
    }
    // 3. Assert: If codePoints is empty, then result is the empty String.
    // 4. Return result.
    new_str(cx, &result)
}

/// String.raw ( template, ...substitutions ) (§22.1.2.4).
fn raw<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let substitutions = args.rest(1);
    // 1. Let substitutionCount be the number of elements in substitutions.
    let substitution_count = substitutions.len() as u64;
    // 2. Let cooked be ? ToObject(template).
    let cooked = cx.to_object(args.get(0))?;
    // 3. Let literals be ? ToObject(? Get(cooked, "raw")).
    let raw = cx.get(cooked, "raw")?;
    let literals = cx.to_object(raw)?;
    // 4. Let literalCount be ? LengthOfArrayLike(literals).
    let literal_count = length_of_array_like(cx, literals)?;
    // 5. If literalCount ≤ 0, return the empty String.
    if literal_count == 0 {
        return Ok(cx.key("").value());
    }
    // 6. Let R be the empty String.
    let mut r = Vec::new();
    // 7. Let nextIndex be 0.
    let mut next_index = 0u64;
    // 8. Repeat,
    loop {
        // (no back edge: a walk to 2^53 - 1 must stop on termination)
        cx.poll_interrupt()?;
        //    a. Let nextLiteralVal be ? Get(literals, ! ToString(𝔽(nextIndex))).
        let next_literal_val = cx.get_index(literals, next_index)?;
        //    b. Let nextLiteral be ? ToString(nextLiteralVal).
        let next_literal = to_units(cx, next_literal_val)?;
        //    c. Set R to the string-concatenation of R and nextLiteral.
        push_units(cx, &mut r, &next_literal)?;
        //    d. If nextIndex + 1 = literalCount, return R.
        if next_index + 1 == literal_count {
            return new_str(cx, &r);
        }
        //    e. If nextIndex < substitutionCount, then
        if next_index < substitution_count {
            //   i. Let nextSubVal be substitutions[nextIndex].
            let next_sub_val = substitutions.get(next_index as usize);
            //   ii. Let nextSub be ? ToString(nextSubVal).
            let next_sub = to_units(cx, next_sub_val)?;
            //   iii. Set R to the string-concatenation of R and nextSub.
            push_units(cx, &mut r, &next_sub)?;
        }
        //    f. Set nextIndex to nextIndex + 1.
        next_index += 1;
    }
}

// ---- %String.prototype% (week 4) -----------------------------------------------------------

/// String.prototype.charCodeAt ( pos ) (§22.1.3.3).
fn char_code_at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    if this.is_undefined_or_null() {
        return Err(cx.throw_type_error("String.prototype.charCodeAt called on null or undefined"));
    }
    // 2. Let S be ? ToString(O).
    let s = cx.to_string(this)?;
    // 3. Let position be ? ToIntegerOrInfinity(pos).
    let position = cx.to_integer_or_infinity(args.get(0))?;
    // 4. Let size be the length of S.
    let size = cx.string_len(s);
    // 5. If position < 0 or position ≥ size, return NaN.
    if position < 0.0 || position >= size as f64 {
        return Ok(Local::number(f64::NAN));
    }
    // 6. Return the Number value for the numeric value of the code unit at index position
    //    within the String S.
    let s = cx.flatten(s)?;
    let unit = cx
        .code_unit_at(s, position as usize)
        .expect("position < size");
    Ok(Local::int32(i32::from(unit)))
}

/// String.prototype.toString ( ) (§22.1.3.29).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisStringValue(this value).
    cx.this_primitive_value(this, PrimitiveType::String)
}

/// String.prototype.valueOf ( ) (§22.1.3.35).
fn value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisStringValue(this value).
    cx.this_primitive_value(this, PrimitiveType::String)
}
