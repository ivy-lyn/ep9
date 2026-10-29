//! `%RegExp.prototype%` methods and accessors (§22.2.6): exec, test, toString, flags, source
//! and the eight flag accessors (RegExpHasFlag, §22.2.6.4.1).

use ep9_vm::{Args, Cx, IntrinsicId, JsObject, JsResult, Local, ObjectClass};

use super::exec::{self, flag, is_regexp_object, slot};

/// The this value as an Object, else a TypeError naming `what` (V8's text).
fn this_object<'s>(cx: &mut Cx<'s>, this: Local<'s>, what: &str) -> JsResult<Local<'s, JsObject>> {
    match this.as_object() {
        Some(o) => Ok(o),
        None => {
            let shown = cx.display(this);
            let msg = format!("{what} called on non-object {shown}");
            Err(cx.throw_type_error(&msg))
        }
    }
}

/// RegExp.prototype.exec ( string ) (§22.2.6.2).
pub(super) fn exec<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let R be the this value.
    // 2. Perform ? RequireInternalSlot(R, [[RegExpMatcher]]).
    let r = match this.as_object() {
        Some(o) if is_regexp_object(cx, o) => o,
        _ => {
            let shown = cx.display(this);
            let msg = format!("RegExp.prototype.exec called on incompatible receiver {shown}");
            return Err(cx.throw_type_error(&msg));
        }
    };
    // 3. Let S be ? ToString(string).
    let s = cx.to_string(args.get(0))?;
    // 4. Return ? RegExpBuiltinExec(R, S).
    match exec::builtin_match(cx, r, s)? {
        None => Ok(Local::null()),
        Some(m) => {
            let bits = exec::flag_bits(cx, r)?;
            Ok(exec::match_array(cx, s, &m, bits)?.value())
        }
    }
}

/// RegExp.prototype.test ( S ) (§22.2.6.16).
pub(super) fn test<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let R be the this value.
    // 2. If R is not an Object, throw a TypeError exception.
    let r = this_object(cx, this, "RegExp.prototype.test")?;
    // 3. Let string be ? ToString(S).
    let s = cx.to_string(args.get(0))?;
    // 4. Let match be ? RegExpExec(R, string).
    let m = exec::regexp_exec(cx, r, s)?;
    // 5. If match is not null, return true; else return false.
    Ok(Local::boolean(!matches!(m, exec::Exec::Null)))
}

/// RegExp.prototype.toString ( ) (§22.2.6.17).
pub(super) fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let R be the this value.
    // 2. If R is not an Object, throw a TypeError exception.
    let r = this_object(cx, this, "RegExp.prototype.toString")?;
    // 3. Let pattern be ? ToString(? Get(R, "source")).
    let source = cx.get(r, "source")?;
    let pattern = cx.to_string(source)?;
    // 4. Let flags be ? ToString(? Get(R, "flags")).
    let flags = cx.get(r, "flags")?;
    let flags = cx.to_string(flags)?;
    // 5. Let result be the string-concatenation of "/", pattern, "/", and flags.
    let mut result = vec![u16::from(b'/')];
    result.extend(crate::string::units(cx, pattern)?);
    result.push(u16::from(b'/'));
    result.extend(crate::string::units(cx, flags)?);
    // 6. Return result.
    Ok(cx.new_string_utf16(&result)?.value())
}

/// get RegExp.prototype.flags (§22.2.6.4).
pub(super) fn flags<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let R be the this value.
    // 2. If R is not an Object, throw a TypeError exception.
    let r = this_object(cx, this, "RegExp.prototype.flags getter")?;
    // 3. Let codeUnits be a new empty List.
    let mut code_units = String::new();
    // 4-11. For each of hasIndices (d), global (g), ignoreCase (i), multiline (m), dotAll
    //    (s), unicode (u), unicodeSets (v), sticky (y): if ToBoolean(? Get(R, name)) is
    //    true, append the code unit.
    for (name, c) in [
        ("hasIndices", 'd'),
        ("global", 'g'),
        ("ignoreCase", 'i'),
        ("multiline", 'm'),
        ("dotAll", 's'),
        ("unicode", 'u'),
        ("unicodeSets", 'v'),
        ("sticky", 'y'),
    ] {
        let v = cx.get(r, name)?;
        if cx.to_boolean(v) {
            code_units.push(c);
        }
    }
    // 12. Return the String value whose code units are the elements of the List codeUnits.
    Ok(cx.new_string(&code_units)?.value())
}

/// RegExpHasFlag ( R, codeUnit ) (§22.2.6.4.1) for the flag bit `bit`; `name` is the accessor
/// for the error text.
fn has_flag<'s>(cx: &mut Cx<'s>, this: Local<'s>, bit: u8, name: &str) -> JsResult<Local<'s>> {
    // (The error texts are built only on the error paths: the flags getter calls these
    // getters on every @@replace, @@match and @@split.)
    // 1. If R is not an Object, throw a TypeError exception.
    let Some(r) = this.as_object() else {
        let shown = cx.display(this);
        let msg = format!("RegExp.prototype.{name} getter called on non-object {shown}");
        return Err(cx.throw_type_error(&msg));
    };
    // 2. If R does not have an [[OriginalFlags]] internal slot, then
    if !is_regexp_object(cx, r) {
        //    a. If SameValue(R, %RegExp.prototype%) is true, return undefined.
        let proto = cx.intrinsic(IntrinsicId::RegExpPrototype);
        if cx.same_value(r, proto) {
            return Ok(Local::undefined());
        }
        //    b. Otherwise, throw a TypeError exception.
        let shown = cx.display(this);
        let msg = format!("RegExp.prototype.{name} getter called on incompatible receiver {shown}");
        return Err(cx.throw_type_error(&msg));
    }
    // 3. Let flags be R.[[OriginalFlags]].
    // 4. If flags contains codeUnit, return true.
    // 5. Return false.
    let bits = exec::flag_bits(cx, r)?;
    Ok(Local::boolean(bits & bit != 0))
}

/// Defines the getter of one flag accessor (§22.2.6.3, .5-.7, .9, .12, .15, .18, .19).
macro_rules! flag_getter {
    ($(#[$doc:meta])* $fn:ident, $bit:expr, $name:literal) => {
        $(#[$doc])*
        pub(super) fn $fn<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            _args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            has_flag(cx, this, $bit, $name)
        }
    };
}

flag_getter!(
    /// get RegExp.prototype.dotAll (§22.2.6.3): RegExpHasFlag(R, "s").
    dot_all, flag::DOT_ALL, "dotAll"
);
flag_getter!(
    /// get RegExp.prototype.global (§22.2.6.5): RegExpHasFlag(R, "g").
    global, flag::GLOBAL, "global"
);
flag_getter!(
    /// get RegExp.prototype.hasIndices (§22.2.6.6): RegExpHasFlag(R, "d").
    has_indices, flag::HAS_INDICES, "hasIndices"
);
flag_getter!(
    /// get RegExp.prototype.ignoreCase (§22.2.6.7): RegExpHasFlag(R, "i").
    ignore_case, flag::IGNORE_CASE, "ignoreCase"
);
flag_getter!(
    /// get RegExp.prototype.multiline (§22.2.6.9): RegExpHasFlag(R, "m").
    multiline, flag::MULTILINE, "multiline"
);
flag_getter!(
    /// get RegExp.prototype.sticky (§22.2.6.15): RegExpHasFlag(R, "y").
    sticky, flag::STICKY, "sticky"
);
flag_getter!(
    /// get RegExp.prototype.unicode (§22.2.6.18): RegExpHasFlag(R, "u").
    unicode, flag::UNICODE, "unicode"
);
flag_getter!(
    /// get RegExp.prototype.unicodeSets (§22.2.6.19): RegExpHasFlag(R, "v").
    unicode_sets, flag::UNICODE_SETS, "unicodeSets"
);

/// RegExp.escape ( S ) (§22.2.5.1, ES2025).
pub(super) fn escape<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If S is not a String, throw a TypeError exception.
    let Some(s) = args.get(0).as_string() else {
        return Err(cx.throw_type_error("RegExp.escape requires a string"));
    };
    let units = crate::string::units(cx, s)?;
    Ok(cx.new_string_utf16(&escape_units(&units))?.value())
}

/// Steps 2-5 of RegExp.escape (§22.2.5.1) on the code units of S.
pub(crate) fn escape_units(s: &[u16]) -> Vec<u16> {
    // 2. Let escaped be the empty String.
    let mut escaped: Vec<u16> = Vec::with_capacity(s.len() + 4);
    // 3. Let cpList be StringToCodePoints(S).
    // 4. For each code point c in cpList, do
    for c in char::decode_utf16(s.iter().copied()) {
        let c = match c {
            Ok(ch) => u32::from(ch),
            Err(e) => u32::from(e.unpaired_surrogate()),
        };
        // a. If escaped is the empty String, and c is matched by either DecimalDigit or
        //    AsciiLetter, then
        //    i-v. escaped = "\x" + the two lowercase hex digits of c.
        if escaped.is_empty() && c < 0x80 && (c as u8).is_ascii_alphanumeric() {
            push_hex_escape(&mut escaped, 'x', c, 2);
        } else {
            // b. Else, set escaped to escaped + EncodeForRegExpEscape(c).
            encode_for_regexp_escape(&mut escaped, c);
        }
    }
    // 5. Return escaped.
    escaped
}

/// Appends `\`, `letter` and `value` as `digits` lowercase hex digits.
fn push_hex_escape(out: &mut Vec<u16>, letter: char, value: u32, digits: usize) {
    out.push(u16::from(b'\\'));
    out.push(letter as u16);
    for k in (0..digits).rev() {
        let d = (value >> (4 * k)) & 0xF;
        out.push(char::from_digit(d, 16).expect("a hex digit") as u16);
    }
}

/// EncodeForRegExpEscape ( c ) (§22.2.5.1.1), appended to `out`.
fn encode_for_regexp_escape(out: &mut Vec<u16>, c: u32) {
    // 1. If c is matched by SyntaxCharacter or c is U+002F (SOLIDUS), then
    //    a. Return the string-concatenation of 0x005C (REVERSE SOLIDUS) and
    //       UTF16EncodeCodePoint(c).
    if c < 0x80 && b"^$\\.*+?()[]{}|/".contains(&(c as u8)) {
        out.extend([u16::from(b'\\'), c as u16]);
        return;
    }
    // 2. If c is the code point listed in some cell of the "Code Point" column of Table 67
    //    (ControlEscape), then return 0x005C followed by the ControlEscape letter.
    let control = match c {
        0x09 => Some(b't'),
        0x0A => Some(b'n'),
        0x0B => Some(b'v'),
        0x0C => Some(b'f'),
        0x0D => Some(b'r'),
        _ => None,
    };
    if let Some(letter) = control {
        out.extend([u16::from(b'\\'), u16::from(letter)]);
        return;
    }
    // 3. Let otherPunctuators be the string-concatenation of ",-=<>#&!%:;@~'`" and the code
    //    unit 0x0022 (QUOTATION MARK).
    // 4. Let toEscape be StringToCodePoints(otherPunctuators).
    let other = c < 0x80 && b",-=<>#&!%:;@~'`\"".contains(&(c as u8));
    // 5. If toEscape contains c, c is matched by either WhiteSpace or LineTerminator, or c
    //    has the same numeric value as a leading surrogate or trailing surrogate, then
    if other || is_white_space_or_line_terminator(c) || (0xD800..=0xDFFF).contains(&c) {
        //  a. Let cNum be the numeric value of c.
        //  b. If cNum ≤ 0xFF, then return "\x" and StringPad(Number::toString(cNum, 16),
        //     2, "0", start).
        if c <= 0xFF {
            push_hex_escape(out, 'x', c, 2);
            return;
        }
        //  c. Let escaped be the empty String.
        //  d. Let codeUnits be UTF16EncodeCodePoint(c).
        //  e. For each code unit cu of codeUnits: escaped += UnicodeEscape(cu).
        //  f. Return escaped.
        let mut buf = [0u16; 2];
        let units: &[u16] = match char::from_u32(c) {
            Some(ch) => ch.encode_utf16(&mut buf),
            None => {
                buf[0] = c as u16;
                &buf[..1]
            }
        };
        for &cu in units {
            push_hex_escape(out, 'u', u32::from(cu), 4);
        }
        return;
    }
    // 6. Return UTF16EncodeCodePoint(c).
    match char::from_u32(c) {
        Some(ch) => {
            let mut buf = [0u16; 2];
            out.extend_from_slice(ch.encode_utf16(&mut buf));
        }
        None => out.push(c as u16),
    }
}

/// WhiteSpace (§12.2: TAB, VT, FF, ZWNBSP, any Zs code point) or LineTerminator (§12.3: LF,
/// CR, LS, PS).
fn is_white_space_or_line_terminator(c: u32) -> bool {
    // TAB, LF, VT, FF, CR; SP, NBSP; the Zs block and LS, PS; ZWNBSP.
    matches!(c, 0x09..=0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029)
        || matches!(c, 0x202F | 0x205F | 0x3000 | 0xFEFF)
}

/// get RegExp.prototype.source (§22.2.6.13).
pub(super) fn source<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let R be the this value.
    // 2. If R is not an Object, throw a TypeError exception.
    let r = this_object(cx, this, "RegExp.prototype.source getter")?;
    // 3. If R does not have an [[OriginalSource]] internal slot, then
    if !is_regexp_object(cx, r) {
        //    a. If SameValue(R, %RegExp.prototype%) is true, return "(?:)".
        let proto = cx.intrinsic(IntrinsicId::RegExpPrototype);
        if cx.same_value(r, proto) {
            return Ok(cx.new_string("(?:)")?.value());
        }
        //    b. Otherwise, throw a TypeError exception.
        let shown = cx.display(this);
        let msg = format!("RegExp.prototype.source getter called on incompatible receiver {shown}");
        return Err(cx.throw_type_error(&msg));
    }
    // 4. Assert: R has an [[OriginalFlags]] internal slot.
    // 5. Let src be R.[[OriginalSource]].
    // 6. Let flags be R.[[OriginalFlags]].
    let src = cx.slot(r, ObjectClass::RegExp, slot::SOURCE)?;
    let src = src.as_string().expect("[[OriginalSource]] is a string");
    let src = crate::string::units(cx, src)?;
    let nested = exec::flag_bits(cx, r)? & flag::UNICODE_SETS != 0;
    // 7. Return EscapeRegExpPattern(src, flags).
    Ok(cx.new_string_utf16(&escape_pattern(&src, nested))?.value())
}

/// EscapeRegExpPattern ( P, F ) (§22.2.6.13.1), as V8 writes it: `/` outside a class becomes
/// `\/`, a line terminator becomes `\n`, `\r`, `\u2028` or `\u2029` (after a backslash only
/// the letters are added), and the empty pattern is `(?:)`. `nested`: the `v` flag (classes
/// nest).
pub(crate) fn escape_pattern(src: &[u16], nested: bool) -> Vec<u16> {
    if src.is_empty() {
        return "(?:)".encode_utf16().collect();
    }
    let mut out = Vec::with_capacity(src.len() + 2);
    let (mut i, mut class_depth) = (0, 0usize);
    let escape_of = |u: u16| -> Option<&'static str> {
        match u {
            0x0A => Some("n"),
            0x0D => Some("r"),
            0x2028 => Some("u2028"),
            0x2029 => Some("u2029"),
            _ => None,
        }
    };
    while i < src.len() {
        let c = src[i];
        if c == u16::from(b'\\') {
            out.push(c);
            if let Some(&next) = src.get(i + 1) {
                match escape_of(next) {
                    Some(e) => out.extend(e.encode_utf16()),
                    None => out.push(next),
                }
            }
            i += 2;
            continue;
        }
        if let Some(e) = escape_of(c) {
            out.push(u16::from(b'\\'));
            out.extend(e.encode_utf16());
        } else if c == u16::from(b'/') && class_depth == 0 {
            out.extend([u16::from(b'\\'), c]);
        } else {
            if c == u16::from(b'[') && (class_depth == 0 || nested) {
                class_depth += 1;
            } else if c == u16::from(b']') && class_depth > 0 {
                class_depth -= 1;
            }
            out.push(c);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape_pattern;

    fn esc(s: &str, nested: bool) -> String {
        let units: Vec<u16> = s.encode_utf16().collect();
        String::from_utf16(&escape_pattern(&units, nested)).unwrap()
    }

    // Each pair is node v24.22.1's `new RegExp(p).source`.
    #[test]
    fn escape_regexp_pattern_matches_node() {
        for (p, want) in [
            ("/", "\\/"),
            ("\\/", "\\/"),
            ("[/]", "[/]"),
            ("[\\]/]", "[\\]/]"),
            ("\n", "\\n"),
            ("\\\n", "\\n"),
            ("\r", "\\r"),
            ("\u{2028}", "\\u2028"),
            ("\u{2029}", "\\u2029"),
            ("", "(?:)"),
            ("a/b[/]c/", "a\\/b[/]c\\/"),
            ("[\\/]", "[\\/]"),
            ("\\\\/", "\\\\\\/"),
            ("\\[/", "\\[\\/"),
        ] {
            assert_eq!(esc(p, false), want, "{p:?}");
        }
        assert_eq!(esc("[[a\\/]]/", true), "[[a\\/]]\\/");
    }
}
