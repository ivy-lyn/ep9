//! Function builtins (§20.2): module `function`, block 2. Targets `%Function%` and
//! `%Function.prototype%` (`IntrinsicId::Function`, `IntrinsicId::FunctionPrototype`).
//!
//! `%Function.prototype%` itself and the `Function` constructor (v0: throws) are VM builtins
//! (block 0). Week 4 (P3-BLTA): `Function.prototype.call`, `apply`, `bind`. M6-P2 FN-SMALL:
//! `toString` gives the source text of a function whose unit embeds its source.
//!
//! | Builtin | Template | Pattern |
//! |---|---|---|
//! | `call` | 6 | IsCallable(this); forward `args.rest(1)` with `Cx::call_args` (one copy, into the callee frame) |
//! | `apply` | companion | CreateListFromArrayLike into an `ArgsBuilder` (VM stack), then `Cx::call_built` |
//! | `bind` | 7 | BoundFunctionCreate (`Cx::new_bound_function`: a BOUND_FUNCTION cell; its `[[Call]]` and `[[Construct]]` are in the VM), then SetFunctionLength and SetFunctionName |
//! | `toString` | companion | the source text (`Cx::function_source_text`, a copy of the unit range made on each call) when the range is the whole `[[SourceText]]` (see [`is_whole_source_text`]); else the NativeFunction form, a builtin's named by its `[[InitialName]]` (`Cx::initial_name`); TypeError for a value that is not callable |
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-function.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsFunction, JsResult,
    Local, ObjectProps, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 2;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const CALL: u16 = 0;
    pub const APPLY: u16 = 1;
    pub const BIND: u16 = 2;
    pub const TO_STRING: u16 = 3;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("call", 1, call),          // 2.0
    BuiltinDef::method("apply", 2, apply),        // 2.1
    BuiltinDef::method("bind", 1, bind),          // 2.2
    BuiltinDef::method("toString", 0, to_string), // 2.3
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[ObjectProps {
    target: IntrinsicId::FunctionPrototype,
    props: &[
        PropDef::method(PropKey::Str("apply"), def::APPLY),
        PropDef::method(PropKey::Str("bind"), def::BIND),
        PropDef::method(PropKey::Str("call"), def::CALL),
        PropDef::method(PropKey::Atom(Atom::ToStringKey), def::TO_STRING),
    ],
}];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "function",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The TypeError of a `Function.prototype` method called on a value that is not callable.
fn not_callable<'s>(cx: &mut Cx<'s>, method: &str, this: Local<'s>) -> ep9_vm::Throw {
    let what = cx.type_of(this);
    let msg =
        format!("Function.prototype.{method} called on a value that is not a function ({what})");
    cx.throw_type_error(&msg)
}

/// Template 6. Function.prototype.call ( thisArg, ...args ) (§20.2.3.3).
fn call<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let func be the this value.
    let func = this;
    // 2. If IsCallable(func) is false, throw a TypeError exception.
    if !func.is_callable() {
        return Err(not_callable(cx, "call", func));
    }
    // 3. Perform PrepareForTailCall() (v0: no tail call; H20-Q3).
    // 4. Return ? Call(func, thisArg, args). The arguments after thisArg go to the callee
    //    frame in one copy (`call_args`), never through a heap list.
    cx.call_args(func, args.get(0), args.rest(1))
}

/// Function.prototype.apply ( thisArg, argArray ) (§20.2.3.1).
fn apply<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let func be the this value.
    let func = this;
    // 2. If IsCallable(func) is false, throw a TypeError exception.
    if !func.is_callable() {
        return Err(not_callable(cx, "apply", func));
    }
    let (this_arg, arg_array) = (args.get(0), args.get(1));
    // 3. If argArray is either undefined or null, then
    if arg_array.is_undefined_or_null() {
        // a. Perform PrepareForTailCall().
        // b. Return ? Call(func, thisArg).
        return cx.call(func, this_arg, &[]);
    }
    // 4. Let argList be ? CreateListFromArrayLike(argArray). It is built on the VM stack
    //    (an ArgsBuilder: precise roots, no heap list).
    let arg_list = cx.create_list_from_array_like(arg_array)?;
    // 5. Perform PrepareForTailCall().
    // 6. Return ? Call(func, thisArg, argList).
    cx.call_built(func, this_arg, arg_list)
}

/// Template 7. Function.prototype.bind ( thisArg, ...args ) (§20.2.3.2).
fn bind<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let Target be the this value.
    // 2. If IsCallable(Target) is false, throw a TypeError exception.
    let Some(target) = this.as_function() else {
        return Err(cx.throw_type_error("Bind must be called on a function"));
    };
    // 3. Let F be ? BoundFunctionCreate(Target, thisArg, args). `args` is a tail of this
    //    call's arguments; the VM copies it into F's bound-argument list.
    let bound_args = args.rest(1);
    let f = cx.new_bound_function(target, args.get(0), bound_args)?;
    // 4. Let L be 0.
    let mut l = 0.0;
    // 5. Let targetHasLength be ? HasOwnProperty(Target, "length").
    // 6. If targetHasLength is true, then
    if cx.has_own_property(target, "length")? {
        // a. Let targetLen be ? Get(Target, "length").
        let target_len = cx.get(target, "length")?;
        // b. If targetLen is a Number, then
        if let Some(n) = target_len.as_number() {
            if n == f64::INFINITY {
                // i. If targetLen is +∞𝔽, set L to +∞.
                l = f64::INFINITY;
            } else if n == f64::NEG_INFINITY {
                // ii. Else if targetLen is -∞𝔽, set L to 0.
                l = 0.0;
            } else {
                // iii. Else,
                // 1. Let targetLenAsInt be ! ToIntegerOrInfinity(targetLen).
                let target_len_as_int = cx.to_integer_or_infinity(target_len)?;
                // 2. Assert: targetLenAsInt is finite.
                debug_assert!(target_len_as_int.is_finite());
                // 3. Let argCount be the number of elements in args.
                let arg_count = bound_args.len() as f64;
                // 4. Set L to max(targetLenAsInt - argCount, 0) (+0, never -0).
                let d = target_len_as_int - arg_count;
                l = if d > 0.0 { d } else { 0.0 };
            }
        }
    }
    // 7. Perform SetFunctionLength(F, L).
    cx.set_function_length(f, l)?;
    // 8. Let targetName be ? Get(Target, "name").
    let mut target_name = cx.get(target, "name")?;
    // 9. If targetName is not a String, set targetName to the empty String.
    if !target_name.is_string() {
        target_name = cx.key("").value();
    }
    // 10. Perform SetFunctionName(F, targetName, "bound").
    cx.set_function_name(f, target_name, Some("bound"))?;
    // 11. Return F.
    Ok(f.value())
}

/// Is `s` an ASCII IdentifierName (`[A-Za-z$_][A-Za-z0-9$_]*`)? Other names are left out of
/// the NativeFunction text rather than checked against the Unicode ID tables.
fn is_ascii_identifier_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '$' || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '$' || c == '_')
}

/// Function.prototype.toString ( ) (§20.2.3.5).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let func be the this value.
    let func = this;
    // 5. Throw a TypeError exception (moved up: a non-callable reaches no other step).
    let Some(func) = func.as_function() else {
        let what = cx.type_of(func);
        let msg =
            format!("Function.prototype.toString requires that 'this' be a Function ({what})");
        return Err(cx.throw_type_error(&msg));
    };
    // 2. If func is an Object, func has a [[SourceText]] internal slot, func.[[SourceText]]
    //    is a sequence of Unicode code points, and HostHasSourceTextAvailable(func) is true,
    //    then return CodePointsToString(func.[[SourceText]]).
    // HostHasSourceTextAvailable(func) is true for a bytecode function whose unit embeds its
    // source and whose source range is its whole [[SourceText]]; false for every other one,
    // which then takes step 4 (the spec lets the host answer per function).
    if let Some(text) = source_text(cx, func)? {
        return Ok(text);
    }
    // 3. If func is a built-in function object, return an implementation-defined String
    //    source code representation of func. The representation must have the syntax of a
    //    NativeFunction. Additionally, if func has an [[InitialName]] internal slot and
    //    func.[[InitialName]] is a String, the portion of the returned String that would be
    //    matched by NativeFunctionAccessor_opt PropertyName must be the value of
    //    func.[[InitialName]].
    //    (Not the `name` property: it may have been redefined or deleted.)
    let name = if let Some(initial_name) = cx.initial_name(func) {
        cx.to_rust_string(initial_name)?
    } else {
        // 4. If func is an Object and IsCallable(func) is true, return an
        //    implementation-defined String source code representation of func. The
        //    representation must have the syntax of a NativeFunction.
        // The name: the own data property `name` (no getter runs) when it is a string of the
        // NativeFunction syntax: an IdentifierName, or "get " / "set " and one. Else none
        // (a proxy too: it is asked nothing, so no trap runs, as in V8).
        let mut name = String::new();
        if !cx.is_proxy(func)
            && let Some(d) = cx.get_own_property(func, "name")?
            && let Some(v) = d.value
            && v.is_string()
        {
            let text = cx.to_rust_string(v)?;
            let bare = text
                .strip_prefix("get ")
                .or_else(|| text.strip_prefix("set "))
                .unwrap_or(&text);
            if is_ascii_identifier_name(bare) {
                name = text;
            }
        }
        name
    };
    // As V8 prints it: `function f() { [native code] }`, `function () { [native code] }`.
    let text = format!("function {name}() {{ [native code] }}");
    Ok(cx.new_string(&text)?.value())
}

/// Step 2 of `toString`: the source text of `func` (a new String, copied from the unit on
/// each call), or `None` when `func` has none we can give: no embedded source (a host,
/// bound or proxy function, a unit compiled without its source), or a range that is not
/// the whole `[[SourceText]]` ([`is_whole_source_text`]). Runs no JS.
fn source_text<'s>(cx: &mut Cx<'s>, func: Local<'s, JsFunction>) -> JsResult<Option<Local<'s>>> {
    let Some(text) = cx.function_source_text(func)? else {
        return Ok(None);
    };
    let constructor = cx.is_constructor(func);
    let rust = cx.to_rust_string(text)?;
    Ok(is_whole_source_text(&rust, constructor).then(|| text.value()))
}

/// Is `text`, the source range ep9_frontend recorded for a function, its whole
/// `[[SourceText]]` (§15.2-§15.8: the text the function's syntax node matched)?
/// `constructor` is IsConstructor of the function.
///
/// The range is right for a function declaration or expression of each kind (`function`,
/// `function*`, `async function`, `async function*`) and for an arrow function (plain or
/// `async`). For a method, a getter, a setter and a class constructor the frontend today
/// records the text from the parameters on (`(a) { }`) or the class body (`{ }`), not from
/// the key or the `class` keyword (`Cx::function_source_text`); the `Function`
/// constructor's function has the text `function(a\n) {\n...\n}` of its wrapper (no
/// `anonymous`, ep9_frontend `function_ctor.rs` F4). Those take the NativeFunction form, as
/// does any text whose shape this check cannot read without a full parse: an arrow whose
/// parameters hold a `/` that is not a comment (a regular expression or a division) or a
/// template. So a `true` is the exact text and a `false` is always safe (step 4).
///
/// | Text starts with | Kind | `true` when |
/// |---|---|---|
/// | `function`, `async` `function` | declaration or expression | not the `Function` constructor's wrapper text |
/// | `class` | class constructor (the frontend does not record this yet) | `constructor` |
/// | `(`, an identifier, `async` | arrow, or a method from its parameters | not `constructor`, and the parameters are followed by `=>` |
/// | anything else (`{` of a class body) | - | never |
fn is_whole_source_text(text: &str, constructor: bool) -> bool {
    let s = text.as_bytes();
    // `function` or `async function` (comments or white space between the two words).
    let function_at = match keyword(s, 0, b"async") {
        Some(after) => {
            let next = skip_trivia(s, after);
            (next > after).then_some(next)
        }
        None => Some(0),
    };
    if function_at.is_some_and(|at| keyword(s, at, b"function").is_some()) {
        return !is_function_ctor_text(text);
    }
    if keyword(s, 0, b"class").is_some() {
        return constructor;
    }
    // An arrow function is never a constructor (§15.3.4); a class constructor is one.
    !constructor && is_arrow_function(s)
}

/// The wrapper text of a function the `Function` constructor made (`function_ctor.rs`
/// `Texts::new`): one of the four kind prefixes right before `(`, then the parameters,
/// `\n) {\n`, the body and `\n}`. Its `[[SourceText]]` (§20.2.1.1.1 step 20) has `anonymous`
/// after the prefix, which the range does not. A function written in this exact shape takes
/// the NativeFunction form too (safe).
fn is_function_ctor_text(text: &str) -> bool {
    [
        "function(",
        "function*(",
        "async function(",
        "async function*(",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
        && text.contains("\n) {\n")
        && text.ends_with("\n}")
}

/// Does `s` start with an arrow function's parameters (`x`, `(...)`, `async x`, `async (...)`)
/// followed by `=>`?
fn is_arrow_function(s: &[u8]) -> bool {
    let arrow_after = |params_at: usize| {
        arrow_params_end(s, params_at)
            .is_some_and(|end| s[skip_trivia(s, end)..].starts_with(b"=>"))
    };
    // `x =>` and `(...) =>` (this also takes `async => 0`, a parameter named `async`).
    if arrow_after(0) {
        return true;
    }
    // `async x =>` and `async (...) =>`.
    keyword(s, 0, b"async").is_some_and(|after| arrow_after(skip_trivia(s, after)))
}

/// The end of the arrow parameters at `i`: one identifier, or a parenthesized list.
fn arrow_params_end(s: &[u8], i: usize) -> Option<usize> {
    if s.get(i) == Some(&b'(') {
        balanced_end(s, i)
    } else {
        identifier_end(s, i)
    }
}

/// The end of the identifier at `i` (ASCII `[A-Za-z$_][A-Za-z0-9$_]*`, or any non-ASCII
/// alphanumeric character); `None` when none starts there. A `\u` escape is not read (the
/// caller then gives the NativeFunction form).
fn identifier_end(s: &[u8], i: usize) -> Option<usize> {
    let text = std::str::from_utf8(s.get(i..)?).ok()?;
    let mut end = 0;
    for (k, c) in text.char_indices() {
        let ok = c == '$'
            || c == '_'
            || c.is_ascii_alphabetic()
            || (k > 0 && c.is_ascii_digit())
            || (!c.is_ascii() && c.is_alphanumeric());
        if !ok {
            break;
        }
        end = k + c.len_utf8();
    }
    (end > 0).then_some(i + end)
}

/// The end of the word `word` at `i` when it is a whole word there (the next byte is not an
/// identifier byte).
fn keyword(s: &[u8], i: usize, word: &[u8]) -> Option<usize> {
    let end = i + word.len();
    let whole = s.get(i..end) == Some(word)
        && !s.get(end).is_some_and(|&b| {
            b == b'$' || b == b'_' || b == b'\\' || b.is_ascii_alphanumeric() || b >= 0x80
        });
    whole.then_some(end)
}

/// Skip white space, line terminators and comments from `i`.
fn skip_trivia(s: &[u8], mut i: usize) -> usize {
    loop {
        match s.get(i..) {
            Some([b'/', b'/', ..]) => i = line_comment_end(s, i),
            Some([b'/', b'*', ..]) => match block_comment_end(s, i) {
                Some(end) => i = end,
                None => return i,
            },
            Some([b, ..]) if b.is_ascii_whitespace() || *b == 0x0B => i += 1,
            Some([b, ..]) if *b >= 0x80 => {
                // A non-ASCII white space or line terminator (NBSP, ZWNBSP, LS, PS, Zs).
                match std::str::from_utf8(&s[i..])
                    .ok()
                    .and_then(|t| t.chars().next())
                {
                    Some(c) if c.is_whitespace() || c == '\u{FEFF}' => i += c.len_utf8(),
                    _ => return i,
                }
            }
            _ => return i,
        }
    }
}

/// The end of the `//` comment at `i`: the next line terminator (LF, CR, LS, PS) or the end.
fn line_comment_end(s: &[u8], i: usize) -> usize {
    let mut j = i + 2;
    while j < s.len() {
        match &s[j..] {
            [b'\n' | b'\r', ..] | [0xE2, 0x80, 0xA8 | 0xA9, ..] => return j,
            _ => j += 1,
        }
    }
    j
}

/// The end of the `/*` comment at `i` (after its `*/`); `None` when it does not close.
fn block_comment_end(s: &[u8], i: usize) -> Option<usize> {
    s.get(i + 2..)?
        .windows(2)
        .position(|w| w == b"*/")
        .map(|k| i + 2 + k + 2)
}

/// The end of the bracketed text that starts with the `(` at `i` (after its matching `)`),
/// skipping strings and comments. `None` (so the NativeFunction form) when the text holds a
/// template or a `/` that is not a comment (a regular expression or a division: telling
/// them apart needs the parser), or when the brackets do not match.
fn balanced_end(s: &[u8], i: usize) -> Option<usize> {
    let mut open: Vec<u8> = Vec::new();
    let mut j = i;
    while j < s.len() {
        match s[j] {
            b'(' | b'[' | b'{' => {
                open.push(s[j]);
                j += 1;
            }
            b')' | b']' | b'}' => {
                let want = match s[j] {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if open.pop() != Some(want) {
                    return None;
                }
                j += 1;
                if open.is_empty() {
                    return Some(j);
                }
            }
            q @ (b'\'' | b'"') => {
                j += 1;
                loop {
                    match *s.get(j)? {
                        b'\\' => j += 2,
                        b'\n' | b'\r' => return None,
                        b if b == q => break,
                        _ => j += 1,
                    }
                }
                j += 1;
            }
            b'/' => match s.get(j + 1) {
                Some(b'/') => j = line_comment_end(s, j),
                Some(b'*') => j = block_comment_end(s, j)?,
                _ => return None,
            },
            b'`' => return None,
            _ => j += 1,
        }
    }
    None
}
