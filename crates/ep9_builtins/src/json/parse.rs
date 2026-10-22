//! JSON.parse ( text [ , reviver ] ) (§25.5.1) and InternalizeJSONProperty (§25.5.1.1).
//!
//! ```text
//! parse    ToString(text) -> code units (a Rust Vec<u16>: no Values)
//!          one loop, no Rust recursion:
//!            value start  { [ -> a new container, pushed on `open`; " -> a string;
//!                         number, true, false, null -> a primitive
//!            value end    the top container takes it (CreateDataProperty), then , ] }
//!          open: a ScratchList (precise roots), 2 entries per open container:
//!                the container, and the pending key of an object (undefined for an array)
//!          the next index of each open array: a Rust Vec<u32>
//! reviver  InternalizeJSONProperty over the result, depth first (recursive, as the spec;
//!          Cx::check_native_stack per level, Cx::poll_interrupt per property)
//! ```
//!
//! The grammar is ECMA-404 (§25.5.1 step 2 parses StringToCodePoints(jsonString) as a JSON
//! text): whitespace is tab, LF, CR and space only; no comments, no trailing commas, no
//! single quotes, no leading zeros, no `+`, `.5` or `5.`; a string holds no code unit below
//! U+0020 and lone surrogates pass through. A key `__proto__` is an ordinary own property
//! (CreateDataProperty), and a later duplicate key replaces the earlier value.

use ep9_vm::{Args, Cx, ErrorKind, JsKey, JsObject, JsResult, Local, ScratchList, Throw};

use crate::array::{index_key, length_of_array_like};

use super::{MAX_DEPTH, enumerable_own_keys, too_deep};

/// JSON.parse ( text [ , reviver ] ) (§25.5.1).
pub(super) fn parse<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let reviver = args.get(1);
    // 1. Let jsonString be ? ToString(text).
    let json_string = cx.to_string(args.get(0))?;
    let len = cx.string_len(json_string);
    let flat = cx.flatten(json_string)?;
    let text: Vec<u16> = (0..len)
        .map(|i| cx.code_unit_at(flat, i).expect("i < length"))
        .collect();
    // 2-4. Parse StringToCodePoints(jsonString) as a JSON text (a SyntaxError if it is not
    //      one), and evaluate it: unfiltered.
    let unfiltered = Parser { text: &text, at: 0 }.run(cx)?;
    // 5. If IsCallable(reviver) is true, then
    if reviver.is_callable() {
        // a. Let root be OrdinaryObjectCreate(%Object.prototype%).
        let root = cx.new_object();
        // b. Let rootName be the empty String.
        let root_name = cx.key("");
        // c. Perform ! CreateDataPropertyOrThrow(root, rootName, unfiltered).
        cx.create_data_property_or_throw(root, root_name, unfiltered)?;
        // d. Return ? InternalizeJSONProperty(root, rootName, reviver).
        return internalize(cx, root, root_name, reviver, 0);
    }
    // 6. Else, return unfiltered.
    Ok(unfiltered)
}

/// InternalizeJSONProperty ( holder, name, reviver ) (§25.5.1.1). Recursive (one level per
/// nesting level of the value; `depth` levels are open), so each level checks the native
/// stack and [`MAX_DEPTH`] first.
fn internalize<'s>(
    cx: &mut Cx<'s>,
    holder: Local<'s, JsObject>,
    name: Local<'s, JsKey>,
    reviver: Local<'s>,
    depth: usize,
) -> JsResult<Local<'s>> {
    cx.check_native_stack()?;
    if depth >= MAX_DEPTH {
        return Err(too_deep(cx));
    }
    // 1. Let val be ? Get(holder, name).
    let val = cx.get(holder, name)?;
    // 2. If val is an Object, then
    if let Some(o) = val.as_object() {
        // a. Let isArray be ? IsArray(val).
        if cx.is_array(o) {
            // b. If isArray is true, then
            //    i. Let len be ? LengthOfArrayLike(val).
            let len = length_of_array_like(cx, o)?;
            //    ii. Let I be 0. iii. Repeat, while I < len,
            for i in 0..len {
                cx.poll_interrupt()?;
                // 1. Let prop be ! ToString(𝔽(I)).
                let prop = index_key(cx, i);
                // 2-4. Revive the element.
                revive_property(cx, o, prop, reviver, depth)?;
            }
        } else {
            // c. Else,
            //    i. Let keys be ? EnumerableOwnProperties(val, key).
            let (keys, n) = enumerable_own_keys(cx, o)?;
            //    ii. For each String P of keys, do
            for i in 0..n {
                cx.poll_interrupt()?;
                let p = cx.get_index(keys, i)?;
                let p = cx.to_property_key(p)?;
                revive_property(cx, o, p, reviver, depth)?;
            }
        }
    }
    // 3. Return ? Call(reviver, holder, « name, val »).
    cx.call(reviver, holder, &[name.into(), val])
}

/// InternalizeJSONProperty step 2.b.iii.2-4 and 2.c.ii.1-3 for the property `prop` of `val`.
fn revive_property<'s>(
    cx: &mut Cx<'s>,
    val: Local<'s, JsObject>,
    prop: Local<'s, JsKey>,
    reviver: Local<'s>,
    depth: usize,
) -> JsResult<()> {
    // 1. Let newElement be ? InternalizeJSONProperty(val, prop, reviver).
    let new_element = internalize(cx, val, prop, reviver, depth + 1)?;
    if new_element.is_undefined() {
        // 2. If newElement is undefined, then perform ? val.[[Delete]](prop).
        cx.delete(val, prop)?;
    } else {
        // 3. Else, perform ? CreateDataProperty(val, prop, newElement).
        cx.create_data_property(val, prop, new_element)?;
    }
    Ok(())
}

/// The JSON text and the read position.
struct Parser<'t> {
    text: &'t [u16],
    at: usize,
}

/// The open containers: [`ScratchList`] entries `2d` (the container) and `2d + 1` (the
/// pending key of an object, undefined for an array) for depth `d`, plus the next index of
/// each array in Rust. Entries past `depth` are stale (rooted until the parse ends).
struct Open<'s> {
    /// The innermost ScratchList while the parse runs (no JS runs), so `len` is its own.
    list: ScratchList<'s>,
    /// The next index of the container at each depth (0 for an object).
    next: Vec<u32>,
}

impl<'s> Open<'s> {
    fn depth(&self) -> usize {
        self.next.len()
    }

    fn put(&self, i: usize, v: Local<'s>) {
        if i < self.list.len() {
            self.list.set(i, v);
        } else {
            debug_assert_eq!(i, self.list.len());
            self.list.push(v);
        }
    }

    fn push(&mut self, container: Local<'s>, key: Local<'s>) {
        let d = self.depth();
        self.put(2 * d, container);
        self.put(2 * d + 1, key);
        self.next.push(0);
    }

    fn pop(&mut self) {
        self.next.pop();
    }

    fn top(&self) -> Local<'s> {
        self.list.get(2 * (self.depth() - 1))
    }
}

/// JSON whitespace (ECMA-404 §4): tab, LF, CR, space.
fn is_ws(u: u16) -> bool {
    matches!(u, 0x09 | 0x0A | 0x0D | 0x20)
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.text.get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(is_ws) {
            self.at += 1;
        }
    }

    /// The SyntaxError at the current position (V8's texts).
    fn error<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = match self.peek() {
            None => "Unexpected end of JSON input".to_string(),
            Some(u) => {
                let c = char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}');
                format!("Unexpected token '{c}' in JSON at position {}", self.at)
            }
        };
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }

    /// Consumes `u` (after whitespace), or a SyntaxError.
    fn expect<'s>(&mut self, cx: &mut Cx<'s>, u: u16) -> JsResult<()> {
        self.skip_ws();
        if self.peek() == Some(u) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(cx))
        }
    }

    /// Parses the whole text: one JSON value with optional whitespace around it.
    fn run<'s>(mut self, cx: &mut Cx<'s>) -> JsResult<Local<'s>> {
        let mut open = Open {
            list: cx.scratch_list(),
            next: Vec::new(),
        };
        'value: loop {
            cx.poll_interrupt()?;
            // A value starts here.
            self.skip_ws();
            let mut value: Local<'s> = match self.peek() {
                Some(0x7B) => {
                    // `{`: an object; `}` or the first key follows.
                    self.at += 1;
                    let o = cx.new_object();
                    self.skip_ws();
                    if self.peek() == Some(0x7D) {
                        self.at += 1;
                        o.value()
                    } else {
                        let key = self.key(cx)?;
                        open.push(o.value(), key.into());
                        continue 'value;
                    }
                }
                Some(0x5B) => {
                    // `[`: an array; `]` or the first element follows.
                    self.at += 1;
                    let a = cx.new_array();
                    self.skip_ws();
                    if self.peek() == Some(0x5D) {
                        self.at += 1;
                        a.value()
                    } else {
                        open.push(a.value(), Local::undefined());
                        continue 'value;
                    }
                }
                Some(0x22) => self.string(cx)?.value(),
                Some(0x2D | 0x30..=0x39) => Local::number(self.number(cx)?),
                Some(0x74) => self.literal(cx, "true", Local::boolean(true))?,
                Some(0x66) => self.literal(cx, "false", Local::boolean(false))?,
                Some(0x6E) => self.literal(cx, "null", Local::null())?,
                _ => return Err(self.error(cx)),
            };
            // The value ends here: the open containers take it, innermost first, while they
            // close.
            loop {
                let d = open.depth();
                if d == 0 {
                    self.skip_ws();
                    if self.at != self.text.len() {
                        return Err(self.error(cx));
                    }
                    return Ok(value);
                }
                let container = open.top().as_object().expect("open containers are objects");
                let key = open.list.get(2 * d - 1);
                if key.is_undefined() {
                    // An array element: CreateDataPropertyOrThrow(A, ! ToString(𝔽(i)), v)
                    // (ArrayLiteral evaluation), so no Array.prototype setter runs.
                    let i = open.next[d - 1];
                    let k = index_key(cx, u64::from(i));
                    cx.create_data_property_or_throw(container, k, value)?;
                    open.next[d - 1] = i + 1;
                    self.skip_ws();
                    match self.peek() {
                        Some(0x2C) => {
                            self.at += 1;
                            continue 'value;
                        }
                        Some(0x5D) => self.at += 1,
                        _ => return Err(self.error(cx)),
                    }
                } else {
                    // An object member: CreateDataProperty (PropertyDefinitionEvaluation of
                    // a JSON object; `__proto__` is an ordinary key here).
                    let k = cx.to_property_key(key)?;
                    cx.create_data_property_or_throw(container, k, value)?;
                    self.skip_ws();
                    match self.peek() {
                        Some(0x2C) => {
                            self.at += 1;
                            self.skip_ws();
                            let key = self.key(cx)?;
                            open.list.set(2 * d - 1, key);
                            continue 'value;
                        }
                        Some(0x7D) => self.at += 1,
                        _ => return Err(self.error(cx)),
                    }
                }
                // The container closed: it is the value for the next one out.
                value = container.value();
                open.pop();
            }
        }
    }

    /// A member name and its `:`: the key (an atom string).
    fn key<'s>(&mut self, cx: &mut Cx<'s>) -> JsResult<Local<'s, JsKey>> {
        if self.peek() != Some(0x22) {
            return Err(self.error(cx));
        }
        let s = self.string(cx)?;
        self.expect(cx, 0x3A)?;
        Ok(cx.intern(s))
    }

    /// `true`, `false` or `null`.
    fn literal<'s>(
        &mut self,
        cx: &mut Cx<'s>,
        word: &str,
        value: Local<'s>,
    ) -> JsResult<Local<'s>> {
        for b in word.bytes() {
            if self.peek() != Some(u16::from(b)) {
                return Err(self.error(cx));
            }
            self.at += 1;
        }
        Ok(value)
    }

    /// A JSON number: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`, rounded to the
    /// nearest double (Rust's `str::parse::<f64>` is correctly rounded).
    fn number<'s>(&mut self, cx: &mut Cx<'s>) -> JsResult<f64> {
        let start = self.at;
        let digit = |u: Option<u16>| u.is_some_and(|u| (0x30..=0x39).contains(&u));
        if self.peek() == Some(0x2D) {
            self.at += 1;
        }
        match self.peek() {
            Some(0x30) => self.at += 1,
            Some(0x31..=0x39) => {
                while digit(self.peek()) {
                    self.at += 1;
                }
            }
            _ => return Err(self.error(cx)),
        }
        let int_end = self.at;
        if self.peek() == Some(0x2E) {
            self.at += 1;
            if !digit(self.peek()) {
                return Err(self.error(cx));
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(0x65 | 0x45)) {
            self.at += 1;
            if matches!(self.peek(), Some(0x2B | 0x2D)) {
                self.at += 1;
            }
            if !digit(self.peek()) {
                return Err(self.error(cx));
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        let units = &self.text[start..self.at];
        // A short integer is exact in an f64: no text round trip.
        if self.at == int_end && units.len() <= 15 {
            let (neg, digits) = match units {
                [0x2D, rest @ ..] => (true, rest),
                _ => (false, units),
            };
            let n = digits
                .iter()
                .fold(0u64, |n, &u| n * 10 + u64::from(u - 0x30)) as f64;
            return Ok(if neg { -n } else { n });
        }
        let text: String = units.iter().map(|&u| u as u8 as char).collect();
        Ok(text
            .parse::<f64>()
            .expect("a JSON number is a Rust float literal"))
    }

    /// A JSON string at `"`: its value as a new string.
    fn string<'s>(&mut self, cx: &mut Cx<'s>) -> JsResult<Local<'s, ep9_vm::JsString>> {
        debug_assert_eq!(self.peek(), Some(0x22));
        self.at += 1;
        let start = self.at;
        // The fast path: no escape, so the value is a slice of the text.
        loop {
            match self.peek() {
                None => return Err(self.unterminated(cx)),
                Some(0x22) => {
                    let s = cx.new_string_utf16(&self.text[start..self.at])?;
                    self.at += 1;
                    return Ok(s);
                }
                Some(0x5C) => break,
                Some(u) if u < 0x20 => return Err(self.bad_control(cx)),
                Some(_) => self.at += 1,
            }
        }
        let mut out: Vec<u16> = self.text[start..self.at].to_vec();
        loop {
            match self.peek() {
                None => return Err(self.unterminated(cx)),
                Some(0x22) => {
                    self.at += 1;
                    return cx.new_string_utf16(&out);
                }
                Some(0x5C) => {
                    self.at += 1;
                    let u = match self.peek() {
                        Some(0x22) => 0x22,
                        Some(0x5C) => 0x5C,
                        Some(0x2F) => 0x2F,
                        Some(0x62) => 0x08,
                        Some(0x66) => 0x0C,
                        Some(0x6E) => 0x0A,
                        Some(0x72) => 0x0D,
                        Some(0x74) => 0x09,
                        Some(0x75) => {
                            let mut u = 0u16;
                            for k in 1..=4 {
                                let d = self
                                    .text
                                    .get(self.at + k)
                                    .and_then(|&h| char::from_u32(u32::from(h)))
                                    .and_then(|c| c.to_digit(16));
                                let Some(d) = d else {
                                    self.at += k;
                                    return Err(self.bad_escape(cx));
                                };
                                u = u * 16 + d as u16;
                            }
                            self.at += 4;
                            u
                        }
                        _ => return Err(self.bad_escape(cx)),
                    };
                    self.at += 1;
                    out.push(u);
                }
                Some(u) if u < 0x20 => return Err(self.bad_control(cx)),
                Some(u) => {
                    self.at += 1;
                    out.push(u);
                }
            }
        }
    }

    fn unterminated<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = format!("Unterminated string in JSON at position {}", self.at);
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }

    fn bad_control<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = format!(
            "Bad control character in string literal in JSON at position {}",
            self.at
        );
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }

    fn bad_escape<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = format!("Bad escaped character in JSON at position {}", self.at);
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }
}
