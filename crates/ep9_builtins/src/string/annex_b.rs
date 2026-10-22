//! Annex B.2.2 String.prototype methods: substr (B.2.2.1) and the HTML methods through
//! CreateHTML (B.2.2.2.1): anchor, big, blink, bold, fixed, fontcolor, fontsize, italics,
//! link, small, strike, sub, sup. trimLeft and trimRight are table aliases of trimStart and
//! trimEnd (string.rs PROPS).

use ep9_vm::{Args, Cx, JsResult, Local};

use super::{
    new_str, proto::relative_start, push_units, this_string, to_units, units, units_range,
};

/// String.prototype.substr ( start, length ) (B.2.2.1).
pub(super) fn substr<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = this_string(cx, this, "substr")?;
    // 3. Let size be the length of S.
    let size = cx.string_len(s);
    // 4. Let intStart be ? ToIntegerOrInfinity(start).
    // 5. If intStart = -∞, set intStart to 0.
    // 6. Else if intStart < 0, set intStart to max(size + intStart, 0).
    // 7. Else, set intStart to min(intStart, size).
    let int_start = relative_start(cx.to_integer_or_infinity(args.get(0))?, size);
    // 8. If length is undefined, let intLength be size; otherwise let intLength be
    //    ? ToIntegerOrInfinity(length).
    let length = args.get(1);
    let int_length = if length.is_undefined() {
        size as f64
    } else {
        cx.to_integer_or_infinity(length)?
    };
    // 9. Set intLength to the result of clamping intLength between 0 and size.
    let int_length = super::clamp(int_length, size);
    // 10. Let intEnd be min(intStart + intLength, size).
    let int_end = (int_start + int_length).min(size);
    // 11. Return the substring of S from intStart to intEnd.
    if int_start >= int_end {
        return Ok(cx.key("").value());
    }
    let substring = units_range(cx, s, int_start, int_end)?;
    new_str(cx, &substring)
}

/// CreateHTML ( string, tag, attribute, value ) (B.2.2.2.1). `attribute` is `None` for the
/// empty String.
fn create_html<'s>(
    cx: &mut Cx<'s>,
    string: Local<'s>,
    tag: &str,
    attribute: Option<(&str, Local<'s>)>,
    name: &str,
) -> JsResult<Local<'s>> {
    // 1. Let str be ? RequireObjectCoercible(string).
    // 2. Let S be ? ToString(str).
    let s = this_string(cx, string, name)?;
    // 3. Let p1 be the string-concatenation of "<" and tag.
    let mut p: Vec<u16> = format!("<{tag}").encode_utf16().collect();
    // 4. If attribute is not the empty String, then
    if let Some((attribute, value)) = attribute {
        //  a. Let V be ? ToString(value).
        let v = to_units(cx, value)?;
        //  b. Let escapedV be the String value that is the same as V except that each
        //     occurrence of the code unit 0x0022 (QUOTATION MARK) in V has been replaced with
        //     the six code unit sequence "&quot;".
        //  c. Set p1 to the string-concatenation of: p1, the code unit 0x0020 (SPACE),
        //     attribute, the code unit 0x003D (EQUALS SIGN), the code unit 0x0022 (QUOTATION
        //     MARK), escapedV, the code unit 0x0022 (QUOTATION MARK).
        p.extend(format!(" {attribute}=\"").encode_utf16());
        for u in v {
            if u == u16::from(b'"') {
                push_units(cx, &mut p, &"&quot;".encode_utf16().collect::<Vec<_>>())?;
            } else {
                push_units(cx, &mut p, &[u])?;
            }
        }
        p.push(u16::from(b'"'));
    }
    // 5. Let p2 be the string-concatenation of p1 and ">".
    p.push(u16::from(b'>'));
    // 6. Let p3 be the string-concatenation of p2 and S.
    let s = units(cx, s)?;
    push_units(cx, &mut p, &s)?;
    // 7. Let p4 be the string-concatenation of p3, "</", tag, and ">".
    let close: Vec<u16> = format!("</{tag}>").encode_utf16().collect();
    push_units(cx, &mut p, &close)?;
    // 8. Return p4.
    new_str(cx, &p)
}

/// An HTML method with no attribute: `Return ? CreateHTML(S, tag, "", "")` with S the this
/// value.
macro_rules! html_plain {
    ($(#[$doc:meta])* $f:ident, $name:literal, $tag:literal) => {
        $(#[$doc])*
        pub(super) fn $f<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            _args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            // 1. Let S be the this value.
            // 2. Return ? CreateHTML(S, tag, "", "").
            create_html(cx, this, $tag, None, $name)
        }
    };
}

/// An HTML method with one attribute: `Return ? CreateHTML(S, tag, attribute, arg)` with S
/// the this value.
macro_rules! html_attr {
    ($(#[$doc:meta])* $f:ident, $name:literal, $tag:literal, $attr:literal) => {
        $(#[$doc])*
        pub(super) fn $f<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            // 1. Let S be the this value.
            // 2. Return ? CreateHTML(S, tag, attribute, arg).
            create_html(cx, this, $tag, Some(($attr, args.get(0))), $name)
        }
    };
}

html_attr!(
    /// String.prototype.anchor ( name ) (B.2.2.2).
    anchor, "anchor", "a", "name"
);
html_plain!(
    /// String.prototype.big ( ) (B.2.2.3).
    big, "big", "big"
);
html_plain!(
    /// String.prototype.blink ( ) (B.2.2.4).
    blink, "blink", "blink"
);
html_plain!(
    /// String.prototype.bold ( ) (B.2.2.5).
    bold, "bold", "b"
);
html_plain!(
    /// String.prototype.fixed ( ) (B.2.2.6).
    fixed, "fixed", "tt"
);
html_attr!(
    /// String.prototype.fontcolor ( colour ) (B.2.2.7).
    fontcolor, "fontcolor", "font", "color"
);
html_attr!(
    /// String.prototype.fontsize ( size ) (B.2.2.8).
    fontsize, "fontsize", "font", "size"
);
html_plain!(
    /// String.prototype.italics ( ) (B.2.2.9).
    italics, "italics", "i"
);
html_attr!(
    /// String.prototype.link ( url ) (B.2.2.10).
    link, "link", "a", "href"
);
html_plain!(
    /// String.prototype.small ( ) (B.2.2.11).
    small, "small", "small"
);
html_plain!(
    /// String.prototype.strike ( ) (B.2.2.12).
    strike, "strike", "strike"
);
html_plain!(
    /// String.prototype.sub ( ) (B.2.2.13).
    sub, "sub", "sub"
);
html_plain!(
    /// String.prototype.sup ( ) (B.2.2.14).
    sup, "sup", "sup"
);
