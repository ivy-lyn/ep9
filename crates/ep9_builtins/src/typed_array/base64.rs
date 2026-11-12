//! The base64 and hex methods of `Uint8Array` (ES2026: `sec-uint8array.frombase64`,
//! `sec-uint8array.fromhex`, `sec-uint8array.prototype.tobase64`,
//! `sec-uint8array.prototype.setfrombase64`, `sec-uint8array.prototype.tohex`,
//! `sec-uint8array.prototype.setfromhex`): module `base64`, block 37. Owner of the tables:
//! TypedArray TA-4 R1; owner of the function bodies: TypedArray TA-4 R5 B64. The module joins
//! the lazy group of `array_buffer` (TA-D14): each table entry targets `%Uint8Array%` or
//! `%Uint8Array.prototype%`.
//!
//! ```text
//! method          first steps                                  the bytes
//! fromBase64      string a String, options (alphabet,          FromBase64, then a new
//!                 lastChunkHandling)                           Uint8Array: SetValueInBuffer
//! fromHex         string a String                              FromHex, then a new Uint8Array
//! toBase64        ValidateUint8Array, options (alphabet,       GetUint8ArrayBytes (after the
//!                 omitPadding)                                 options: a getter can detach)
//! toHex           ValidateUint8Array                           GetUint8ArrayBytes
//! setFromBase64   ValidateUint8Array, string a String,         FromBase64 up to the length,
//!                 options, then the bounds check               SetUint8ArrayBytes, then the
//!                                                              error (if any), { read, written }
//! setFromHex      ValidateUint8Array, string a String, bounds  FromHex up to the length, as above
//! ```
//!
//! The decoders (`from_base64_units`, `from_hex_units`) are plain Rust over the code units
//! of the string (copied out first): they run no JS and allocate nothing on the JS heap, so
//! they do not poll (the `string` module rule). Buffer bytes are read and written only
//! through `Cx` (`get_value_from_buffer`, `set_value_in_buffer`, element type Uint8 with
//! order unordered), so a SharedArrayBuffer view takes the `ep9_vm` shared-memory accessors;
//! each loop over bytes polls interrupts once per byte. No Value lives in Rust memory: the
//! bytes and the code units are Rust lists.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-base64.txt`. The ids follow node v26's key order. A missing definition is
//! appended at the end (37.6 and on), never inserted.

use ep9_vm::abi::JsObject;
use ep9_vm::{
    Args, BuiltinDef, BuiltinModule, Cx, ErrorKind, IntrinsicId, JsResult, Local, ObjectProps,
    PropDef, PropKey,
};

use super::ctor::allocate;
use crate::string::{MAX_LENGTH, units};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 37;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("fromBase64", 1, from_base64), // 0
    BuiltinDef::method("fromHex", 1, from_hex),       // 1
    BuiltinDef::method("toBase64", 0, to_base64),     // 2
    BuiltinDef::method("setFromBase64", 1, set_from_base64), // 3
    BuiltinDef::method("toHex", 0, to_hex),           // 4
    BuiltinDef::method("setFromHex", 1, set_from_hex), // 5
];

/// `key` = the function of definition `d` ({w, c}).
const fn method(key: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(key), d)
}

/// The properties this module adds, in node v26's key order: on `%Uint8Array%` after
/// `BYTES_PER_ELEMENT` (module `typed_array` installs first), and on `%Uint8Array.prototype%`
/// after `constructor` and `BYTES_PER_ELEMENT`. All {w, c}.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Uint8Array,
        props: &[method("fromBase64", 0), method("fromHex", 1)],
    },
    ObjectProps {
        target: IntrinsicId::Uint8ArrayPrototype,
        props: &[
            method("toBase64", 2),
            method("setFromBase64", 3),
            method("toHex", 4),
            method("setFromHex", 5),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it after `typed_array_proto`).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "base64",
    block: BLOCK,
    defs: DEFS,
    intrinsics: &[],
    props: PROPS,
};

/// The row of `Uint8Array` in spec Table 71 (the `element` of the `ep9_vm` typed ABI).
const UINT8: u8 = 1;

/// The `[[LittleEndian]]` value of the agent (one byte: the order does not matter).
const LITTLE_ENDIAN: bool = cfg!(target_endian = "little");

/// The TypeError text of a receiver that went out of bounds (or was detached).
const OUT_OF_BOUNDS: &str =
    "Cannot perform this operation on a detached or out-of-bounds TypedArray";

/// The standard base64 alphabet (RFC 4648 Table 1).
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The base64url alphabet (RFC 4648 Table 2).
const BASE64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// The lower-case hex digits (Number::toString(byte, 16)).
const HEX: &[u8; 16] = b"0123456789abcdef";

// ---- the decoders (no JS, no JS heap) -------------------------------------------------------

/// The `lastChunkHandling` option.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LastChunk {
    Loose,
    Strict,
    StopBeforePartial,
}

/// The SyntaxError texts of the decoders.
mod error {
    pub const BAD_CHAR: &str = "Found a character that cannot be part of a valid base64 string";
    pub const ONE_CHAR: &str =
        "The base64 input ends with a single character, not counting padding (=)";
    pub const MISSING_PADDING: &str =
        "The base64 input ends with a partial chunk and no padding (lastChunkHandling: strict)";
    pub const EARLY_PADDING: &str =
        "The base64 input has padding (=) after fewer than 2 characters";
    pub const SHORT_PADDING: &str = "The base64 input ends with a single padding character (=)";
    pub const AFTER_PADDING: &str = "The base64 input has characters after the padding (=)";
    pub const EXTRA_BITS: &str = "The base64 input ends with non-zero padding bits";
    pub const ODD_HEX: &str = "The hex input has an odd number of characters";
    pub const BAD_HEX: &str = "Found a character that cannot be part of a valid hex string";
}

/// The result record of FromBase64 and FromHex: `[[Read]]`, `[[Bytes]]` and `[[Error]]` (the
/// text of a SyntaxError, or `None`).
#[derive(PartialEq, Eq, Debug)]
struct Decoded {
    read: usize,
    bytes: Vec<u8>,
    error: Option<&'static str>,
}

impl Decoded {
    fn ok(read: usize, bytes: Vec<u8>) -> Decoded {
        Decoded {
            read,
            bytes,
            error: None,
        }
    }

    fn err(read: usize, bytes: Vec<u8>, text: &'static str) -> Decoded {
        Decoded {
            read,
            bytes,
            error: Some(text),
        }
    }
}

/// SkipAsciiWhitespace ( string, index ): the first index at or after `index` whose code unit
/// is not U+0009, U+000A, U+000C, U+000D or U+0020 (or the length).
fn skip_ascii_whitespace(s: &[u16], mut index: usize) -> usize {
    while index < s.len() && matches!(s[index], 0x09 | 0x0A | 0x0C | 0x0D | 0x20) {
        index += 1;
    }
    index
}

/// The 6-bit value of `c` in the standard base64 alphabet, else `None`.
fn base64_value(c: u16) -> Option<u8> {
    match c {
        0x41..=0x5A => Some((c - 0x41) as u8),      // A-Z
        0x61..=0x7A => Some((c - 0x61 + 26) as u8), // a-z
        0x30..=0x39 => Some((c - 0x30 + 52) as u8), // 0-9
        0x2B => Some(62),                           // +
        0x2F => Some(63),                           // /
        _ => None,
    }
}

/// DecodeBase64Chunk ( chunk ) of a chunk of 4 characters (as 6-bit values): 3 bytes.
fn decode_chunk(chunk: [u8; 4]) -> [u8; 3] {
    let n = (u32::from(chunk[0]) << 18)
        | (u32::from(chunk[1]) << 12)
        | (u32::from(chunk[2]) << 6)
        | u32::from(chunk[3]);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

/// DecodeFinalBase64Chunk ( chunk, throwOnExtraBits ) of the first `len` (2 or 3) values of
/// `chunk`: 1 or 2 bytes, appended to `bytes`. `Err` (the extra-bits SyntaxError, `bytes`
/// unchanged) if `throw_on_extra_bits` and the bits after the last byte are not all 0.
fn decode_final_chunk(
    chunk: [u8; 4],
    len: usize,
    throw_on_extra_bits: bool,
    bytes: &mut Vec<u8>,
) -> Result<(), &'static str> {
    debug_assert!(len == 2 || len == 3);
    // 1-3. Pad the chunk with "AA" or "A" (value 0); 4. decode it.
    let mut padded = chunk;
    padded[len..].fill(0);
    let decoded = decode_chunk(padded);
    // 5.a / 6.a. If throwOnExtraBits is true and the byte after the kept ones is not 0, throw
    //            a SyntaxError (the low 4 or 2 bits of the last value are that byte's bits).
    if throw_on_extra_bits && decoded[len - 1] != 0 {
        return Err(error::EXTRA_BITS);
    }
    // 5.b / 6.b. Keep 1 or 2 bytes.
    bytes.extend_from_slice(&decoded[..len - 1]);
    Ok(())
}

/// FromBase64 ( string, alphabet, lastChunkHandling [ , maxLength ] ) on the code units `s`;
/// `base64url` is the alphabet; `max_length` is `usize::MAX` when maxLength is not present
/// (more than any byte count of a string). The step numbers are those of the spec algorithm.
fn from_base64_units(s: &[u16], base64url: bool, last: LastChunk, max_length: usize) -> Decoded {
    // 1. (maxLength not present: 2^53 - 1; here usize::MAX.)
    // 2. If maxLength = 0, return { [[Read]]: 0, [[Bytes]]: « », [[Error]]: none }.
    if max_length == 0 {
        return Decoded::ok(0, Vec::new());
    }
    // 3-8. read 0, bytes « », chunk "", chunkLength 0, index 0, length.
    let mut read = 0;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4];
    let mut chunk_length = 0;
    let mut index = 0;
    let length = s.len();
    // 9. Repeat:
    loop {
        // a. Set index to SkipAsciiWhitespace(string, index).
        index = skip_ascii_whitespace(s, index);
        // b. If index = length, then
        if index == length {
            // i. If chunkLength > 0, then
            if chunk_length > 0 {
                match last {
                    // 1. stop-before-partial: return { read, bytes, none }.
                    LastChunk::StopBeforePartial => return Decoded::ok(read, bytes),
                    // 2. loose: a. chunkLength = 1 is a SyntaxError; b. else append
                    //    DecodeFinalBase64Chunk(chunk, false).
                    LastChunk::Loose => {
                        if chunk_length == 1 {
                            return Decoded::err(read, bytes, error::ONE_CHAR);
                        }
                        decode_final_chunk(chunk, chunk_length, false, &mut bytes)
                            .expect("no extra-bits check");
                    }
                    // 3. strict: a SyntaxError.
                    LastChunk::Strict => {
                        return Decoded::err(read, bytes, error::MISSING_PADDING);
                    }
                }
            }
            // ii. Return { [[Read]]: length, [[Bytes]]: bytes, [[Error]]: none }.
            return Decoded::ok(length, bytes);
        }
        // c-d. Let char be the code unit at index; set index to index + 1.
        let mut c = s[index];
        index += 1;
        // e. If char is "=", then
        if c == u16::from(b'=') {
            // i. If chunkLength < 2, a SyntaxError.
            if chunk_length < 2 {
                return Decoded::err(read, bytes, error::EARLY_PADDING);
            }
            // ii. Set index to SkipAsciiWhitespace(string, index).
            index = skip_ascii_whitespace(s, index);
            // iii. If chunkLength = 2, then
            if chunk_length == 2 {
                // 1. If index = length: stop-before-partial returns { read, bytes, none };
                //    else a SyntaxError.
                if index == length {
                    if last == LastChunk::StopBeforePartial {
                        return Decoded::ok(read, bytes);
                    }
                    return Decoded::err(read, bytes, error::SHORT_PADDING);
                }
                // 2-3. If the code unit at index is "=", set index to
                //      SkipAsciiWhitespace(string, index + 1).
                if s[index] == u16::from(b'=') {
                    index = skip_ascii_whitespace(s, index + 1);
                }
            }
            // iv. If index < length, a SyntaxError.
            if index < length {
                return Decoded::err(read, bytes, error::AFTER_PADDING);
            }
            // v-vii. throwOnExtraBits is true for strict; append
            //        DecodeFinalBase64Chunk(chunk, throwOnExtraBits).
            let strict = last == LastChunk::Strict;
            if let Err(text) = decode_final_chunk(chunk, chunk_length, strict, &mut bytes) {
                return Decoded::err(read, bytes, text);
            }
            // viii. Return { [[Read]]: length, [[Bytes]]: bytes, [[Error]]: none }.
            return Decoded::ok(length, bytes);
        }
        // f. If alphabet is "base64url": "+" and "/" are a SyntaxError; "-" is "+", "_" is "/".
        if base64url {
            if c == u16::from(b'+') || c == u16::from(b'/') {
                return Decoded::err(read, bytes, error::BAD_CHAR);
            } else if c == u16::from(b'-') {
                c = u16::from(b'+');
            } else if c == u16::from(b'_') {
                c = u16::from(b'/');
            }
        }
        // g. If char is not in the standard base64 alphabet, a SyntaxError.
        let Some(value) = base64_value(c) else {
            return Decoded::err(read, bytes, error::BAD_CHAR);
        };
        // h. Let remaining be maxLength - the length of bytes.
        let remaining = max_length - bytes.len();
        // i. If remaining = 1 and chunkLength = 2, or remaining = 2 and chunkLength = 3,
        //    return { read, bytes, none } (only with a maxLength).
        if (remaining == 1 && chunk_length == 2) || (remaining == 2 && chunk_length == 3) {
            return Decoded::ok(read, bytes);
        }
        // j-k. Append char to chunk.
        chunk[chunk_length] = value;
        chunk_length += 1;
        // l. If chunkLength = 4, then
        if chunk_length == 4 {
            // i-iii. Append DecodeBase64Chunk(chunk); chunk "", chunkLength 0.
            bytes.extend_from_slice(&decode_chunk(chunk));
            chunk_length = 0;
            // iv. Set read to index.
            read = index;
            // v. If the length of bytes = maxLength, return { read, bytes, none }.
            if bytes.len() == max_length {
                return Decoded::ok(read, bytes);
            }
        }
    }
}

/// The value of the hex digit `c` (0-9, a-f, A-F), else `None`.
fn hex_value(c: u16) -> Option<u8> {
    match c {
        0x30..=0x39 => Some((c - 0x30) as u8),
        0x61..=0x66 => Some((c - 0x61 + 10) as u8),
        0x41..=0x46 => Some((c - 0x41 + 10) as u8),
        _ => None,
    }
}

/// FromHex ( string [ , maxLength ] ) on the code units `s`; `max_length` is `usize::MAX`
/// when maxLength is not present.
fn from_hex_units(s: &[u16], max_length: usize) -> Decoded {
    // 1-4. length, bytes « », read 0.
    let length = s.len();
    let mut bytes = Vec::new();
    let mut read = 0;
    // 5. If length modulo 2 ≠ 0, return { [[Read]]: 0, [[Bytes]]: « », [[Error]]: a
    //    SyntaxError }.
    if !length.is_multiple_of(2) {
        return Decoded::err(0, bytes, error::ODD_HEX);
    }
    // 6. Repeat, while read < length and the length of bytes < maxLength:
    while read < length && bytes.len() < max_length {
        // a-b. hexits: the 2 code units at read; a code unit that is not a hex digit is a
        //      SyntaxError.
        let (Some(hi), Some(lo)) = (hex_value(s[read]), hex_value(s[read + 1])) else {
            return Decoded::err(read, bytes, error::BAD_HEX);
        };
        // c-e. read + 2; append the byte.
        read += 2;
        bytes.push(hi << 4 | lo);
    }
    // 7. Return { [[Read]]: read, [[Bytes]]: bytes, [[Error]]: none }.
    Decoded::ok(read, bytes)
}

/// The base64 text of `bytes` (RFC 4648 §4, or §5 for base64url), with `=` padding unless
/// `omit_padding`.
fn encode_base64(bytes: &[u8], base64url: bool, omit_padding: bool) -> Vec<u8> {
    let table = if base64url { BASE64_URL } else { BASE64 };
    let mut out = Vec::with_capacity(base64_length(bytes.len(), omit_padding));
    let (chunks, rest) = bytes.as_chunks::<3>();
    for c in chunks {
        let n = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
        for shift in [18, 12, 6, 0] {
            out.push(table[(n >> shift) as usize & 63]);
        }
    }
    if !rest.is_empty() {
        let n = (u32::from(rest[0]) << 16) | rest.get(1).map_or(0, |&b| u32::from(b) << 8);
        // 1 byte gives 2 characters, 2 bytes give 3.
        for shift in [18, 12, 6].into_iter().take(rest.len() + 1) {
            out.push(table[(n >> shift) as usize & 63]);
        }
        if !omit_padding {
            out.resize(out.len() + 3 - rest.len(), b'=');
        }
    }
    out
}

/// The length of the base64 text of `n` bytes.
fn base64_length(n: usize, omit_padding: bool) -> usize {
    let full = n / 3 * 4;
    match n % 3 {
        0 => full,
        r if omit_padding => full + r + 1,
        _ => full + 4,
    }
}

// ---- the abstract operations on the JS side ---------------------------------------------------

/// ValidateUint8Array ( ta ): `this` if it is a TypedArray whose `[[TypedArrayName]]` is
/// "Uint8Array", else a TypeError naming `what`.
fn validate_uint8_array<'s>(
    cx: &mut Cx<'s>,
    ta: Local<'s>,
    what: &str,
) -> JsResult<Local<'s, JsObject>> {
    // 1. Perform ? RequireInternalSlot(ta, [[TypedArrayName]]).
    // 2. If ta.[[TypedArrayName]] is not "Uint8Array", throw a TypeError exception.
    match (cx.typed_array_element(ta), ta.as_object()) {
        (Some(UINT8), Some(o)) => Ok(o),
        _ => Err(crate::array_buffer::incompatible(cx, what)),
    }
}

/// The code units of `string` if it is a String, else the TypeError of step "If string is not
/// a String, throw a TypeError exception".
fn string_units<'s>(cx: &mut Cx<'s>, string: Local<'s>) -> JsResult<Vec<u16>> {
    match string.as_string() {
        Some(s) => units(cx, s),
        None => Err(cx.throw_type_error("The input argument must be a string")),
    }
}

/// GetOptionsObject ( options ): undefined gives OrdinaryObjectCreate(null); an Object is
/// itself; any other value is a TypeError.
fn get_options_object<'s>(cx: &mut Cx<'s>, options: Local<'s>) -> JsResult<Local<'s, JsObject>> {
    // 1. If options is undefined, return OrdinaryObjectCreate(null).
    if options.is_undefined() {
        return Ok(cx.new_object_with_proto(Local::null()));
    }
    // 2. If options is an Object, return options.
    if let Some(o) = options.as_object() {
        return Ok(o);
    }
    // 3. Throw a TypeError exception.
    Err(cx.throw_type_error("The options argument must be an object"))
}

/// Is `v` the String `text` (ASCII; no ToString: a String object or any other value is not)?
fn is_string_value<'s>(cx: &mut Cx<'s>, v: Local<'s>, text: &str) -> JsResult<bool> {
    let Some(s) = v.as_string() else {
        return Ok(false);
    };
    if cx.string_len(s) != text.len() {
        return Ok(false);
    }
    Ok(units(cx, s)?
        .iter()
        .copied()
        .eq(text.bytes().map(u16::from)))
}

/// The `alphabet` option: `? Get(opts, "alphabet")`; undefined is "base64"; a value that is
/// neither "base64" nor "base64url" is a TypeError. True for "base64url".
fn alphabet_option<'s>(cx: &mut Cx<'s>, opts: Local<'s, JsObject>) -> JsResult<bool> {
    let alphabet = cx.get(opts, "alphabet")?;
    if alphabet.is_undefined() || is_string_value(cx, alphabet, "base64")? {
        Ok(false)
    } else if is_string_value(cx, alphabet, "base64url")? {
        Ok(true)
    } else {
        Err(cx.throw_type_error("The alphabet option must be \"base64\" or \"base64url\""))
    }
}

/// The `lastChunkHandling` option: `? Get(opts, "lastChunkHandling")`; undefined is "loose";
/// a value that is not one of "loose", "strict" or "stop-before-partial" is a TypeError.
fn last_chunk_option<'s>(cx: &mut Cx<'s>, opts: Local<'s, JsObject>) -> JsResult<LastChunk> {
    let v = cx.get(opts, "lastChunkHandling")?;
    if v.is_undefined() || is_string_value(cx, v, "loose")? {
        Ok(LastChunk::Loose)
    } else if is_string_value(cx, v, "strict")? {
        Ok(LastChunk::Strict)
    } else if is_string_value(cx, v, "stop-before-partial")? {
        Ok(LastChunk::StopBeforePartial)
    } else {
        Err(cx.throw_type_error(
            "The lastChunkHandling option must be \"loose\", \"strict\" or \"stop-before-partial\"",
        ))
    }
}

/// GetUint8ArrayBytes ( ta ): the bytes of the Uint8Array `ta`, read with
/// GetValueFromBuffer(buffer, byteOffset + index, uint8, true, unordered). A TypeError if
/// `ta` is out of bounds (or detached). A RangeError (before any read) if a text of
/// `text_length(len)` code units would pass the string length limit.
fn get_uint8_array_bytes<'s>(
    cx: &mut Cx<'s>,
    ta: Local<'s, JsObject>,
    text_length: impl Fn(usize) -> usize,
) -> JsResult<Vec<u8>> {
    // 1. Let buffer be ta.[[ViewedArrayBuffer]].
    let buffer = cx.typed_array_buffer(ta)?;
    // 2. Let taRecord be MakeTypedArrayWithBufferWitnessRecord(ta, seq-cst).
    // 3. If IsTypedArrayOutOfBounds(taRecord) is true, throw a TypeError exception.
    // 4. Let len be TypedArrayLength(taRecord).
    let Some(len) = cx.typed_array_length(ta)? else {
        return Err(cx.throw_type_error(OUT_OF_BOUNDS));
    };
    // The result text would pass the string length limit: the RangeError of the string,
    // raised before the bytes are read (no JS runs between, so the order is not visible).
    let len = usize::try_from(len).unwrap_or(usize::MAX);
    if len > MAX_LENGTH || text_length(len) > MAX_LENGTH {
        return Err(cx.throw_range_error("Invalid string length"));
    }
    // 5. Let byteOffset be ta.[[ByteOffset]].
    let byte_offset = cx.typed_array_byte_offset(ta)?;
    // 6-8. Read each byte (no JS runs in the loop: it polls).
    let mut bytes = Vec::with_capacity(len);
    for index in 0..len as u64 {
        cx.poll_interrupt()?;
        let byte = cx.get_value_from_buffer(buffer, byte_offset + index, UINT8, LITTLE_ENDIAN)?;
        bytes.push(byte.as_number().expect("a Uint8 element is a Number") as u8);
    }
    // 9. Return bytes.
    Ok(bytes)
}

/// SetUint8ArrayBytes ( into, bytes ): writes `bytes` from `into.[[ByteOffset]]` with
/// SetValueInBuffer(buffer, offset + index, uint8, byte, true, unordered).
fn set_uint8_array_bytes<'s>(
    cx: &mut Cx<'s>,
    into: Local<'s, JsObject>,
    bytes: &[u8],
) -> JsResult<()> {
    // 1. Let offset be into.[[ByteOffset]].
    let offset = cx.typed_array_byte_offset(into)?;
    let buffer = cx.typed_array_buffer(into)?;
    // 2-4. For each byte: SetValueInBuffer (no JS runs in the loop: it polls).
    for (index, &byte) in bytes.iter().enumerate() {
        cx.poll_interrupt()?;
        let value = Local::number(f64::from(byte));
        cx.set_value_in_buffer(buffer, offset + index as u64, UINT8, value, LITTLE_ENDIAN)?;
    }
    Ok(())
}

/// The bytes of a FromBase64 or FromHex result in a new Uint8Array (AllocateTypedArray of
/// `%Uint8Array%` with "%Uint8Array.prototype%", then the bytes), or its SyntaxError.
fn new_uint8_array<'s>(cx: &mut Cx<'s>, result: Decoded) -> JsResult<Local<'s>> {
    // If result.[[Error]] is not none, throw result.[[Error]].
    if let Some(text) = result.error {
        return Err(cx.throw_error(ErrorKind::SyntaxError, text));
    }
    // Let ta be ? AllocateTypedArray("Uint8Array", %Uint8Array%, "%Uint8Array.prototype%",
    // resultLength).
    let proto = cx.intrinsic(IntrinsicId::Uint8ArrayPrototype);
    let ta = allocate(cx, proto, UINT8, result.bytes.len() as u64)?;
    // Set the value at each index of ta.[[ViewedArrayBuffer]].[[ArrayBufferData]] to the value
    // at the corresponding index of result.[[Bytes]].
    set_uint8_array_bytes(cx, ta, &result.bytes)?;
    // Return ta.
    Ok(ta.value())
}

/// The end of setFromBase64 and setFromHex: SetUint8ArrayBytes(into, bytes), then the error
/// of the result (after the bytes before it are written), else `{ read, written }`.
fn set_result<'s>(
    cx: &mut Cx<'s>,
    into: Local<'s, JsObject>,
    result: Decoded,
) -> JsResult<Local<'s>> {
    // Let written be the length of bytes. (The decoder ran no JS: the buffer is as it was.)
    let written = result.bytes.len();
    // Perform SetUint8ArrayBytes(into, bytes).
    set_uint8_array_bytes(cx, into, &result.bytes)?;
    // If result.[[Error]] is not none, throw result.[[Error]].
    if let Some(text) = result.error {
        return Err(cx.throw_error(ErrorKind::SyntaxError, text));
    }
    // resultObject: OrdinaryObjectCreate(%Object.prototype%), then "read" and "written".
    let out = cx.new_object();
    cx.create_data_property_or_throw(out, "read", Local::number(result.read as f64))?;
    cx.create_data_property_or_throw(out, "written", Local::number(written as f64))?;
    Ok(out.value())
}

/// The bound of a set method: TypedArrayLength of MakeTypedArrayWithBufferWitnessRecord(into,
/// seq-cst), or a TypeError if `into` is out of bounds (or detached).
fn set_target_length<'s>(cx: &mut Cx<'s>, into: Local<'s, JsObject>) -> JsResult<usize> {
    match cx.typed_array_length(into)? {
        Some(n) => Ok(usize::try_from(n).unwrap_or(usize::MAX)),
        None => Err(cx.throw_type_error(OUT_OF_BOUNDS)),
    }
}

// ---- the 6 functions -------------------------------------------------------------------------

/// `Uint8Array.fromBase64 ( string [ , options ] )` (`sec-uint8array.frombase64`).
fn from_base64<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If string is not a String, throw a TypeError exception.
    let s = string_units(cx, args.get(0))?;
    // 2. Let opts be ? GetOptionsObject(options).
    let opts = get_options_object(cx, args.get(1))?;
    // 3-5. alphabet; 6-8. lastChunkHandling.
    let base64url = alphabet_option(cx, opts)?;
    let last = last_chunk_option(cx, opts)?;
    // 9. Let result be FromBase64(string, alphabet, lastChunkHandling).
    let result = from_base64_units(&s, base64url, last, usize::MAX);
    // 10-14. The error, or a new Uint8Array of the bytes.
    new_uint8_array(cx, result)
}

/// `Uint8Array.fromHex ( string )` (`sec-uint8array.fromhex`).
fn from_hex<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If string is not a String, throw a TypeError exception.
    let s = string_units(cx, args.get(0))?;
    // 2. Let result be FromHex(string).
    let result = from_hex_units(&s, usize::MAX);
    // 3-7. The error, or a new Uint8Array of the bytes.
    new_uint8_array(cx, result)
}

/// `Uint8Array.prototype.toBase64 ( [ options ] )` (`sec-uint8array.prototype.tobase64`).
fn to_base64<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let O be the this value; ? ValidateUint8Array(O).
    let o = validate_uint8_array(cx, this, "Uint8Array.prototype.toBase64")?;
    // 3. Let opts be ? GetOptionsObject(options).
    let opts = get_options_object(cx, args.get(0))?;
    // 4-6. alphabet.
    let base64url = alphabet_option(cx, opts)?;
    // 7. Let omitPadding be ToBoolean(? Get(opts, "omitPadding")).
    let omit = cx.get(opts, "omitPadding")?;
    let omit_padding = cx.to_boolean(omit);
    // 8. Let toEncode be ? GetUint8ArrayBytes(O) (after the getters: they can detach O).
    let bytes = get_uint8_array_bytes(cx, o, |n| base64_length(n, omit_padding))?;
    // 9-10. outAscii: the base64 or base64url encoding (RFC 4648 §4 or §5), padded unless
    //       omitPadding. 11. Return CodePointsToString(outAscii).
    let text = encode_base64(&bytes, base64url, omit_padding);
    let text = core::str::from_utf8(&text).expect("base64 text is ASCII");
    Ok(cx.new_string(text)?.value())
}

/// `Uint8Array.prototype.setFromBase64 ( string [ , options ] )`
/// (`sec-uint8array.prototype.setfrombase64`).
fn set_from_base64<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let into be the this value; ? ValidateUint8Array(into).
    let into = validate_uint8_array(cx, this, "Uint8Array.prototype.setFromBase64")?;
    // 3. If string is not a String, throw a TypeError exception.
    let s = string_units(cx, args.get(0))?;
    // 4. Let opts be ? GetOptionsObject(options).
    let opts = get_options_object(cx, args.get(1))?;
    // 5-8. alphabet; 9-12. lastChunkHandling.
    let base64url = alphabet_option(cx, opts)?;
    let last = last_chunk_option(cx, opts)?;
    // 13-15. taRecord (seq-cst); out of bounds is a TypeError; byteLength (after the getters:
    //        they can detach or shrink the buffer).
    let byte_length = set_target_length(cx, into)?;
    // 16. Let result be FromBase64(string, alphabet, lastChunkHandling, byteLength).
    let result = from_base64_units(&s, base64url, last, byte_length);
    debug_assert!(result.bytes.len() <= byte_length);
    // 17-26. Write the bytes, then the error or { read, written }.
    set_result(cx, into, result)
}

/// `Uint8Array.prototype.toHex ( )` (`sec-uint8array.prototype.tohex`).
fn to_hex<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let O be the this value; ? ValidateUint8Array(O).
    let o = validate_uint8_array(cx, this, "Uint8Array.prototype.toHex")?;
    // 3. Let toEncode be ? GetUint8ArrayBytes(O).
    let bytes = get_uint8_array_bytes(cx, o, |n| n.saturating_mul(2))?;
    // 4-5. For each byte: Number::toString(byte, 16), padded to 2 digits with "0".
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 15)]));
    }
    // 6. Return out.
    Ok(cx.new_string(&out)?.value())
}

/// `Uint8Array.prototype.setFromHex ( string )` (`sec-uint8array.prototype.setfromhex`).
fn set_from_hex<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let into be the this value; ? ValidateUint8Array(into).
    let into = validate_uint8_array(cx, this, "Uint8Array.prototype.setFromHex")?;
    // 3. If string is not a String, throw a TypeError exception.
    let s = string_units(cx, args.get(0))?;
    // 4-6. taRecord (seq-cst); out of bounds is a TypeError; byteLength.
    let byte_length = set_target_length(cx, into)?;
    // 7. Let result be FromHex(string, byteLength).
    let result = from_hex_units(&s, byte_length);
    debug_assert!(result.bytes.len() <= byte_length);
    // 8-17. Write the bytes, then the error or { read, written }.
    set_result(cx, into, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn b64(s: &str, last: LastChunk, max: usize) -> Decoded {
        from_base64_units(&u(s), false, last, max)
    }

    #[test]
    fn decodes_full_and_final_chunks() {
        let d = b64("Zm9vYmFy", LastChunk::Loose, usize::MAX);
        assert_eq!(d, Decoded::ok(8, b"foobar".to_vec()));
        // A final chunk of 2 or 3 characters, padded or not (loose).
        assert_eq!(b64("Zg==", LastChunk::Loose, usize::MAX).bytes, b"f");
        assert_eq!(b64("Zg", LastChunk::Loose, usize::MAX).bytes, b"f");
        assert_eq!(b64("Zm8", LastChunk::Loose, usize::MAX).bytes, b"fo");
        // Whitespace anywhere, also between "=" signs.
        assert_eq!(
            b64(" Z g = \t= ", LastChunk::Strict, usize::MAX).bytes,
            b"f"
        );
    }

    #[test]
    fn the_last_chunk_options() {
        let strict = b64("Zm9vZg", LastChunk::Strict, usize::MAX);
        assert_eq!(
            strict,
            Decoded::err(4, b"foo".to_vec(), error::MISSING_PADDING)
        );
        let stop = b64("Zm9vZg", LastChunk::StopBeforePartial, usize::MAX);
        assert_eq!(stop, Decoded::ok(4, b"foo".to_vec()));
        // Extra bits: an error only for strict.
        assert_eq!(b64("Zh==", LastChunk::Loose, usize::MAX).bytes, b"f");
        let extra = b64("Zh==", LastChunk::Strict, usize::MAX);
        assert_eq!(extra.error, Some(error::EXTRA_BITS));
        // One character is an error for loose too.
        assert_eq!(
            b64("Z", LastChunk::Loose, usize::MAX).error,
            Some(error::ONE_CHAR)
        );
        // "Zg=" with stop-before-partial stops before the chunk.
        let half = b64("Zm9vZg=", LastChunk::StopBeforePartial, usize::MAX);
        assert_eq!(half, Decoded::ok(4, b"foo".to_vec()));
        assert_eq!(
            b64("Zg=", LastChunk::Loose, usize::MAX).error,
            Some(error::SHORT_PADDING)
        );
        assert_eq!(
            b64("Z===", LastChunk::Loose, usize::MAX).error,
            Some(error::EARLY_PADDING)
        );
        assert_eq!(
            b64("Zg===", LastChunk::Loose, usize::MAX).error,
            Some(error::AFTER_PADDING)
        );
    }

    #[test]
    fn a_max_length_stops_before_a_chunk_that_does_not_fit() {
        assert_eq!(
            b64("Zm9vYmFy", LastChunk::Loose, 5),
            Decoded::ok(4, b"foo".to_vec())
        );
        assert_eq!(
            b64("Zm9vYmE=", LastChunk::Loose, 4),
            Decoded::ok(4, b"foo".to_vec())
        );
        assert_eq!(
            b64("Zm9vYmE=", LastChunk::Loose, 5),
            Decoded::ok(8, b"fooba".to_vec())
        );
        assert_eq!(
            b64("Zm9vYmFy", LastChunk::Loose, 0),
            Decoded::ok(0, Vec::new())
        );
        assert_eq!(
            b64("Zm9v Ym", LastChunk::Loose, 3),
            Decoded::ok(4, b"foo".to_vec())
        );
    }

    #[test]
    fn the_url_alphabet() {
        let d = from_base64_units(&u("x-_y"), true, LastChunk::Loose, usize::MAX);
        assert_eq!(d.bytes, [199, 239, 242]);
        let d = from_base64_units(&u("x+/y"), true, LastChunk::Loose, usize::MAX);
        assert_eq!(d.error, Some(error::BAD_CHAR));
        let d = from_base64_units(&u("x-_y"), false, LastChunk::Loose, usize::MAX);
        assert_eq!(d.error, Some(error::BAD_CHAR));
    }

    #[test]
    fn hex() {
        assert_eq!(
            from_hex_units(&u("0aFf"), usize::MAX),
            Decoded::ok(4, vec![10, 255])
        );
        assert_eq!(
            from_hex_units(&u("0aF"), usize::MAX).error,
            Some(error::ODD_HEX)
        );
        assert_eq!(
            from_hex_units(&u("aaag"), usize::MAX),
            Decoded::err(2, vec![170], error::BAD_HEX)
        );
        assert_eq!(
            from_hex_units(&u("aabbcc"), 2),
            Decoded::ok(4, vec![170, 187])
        );
    }

    #[test]
    fn encodes() {
        assert_eq!(encode_base64(b"", false, false), b"");
        assert_eq!(encode_base64(b"f", false, false), b"Zg==");
        assert_eq!(encode_base64(b"fo", false, false), b"Zm8=");
        assert_eq!(encode_base64(b"foo", false, false), b"Zm9v");
        assert_eq!(encode_base64(b"f", false, true), b"Zg");
        assert_eq!(
            encode_base64(&[199, 239, 242, 255], true, false),
            b"x-_y_w=="
        );
        for n in 0..10 {
            for omit in [false, true] {
                let len = encode_base64(&vec![7; n], false, omit).len();
                assert_eq!(len, base64_length(n, omit));
            }
        }
    }
}
