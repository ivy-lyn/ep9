//! Global functions (§19.2): module `global`, block 9. Targets: the global object
//! (`IntrinsicId::GlobalObject`) and `%Error%` (`IntrinsicId::Error`, for `Error.isError`).
//!
//! ```text
//! decodeURI(s), decodeURIComponent(s)   Decode (§19.2.6.6): %XX escapes, UTF-8 checked;
//!                                       decodeURI keeps the escapes of ";/?:@&=+$,#"
//! encodeURI(s), encodeURIComponent(s)   Encode (§19.2.6.5): UTF-8, %XX upper case; a lone
//!                                       surrogate is a URIError
//! isFinite(v)   ToNumber(v) is finite
//! isNaN(v)      ToNumber(v) is NaN
//! Error.isError(v)                      §20.5.2.1: v has [[ErrorData]] (Cx::is_error)
//! escape(s)     Annex B §B.2.1.1: %XX below U+0100, %uXXXX above, upper case; the ASCII
//!               word chars and "@*+-./" stay
//! unescape(s)   Annex B §B.2.1.2: %uXXXX and %XX (hex digits of either case) back to code
//!               units; any other "%" stays
//! ```
//!
//! The global value properties (`globalThis`, `Infinity`, `NaN`, `undefined`) come from the VM
//! table (natives.rs); each constructor module adds its own global binding. `eval` is a VM
//! builtin (`%eval%`). `parseFloat` and `parseInt` are definitions of the `number` module
//! (number.rs), which installs each one function object as both `Number.parseX` and the global
//! `parseX` (§21.1.2.12-13; M2-P5-PLAN-D4): here, `Number.parseInt` would land after
//! `Number.EPSILON`, out of V8's key order.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-global.txt`.

use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, ErrorKind, IntrinsicDef, IntrinsicId, JsResult, Local,
    ObjectProps, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 9;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("isFinite", 1, is_finite),   // 0
    BuiltinDef::method("isNaN", 1, is_nan),         // 1
    BuiltinDef::method("decodeURI", 1, decode_uri), // 2
    BuiltinDef::method("decodeURIComponent", 1, decode_uri_component), // 3
    BuiltinDef::method("encodeURI", 1, encode_uri), // 4
    BuiltinDef::method("encodeURIComponent", 1, encode_uri_component), // 5
    BuiltinDef::method("isError", 1, is_error),     // 6
    BuiltinDef::method("escape", 1, escape),        // 7
    BuiltinDef::method("unescape", 1, unescape),    // 8
];

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const IS_FINITE: u16 = 0;
    pub const IS_NAN: u16 = 1;
    pub const DECODE_URI: u16 = 2;
    pub const DECODE_URI_COMPONENT: u16 = 3;
    pub const ENCODE_URI: u16 = 4;
    pub const ENCODE_URI_COMPONENT: u16 = 5;
    pub const IS_ERROR: u16 = 6;
    pub const ESCAPE: u16 = 7;
    pub const UNESCAPE: u16 = 8;
}

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.2.6.1-4, §B.2.1.1-2, §19.2.2, §19.2.3 ({w, c}), in V8's order.
        props: &[
            PropDef::method(PropKey::Str("decodeURI"), def::DECODE_URI),
            PropDef::method(
                PropKey::Str("decodeURIComponent"),
                def::DECODE_URI_COMPONENT,
            ),
            PropDef::method(PropKey::Str("encodeURI"), def::ENCODE_URI),
            PropDef::method(
                PropKey::Str("encodeURIComponent"),
                def::ENCODE_URI_COMPONENT,
            ),
            PropDef::method(PropKey::Str("escape"), def::ESCAPE),
            PropDef::method(PropKey::Str("unescape"), def::UNESCAPE),
            PropDef::method(PropKey::Str("isFinite"), def::IS_FINITE),
            PropDef::method(PropKey::Str("isNaN"), def::IS_NAN),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Error,
        // §20.5.2.1 Error.isError ({w, c}): after the VM's `prototype` (V8's order).
        props: &[PropDef::method(PropKey::Str("isError"), def::IS_ERROR)],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "global",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// isFinite ( number ) (§19.2.2).
fn is_finite<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let num be ? ToNumber(number).
    let num = cx.to_number(args.get(0))?;
    // 2. If num is not finite, return false.
    // 3. Otherwise, return true.
    Ok(Local::boolean(num.is_finite()))
}

/// isNaN ( number ) (§19.2.3).
fn is_nan<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let num be ? ToNumber(number).
    let num = cx.to_number(args.get(0))?;
    // 2. If num is NaN, return true.
    // 3. Otherwise, return false.
    Ok(Local::boolean(num.is_nan()))
}

// ---- URI handling functions (§19.2.6) ----------------------------------------------------------

/// uriReserved (`;/?:@&=+$,`) and `#`: the extra unescaped set of encodeURI and the escapes
/// that decodeURI keeps.
const URI_RESERVED_AND_HASH: &[u8] = b";/?:@&=+$,#";

/// The code units of ToString(v).
fn string_units<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Vec<u16>> {
    let s = cx.to_string(v)?;
    let len = cx.string_len(s);
    let flat = cx.flatten(s)?;
    Ok((0..len)
        .map(|i| cx.code_unit_at(flat, i).expect("an index below the length"))
        .collect())
}

/// The URIError of Encode and Decode (V8's text).
fn uri_error<'s>(cx: &mut Cx<'s>) -> ep9_vm::Throw {
    cx.throw_error(ErrorKind::UriError, "URI malformed")
}

/// Encode ( string, extraUnescaped ) (§19.2.6.5) on code units: `None` for a lone
/// surrogate (the URIError).
pub fn encode(units: &[u16], extra_unescaped: &[u8]) -> Option<Vec<u16>> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    // 1. Let len be the length of string.
    // 2. Let R be the empty String.
    let mut r: Vec<u16> = Vec::with_capacity(units.len());
    // 3. Let alwaysUnescaped be the string-concatenation of the ASCII word characters and
    //    "-.!~*'()".
    // 4. Let unescapedSet be the string-concatenation of alwaysUnescaped and extraUnescaped.
    let unescaped = |c: u16| {
        u8::try_from(c).is_ok_and(|b| {
            b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) || extra_unescaped.contains(&b)
        })
    };
    // 5. Let k be 0.
    let mut k = 0;
    // 6. Repeat, while k < len,
    while k < units.len() {
        // a. Let C be the code unit at index k within string.
        let c = units[k];
        // b. If unescapedSet contains C, then
        if unescaped(c) {
            // i. Set k to k + 1.
            // ii. Set R to the string-concatenation of R and C.
            r.push(c);
            k += 1;
            continue;
        }
        // c. Else,
        //    i. Let cp be CodePointAt(string, k).
        //    ii. If cp.[[IsUnpairedSurrogate]] is true, throw a URIError exception.
        let (cp, count) = match c {
            0xD800..=0xDBFF => match units.get(k + 1) {
                Some(&t @ 0xDC00..=0xDFFF) => (
                    0x10000 + ((u32::from(c) - 0xD800) << 10) + (u32::from(t) - 0xDC00),
                    2,
                ),
                _ => return None,
            },
            0xDC00..=0xDFFF => return None,
            _ => (u32::from(c), 1),
        };
        //    iii. Set k to k + cp.[[CodeUnitCount]].
        k += count;
        //    iv. Let Octets be the List of octets resulting by applying the UTF-8
        //        transformation to cp.[[CodePoint]].
        let ch = char::from_u32(cp).expect("not a surrogate");
        let mut buf = [0u8; 4];
        //    v. For each element octet of Octets, do
        //       1. Let hex be the String representation of octet, formatted as an uppercase
        //          hexadecimal number.
        //       2. Set R to the string-concatenation of R, "%", and ! StringPad(hex, 2, "0",
        //          start).
        for &octet in ch.encode_utf8(&mut buf).as_bytes() {
            r.push(u16::from(b'%'));
            r.push(u16::from(HEX[usize::from(octet >> 4)]));
            r.push(u16::from(HEX[usize::from(octet & 0xF)]));
        }
    }
    // 7. Return R.
    Some(r)
}

/// ParseHexOctet ( string, position ) (§19.2.6.7): the two hex digits at `at`, if they are.
fn parse_hex_octet(units: &[u16], at: usize) -> Option<u8> {
    let digit = |u: u16| char::from_u32(u32::from(u))?.to_digit(16);
    let hi = digit(*units.get(at)?)?;
    let lo = digit(*units.get(at + 1)?)?;
    Some((hi * 16 + lo) as u8)
}

/// Decode ( string, preserveEscapeSet ) (§19.2.6.6) on code units: `None` for a malformed
/// escape or octet sequence (the URIError).
pub fn decode(units: &[u16], preserve_escape_set: &[u8]) -> Option<Vec<u16>> {
    // 1. Let len be the length of string.
    let len = units.len();
    // 2. Let R be the empty String.
    let mut r: Vec<u16> = Vec::with_capacity(len);
    // 3. Let k be 0.
    let mut k = 0;
    // 4. Repeat, while k < len,
    while k < len {
        // a. Let C be the code unit at index k within string.
        let c = units[k];
        // c. If C is the code unit 0x0025 (PERCENT SIGN), then
        if c != u16::from(b'%') {
            // b. Let S be C.
            r.push(c);
            k += 1;
            continue;
        }
        // i. If k + 3 > len, throw a URIError exception.
        // ii. Let escape be the substring of string from k to k + 3.
        // iii. Let B be ParseHexOctet(string, k + 1).
        // iv. If B is not an integer, throw a URIError exception.
        if k + 3 > len {
            return None;
        }
        let b = parse_hex_octet(units, k + 1)?;
        // v. Set k to k + 2.
        let start = k;
        k += 2;
        // vi. Let n be the number of leading 1 bits in B.
        let n = b.leading_ones() as usize;
        if n == 0 {
            // vii. If n = 0, then
            //      1. Let asciiChar be the code unit whose numeric value is B.
            //      2. If preserveEscapeSet contains asciiChar, let S be escape. Otherwise,
            //         let S be asciiChar.
            if preserve_escape_set.contains(&b) {
                r.extend_from_slice(&units[start..start + 3]);
            } else {
                r.push(u16::from(b));
            }
        } else {
            // viii. Else,
            //       1. If n = 1 or n > 4, throw a URIError exception.
            if n == 1 || n > 4 {
                return None;
            }
            //       2. Let Octets be « B ».
            let mut octets = [b, 0, 0, 0];
            //       3. Let j be 1.
            //       4. Repeat, while j < n,
            for octet in octets.iter_mut().take(n).skip(1) {
                // a. Set k to k + 1.
                k += 1;
                // b. If k + 3 > len, throw a URIError exception.
                // c. If the code unit at index k within string is not the code unit 0x0025
                //    (PERCENT SIGN), throw a URIError exception.
                // d. Let continuationByte be ParseHexOctet(string, k + 1).
                // e. If continuationByte is not an integer, throw a URIError exception.
                if k + 3 > len || units[k] != u16::from(b'%') {
                    return None;
                }
                *octet = parse_hex_octet(units, k + 1)?;
                // f. Append continuationByte to Octets.
                // g. Set k to k + 2.
                // h. Set j to j + 1.
                k += 2;
            }
            //       5. Assert: The length of Octets is n.
            //       6. If Octets does not contain a valid UTF-8 encoding of a Unicode code
            //          point, throw a URIError exception. (No overlong form, no surrogate,
            //          nothing above U+10FFFF: `str::from_utf8` checks exactly that.)
            //       7. Let V be the code point obtained by applying the UTF-8 transformation
            //          to Octets, that is, from a List of octets into a 21-bit value.
            //       8. Let S be UTF16EncodeCodePoint(V).
            let text = core::str::from_utf8(&octets[..n]).ok()?;
            let ch = text.chars().next().expect("n octets of one code point");
            let mut buf = [0u16; 2];
            r.extend_from_slice(ch.encode_utf16(&mut buf));
        }
        // d. Set R to the string-concatenation of R and S.
        // e. Set k to k + 1.
        k += 1;
    }
    // 5. Return R.
    Some(r)
}

/// The text of `units` as a new string, or the URIError.
fn uri_result<'s>(cx: &mut Cx<'s>, units: Option<Vec<u16>>) -> JsResult<Local<'s>> {
    match units {
        Some(u) => Ok(cx.new_string_utf16(&u)?.value()),
        None => Err(uri_error(cx)),
    }
}

/// decodeURI ( encodedURI ) (§19.2.6.1).
fn decode_uri<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let uriString be ? ToString(encodedURI).
    let units = string_units(cx, args.get(0))?;
    // 2. Let preserveEscapeSet be ";/?:@&=+$,#".
    // 3. Return ? Decode(uriString, preserveEscapeSet).
    let r = decode(&units, URI_RESERVED_AND_HASH);
    uri_result(cx, r)
}

/// decodeURIComponent ( encodedURIComponent ) (§19.2.6.2).
fn decode_uri_component<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let componentString be ? ToString(encodedURIComponent).
    let units = string_units(cx, args.get(0))?;
    // 2. Let preserveEscapeSet be the empty String.
    // 3. Return ? Decode(componentString, preserveEscapeSet).
    let r = decode(&units, b"");
    uri_result(cx, r)
}

/// encodeURI ( uri ) (§19.2.6.3).
fn encode_uri<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let uriString be ? ToString(uri).
    let units = string_units(cx, args.get(0))?;
    // 2. Let extraUnescaped be ";/?:@&=+$,#".
    // 3. Return ? Encode(uriString, extraUnescaped).
    let r = encode(&units, URI_RESERVED_AND_HASH);
    uri_result(cx, r)
}

/// encodeURIComponent ( uriComponent ) (§19.2.6.4).
fn encode_uri_component<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let componentString be ? ToString(uriComponent).
    let units = string_units(cx, args.get(0))?;
    // 2. Let extraUnescaped be the empty String.
    // 3. Return ? Encode(componentString, extraUnescaped).
    let r = encode(&units, b"");
    uri_result(cx, r)
}

// ---- escape, unescape (Annex B §B.2.1) ---------------------------------------------------------

/// The length of S for the code unit C in escape (§B.2.1.1 step 6.b-c): 1 for a code unit of
/// unescapedSet (the ASCII word characters and "@*+-./"), 3 for "%XX", 6 for "%uXXXX".
fn escaped_width(c: u16) -> usize {
    match u8::try_from(c) {
        Ok(b) if b.is_ascii_alphanumeric() || b"_@*+-./".contains(&b) => 1,
        _ if c < 256 => 3,
        _ => 6,
    }
}

/// The length of escape(`units`), so the caller can refuse a text above the String length
/// limit before it allocates.
pub fn escaped_len(units: &[u16]) -> usize {
    units.iter().map(|&c| escaped_width(c)).sum()
}

/// The code units of escape(`units`) (§B.2.1.1 steps 2-7).
pub fn escape_units(units: &[u16]) -> Vec<u16> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let hex = |n: u16, shift: u32| u16::from(HEX[usize::from((n >> shift) & 0xF)]);
    // 2. Let len be the length of string.
    // 3. Let R be the empty String.
    let mut r: Vec<u16> = Vec::with_capacity(escaped_len(units));
    // 4. Let unescapedSet be the string-concatenation of the ASCII word characters and
    //    "@*+-./" (`escaped_width` 1).
    // 5. Let k be 0.
    // 6. Repeat, while k < len,
    //    a. Let C be the code unit at index k within string.
    for &c in units {
        // b. If unescapedSet contains C, then
        //    i. Let S be C.
        if escaped_width(c) == 1 {
            r.push(c);
        // c. Else,
        //    i. Let n be the numeric value of C.
        //    ii. If n < 256, then
        //        1. Let hex be the String representation of n, formatted as an uppercase
        //           hexadecimal number.
        //        2. Let S be the string-concatenation of "%" and ! StringPad(hex, 2, "0",
        //           start).
        } else if c < 256 {
            r.extend_from_slice(&[u16::from(b'%'), hex(c, 4), hex(c, 0)]);
        //    iii. Else,
        //        1. Let hex be the String representation of n, formatted as an uppercase
        //           hexadecimal number.
        //        2. Let S be the string-concatenation of "%u" and ! StringPad(hex, 4, "0",
        //           start).
        } else {
            r.extend_from_slice(&[
                u16::from(b'%'),
                u16::from(b'u'),
                hex(c, 12),
                hex(c, 8),
                hex(c, 4),
                hex(c, 0),
            ]);
        }
        // d. Set R to the string-concatenation of R and S.
        // e. Set k to k + 1.
    }
    // 7. Return R.
    r
}

/// The code units of unescape(`units`) (§B.2.1.2 steps 2-6).
pub fn unescape_units(units: &[u16]) -> Vec<u16> {
    // ParseText(hexDigits, HexDigits[~Sep]): every code unit is a hex digit (no separator).
    let hex_value = |digits: &[u16]| {
        digits.iter().try_fold(0u16, |v, &u| {
            let d = char::from_u32(u32::from(u))?.to_digit(16)?;
            Some(v * 16 + d as u16)
        })
    };
    // 2. Let len be the length of string.
    let len = units.len();
    // 3. Let R be the empty String.
    let mut r: Vec<u16> = Vec::with_capacity(len);
    // 4. Let k be 0.
    let mut k = 0;
    // 5. Repeat, while k < len,
    while k < len {
        // a. Let C be the code unit at index k within string.
        let mut c = units[k];
        // b. If C is the code unit 0x0025 (PERCENT SIGN), then
        if c == u16::from(b'%') {
            // i. Let hexDigits be the empty String.
            // ii. Let optionalAdvance be 0.
            // iii. If k + 5 < len and the code unit at index k + 1 within string is the code
            //      unit 0x0075 (LATIN SMALL LETTER U), then
            //      1. Set hexDigits to the substring of string from k + 2 to k + 6.
            //      2. Set optionalAdvance to 5.
            // iv. Else if k + 3 ≤ len, then
            //      1. Set hexDigits to the substring of string from k + 1 to k + 3.
            //      2. Set optionalAdvance to 2.
            let (hex_digits, optional_advance) = if k + 5 < len && units[k + 1] == u16::from(b'u') {
                (&units[k + 2..k + 6], 5)
            } else if k + 3 <= len {
                (&units[k + 1..k + 3], 2)
            } else {
                (&units[..0], 0)
            };
            // v. Let parseResult be ParseText(hexDigits, HexDigits[~Sep]).
            // vi. If parseResult is a Parse Node, then
            //     1. Let n be the MV of parseResult.
            //     2. Set C to the code unit whose numeric value is n.
            //     3. Set k to k + optionalAdvance.
            if !hex_digits.is_empty()
                && let Some(n) = hex_value(hex_digits)
            {
                c = n;
                k += optional_advance;
            }
        }
        // c. Set R to the string-concatenation of R and C.
        r.push(c);
        // d. Set k to k + 1.
        k += 1;
    }
    // 6. Return R.
    r
}

/// escape ( string ) (Annex B §B.2.1.1).
fn escape<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set string to ? ToString(string).
    let units = string_units(cx, args.get(0))?;
    // A text above the String length limit is the RangeError of `new_string_utf16`, before
    // the up to 6 code units per code unit are allocated.
    if escaped_len(&units) > crate::string::MAX_LENGTH {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    let r = escape_units(&units);
    Ok(cx.new_string_utf16(&r)?.value())
}

/// unescape ( string ) (Annex B §B.2.1.2).
fn unescape<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Set string to ? ToString(string).
    let units = string_units(cx, args.get(0))?;
    let r = unescape_units(&units);
    Ok(cx.new_string_utf16(&r)?.value())
}

// ---- Error.isError (§20.5.2.1) ------------------------------------------------------------------

/// Error.isError ( arg ) (§20.5.2.1).
fn is_error<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return IsError(arg).
    // IsError ( argument ) (§20.5.8.2):
    // 1. If argument is not an Object, return false.
    // 2. If argument has an [[ErrorData]] internal slot, return true.
    // 3. Return false.
    Ok(Local::boolean(cx.is_error(args.get(0))))
}
