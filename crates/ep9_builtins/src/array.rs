//! Array builtins (§23.1): module `array`, block 3. Targets `%Array%` and `%Array.prototype%`
//! (`IntrinsicId::Array`, `IntrinsicId::ArrayPrototype`), and `%Object%` for the two statics
//! that read an iterable into groups or entries (`Object.fromEntries`, `Object.groupBy`).
//!
//! The `Array` constructor and `Array.prototype.join`/`toString`/`push`/`values`/`keys`/
//! `entries` are VM builtins (block 0, `natives.rs`).
//! Week 4 (P3-BLTA): `Array.isArray`, `Array.prototype.pop`, `indexOf`. M2-P4 (B-ARRAY): the
//! rest of ES2025 §23.1 except `@@unscopables` (it needs an object-valued table entry).
//!
//! | Builtin | Template | Pattern |
//! |---|---|---|
//! | `Array.isArray` | 1 | a pure static predicate |
//! | `pop`, `indexOf` and the other generic methods | 6 | generic over array-likes: every element access goes through the index ObjectOps (`get_index`, `has_index`, `set_index`, `delete_index`), which need no key for an element or a missing index; each loop polls once per pass |
//! | `Array.from`, `Object.fromEntries`, `Object.groupBy` | 7 | an iterable (`get_iterator`, `iterator_step_value`, `iterator_close`) |
//! | `map`, `filter`, `slice`, `splice`, `concat`, `flat`, `flatMap` | 6 | ArraySpeciesCreate (`array_species_create`): `@@species` read through `%Symbol%` |
//! | `sort`, `toSorted` | 6 | SortIndexedProperties: the values in a `ScratchList`, a stable merge sort of indices (`merge_sort`) |
//! | `get Array[@@species]` | 5 | a getter with a symbol key |
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-array.txt`.

use core::cmp::Ordering;

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsKey, JsObject, JsResult,
    JsString, JsSymbol, Local, ObjectProps, PropDef, PropKey, ScratchList, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 3;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const IS_ARRAY: u16 = 0;
    pub const POP: u16 = 1;
    pub const INDEX_OF: u16 = 2;
    // ---- M2-P4 (B-ARRAY) ----
    pub const FROM: u16 = 3;
    pub const OF: u16 = 4;
    pub const SPECIES: u16 = 5;
    pub const AT: u16 = 6;
    pub const CONCAT: u16 = 7;
    pub const COPY_WITHIN: u16 = 8;
    pub const EVERY: u16 = 9;
    pub const FILL: u16 = 10;
    pub const FILTER: u16 = 11;
    pub const FIND: u16 = 12;
    pub const FIND_INDEX: u16 = 13;
    pub const FIND_LAST: u16 = 14;
    pub const FIND_LAST_INDEX: u16 = 15;
    pub const FLAT: u16 = 16;
    pub const FLAT_MAP: u16 = 17;
    pub const FOR_EACH: u16 = 18;
    pub const INCLUDES: u16 = 19;
    pub const LAST_INDEX_OF: u16 = 20;
    pub const MAP: u16 = 21;
    pub const REDUCE: u16 = 22;
    pub const REDUCE_RIGHT: u16 = 23;
    pub const REVERSE: u16 = 24;
    pub const SHIFT: u16 = 25;
    pub const SLICE: u16 = 26;
    pub const SOME: u16 = 27;
    pub const SORT: u16 = 28;
    pub const SPLICE: u16 = 29;
    pub const TO_LOCALE_STRING: u16 = 30;
    pub const TO_REVERSED: u16 = 31;
    pub const TO_SORTED: u16 = 32;
    pub const TO_SPLICED: u16 = 33;
    pub const UNSHIFT: u16 = 34;
    pub const WITH: u16 = 35;
    pub const FROM_ENTRIES: u16 = 36;
    pub const GROUP_BY: u16 = 37;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("isArray", 1, is_array), // 3.0
    BuiltinDef::method("pop", 0, pop),          // 3.1
    BuiltinDef::method("indexOf", 1, index_of), // 3.2
    // ---- M2-P4 (B-ARRAY) ----
    BuiltinDef::method("from", 1, from),              // 3.3
    BuiltinDef::method("of", 0, of),                  // 3.4
    BuiltinDef::getter("[Symbol.species]", species),  // 3.5
    BuiltinDef::method("at", 1, at),                  // 3.6
    BuiltinDef::method("concat", 1, concat),          // 3.7
    BuiltinDef::method("copyWithin", 2, copy_within), // 3.8
    BuiltinDef::method("every", 1, every),            // 3.9
    BuiltinDef::method("fill", 1, fill),              // 3.10
    BuiltinDef::method("filter", 1, filter),          // 3.11
    BuiltinDef::method("find", 1, find),              // 3.12
    BuiltinDef::method("findIndex", 1, find_index),   // 3.13
    BuiltinDef::method("findLast", 1, find_last),     // 3.14
    BuiltinDef::method("findLastIndex", 1, find_last_index), // 3.15
    BuiltinDef::method("flat", 0, flat),              // 3.16
    BuiltinDef::method("flatMap", 1, flat_map),       // 3.17
    BuiltinDef::method("forEach", 1, for_each),       // 3.18
    BuiltinDef::method("includes", 1, includes),      // 3.19
    BuiltinDef::method("lastIndexOf", 1, last_index_of), // 3.20
    BuiltinDef::method("map", 1, map),                // 3.21
    BuiltinDef::method("reduce", 1, reduce),          // 3.22
    BuiltinDef::method("reduceRight", 1, reduce_right), // 3.23
    BuiltinDef::method("reverse", 0, reverse),        // 3.24
    BuiltinDef::method("shift", 0, shift),            // 3.25
    BuiltinDef::method("slice", 2, slice),            // 3.26
    BuiltinDef::method("some", 1, some),              // 3.27
    BuiltinDef::method("sort", 1, sort),              // 3.28
    BuiltinDef::method("splice", 2, splice),          // 3.29
    BuiltinDef::method("toLocaleString", 0, to_locale_string), // 3.30
    BuiltinDef::method("toReversed", 0, to_reversed), // 3.31
    BuiltinDef::method("toSorted", 1, to_sorted),     // 3.32
    BuiltinDef::method("toSpliced", 2, to_spliced),   // 3.33
    BuiltinDef::method("unshift", 1, unshift),        // 3.34
    BuiltinDef::method("with", 2, with),              // 3.35
    BuiltinDef::method("fromEntries", 1, object_from_entries), // 3.36
    BuiltinDef::method("groupBy", 2, object_group_by), // 3.37
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Array,
        props: &[
            PropDef::method(PropKey::Str("isArray"), def::IS_ARRAY),
            PropDef::method(PropKey::Str("from"), def::FROM),
            PropDef::method(PropKey::Str("of"), def::OF),
            // §23.1.2.5: an accessor {c} with a getter and no setter.
            PropDef::accessor(
                PropKey::Symbol(WellKnownSymbol::SPECIES),
                Some(def::SPECIES),
                None,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::ArrayPrototype,
        props: &[
            PropDef::method(PropKey::Str("pop"), def::POP),
            PropDef::method(PropKey::Str("indexOf"), def::INDEX_OF),
            PropDef::method(PropKey::Str("at"), def::AT),
            PropDef::method(PropKey::Str("concat"), def::CONCAT),
            PropDef::method(PropKey::Str("copyWithin"), def::COPY_WITHIN),
            PropDef::method(PropKey::Str("every"), def::EVERY),
            PropDef::method(PropKey::Str("fill"), def::FILL),
            PropDef::method(PropKey::Str("filter"), def::FILTER),
            PropDef::method(PropKey::Str("find"), def::FIND),
            PropDef::method(PropKey::Str("findIndex"), def::FIND_INDEX),
            PropDef::method(PropKey::Str("findLast"), def::FIND_LAST),
            PropDef::method(PropKey::Str("findLastIndex"), def::FIND_LAST_INDEX),
            PropDef::method(PropKey::Str("flat"), def::FLAT),
            PropDef::method(PropKey::Str("flatMap"), def::FLAT_MAP),
            PropDef::method(PropKey::Str("forEach"), def::FOR_EACH),
            PropDef::method(PropKey::Str("includes"), def::INCLUDES),
            PropDef::method(PropKey::Str("lastIndexOf"), def::LAST_INDEX_OF),
            PropDef::method(PropKey::Str("map"), def::MAP),
            PropDef::method(PropKey::Str("reduce"), def::REDUCE),
            PropDef::method(PropKey::Str("reduceRight"), def::REDUCE_RIGHT),
            PropDef::method(PropKey::Str("reverse"), def::REVERSE),
            PropDef::method(PropKey::Str("shift"), def::SHIFT),
            PropDef::method(PropKey::Str("slice"), def::SLICE),
            PropDef::method(PropKey::Str("some"), def::SOME),
            PropDef::method(PropKey::Str("sort"), def::SORT),
            PropDef::method(PropKey::Str("splice"), def::SPLICE),
            PropDef::method(PropKey::Str("toLocaleString"), def::TO_LOCALE_STRING),
            PropDef::method(PropKey::Str("toReversed"), def::TO_REVERSED),
            PropDef::method(PropKey::Str("toSorted"), def::TO_SORTED),
            PropDef::method(PropKey::Str("toSpliced"), def::TO_SPLICED),
            PropDef::method(PropKey::Str("unshift"), def::UNSHIFT),
            PropDef::method(PropKey::Str("with"), def::WITH),
        ],
    },
    // §20.1.2.7, §20.1.2.12. Several modules may target one intrinsic (builtins.rs); a key
    // defined twice on one object is a table error, so object.rs never defines these.
    ObjectProps {
        target: IntrinsicId::Object,
        props: &[
            PropDef::method(PropKey::Str("fromEntries"), def::FROM_ENTRIES),
            PropDef::method(PropKey::Str("groupBy"), def::GROUP_BY),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "array",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- helpers shared with the other modules of this crate ------------------------------------

/// The property key of the integer index `i` (`! ToString(𝔽(i))`, an atom), for `i` up to
/// 2^53 - 1: decimal digits, the same text as Number::toString for these integers. Formats
/// on the native stack; interning finds an existing atom without allocating.
pub(crate) fn index_key<'s>(cx: &mut Cx<'s>, i: u64) -> Local<'s, JsKey> {
    let mut buf = [0u8; 20];
    let mut at = buf.len();
    let mut n = i;
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let text = core::str::from_utf8(&buf[at..]).expect("ASCII digits");
    cx.key(text)
}

/// LengthOfArrayLike ( obj ) (§7.3.18).
pub(crate) fn length_of_array_like<'s>(cx: &mut Cx<'s>, obj: Local<'s, JsObject>) -> JsResult<u64> {
    // 1. Return ℝ(? ToLength(? Get(obj, "length"))).
    let len = cx.get(obj, "length")?;
    cx.to_length(len)
}

// ---- helpers of this module ------------------------------------------------------------------

/// 2^53 - 1, the largest length (§7.1.20 ToLength).
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// 2^32 - 1, the largest array length (§10.4.2.2 ArrayCreate step 1).
const MAX_ARRAY_LENGTH: u64 = u32::MAX as u64;

/// How deep `flat` and `flatMap` recurse into nested arrays before the RangeError a native
/// stack overflow would give (FlattenIntoArray recurses once per level; V8 throws the same
/// RangeError at its stack limit). Each level also calls `Cx::check_native_stack`, which
/// throws first on a small native stack.
const MAX_FLATTEN_DEPTH: u32 = 10_000;

/// 𝔽(n) for an index or a length (at most 2^53 - 1, so exact).
fn number(n: u64) -> Local<'static> {
    Local::number(n as f64)
}

/// The well-known symbol `Symbol.<name>`, read from `%Symbol%`: its well-known symbol
/// properties are non-writable and non-configurable (§20.4.2), so the read always gives the
/// symbol.
fn well_known<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s, JsSymbol>> {
    if !cx.has_intrinsic(IntrinsicId::Symbol) {
        return Err(cx.not_implemented("well-known symbols without the Symbol module"));
    }
    let symbol = cx.intrinsic(IntrinsicId::Symbol);
    let v = cx.get(symbol, name)?;
    match v.as_symbol() {
        Some(s) => Ok(s),
        None => Err(cx.throw_type_error("Symbol.<name> is not a symbol")),
    }
}

/// The TypeError for a callback argument that is not callable.
fn not_callable<'s>(cx: &mut Cx<'s>, method: &str) -> ep9_vm::Throw {
    let msg = format!("Array.prototype.{method}: the callback is not a function");
    cx.throw_type_error(&msg)
}

/// The callback argument `f` of `method`, if IsCallable(f), else a TypeError.
fn callback<'s>(cx: &mut Cx<'s>, f: Local<'s>, method: &str) -> JsResult<Local<'s>> {
    if cx.is_callable(f) {
        Ok(f)
    } else {
        Err(not_callable(cx, method))
    }
}

/// CreateDataPropertyOrThrow(O, ! ToString(𝔽(k)), v) (§7.3.7).
fn create_index<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, k: u64, v: Local<'s>) -> JsResult<()> {
    cx.create_data_property_index_or_throw(o, k, v)
}

/// Set(O, "length", 𝔽(len), true) (§7.3.4).
fn set_length<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, len: u64) -> JsResult<()> {
    cx.set(o, "length", number(len), true).map(|_| ())
}

/// The relative index of `slice`, `fill`, `copyWithin`, `splice` and the others (for
/// example §23.1.3.28 steps 3-5): ToIntegerOrInfinity(`v`) taken from the end when
/// negative, clamped to [0, len]. `v` undefined gives `default`.
fn relative_index<'s>(cx: &mut Cx<'s>, v: Local<'s>, len: u64, default: u64) -> JsResult<u64> {
    if v.is_undefined() {
        return Ok(default);
    }
    let rel = cx.to_integer_or_infinity(v)?;
    let len_f = len as f64;
    Ok(if rel == f64::NEG_INFINITY {
        0
    } else if rel < 0.0 {
        (len_f + rel).max(0.0) as u64
    } else {
        rel.min(len_f) as u64
    })
}

/// SameValueZero ( x, y ) (§7.2.11).
fn same_value_zero<'s>(cx: &Cx<'s>, x: Local<'s>, y: Local<'s>) -> bool {
    if let (Some(a), Some(b)) = (x.as_number(), y.as_number()) {
        return a == b || (a.is_nan() && b.is_nan());
    }
    cx.same_value(x, y)
}

/// ArrayCreate ( length ) (§10.4.2.2) with the prototype %Array.prototype%.
fn array_create<'s>(cx: &mut Cx<'s>, length: u64) -> JsResult<Local<'s, JsObject>> {
    // 1. If length > 2^32 - 1, throw a RangeError exception.
    if length > MAX_ARRAY_LENGTH {
        return Err(cx.throw_range_error("Invalid array length"));
    }
    // 2-7. A new Array exotic object with "length" length. (v0: past the dense limit the
    //      length store throws the RangeError, H18.)
    let a: Local<'s, JsObject> = cx.new_array().into();
    if length > 0 {
        set_length(cx, a, length)?;
    }
    Ok(a)
}

/// ArraySpeciesCreate ( originalArray, length ) (§10.4.2.3).
fn array_species_create<'s>(
    cx: &mut Cx<'s>,
    original: Local<'s, JsObject>,
    length: u64,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Let isArray be ? IsArray(originalArray).
    // 2. If isArray is false, return ? ArrayCreate(length).
    if !cx.is_array(original) {
        return array_create(cx, length);
    }
    // 3. Let C be ? Get(originalArray, "constructor").
    let mut c = cx.get(original, "constructor")?;
    // 4. If IsConstructor(C) is true, then
    //    a-b. Let thisRealm be the current Realm Record; let realmC be ? GetFunctionRealm(C).
    //    c. If thisRealm and realmC are not the same Realm Record, then: if SameValue(C,
    //       realmC.[[Intrinsics]].[[%Array%]]) is true, set C to undefined. (C is realmC's
    //       %Array% and not the current one: each realm has its own %Array%.)
    if cx.is_constructor(c) {
        let realm_array = cx.function_realm_intrinsic(c, IntrinsicId::Array);
        if c.is_same_bits(&realm_array) && !c.is_same_bits(&cx.intrinsic(IntrinsicId::Array)) {
            c = Local::undefined();
        }
    }
    // 5. If C is an Object, then
    if let Some(co) = c.as_object() {
        // a. Set C to ? Get(C, %Symbol.species%).
        let species = well_known(cx, "species")?;
        c = cx.get(co, species)?;
        // b. If C is null, set C to undefined.
        if c.is_null() {
            c = Local::undefined();
        }
    }
    // 6. If C is undefined, return ? ArrayCreate(length).
    if c.is_undefined() {
        return array_create(cx, length);
    }
    // 7. If IsConstructor(C) is false, throw a TypeError exception.
    if !cx.is_constructor(c) {
        return Err(cx.throw_type_error("object.constructor[Symbol.species] is not a constructor"));
    }
    // 8. Return ? Construct(C, « 𝔽(length) »). For %Array% that is ArrayCreate(length) (the
    //    Array constructor with one Number argument, §23.1.1.1 step 6).
    if c.is_same_bits(&cx.intrinsic(IntrinsicId::Array)) {
        return array_create(cx, length);
    }
    cx.construct(c, &[number(length)], c)
}

/// The kinds of the loops over an array-like that call a callback per present element.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Walk {
    Every,
    Some,
    ForEach,
}

/// `every`, `some` and `forEach` (§23.1.3.6, §23.1.3.29, §23.1.3.15): the same steps with a
/// different result.
fn each<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    kind: Walk,
    method: &str,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), method)?;
    let this_arg = args.get(1);
    // 4. Let k be 0.
    let mut k = 0;
    // 5. Repeat, while k < len,
    while k < len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let kValue be ? Get(O, Pk).
            let k_value = cx.get_index(o, k)?;
            // ii. Let testResult be ToBoolean(? Call(callbackfn, thisArg, « kValue, 𝔽(k), O »)).
            let r = cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
            let test = cx.to_boolean(r);
            // iii. every: if testResult is false, return false. some: if true, return true.
            match kind {
                Walk::Every if !test => return Ok(Local::boolean(false)),
                Walk::Some if test => return Ok(Local::boolean(true)),
                _ => {}
            }
        }
        // d. Set k to k + 1.
        k += 1;
    }
    // 6. every: return true; some: return false; forEach: return undefined.
    Ok(match kind {
        Walk::Every => Local::boolean(true),
        Walk::Some => Local::boolean(false),
        Walk::ForEach => Local::undefined(),
    })
}

/// The direction of FindViaPredicate (§23.1.3.12.1).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Ascending,
    Descending,
}

/// FindViaPredicate ( O, len, direction, predicate, thisArg ) (§23.1.3.12.1) after
/// `find`'s steps 1-2: (index, value), index -1 if no element matches.
fn find_via_predicate<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    direction: Direction,
    method: &str,
) -> JsResult<(f64, Local<'s>)> {
    // (find) 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // (find) 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 1. If IsCallable(predicate) is false, throw a TypeError exception.
    let predicate = callback(cx, args.get(0), method)?;
    let this_arg = args.get(1);
    // 2-3. The indices from 0 to len - 1, or from len - 1 to 0.
    for i in 0..len {
        cx.poll_interrupt()?;
        let k = match direction {
            Direction::Ascending => i,
            Direction::Descending => len - 1 - i,
        };
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. NOTE: If O is a TypedArray, ... (no TypedArrays in v0).
        // c. Let kValue be ? Get(O, Pk).
        let k_value = cx.get_index(o, k)?;
        // d. Let testResult be ? Call(predicate, thisArg, « kValue, 𝔽(k), O »).
        let r = cx.call(predicate, this_arg, &[k_value, number(k), o.value()])?;
        // e. If ToBoolean(testResult) is true, return the Record { [[Index]]: 𝔽(k),
        //    [[Value]]: kValue }.
        if cx.to_boolean(r) {
            return Ok((k as f64, k_value));
        }
    }
    // 5. Return the Record { [[Index]]: -1𝔽, [[Value]]: undefined }.
    Ok((-1.0, Local::undefined()))
}

/// FlattenIntoArray ( target, source, sourceLen, start, depth [ , mapperFunction [ ,
/// thisArg ] ] ) (§23.1.3.13.1): the next target index. `level` counts the recursion.
#[allow(clippy::too_many_arguments)]
fn flatten_into_array<'s>(
    cx: &mut Cx<'s>,
    target: Local<'s, JsObject>,
    source: Local<'s, JsObject>,
    source_len: u64,
    start: u64,
    depth: f64,
    mapper: Option<(Local<'s>, Local<'s>)>,
    level: u32,
) -> JsResult<u64> {
    // One recursion step: the native stack check (H23 G6), then the level cap.
    cx.check_native_stack()?;
    if level > MAX_FLATTEN_DEPTH {
        return Err(cx.throw_range_error("Maximum call stack size exceeded"));
    }
    // 1. Assert: If mapperFunction is present, ... and depth is 1.
    // 2. Let targetIndex be start.
    let mut target_index = start;
    // 3. Let sourceIndex be +0𝔽.
    let mut source_index = 0;
    // 4. Repeat, while ℝ(sourceIndex) < sourceLen,
    while source_index < source_len {
        cx.poll_interrupt()?;
        // a. Let P be ! ToString(sourceIndex).
        // b. Let exists be ? HasProperty(source, P).
        // c. If exists is true, then
        if cx.has_index(source, source_index)? {
            // i. Let element be ? Get(source, P).
            let mut element = cx.get_index(source, source_index)?;
            // ii. If mapperFunction is present, then
            if let Some((f, this_arg)) = mapper {
                // 1. Set element to ? Call(mapperFunction, thisArg, « element, sourceIndex,
                //    source »).
                element = cx.call(
                    f,
                    this_arg,
                    &[element, number(source_index), source.value()],
                )?;
            }
            // iii. Let shouldFlatten be false.
            // iv. If depth > 0, then set shouldFlatten to ? IsArray(element).
            let should_flatten = depth > 0.0 && cx.is_array(element);
            // v. If shouldFlatten is true, then
            if should_flatten {
                let element = element.as_object().expect("an array is an object");
                // 1. If depth = +∞, let newDepth be +∞.
                // 2. Else, let newDepth be depth - 1.
                let new_depth = if depth == f64::INFINITY {
                    depth
                } else {
                    depth - 1.0
                };
                // 3. Let elementLen be ? LengthOfArrayLike(element).
                let element_len = length_of_array_like(cx, element)?;
                // 4. Set targetIndex to ? FlattenIntoArray(target, element, elementLen,
                //    targetIndex, newDepth).
                target_index = flatten_into_array(
                    cx,
                    target,
                    element,
                    element_len,
                    target_index,
                    new_depth,
                    None,
                    level + 1,
                )?;
            } else {
                // vi. Else,
                // 1. If targetIndex ≥ 2^53 - 1, throw a TypeError exception.
                if target_index >= MAX_SAFE_INTEGER {
                    return Err(cx.throw_type_error("the flattened array is too long"));
                }
                // 2. Perform ? CreateDataPropertyOrThrow(target, ! ToString(𝔽(targetIndex)),
                //    element).
                create_index(cx, target, target_index, element)?;
                // 3. Set targetIndex to targetIndex + 1.
                target_index += 1;
            }
        }
        // d. Set sourceIndex to sourceIndex + 1𝔽.
        source_index += 1;
    }
    // 5. Return targetIndex.
    Ok(target_index)
}

/// Which elements SortIndexedProperties reads (§23.1.3.30.1 `holes`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holes {
    SkipHoles,
    ReadThroughHoles,
}

/// SortIndexedProperties ( obj, len, SortCompare, holes ) (§23.1.3.30.1), steps 1-3: the
/// values of `obj` from index 0 to `len - 1` appended to `items` (holes skipped or read).
fn collect_items<'s>(
    cx: &mut Cx<'s>,
    obj: Local<'s, JsObject>,
    len: u64,
    holes: Holes,
    items: &ScratchList<'s>,
) -> JsResult<()> {
    // 1. Let items be a new empty List.
    // 2. Let k be 0.
    // 3. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. If holes is skip-holes, then let kRead be ? HasProperty(obj, Pk).
        // c. Else, let kRead be true.
        let k_read = holes == Holes::ReadThroughHoles || cx.has_index(obj, k)?;
        // d. If kRead is true, then
        if k_read {
            // i. Let kValue be ? Get(obj, Pk).
            let k_value = cx.get_index(obj, k)?;
            // ii. Append kValue to items.
            items.push(k_value);
        }
        // e. Set k to k + 1.
    }
    Ok(())
}

/// SortIndexedProperties step 4 with SortCompare (§23.1.3.30.2) as the comparator: the order
/// of `items` as a list of indices into it. A stable merge sort ([`merge_sort`]); undefined
/// values go last. Without `comparefn`, SortCompare converts x and then y with ToString at
/// every comparison (§23.1.3.30.2 steps 5-6), so an object's `toString` runs once per
/// comparison it takes part in and a throw ends the sort; only the sequence of SortCompare
/// calls is implementation-defined (§23.1.3.30.1 step 4). When every value is a primitive,
/// ToString runs no user code and its result never changes, so each one is converted once,
/// before the first comparison.
fn sorted_order<'s>(
    cx: &mut Cx<'s>,
    items: &ScratchList<'s>,
    comparefn: Local<'s>,
) -> JsResult<Vec<usize>> {
    let n = items.len();
    // SortCompare steps 1-3: undefined values come after every other value, and compare
    // equal among themselves (stable: they keep their order).
    let defined: Vec<usize> = (0..n).filter(|&i| !items.get(i).is_undefined()).collect();
    let undefined = (0..n).filter(|&i| items.get(i).is_undefined());
    let mut order = defined;
    if comparefn.is_undefined() {
        if order.len() >= 2 && order.iter().any(|&i| items.get(i).as_object().is_some()) {
            // SortCompare steps 5-7 at each comparison: ToString(x), then ToString(y), then
            // IsLessThan on the code units. An object's ToString runs user code (counted and
            // ordered as the spec says), and an Err from it ends the sort.
            merge_sort(cx, &mut order, &mut |cx, a, b| {
                let x = cx.to_string(items.get(a))?;
                let y = cx.to_string(items.get(b))?;
                Ok(compare_strings(cx, x, y)? == Ordering::Greater)
            })?;
        } else if order.len() >= 2 {
            // Only primitives: ToString of each value once (no user code runs, and a Symbol
            // throws the same TypeError before any comparison as at its first one), then
            // IsLessThan on the code units at each comparison. The strings are rooted in a
            // second list (pushed after `items`, dropped before it: LIFO).
            let strings = cx.scratch_list();
            for &i in &order {
                cx.poll_interrupt()?;
                let s = cx.to_string(items.get(i))?;
                let s = cx.flatten(s)?;
                strings.push(s);
            }
            // `strings[j]` is the string of `items[order[j]]`.
            let mut by_string: Vec<usize> = (0..order.len()).collect();
            merge_sort(cx, &mut by_string, &mut |cx, a, b| {
                let (x, y) = (strings.get(a), strings.get(b));
                let x: Local<'s, JsString> = x.downcast().expect("a string");
                let y: Local<'s, JsString> = y.downcast().expect("a string");
                Ok(compare_strings(cx, x, y)? == Ordering::Greater)
            })?;
            order = by_string.into_iter().map(|j| order[j]).collect();
        }
    } else {
        merge_sort(cx, &mut order, &mut |cx, a, b| {
            // SortCompare step 4: If comparefn is not undefined, then
            //   a. Let v be ? ToNumber(? Call(comparefn, undefined, « x, y »)).
            //   b. If v is NaN, return +0𝔽.
            //   c. Return v.
            let r = cx.call(comparefn, Local::undefined(), &[items.get(a), items.get(b)])?;
            let v = cx.to_number(r)?;
            Ok(v > 0.0)
        })?;
    }
    order.extend(undefined);
    Ok(order)
}

/// IsLessThan on two strings (§7.2.13 step 3): the order of their code units.
fn compare_strings<'s>(
    cx: &mut Cx<'s>,
    x: Local<'s, JsString>,
    y: Local<'s, JsString>,
) -> JsResult<Ordering> {
    let x = cx.flatten(x)?;
    let y = cx.flatten(y)?;
    let mut i = 0;
    loop {
        match (cx.code_unit_at(x, i), cx.code_unit_at(y, i)) {
            (Some(a), Some(b)) if a == b => i += 1,
            (Some(a), Some(b)) => return Ok(a.cmp(&b)),
            (Some(_), None) => return Ok(Ordering::Greater),
            (None, Some(_)) => return Ok(Ordering::Less),
            (None, None) => return Ok(Ordering::Equal),
        }
    }
}

/// A stable bottom-up merge sort of `order`. `after(cx, a, b)` is true when `a` must come
/// after `b` (SortCompare(a, b) > 0); an `Err` from it ends the sort. Only indices are moved:
/// the values stay in their `ScratchList` (rule R2).
fn merge_sort<'s>(
    cx: &mut Cx<'s>,
    order: &mut Vec<usize>,
    after: &mut dyn FnMut(&mut Cx<'s>, usize, usize) -> JsResult<bool>,
) -> JsResult<()> {
    let n = order.len();
    let mut buf = vec![0usize; n];
    let mut width = 1;
    while width < n {
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            let (mut l, mut r, mut out) = (lo, mid, lo);
            while l < mid && r < hi {
                // A host comparator never polls: one budget unit per comparison.
                cx.poll_interrupt()?;
                if after(cx, order[l], order[r])? {
                    buf[out] = order[r];
                    r += 1;
                } else {
                    buf[out] = order[l];
                    l += 1;
                }
                out += 1;
            }
            buf[out..out + (mid - l)].copy_from_slice(&order[l..mid]);
            out += mid - l;
            buf[out..out + (hi - r)].copy_from_slice(&order[r..hi]);
            lo = hi;
        }
        core::mem::swap(order, &mut buf);
        width *= 2;
    }
    Ok(())
}

/// The `comparefn` check of `sort` and `toSorted` (§23.1.3.30 step 1, §23.1.3.34 step 1):
/// undefined or callable, else a TypeError.
fn compare_fn<'s>(cx: &mut Cx<'s>, comparefn: Local<'s>, method: &str) -> JsResult<Local<'s>> {
    if comparefn.is_undefined() || cx.is_callable(comparefn) {
        return Ok(comparefn);
    }
    let msg = format!(
        "Array.prototype.{method}: the comparison function must be either a function or undefined"
    );
    Err(cx.throw_type_error(&msg))
}

// ---- builtins: statics -------------------------------------------------------------------------

/// Template 1. Array.isArray ( arg ) (§23.1.2.2).
fn is_array<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? IsArray(arg). (v0: no proxies, so IsArray never throws.)
    Ok(Local::boolean(cx.is_array(args.get(0))))
}

/// Array.from ( items [ , mapper [ , thisArg ] ] ) (§23.1.2.1).
fn from<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (items, mapper, this_arg) = (args.get(0), args.get(1), args.get(2));
    // 1. Let C be the this value.
    let c = this;
    // 2. If mapper is undefined, let mapping be false.
    // 3. Else, if IsCallable(mapper) is false, throw a TypeError exception; mapping is true.
    let mapping = !mapper.is_undefined();
    if mapping && !cx.is_callable(mapper) {
        return Err(cx.throw_type_error("Array.from: the mapper is not a function"));
    }
    // 4. Let usingIterator be ? GetMethod(items, %Symbol.iterator%).
    let iterator_symbol = well_known(cx, "iterator")?;
    if items.is_undefined_or_null() {
        // GetMethod -> GetV -> ToObject throws for undefined and null.
        return Err(cx.throw_type_error("Array.from: items is undefined or null"));
    }
    let using_iterator = cx.get_v(items, iterator_symbol)?;
    let using_iterator = if using_iterator.is_undefined_or_null() {
        None
    } else if cx.is_callable(using_iterator) {
        Some(using_iterator)
    } else {
        return Err(cx.throw_type_error("Array.from: items[Symbol.iterator] is not a function"));
    };
    // 5. If usingIterator is not undefined, then
    if let Some(method) = using_iterator {
        // a. If IsConstructor(C) is true, let A be ? Construct(C).
        // b. Else, let A be ! ArrayCreate(0).
        let a = if cx.is_constructor(c) {
            cx.construct(c, &[], c)?
        } else {
            array_create(cx, 0)?
        };
        // c. Let iteratorRecord be ? GetIteratorFromMethod(items, usingIterator).
        let mut iterator_record = cx.get_iterator_from_method(items, method)?;
        // d. Let k be 0.
        let mut k: u64 = 0;
        // e. Repeat,
        loop {
            // i. If k ≥ 2^53 - 1, then let error be ThrowCompletion(a TypeError); return
            //    ? IteratorClose(iteratorRecord, error).
            if k >= MAX_SAFE_INTEGER {
                let error = Err(cx.throw_type_error("Array.from: too many items"));
                return cx.iterator_close(&iterator_record, error);
            }
            // ii. Let Pk be ! ToString(𝔽(k)).
            // iii. Let next be ? IteratorStepValue(iteratorRecord).
            // iv. If next is done, then
            let Some(next) = cx.iterator_step_value(&mut iterator_record)? else {
                // 1. Perform ? Set(A, "length", 𝔽(k), true).
                set_length(cx, a, k)?;
                // 2. Return A.
                return Ok(a.value());
            };
            // v. If mapping is true, then
            //    1. Let mappedValue be Completion(Call(mapper, thisArg, « next, 𝔽(k) »)).
            //    2. IfAbruptCloseIterator(mappedValue, iteratorRecord).
            // vi. Else, let mappedValue be next.
            let mapped_value = if mapping {
                match cx.call(mapper, this_arg, &[next, number(k)]) {
                    Ok(v) => v,
                    Err(e) => return cx.iterator_close(&iterator_record, Err(e)),
                }
            } else {
                next
            };
            // vii. Let defineStatus be Completion(CreateDataPropertyOrThrow(A, Pk,
            //      mappedValue)).
            // viii. IfAbruptCloseIterator(defineStatus, iteratorRecord).
            if let Err(e) = create_index(cx, a, k, mapped_value) {
                return cx.iterator_close(&iterator_record, Err(e));
            }
            // ix. Set k to k + 1.
            k += 1;
        }
    }
    // 6. NOTE: items is not an Iterable, so assume it is an array-like object.
    // 7. Let arrayLike be ! ToObject(items).
    let array_like = cx.to_object(items)?;
    // 8. Let len be ? LengthOfArrayLike(arrayLike).
    let len = length_of_array_like(cx, array_like)?;
    // 9. If IsConstructor(C) is true, let A be ? Construct(C, « 𝔽(len) »).
    // 10. Else, let A be ? ArrayCreate(len).
    let a = if cx.is_constructor(c) {
        cx.construct(c, &[number(len)], c)?
    } else {
        array_create(cx, len)?
    };
    // 11. Let k be 0.
    // 12. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kValue be ? Get(arrayLike, Pk).
        let k_value = cx.get_index(array_like, k)?;
        // c. If mapping is true, let mappedValue be ? Call(mapper, thisArg, « kValue, 𝔽(k) »).
        // d. Else, let mappedValue be kValue.
        let mapped_value = if mapping {
            cx.call(mapper, this_arg, &[k_value, number(k)])?
        } else {
            k_value
        };
        // e. Perform ? CreateDataPropertyOrThrow(A, Pk, mappedValue).
        create_index(cx, a, k, mapped_value)?;
        // f. Set k to k + 1.
    }
    // 13. Perform ? Set(A, "length", 𝔽(len), true).
    set_length(cx, a, len)?;
    // 14. Return A.
    Ok(a.value())
}

/// Array.of ( ...items ) (§23.1.2.3).
fn of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let len be the number of elements in items.
    let len = args.len() as u64;
    // 2. Let lenNumber be 𝔽(len).
    // 3. Let C be the this value.
    let c = this;
    // 4. If IsConstructor(C) is true, let A be ? Construct(C, « lenNumber »).
    // 5. Else, let A be ? ArrayCreate(len).
    let a = if cx.is_constructor(c) {
        cx.construct(c, &[number(len)], c)?
    } else {
        array_create(cx, len)?
    };
    // 6. Let k be 0.
    // 7. Repeat, while k < len,
    for (k, item) in args.iter().enumerate() {
        // a. Let kValue be items[k].
        // b. Let Pk be ! ToString(𝔽(k)).
        // c. Perform ? CreateDataPropertyOrThrow(A, Pk, kValue).
        create_index(cx, a, k as u64, item)?;
        // d. Set k to k + 1.
    }
    // 8. Perform ? Set(A, "length", lenNumber, true).
    set_length(cx, a, len)?;
    // 9. Return A.
    Ok(a.value())
}

/// Template 5. get Array [ %Symbol.species% ] (§23.1.2.5).
fn species<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return the this value.
    Ok(this)
}

/// Object.fromEntries ( iterable ) (§20.1.2.7), with AddEntriesFromIterable (§24.1.1.2) and
/// CreateDataPropertyOnObject as the adder (TEMPLATES.md Template 7).
fn object_from_entries<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let iterable = args.get(0);
    // 1. Perform ? RequireObjectCoercible(iterable).
    if iterable.is_undefined_or_null() {
        return Err(cx.throw_type_error("Cannot convert undefined or null to object"));
    }
    // 2. Let obj be OrdinaryObjectCreate(%Object.prototype%).
    let obj = cx.new_object();
    // 3-5. Return ? AddEntriesFromIterable(obj, iterable, adder).
    //   1. Let iteratorRecord be ? GetIterator(iterable, sync).
    let mut iterator_record = cx.get_iterator(iterable)?;
    //   2. Repeat,
    //      a. Let next be ? IteratorStepValue(iteratorRecord).
    //      b. If next is done, return target.
    while let Some(next) = cx.iterator_step_value(&mut iterator_record)? {
        //  c-i. Each abrupt completion: return ? IteratorClose(iteratorRecord, completion).
        if let Err(throw) = add_entry(cx, obj, next) {
            return cx.iterator_close(&iterator_record, Err(throw));
        }
    }
    Ok(obj.value())
}

/// AddEntriesFromIterable steps 2.c-2.i for one value of the iterator.
fn add_entry<'s>(cx: &mut Cx<'s>, obj: Local<'s, JsObject>, next: Local<'s>) -> JsResult<()> {
    // c. If next is not an Object, then let error be ThrowCompletion(a TypeError).
    let Some(next) = next.as_object() else {
        return Err(cx.throw_type_error("Iterator value is not an entry object"));
    };
    // d. Let k be Completion(Get(next, "0")).
    let k = cx.get_index(next, 0)?;
    // f. Let v be Completion(Get(next, "1")).
    let v = cx.get_index(next, 1)?;
    // h. Let status be Completion(Call(adder, target, « k, v »)): CreateDataPropertyOnObject
    //    is ToPropertyKey, then ! CreateDataPropertyOrThrow.
    let key = cx.to_property_key(k)?;
    cx.create_data_property_or_throw(obj, key, v)
}

/// Object.groupBy ( items, callback ) (§20.1.2.12), with GroupBy (§7.3.35, property keys)
/// and AddValueToKeyedGroup (§7.3.34). The groups are the result object itself: each key
/// gets its array when its first value comes, so the keys are created in the order of the
/// groups list. Neither the object nor the arrays are visible to `callback`.
fn object_group_by<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (items, callback) = (args.get(0), args.get(1));
    // GroupBy 1. Perform ? RequireObjectCoercible(items).
    if items.is_undefined_or_null() {
        return Err(cx.throw_type_error("Object.groupBy: items is undefined or null"));
    }
    // GroupBy 2. If IsCallable(callback) is false, throw a TypeError exception.
    if !cx.is_callable(callback) {
        return Err(cx.throw_type_error("Object.groupBy: the callback is not a function"));
    }
    // 2. Let obj be OrdinaryObjectCreate(null). (Made first: it holds the groups.)
    let obj = cx.new_object_with_proto(Local::null());
    // GroupBy 3. Let groups be a new empty List.
    // GroupBy 4. Let iteratorRecord be ? GetIterator(items, sync).
    let mut iterator_record = cx.get_iterator(items)?;
    // GroupBy 5. Let k be 0.
    let mut k: u64 = 0;
    // GroupBy 6. Repeat,
    loop {
        // a. If k ≥ 2^53 - 1, then let error be ThrowCompletion(a TypeError); return
        //    ? IteratorClose(iteratorRecord, error).
        if k >= MAX_SAFE_INTEGER {
            let error = Err(cx.throw_type_error("Object.groupBy: too many items"));
            return cx.iterator_close(&iterator_record, error);
        }
        // b. Let next be ? IteratorStepValue(iteratorRecord).
        // c. If next is done, then return groups.
        let Some(value) = cx.iterator_step_value(&mut iterator_record)? else {
            break;
        };
        // d. Let value be next.
        // e. Let key be Completion(Call(callback, undefined, « value, 𝔽(k) »)).
        // f. IfAbruptCloseIterator(key, iteratorRecord).
        // g. If keyCoercion is property, then
        //    i. Set key to Completion(ToPropertyKey(key)).
        //    ii. IfAbruptCloseIterator(key, iteratorRecord).
        // i. Perform AddValueToKeyedGroup(groups, key, value).
        if let Err(e) = group_value(cx, obj, callback, value, k) {
            return cx.iterator_close(&iterator_record, Err(e));
        }
        // j. Set k to k + 1.
        k += 1;
    }
    // 3. For each Record { [[Key]], [[Elements]] } g of groups: elements =
    //    CreateArrayFromList(g.[[Elements]]); ! CreateDataPropertyOrThrow(obj, g.[[Key]],
    //    elements). (Done as the groups grew.)
    // 4. Return obj.
    Ok(obj.value())
}

/// GroupBy steps 6.e-6.i for one value: the key, then AddValueToKeyedGroup on the result
/// object `obj`.
fn group_value<'s>(
    cx: &mut Cx<'s>,
    obj: Local<'s, JsObject>,
    callback: Local<'s>,
    value: Local<'s>,
    k: u64,
) -> JsResult<()> {
    let key = cx.call(callback, Local::undefined(), &[value, number(k)])?;
    let key = cx.to_property_key(key)?;
    // AddValueToKeyedGroup 1. For each Record g of groups: if SameValue(g.[[Key]], key),
    //    append value to g.[[Elements]] and return.
    if let Some(d) = cx.get_own_property(obj, key)?
        && let Some(group) = d.value.and_then(|v| v.as_object())
    {
        let len = length_of_array_like(cx, group)?;
        create_index(cx, group, len, value)?;
        return Ok(());
    }
    // 2. Let group be the Record { [[Key]]: key, [[Elements]]: « value » }.
    // 3. Append group to groups.
    let group = cx.create_array_from_list(&[value])?;
    cx.create_data_property_or_throw(obj, key, group)
}

// ---- builtins: Array.prototype -----------------------------------------------------------------

/// Array.prototype.at ( index ) (§23.1.3.1).
fn at<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)? as f64;
    // 3. Let relativeIndex be ? ToIntegerOrInfinity(index).
    let relative_index = cx.to_integer_or_infinity(args.get(0))?;
    // 4. If relativeIndex ≥ 0, then let k be relativeIndex.
    // 5. Else, let k be len + relativeIndex.
    let k = if relative_index >= 0.0 {
        relative_index
    } else {
        len + relative_index
    };
    // 6. If k < 0 or k ≥ len, return undefined.
    if k < 0.0 || k >= len {
        return Ok(Local::undefined());
    }
    // 7. Return ? Get(O, ! ToString(𝔽(k))).
    cx.get_index(o, k as u64)
}

/// Array.prototype.concat ( ...items ) (§23.1.3.2).
fn concat<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let A be ? ArraySpeciesCreate(O, 0).
    let a = array_species_create(cx, o, 0)?;
    // 3. Let n be 0.
    let mut n: u64 = 0;
    // 4. Prepend O to items.
    // 5. For each element E of items, do
    for i in 0..=args.len() {
        let e = if i == 0 { o.value() } else { args.get(i - 1) };
        // a. Let spreadable be ? IsConcatSpreadable(E).
        // b. If spreadable is true, then
        if let Some(e) = is_concat_spreadable(cx, e)? {
            // i. Let len be ? LengthOfArrayLike(E).
            let len = length_of_array_like(cx, e)?;
            // ii. If n + len > 2^53 - 1, throw a TypeError exception.
            if n + len > MAX_SAFE_INTEGER {
                return Err(cx.throw_type_error("Array.prototype.concat: the result is too long"));
            }
            // iii. Let k be 0.
            // iv. Repeat, while k < len,
            for k in 0..len {
                cx.poll_interrupt()?;
                // 1. Let Pk be ! ToString(𝔽(k)).
                // 2. Let exists be ? HasProperty(E, Pk).
                // 3. If exists is true, then
                if cx.has_index(e, k)? {
                    // a. Let subElement be ? Get(E, Pk).
                    let sub_element = cx.get_index(e, k)?;
                    // b. Perform ? CreateDataPropertyOrThrow(A, ! ToString(𝔽(n)), subElement).
                    create_index(cx, a, n, sub_element)?;
                }
                // 4. Set n to n + 1.
                n += 1;
                // 5. Set k to k + 1.
            }
        } else {
            // c. Else,
            // i. NOTE: E is added as a single item rather than spread.
            // ii. If n ≥ 2^53 - 1, throw a TypeError exception.
            if n >= MAX_SAFE_INTEGER {
                return Err(cx.throw_type_error("Array.prototype.concat: the result is too long"));
            }
            // iii. Perform ? CreateDataPropertyOrThrow(A, ! ToString(𝔽(n)), E).
            create_index(cx, a, n, e)?;
            // iv. Set n to n + 1.
            n += 1;
        }
    }
    // 6. Perform ? Set(A, "length", 𝔽(n), true).
    set_length(cx, a, n)?;
    // 7. Return A.
    Ok(a.value())
}

/// IsConcatSpreadable ( O ) (§23.1.3.2.1): `Some(O)` when it is true.
fn is_concat_spreadable<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
) -> JsResult<Option<Local<'s, JsObject>>> {
    // 1. If O is not an Object, return false.
    let Some(obj) = o.as_object() else {
        return Ok(None);
    };
    // 2. Let spreadable be ? Get(O, %Symbol.isConcatSpreadable%).
    let symbol = well_known(cx, "isConcatSpreadable")?;
    let spreadable = cx.get(obj, symbol)?;
    // 3. If spreadable is not undefined, return ToBoolean(spreadable).
    // 4. Return ? IsArray(O).
    let yes = if spreadable.is_undefined() {
        cx.is_array(obj)
    } else {
        cx.to_boolean(spreadable)
    };
    Ok(yes.then_some(obj))
}

/// Array.prototype.copyWithin ( target, start [ , end ] ) (§23.1.3.4).
fn copy_within<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3-5. Let relativeTarget be ? ToIntegerOrInfinity(target); to from it.
    let to = relative_index(cx, args.get(0), len, 0)?;
    // 6-8. Let relativeStart be ? ToIntegerOrInfinity(start); from from it.
    let from = relative_index(cx, args.get(1), len, 0)?;
    // 9-11. If end is undefined, let relativeEnd be len; final from it.
    let final_ = relative_index(cx, args.get(2), len, len)?;
    // 12. Let count be min(final - from, len - to).
    let count = final_.saturating_sub(from).min(len - to);
    // 13. If from < to and to < from + count, then
    let (mut from, mut to, backward) = if from < to && to < from + count {
        // a. Let direction be -1.
        // b. Set from to from + count - 1.
        // c. Set to to to + count - 1.
        (from + count - 1, to + count - 1, true)
    } else {
        // 14. Else, let direction be 1.
        (from, to, false)
    };
    // 15. Repeat, while count > 0,
    let mut count = count;
    while count > 0 {
        cx.poll_interrupt()?;
        // a. Let fromKey be ! ToString(𝔽(from)).
        // b. Let toKey be ! ToString(𝔽(to)).
        // c. Let fromPresent be ? HasProperty(O, fromKey).
        // d. If fromPresent is true, then
        if cx.has_index(o, from)? {
            // i. Let fromValue be ? Get(O, fromKey).
            let from_value = cx.get_index(o, from)?;
            // ii. Perform ? Set(O, toKey, fromValue, true).
            cx.set_index(o, to, from_value, true)?;
        } else {
            // e. Else,
            // i. Assert: fromPresent is false.
            // ii. Perform ? DeletePropertyOrThrow(O, toKey).
            cx.delete_index(o, to, true)?;
        }
        // f. Set from to from + direction.
        // g. Set to to to + direction.
        // h. Set count to count - 1.
        count -= 1;
        if count > 0 {
            if backward {
                from -= 1;
                to -= 1;
            } else {
                from += 1;
                to += 1;
            }
        }
    }
    // 16. Return O.
    Ok(o.value())
}

/// Array.prototype.every ( callbackfn [ , thisArg ] ) (§23.1.3.6).
fn every<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    each(cx, this, args, Walk::Every, "every")
}

/// Array.prototype.fill ( value [ , start [ , end ] ] ) (§23.1.3.7).
fn fill<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let value = args.get(0);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3-5. Let relativeStart be ? ToIntegerOrInfinity(start); k from it.
    let k = relative_index(cx, args.get(1), len, 0)?;
    // 6-8. If end is undefined, let relativeEnd be len; final from it.
    let final_ = relative_index(cx, args.get(2), len, len)?;
    // 9. Repeat, while k < final,
    for k in k..final_ {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Perform ? Set(O, Pk, value, true).
        cx.set_index(o, k, value, true)?;
        // c. Set k to k + 1.
    }
    // 10. Return O.
    Ok(o.value())
}

/// Array.prototype.filter ( callbackfn [ , thisArg ] ) (§23.1.3.8).
fn filter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), "filter")?;
    let this_arg = args.get(1);
    // 4. Let A be ? ArraySpeciesCreate(O, 0).
    let a = array_species_create(cx, o, 0)?;
    // 5. Let k be 0.
    // 6. Let to be 0.
    let mut to = 0;
    // 7. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let kValue be ? Get(O, Pk).
            let k_value = cx.get_index(o, k)?;
            // ii. Let selected be ToBoolean(? Call(callbackfn, thisArg, « kValue, 𝔽(k), O »)).
            let r = cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
            // iii. If selected is true, then
            if cx.to_boolean(r) {
                // 1. Perform ? CreateDataPropertyOrThrow(A, ! ToString(𝔽(to)), kValue).
                create_index(cx, a, to, k_value)?;
                // 2. Set to to to + 1.
                to += 1;
            }
        }
        // d. Set k to k + 1.
    }
    // 8. Return A.
    Ok(a.value())
}

/// Array.prototype.find ( predicate [ , thisArg ] ) (§23.1.3.9).
fn find<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 3. Let findRec be ? FindViaPredicate(O, len, ascending, predicate, thisArg).
    // 4. Return findRec.[[Value]].
    Ok(find_via_predicate(cx, this, args, Direction::Ascending, "find")?.1)
}

/// Array.prototype.findIndex ( predicate [ , thisArg ] ) (§23.1.3.10).
fn find_index<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 3. Let findRec be ? FindViaPredicate(O, len, ascending, predicate, thisArg).
    // 4. Return findRec.[[Index]].
    let (index, _) = find_via_predicate(cx, this, args, Direction::Ascending, "findIndex")?;
    Ok(Local::number(index))
}

/// Array.prototype.findLast ( predicate [ , thisArg ] ) (§23.1.3.11).
fn find_last<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 3. Let findRec be ? FindViaPredicate(O, len, descending, predicate, thisArg).
    // 4. Return findRec.[[Value]].
    Ok(find_via_predicate(cx, this, args, Direction::Descending, "findLast")?.1)
}

/// Array.prototype.findLastIndex ( predicate [ , thisArg ] ) (§23.1.3.12).
fn find_last_index<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 3. Let findRec be ? FindViaPredicate(O, len, descending, predicate, thisArg).
    // 4. Return findRec.[[Index]].
    let (index, _) = find_via_predicate(cx, this, args, Direction::Descending, "findLastIndex")?;
    Ok(Local::number(index))
}

/// Array.prototype.flat ( \[ depth \] ) (§23.1.3.13).
fn flat<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let depth = args.get(0);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let sourceLen be ? LengthOfArrayLike(O).
    let source_len = length_of_array_like(cx, o)?;
    // 3. Let depthNum be 1.
    // 4. If depth is not undefined, then
    let depth_num = if depth.is_undefined() {
        1.0
    } else {
        // a. Set depthNum to ? ToIntegerOrInfinity(depth).
        // b. If depthNum < 0, set depthNum to 0.
        cx.to_integer_or_infinity(depth)?.max(0.0)
    };
    // 5. Let A be ? ArraySpeciesCreate(O, 0).
    let a = array_species_create(cx, o, 0)?;
    // 6. Perform ? FlattenIntoArray(A, O, sourceLen, 0, depthNum).
    flatten_into_array(cx, a, o, source_len, 0, depth_num, None, 0)?;
    // 7. Return A.
    Ok(a.value())
}

/// Array.prototype.flatMap ( mapperFunction [ , thisArg ] ) (§23.1.3.14).
fn flat_map<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let sourceLen be ? LengthOfArrayLike(O).
    let source_len = length_of_array_like(cx, o)?;
    // 3. If IsCallable(mapperFunction) is false, throw a TypeError exception.
    let mapper = callback(cx, args.get(0), "flatMap")?;
    // 4. Let A be ? ArraySpeciesCreate(O, 0).
    let a = array_species_create(cx, o, 0)?;
    // 5. Perform ? FlattenIntoArray(A, O, sourceLen, 0, 1, mapperFunction, thisArg).
    flatten_into_array(cx, a, o, source_len, 0, 1.0, Some((mapper, args.get(1))), 0)?;
    // 6. Return A.
    Ok(a.value())
}

/// Array.prototype.forEach ( callbackfn [ , thisArg ] ) (§23.1.3.15).
fn for_each<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    each(cx, this, args, Walk::ForEach, "forEach")
}

/// Array.prototype.includes ( searchElement [ , fromIndex ] ) (§23.1.3.16).
fn includes<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, return false.
    if len == 0 {
        return Ok(Local::boolean(false));
    }
    // 4. Let n be ? ToIntegerOrInfinity(fromIndex).
    // 5. Assert: If fromIndex is undefined, then n is 0.
    let n = cx.to_integer_or_infinity(args.get(1))?;
    // 6. If n = +∞, return false.
    // 7. Else if n = -∞, set n to 0.
    // 8-9. k = n if n ≥ 0, else max(len + n, 0).
    let k = relative_index(cx, Local::number(n), len, 0)?;
    // 10. Repeat, while k < len,
    for k in k..len {
        cx.poll_interrupt()?;
        // a. Let elementK be ? Get(O, ! ToString(𝔽(k))).
        let element_k = cx.get_index(o, k)?;
        // b. If SameValueZero(searchElement, elementK) is true, return true.
        if same_value_zero(cx, search_element, element_k) {
            return Ok(Local::boolean(true));
        }
        // c. Set k to k + 1.
    }
    // 11. Return false.
    Ok(Local::boolean(false))
}

/// Array.prototype.lastIndexOf ( searchElement [ , fromIndex ] ) (§23.1.3.20).
fn last_index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, return -1𝔽.
    if len == 0 {
        return Ok(Local::int32(-1));
    }
    // 4. If fromIndex is present, let n be ? ToIntegerOrInfinity(fromIndex); else let n be
    //    len - 1.
    let n = if args.len() > 1 {
        cx.to_integer_or_infinity(args.get(1))?
    } else {
        (len - 1) as f64
    };
    // 5. If n = -∞, return -1𝔽.
    if n == f64::NEG_INFINITY {
        return Ok(Local::int32(-1));
    }
    // 6. If n ≥ 0, then let k be min(n, len - 1).
    // 7. Else, let k be len + n.
    let k = if n >= 0.0 {
        n.min((len - 1) as f64)
    } else {
        len as f64 + n
    };
    if k < 0.0 {
        return Ok(Local::int32(-1));
    }
    // 8. Repeat, while k ≥ 0,
    let mut k = k as u64;
    loop {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let elementK be ? Get(O, Pk).
            let element_k = cx.get_index(o, k)?;
            // ii. If IsStrictlyEqual(searchElement, elementK) is true, return 𝔽(k).
            if cx.is_strictly_equal(search_element, element_k) {
                return Ok(number(k));
            }
        }
        // d. Set k to k - 1.
        if k == 0 {
            break;
        }
        k -= 1;
    }
    // 9. Return -1𝔽.
    Ok(Local::int32(-1))
}

/// Array.prototype.map ( callbackfn [ , thisArg ] ) (§23.1.3.21).
fn map<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), "map")?;
    let this_arg = args.get(1);
    // 4. Let A be ? ArraySpeciesCreate(O, len).
    let a = array_species_create(cx, o, len)?;
    // 5. Let k be 0.
    // 6. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let kValue be ? Get(O, Pk).
            let k_value = cx.get_index(o, k)?;
            // ii. Let mappedValue be ? Call(callbackfn, thisArg, « kValue, 𝔽(k), O »).
            let mapped_value = cx.call(f, this_arg, &[k_value, number(k), o.value()])?;
            // iii. Perform ? CreateDataPropertyOrThrow(A, Pk, mappedValue).
            create_index(cx, a, k, mapped_value)?;
        }
        // d. Set k to k + 1.
    }
    // 7. Return A.
    Ok(a.value())
}

/// Array.prototype.reduce and reduceRight ( callbackfn [ , initialValue ] ) (§23.1.3.24,
/// §23.1.3.25): the same steps in the two directions.
fn reduce_with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    direction: Direction,
    method: &str,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If IsCallable(callbackfn) is false, throw a TypeError exception.
    let f = callback(cx, args.get(0), method)?;
    // 4. If len = 0 and initialValue is not present, throw a TypeError exception.
    let has_initial = args.len() >= 2;
    if len == 0 && !has_initial {
        let msg =
            format!("Array.prototype.{method}: reduce of an empty array with no initial value");
        return Err(cx.throw_type_error(&msg));
    }
    // The index of step i of the walk: k = 0, 1, ... (reduce) or len - 1, len - 2, ...
    // (reduceRight).
    let index = |i: u64| match direction {
        Direction::Ascending => i,
        Direction::Descending => len - 1 - i,
    };
    // 5. Let k be 0 (reduceRight: len - 1).
    let mut i = 0;
    // 6. Let accumulator be undefined.
    // 7. If initialValue is present, then
    let mut accumulator = if has_initial {
        // a. Set accumulator to initialValue.
        args.get(1)
    } else {
        // 8. Else,
        // a. Let kPresent be false.
        // b. Repeat, while kPresent is false and k < len (reduceRight: k ≥ 0),
        let mut found = None;
        while found.is_none() && i < len {
            cx.poll_interrupt()?;
            let k = index(i);
            // i. Let Pk be ! ToString(𝔽(k)).
            // ii. Set kPresent to ? HasProperty(O, Pk).
            // iii. If kPresent is true, then
            if cx.has_index(o, k)? {
                // 1. Set accumulator to ? Get(O, Pk).
                found = Some(cx.get_index(o, k)?);
            }
            // iv. Set k to k + 1 (reduceRight: k - 1).
            i += 1;
        }
        // c. If kPresent is false, throw a TypeError exception.
        match found {
            Some(v) => v,
            None => {
                let msg = format!(
                    "Array.prototype.{method}: reduce of an empty array with no initial value"
                );
                return Err(cx.throw_type_error(&msg));
            }
        }
    };
    // 9. Repeat, while k < len (reduceRight: k ≥ 0),
    while i < len {
        cx.poll_interrupt()?;
        let k = index(i);
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let kValue be ? Get(O, Pk).
            let k_value = cx.get_index(o, k)?;
            // ii. Set accumulator to ? Call(callbackfn, undefined, « accumulator, kValue,
            //     𝔽(k), O »).
            accumulator = cx.call(
                f,
                Local::undefined(),
                &[accumulator, k_value, number(k), o.value()],
            )?;
        }
        // d. Set k to k + 1 (reduceRight: k - 1).
        i += 1;
    }
    // 10. Return accumulator.
    Ok(accumulator)
}

/// Array.prototype.reduce ( callbackfn [ , initialValue ] ) (§23.1.3.24).
fn reduce<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    reduce_with(cx, this, args, Direction::Ascending, "reduce")
}

/// Array.prototype.reduceRight ( callbackfn [ , initialValue ] ) (§23.1.3.25).
fn reduce_right<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    reduce_with(cx, this, args, Direction::Descending, "reduceRight")
}

/// Array.prototype.reverse ( ) (§23.1.3.26).
fn reverse<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. Let middle be floor(len / 2).
    let middle = len / 2;
    // 4. Let lower be 0.
    // 5. Repeat, while lower ≠ middle,
    for lower in 0..middle {
        cx.poll_interrupt()?;
        // a. Let upper be len - lower - 1.
        let upper = len - lower - 1;
        // b. Let upperP be ! ToString(𝔽(upper)).
        // c. Let lowerP be ! ToString(𝔽(lower)).
        // d. Let lowerExists be ? HasProperty(O, lowerP).
        let lower_exists = cx.has_index(o, lower)?;
        // e. If lowerExists is true, then let lowerValue be ? Get(O, lowerP).
        let lower_value = if lower_exists {
            cx.get_index(o, lower)?
        } else {
            Local::undefined()
        };
        // f. Let upperExists be ? HasProperty(O, upperP).
        let upper_exists = cx.has_index(o, upper)?;
        // g. If upperExists is true, then let upperValue be ? Get(O, upperP).
        let upper_value = if upper_exists {
            cx.get_index(o, upper)?
        } else {
            Local::undefined()
        };
        match (lower_exists, upper_exists) {
            // h. If lowerExists is true and upperExists is true, then
            (true, true) => {
                // i. Perform ? Set(O, lowerP, upperValue, true).
                cx.set_index(o, lower, upper_value, true)?;
                // ii. Perform ? Set(O, upperP, lowerValue, true).
                cx.set_index(o, upper, lower_value, true)?;
            }
            // i. Else if lowerExists is false and upperExists is true, then
            (false, true) => {
                // i. Perform ? Set(O, lowerP, upperValue, true).
                cx.set_index(o, lower, upper_value, true)?;
                // ii. Perform ? DeletePropertyOrThrow(O, upperP).
                cx.delete_index(o, upper, true)?;
            }
            // j. Else if lowerExists is true and upperExists is false, then
            (true, false) => {
                // i. Perform ? DeletePropertyOrThrow(O, lowerP).
                cx.delete_index(o, lower, true)?;
                // ii. Perform ? Set(O, upperP, lowerValue, true).
                cx.set_index(o, upper, lower_value, true)?;
            }
            // k. Else, no action is required.
            (false, false) => {}
        }
        // l. Set lower to lower + 1.
    }
    // 6. Return O.
    Ok(o.value())
}

/// Array.prototype.shift ( ) (§23.1.3.27).
fn shift<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, then
    if len == 0 {
        // a. Perform ? Set(O, "length", +0𝔽, true).
        set_length(cx, o, 0)?;
        // b. Return undefined.
        return Ok(Local::undefined());
    }
    // 4. Let first be ? Get(O, "0").
    let first = cx.get_index(o, 0)?;
    // 5. Let k be 1.
    // 6. Repeat, while k < len,
    for k in 1..len {
        cx.poll_interrupt()?;
        // a. Let from be ! ToString(𝔽(k)).
        // b. Let to be ! ToString(𝔽(k - 1)).
        // c. Let fromPresent be ? HasProperty(O, from).
        // d. If fromPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let fromVal be ? Get(O, from).
            let from_val = cx.get_index(o, k)?;
            // ii. Perform ? Set(O, to, fromVal, true).
            cx.set_index(o, k - 1, from_val, true)?;
        } else {
            // e. Else,
            // i. Assert: fromPresent is false.
            // ii. Perform ? DeletePropertyOrThrow(O, to).
            cx.delete_index(o, k - 1, true)?;
        }
        // f. Set k to k + 1.
    }
    // 7. Perform ? DeletePropertyOrThrow(O, ! ToString(𝔽(len - 1))).
    cx.delete_index(o, len - 1, true)?;
    // 8. Perform ? Set(O, "length", 𝔽(len - 1), true).
    set_length(cx, o, len - 1)?;
    // 9. Return first.
    Ok(first)
}

/// Array.prototype.slice ( start, end ) (§23.1.3.28).
fn slice<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3-5. Let relativeStart be ? ToIntegerOrInfinity(start); k from it.
    let mut k = relative_index(cx, args.get(0), len, 0)?;
    // 6-8. If end is undefined, let relativeEnd be len; final from it.
    let final_ = relative_index(cx, args.get(1), len, len)?;
    // 9. Let count be max(final - k, 0).
    let count = final_.saturating_sub(k);
    // 10. Let A be ? ArraySpeciesCreate(O, count).
    let a = array_species_create(cx, o, count)?;
    // 11. Let n be 0.
    let mut n = 0;
    // 12. Repeat, while k < final,
    while k < final_ {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let kValue be ? Get(O, Pk).
            let k_value = cx.get_index(o, k)?;
            // ii. Perform ? CreateDataPropertyOrThrow(A, ! ToString(𝔽(n)), kValue).
            create_index(cx, a, n, k_value)?;
        }
        // d. Set k to k + 1.
        k += 1;
        // e. Set n to n + 1.
        n += 1;
    }
    // 13. Perform ? Set(A, "length", 𝔽(n), true).
    set_length(cx, a, n)?;
    // 14. Return A.
    Ok(a.value())
}

/// Array.prototype.some ( callbackfn [ , thisArg ] ) (§23.1.3.29).
fn some<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    each(cx, this, args, Walk::Some, "some")
}

/// Array.prototype.sort ( comparefn ) (§23.1.3.30).
fn sort<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If comparefn is not undefined and IsCallable(comparefn) is false, throw a
    //    TypeError exception.
    let comparefn = compare_fn(cx, args.get(0), "sort")?;
    // 2. Let obj be ? ToObject(this value).
    let obj = cx.to_object(this)?;
    // 3. Let len be ? LengthOfArrayLike(obj).
    let len = length_of_array_like(cx, obj)?;
    // 4. Let SortCompare be a new Abstract Closure ... (sorted_order).
    // 5. Let sortedList be ? SortIndexedProperties(obj, len, SortCompare, skip-holes).
    let items = cx.scratch_list();
    collect_items(cx, obj, len, Holes::SkipHoles, &items)?;
    let order = sorted_order(cx, &items, comparefn)?;
    // 6. Let itemCount be the number of elements in sortedList.
    let item_count = order.len() as u64;
    // 7. Let j be 0.
    // 8. Repeat, while j < itemCount,
    for (j, &i) in order.iter().enumerate() {
        cx.poll_interrupt()?;
        // a. Perform ? Set(obj, ! ToString(𝔽(j)), sortedList[j], true).
        cx.set_index(obj, j as u64, items.get(i), true)?;
        // b. Set j to j + 1.
    }
    // 9. NOTE: The call to SortIndexedProperties in step 5 uses skip-holes. The remaining
    //    indices are deleted to preserve the number of holes that were detected and excluded
    //    from the sort.
    // 10. Repeat, while j < len,
    for j in item_count..len {
        cx.poll_interrupt()?;
        // a. Perform ? DeletePropertyOrThrow(obj, ! ToString(𝔽(j))).
        cx.delete_index(obj, j, true)?;
        // b. Set j to j + 1.
    }
    // 11. Return obj.
    Ok(obj.value())
}

/// The actual start and skip count of `splice` and `toSpliced` (§23.1.3.31 steps 3-10,
/// §23.1.3.35 steps 3-10).
fn splice_range<'s>(cx: &mut Cx<'s>, args: Args<'s>, len: u64) -> JsResult<(u64, u64)> {
    // 3-6. Let relativeStart be ? ToIntegerOrInfinity(start); actualStart from it.
    let actual_start = relative_index(cx, args.get(0), len, 0)?;
    let actual_skip = match args.len() {
        // 8. If start is not present, then let actualSkipCount be 0.
        0 => 0,
        // 9. Else if skipCount is not present, then let actualSkipCount be len - actualStart.
        1 => len - actual_start,
        // 10. Else,
        _ => {
            // a. Let sc be ? ToIntegerOrInfinity(skipCount).
            let sc = cx.to_integer_or_infinity(args.get(1))?;
            // b. Let actualSkipCount be the result of clamping sc between 0 and
            //    len - actualStart.
            sc.clamp(0.0, (len - actual_start) as f64) as u64
        }
    };
    Ok((actual_start, actual_skip))
}

/// Array.prototype.splice ( start, deleteCount, ...items ) (§23.1.3.31).
fn splice<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3-10. actualStart, actualDeleteCount.
    let (actual_start, actual_delete_count) = splice_range(cx, args, len)?;
    // 7. Let itemCount be the number of elements in items.
    let items = args.rest(2);
    let item_count = items.len() as u64;
    // 11. If len + itemCount - actualDeleteCount > 2^53 - 1, throw a TypeError exception.
    if len + item_count - actual_delete_count > MAX_SAFE_INTEGER {
        return Err(cx.throw_type_error("Array.prototype.splice: the result is too long"));
    }
    // 12. Let A be ? ArraySpeciesCreate(O, actualDeleteCount).
    let a = array_species_create(cx, o, actual_delete_count)?;
    // 13. Let k be 0.
    // 14. Repeat, while k < actualDeleteCount,
    for k in 0..actual_delete_count {
        cx.poll_interrupt()?;
        // a. Let from be ! ToString(𝔽(actualStart + k)).
        let from = actual_start + k;
        // b. If ? HasProperty(O, from) is true, then
        if cx.has_index(o, from)? {
            // i. Let fromValue be ? Get(O, from).
            let from_value = cx.get_index(o, from)?;
            // ii. Perform ? CreateDataPropertyOrThrow(A, ! ToString(𝔽(k)), fromValue).
            create_index(cx, a, k, from_value)?;
        }
        // c. Set k to k + 1.
    }
    // 15. Perform ? Set(A, "length", 𝔽(actualDeleteCount), true).
    set_length(cx, a, actual_delete_count)?;
    // 16. If itemCount < actualDeleteCount, then
    if item_count < actual_delete_count {
        // a. Set k to actualStart.
        // b. Repeat, while k < (len - actualDeleteCount),
        for k in actual_start..len - actual_delete_count {
            cx.poll_interrupt()?;
            // i. Let from be ! ToString(𝔽(k + actualDeleteCount)).
            // ii. Let to be ! ToString(𝔽(k + itemCount)).
            let (from, to) = (k + actual_delete_count, k + item_count);
            move_or_delete(cx, o, from, to)?;
            // v. Set k to k + 1.
        }
        // c. Set k to len.
        // d. Repeat, while k > (len - actualDeleteCount + itemCount),
        let mut k = len;
        while k > len - actual_delete_count + item_count {
            cx.poll_interrupt()?;
            // i. Perform ? DeletePropertyOrThrow(O, ! ToString(𝔽(k - 1))).
            cx.delete_index(o, k - 1, true)?;
            // ii. Set k to k - 1.
            k -= 1;
        }
    } else if item_count > actual_delete_count {
        // 17. Else if itemCount > actualDeleteCount, then
        // a. Set k to (len - actualDeleteCount).
        // b. Repeat, while k > actualStart,
        let mut k = len - actual_delete_count;
        while k > actual_start {
            cx.poll_interrupt()?;
            // i. Let from be ! ToString(𝔽(k + actualDeleteCount - 1)).
            // ii. Let to be ! ToString(𝔽(k + itemCount - 1)).
            let (from, to) = (k + actual_delete_count - 1, k + item_count - 1);
            move_or_delete(cx, o, from, to)?;
            // v. Set k to k - 1.
            k -= 1;
        }
    }
    // 18. Set k to actualStart.
    // 19. For each element E of items, do
    for (i, e) in items.iter().enumerate() {
        // a. Perform ? Set(O, ! ToString(𝔽(k)), E, true).
        cx.set_index(o, actual_start + i as u64, e, true)?;
        // b. Set k to k + 1.
    }
    // 20. Perform ? Set(O, "length", 𝔽(len - actualDeleteCount + itemCount), true).
    set_length(cx, o, len - actual_delete_count + item_count)?;
    // 21. Return A.
    Ok(a.value())
}

/// The move step of `splice` and `unshift` (for example §23.1.3.31 step 16.b.iii-iv): if
/// `from` is present, Set(O, to, ? Get(O, from), true); else DeletePropertyOrThrow(O, to).
fn move_or_delete<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, from: u64, to: u64) -> JsResult<()> {
    // iii. If ? HasProperty(O, from) is true, then
    if cx.has_index(o, from)? {
        // 1. Let fromValue be ? Get(O, from).
        let from_value = cx.get_index(o, from)?;
        // 2. Perform ? Set(O, to, fromValue, true).
        cx.set_index(o, to, from_value, true)?;
    } else {
        // iv. Else,
        // 1. Perform ? DeletePropertyOrThrow(O, to).
        cx.delete_index(o, to, true)?;
    }
    Ok(())
}

/// Array.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] ) (§23.1.3.32, the
/// ECMA-262 form without ECMA-402).
fn to_locale_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let array be ? ToObject(this value).
    let array = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(array).
    let len = length_of_array_like(cx, array)?;
    // 3. Let separator be the implementation-defined list-separator String appropriate for
    //    the host environment's current locale (such as ", "). (v0: "," as V8.)
    let separator = u16::from(b',');
    // 4. Let R be the empty String. (Code units; strings are made once, at the end.)
    let mut r: Vec<u16> = Vec::new();
    // 5. Let k be 0.
    // 6. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. If k > 0, set R to the string-concatenation of R and separator.
        if k > 0 {
            r.push(separator);
        }
        // b. Let element be ? Get(array, ! ToString(𝔽(k))).
        let element = cx.get_index(array, k)?;
        // c. If element is neither undefined nor null, then
        if !element.is_undefined_or_null() {
            // i. Let S be ? ToString(? Invoke(element, "toLocaleString")).
            let f = cx.get_v(element, "toLocaleString")?;
            if !cx.is_callable(f) {
                return Err(cx.throw_type_error("toLocaleString is not a function"));
            }
            let s = cx.call(f, element, &[])?;
            let s = cx.to_string(s)?;
            let s = cx.flatten(s)?;
            // ii. Set R to the string-concatenation of R and S.
            let mut i = 0;
            while let Some(u) = cx.code_unit_at(s, i) {
                r.push(u);
                i += 1;
            }
        }
        // The string length limit: `new_string_utf16` throws the RangeError.
        if r.len() > 1 << 30 {
            cx.new_string_utf16(&r)?;
        }
        // d. Set k to k + 1.
    }
    // 7. Return R.
    Ok(cx.new_string_utf16(&r)?.value())
}

/// Array.prototype.toReversed ( ) (§23.1.3.33).
fn to_reversed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. Let A be ? ArrayCreate(len).
    let a = array_create(cx, len)?;
    // 4. Let k be 0.
    // 5. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let from be ! ToString(𝔽(len - k - 1)).
        // b. Let Pk be ! ToString(𝔽(k)).
        // c. Let fromValue be ? Get(O, from).
        let from_value = cx.get_index(o, len - k - 1)?;
        // d. Perform ! CreateDataPropertyOrThrow(A, Pk, fromValue).
        create_index(cx, a, k, from_value)?;
        // e. Set k to k + 1.
    }
    // 6. Return A.
    Ok(a.value())
}

/// Array.prototype.toSorted ( comparefn ) (§23.1.3.34).
fn to_sorted<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If comparefn is not undefined and IsCallable(comparefn) is false, throw a
    //    TypeError exception.
    let comparefn = compare_fn(cx, args.get(0), "toSorted")?;
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 4. Let A be ? ArrayCreate(len).
    let a = array_create(cx, len)?;
    // 5. Let SortCompare be a new Abstract Closure ... (sorted_order).
    // 6. Let sortedList be ? SortIndexedProperties(O, len, SortCompare, read-through-holes).
    let items = cx.scratch_list();
    collect_items(cx, o, len, Holes::ReadThroughHoles, &items)?;
    let order = sorted_order(cx, &items, comparefn)?;
    // 7. Let j be 0.
    // 8. Repeat, while j < len,
    for (j, &i) in order.iter().enumerate() {
        cx.poll_interrupt()?;
        // a. Perform ! CreateDataPropertyOrThrow(A, ! ToString(𝔽(j)), sortedList[j]).
        create_index(cx, a, j as u64, items.get(i))?;
        // b. Set j to j + 1.
    }
    // 9. Return A.
    Ok(a.value())
}

/// Array.prototype.toSpliced ( start, skipCount, ...items ) (§23.1.3.35).
fn to_spliced<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3-10. actualStart, actualSkipCount.
    let (actual_start, actual_skip_count) = splice_range(cx, args, len)?;
    // 7. Let insertCount be the number of elements in items.
    let items = args.rest(2);
    let insert_count = items.len() as u64;
    // 11. Let newLen be len + insertCount - actualSkipCount.
    let new_len = len + insert_count - actual_skip_count;
    // 12. If newLen > 2^53 - 1, throw a TypeError exception.
    if new_len > MAX_SAFE_INTEGER {
        return Err(cx.throw_type_error("Array.prototype.toSpliced: the result is too long"));
    }
    // 13. Let A be ? ArrayCreate(newLen).
    let a = array_create(cx, new_len)?;
    // 14. Let i be 0.
    // 15. Let r be actualStart + actualSkipCount.
    let mut r = actual_start + actual_skip_count;
    // 16. Repeat, while i < actualStart,
    for i in 0..actual_start {
        cx.poll_interrupt()?;
        // a. Let Pi be ! ToString(𝔽(i)).
        // b. Let iValue be ? Get(O, Pi).
        let i_value = cx.get_index(o, i)?;
        // c. Perform ! CreateDataPropertyOrThrow(A, Pi, iValue).
        create_index(cx, a, i, i_value)?;
        // d. Set i to i + 1.
    }
    let mut i = actual_start;
    // 17. For each element E of items, do
    for e in items.iter() {
        // a. Let Pi be ! ToString(𝔽(i)).
        // b. Perform ! CreateDataPropertyOrThrow(A, Pi, E).
        create_index(cx, a, i, e)?;
        // c. Set i to i + 1.
        i += 1;
    }
    // 18. Repeat, while i < newLen,
    while i < new_len {
        cx.poll_interrupt()?;
        // a. Let Pi be ! ToString(𝔽(i)).
        // b. Let from be ! ToString(𝔽(r)).
        // c. Let fromValue be ? Get(O, from).
        let from_value = cx.get_index(o, r)?;
        // d. Perform ! CreateDataPropertyOrThrow(A, Pi, fromValue).
        create_index(cx, a, i, from_value)?;
        // e. Set i to i + 1.
        i += 1;
        // f. Set r to r + 1.
        r += 1;
    }
    // 19. Return A.
    Ok(a.value())
}

/// Array.prototype.unshift ( ...items ) (§23.1.3.37).
fn unshift<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. Let argCount be the number of elements in items.
    let arg_count = args.len() as u64;
    // 4. If argCount > 0, then
    if arg_count > 0 {
        // a. If len + argCount > 2^53 - 1, throw a TypeError exception.
        if len + arg_count > MAX_SAFE_INTEGER {
            return Err(cx.throw_type_error("Array.prototype.unshift: the result is too long"));
        }
        // b. Let k be len.
        // c. Repeat, while k > 0,
        let mut k = len;
        while k > 0 {
            cx.poll_interrupt()?;
            // i. Let from be ! ToString(𝔽(k - 1)).
            // ii. Let to be ! ToString(𝔽(k + argCount - 1)).
            // iii-iv. Move or delete.
            move_or_delete(cx, o, k - 1, k + arg_count - 1)?;
            // v. Set k to k - 1.
            k -= 1;
        }
        // d. Let j be +0𝔽.
        // e. For each element E of items, do
        for (j, e) in args.iter().enumerate() {
            // i. Perform ? Set(O, ! ToString(j), E, true).
            cx.set_index(o, j as u64, e, true)?;
            // ii. Set j to j + 1𝔽.
        }
    }
    // 5. Perform ? Set(O, "length", 𝔽(len + argCount), true).
    set_length(cx, o, len + arg_count)?;
    // 6. Return 𝔽(len + argCount).
    Ok(number(len + arg_count))
}

/// Array.prototype.with ( index, value ) (§23.1.3.39).
fn with<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let value = args.get(1);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. Let relativeIndex be ? ToIntegerOrInfinity(index).
    let relative_index = cx.to_integer_or_infinity(args.get(0))?;
    // 4. If relativeIndex ≥ 0, let actualIndex be relativeIndex.
    // 5. Else, let actualIndex be len + relativeIndex.
    let actual_index = if relative_index >= 0.0 {
        relative_index
    } else {
        len as f64 + relative_index
    };
    // 6. If actualIndex ≥ len or actualIndex < 0, throw a RangeError exception.
    if actual_index >= len as f64 || actual_index < 0.0 {
        return Err(cx.throw_range_error("Invalid index"));
    }
    let actual_index = actual_index as u64;
    // 7. Let A be ? ArrayCreate(len).
    let a = array_create(cx, len)?;
    // 8. Let k be 0.
    // 9. Repeat, while k < len,
    for k in 0..len {
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)).
        // b. If k = actualIndex, let fromValue be value.
        // c. Else, let fromValue be ? Get(O, Pk).
        let from_value = if k == actual_index {
            value
        } else {
            cx.get_index(o, k)?
        };
        // d. Perform ! CreateDataPropertyOrThrow(A, Pk, fromValue).
        create_index(cx, a, k, from_value)?;
        // e. Set k to k + 1.
    }
    // 10. Return A.
    Ok(a.value())
}

/// Array.prototype.pop ( ) (§23.1.3.22).
fn pop<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, then
    if len == 0 {
        // a. Perform ? Set(O, "length", +0𝔽, true).
        cx.set(o, "length", Local::int32(0), true)?;
        // b. Return undefined.
        return Ok(Local::undefined());
    }
    // 4. Else,
    //    a. Assert: len > 0.
    //    b. Let newLen be 𝔽(len - 1).
    let new_len = len - 1;
    //    c. Let index be ! ToString(newLen). (`get_index` and `delete_index` take newLen.)
    //    d. Let element be ? Get(O, index).
    let element = cx.get_index(o, new_len)?;
    //    e. Perform ? DeletePropertyOrThrow(O, index).
    cx.delete_index(o, new_len, true)?;
    //    f. Perform ? Set(O, "length", newLen, true).
    cx.set(o, "length", Local::number(new_len as f64), true)?;
    //    g. Return element.
    Ok(element)
}

/// Array.prototype.indexOf ( searchElement [ , fromIndex ] ) (§23.1.3.17).
fn index_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let search_element = args.get(0);
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 2. Let len be ? LengthOfArrayLike(O).
    let len = length_of_array_like(cx, o)?;
    // 3. If len = 0, return -1𝔽.
    if len == 0 {
        return Ok(Local::int32(-1));
    }
    // 4. Let n be ? ToIntegerOrInfinity(fromIndex).
    // 5. Assert: If fromIndex is undefined, then n is 0.
    let mut n = cx.to_integer_or_infinity(args.get(1))?;
    // 6. If n = +∞, return -1𝔽.
    if n == f64::INFINITY {
        return Ok(Local::int32(-1));
    }
    // 7. Else if n = -∞, set n to 0.
    if n == f64::NEG_INFINITY {
        n = 0.0;
    }
    // 8. If n ≥ 0, then
    let mut k = if n >= 0.0 {
        // a. Let k be n.
        n
    } else {
        // 9. Else,
        //    a. Let k be len + n.
        //    b. If k < 0, set k to 0.
        (len as f64 + n).max(0.0)
    } as u64;
    // 10. Repeat, while k < len,
    while k < len {
        // The loop has no back edge and a hole runs no JS: one budget unit per index, so
        // termination stops a walk to 2^53 - 1.
        cx.poll_interrupt()?;
        // a. Let Pk be ! ToString(𝔽(k)). (`has_index` and `get_index` take k: no key for an
        //    element or a missing index.)
        // b. Let kPresent be ? HasProperty(O, Pk).
        // c. If kPresent is true, then
        if cx.has_index(o, k)? {
            // i. Let elementK be ? Get(O, Pk).
            let element_k = cx.get_index(o, k)?;
            // ii. If IsStrictlyEqual(searchElement, elementK) is true, return 𝔽(k).
            if cx.is_strictly_equal(search_element, element_k) {
                return Ok(Local::number(k as f64));
            }
        }
        // d. Set k to k + 1.
        k += 1;
    }
    // 11. Return -1𝔽.
    Ok(Local::int32(-1))
}
