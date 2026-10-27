//! Exceptions: Error objects with a stack, throw helpers and the unwinder.
//!
//! ```text
//! throw at (fp, pc) ──> pending = exception ──> unwind(fp, pc)
//!   frame loop:  bytecode frame? handler table: innermost range that holds the offset
//!                  found: context = handler register; acc = exception; pc = target
//!                return pc == EXIT stub? -> out_pc = EXIT stub, acc = 0: the run returns
//!                                           Err(Throw) to Rust, the exception stays pending
//!                else pop: fp = caller fp; offset = (return pc - 1) in the caller
//! ```
//!
//! Positions (H14): the current offset of a frame is the return pc of the frame above it,
//! minus one (inside the call instruction). When that return pc is a VM stub (an entry from
//! Rust), the frame's own header slot 7 holds the offset that `Runtime::enter` saved.

use ep9_value::Throw;

use crate::conv::Res;
use crate::frame::{CONSTRUCT_BIT, HEADER, int32_of, native_ptr, slot};
use crate::interp::{Cx, EXIT_STUB};
use crate::object::{self, attr, is_error, is_string, own_define};
use crate::runtime::{Atom, ErrorKind, Runtime, Site};
use crate::unit::FunctionInfo;

/// A read-only view of one VM frame.
#[derive(Clone, Copy)]
pub(crate) struct FrameRef(pub *mut u64);

impl FrameRef {
    #[inline]
    fn word(self, s: isize) -> u64 {
        // SAFETY: a FrameRef wraps a live frame pointer; header slots lie below it.
        unsafe { *self.0.offset(s) }
    }

    pub fn info<'a>(self) -> &'a FunctionInfo {
        // SAFETY: header slot 4 holds the native word of a FunctionInfo that outlives frames.
        unsafe { &*native_ptr::<FunctionInfo>(self.word(slot::FUNCTION_INFO)) }
    }

    pub fn caller(self) -> FrameRef {
        FrameRef(native_ptr(self.word(slot::CALLER_FP)))
    }

    pub fn return_pc(self) -> *const u8 {
        native_ptr::<u8>(self.word(slot::RETURN_PC))
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    pub fn argc(self) -> u32 {
        int32_of(self.word(slot::ARGC)) as u32 & !CONSTRUCT_BIT
    }

    pub fn saved_offset(self) -> u32 {
        int32_of(self.word(slot::BYTECODE_OFFSET)) as u32
    }

    /// Argument `i` (`i < max(argc, parameter_count)`).
    pub fn arg(self, i: u32) -> u64 {
        self.word(slot::PARAM0 - i as isize)
    }
}

/// One JS frame of a captured stack.
#[derive(Clone, Debug)]
pub(crate) struct StackFrame {
    pub name: String,
    pub site: Option<Site>,
}

/// Walks the VM frames from the top (`rt.stack().fp()`); returns up to `limit` JS frames.
pub(crate) fn capture(rt: &Runtime, limit: usize) -> Vec<StackFrame> {
    let mut out = Vec::new();
    let mut f = FrameRef(rt.stack().fp());
    let mut above_ret: *const u8 = core::ptr::null();
    while !f.is_null() && out.len() < limit {
        let fi = f.info();
        if fi.is_bytecode() {
            let off = fi
                .offset_of(above_ret.wrapping_sub(1))
                .unwrap_or_else(|| f.saved_offset());
            let site = fi
                .loaded_unit()
                .zip(fi.source_offset(off))
                .map(|(u, o)| Site {
                    unit: u.index,
                    offset: o,
                });
            out.push(StackFrame {
                name: fi.name.clone(),
                site,
            });
        }
        above_ret = f.return_pc();
        f = f.caller();
    }
    out
}

/// `file:line:col` of a site (UTF-16 column), or `file` when the unit has no source.
pub(crate) fn site_text(rt: &Runtime, s: Site) -> String {
    // SAFETY: unit indices in sites come from loaded units, which live as long as `rt`.
    let u = unsafe { &*rt.unit(s.unit) };
    match u.line_column(s.offset) {
        Some((l, c)) => format!("{}:{l}:{c}", u.file),
        None => u.file.clone(),
    }
}

pub(crate) fn stack_lines(rt: &Runtime, frames: &[StackFrame]) -> String {
    let mut s = String::new();
    for f in frames {
        let at = f
            .site
            .map(|x| site_text(rt, x))
            .unwrap_or_else(|| "<anonymous>".into());
        if f.name.is_empty() {
            s.push_str(&format!("\n    at {at}"));
        } else {
            s.push_str(&format!("\n    at {} ({at})", f.name));
        }
    }
    s
}

/// "Name: message" of an error object, without running JS: a `name` or `message` accessor
/// counts as absent, and a routed object's hooks are not asked (`property::get_data_from`).
pub(crate) fn error_header(cx: &mut Cx, e: u64) -> String {
    let rt = cx.rt();
    let read = |cx: &mut Cx, a: Atom| -> String {
        let key = rt.atom(a);
        match crate::property::get_data_from(cx, e, key) {
            // Rope-safe and no GC point (an error report may run near the heap limit).
            Some(v) if is_string(v) => crate::string::to_rust_string(v),
            _ => String::new(),
        }
    };
    let name = read(cx, Atom::Name);
    let msg = read(cx, Atom::Message);
    let name = if name.is_empty() {
        "Error".into()
    } else {
        name
    };
    if msg.is_empty() {
        name
    } else {
        format!("{name}: {msg}")
    }
}

/// Captures the VM stack into the `stack` property of the new error `e` and records its site.
pub(crate) fn install_stack(cx: &mut Cx, e: u64) {
    let rt = cx.rt();
    let frames = capture(rt, rt.stack_trace_limit);
    if let Some(site) = frames.iter().find_map(|f| f.site) {
        object::set_error_site(rt, e, site.unit, site.offset);
    }
    let text = format!("{}{}", error_header(cx, e), stack_lines(rt, &frames));
    let s = message_string(rt, &text);
    own_define(rt, e, rt.atom(Atom::Stack), s, attr::HIDDEN);
}

/// The text V8 gives a message that cannot be made (node v24.22.1: `undefined[s]` with `s` of
/// 2^29 - 24 units throws a TypeError whose message is this text).
const OVER_LIMIT_MESSAGE: &str = "<error>";

/// A heap string of a message or stack text built in Rust (H19-Q5). Quoted values stay whole,
/// as in V8; a text over the string length limit (2^29 - 24 code units) becomes
/// [`OVER_LIMIT_MESSAGE`], so a huge key in a message never reaches the allocator's limit
/// assert. GC point.
pub(crate) fn message_string(rt: &Runtime, text: &str) -> u64 {
    object::new_string(rt, bounded_text(text, crate::string::MAX_STRING_LENGTH))
}

/// One part of a message built by [`bounded_message`].
#[derive(Clone, Copy)]
pub(crate) enum Part<'a> {
    /// Rust text of the message itself (short).
    Text(&'a str),
    /// The text of a key or value as V8 quotes it: a string's text (whole, H19-Q5), a
    /// symbol's `Symbol(desc)`, else `conv::describe` (a number's text, `undefined`, ...).
    Key(u64),
}

/// The UTF-16 length of [`Part::Key`]'s text, from the string lengths: O(1) for a rope.
fn key_units(k: u64) -> usize {
    if is_string(k) {
        crate::string::str_len(k)
    } else if crate::kinds::is_symbol(k) {
        let d = crate::symbol::description(k);
        "Symbol()".len()
            + if is_string(d) {
                crate::string::str_len(d)
            } else {
                0
            }
    } else {
        crate::conv::describe(k).encode_utf16().count()
    }
}

/// The text of [`Part::Key`] (lone surrogates become U+FFFD, one unit each, so its UTF-16
/// length is [`key_units`]).
fn key_text(k: u64) -> String {
    if is_string(k) {
        crate::string::to_rust_string(k)
    } else if crate::kinds::is_symbol(k) {
        crate::symbol::descriptive_text(k)
    } else {
        crate::conv::describe(k)
    }
}

/// A message that quotes keys or values, as Rust text for [`throw_error`] (H19-Q5; M4-P4
/// fixer B, Codex H19 H-1): the UTF-16 length is counted first, from the string lengths, so
/// a message over the length limit is [`OVER_LIMIT_MESSAGE`] with no copy of any quoted
/// string (a rope near the limit is never flattened or read into Rust text). Under the limit
/// every quoted value is whole, as in V8. Not a GC point.
pub(crate) fn bounded_message(parts: &[Part<'_>]) -> String {
    let units: usize = parts
        .iter()
        .map(|p| match *p {
            Part::Text(t) => t.encode_utf16().count(),
            Part::Key(k) => key_units(k),
        })
        .sum();
    if units > crate::string::MAX_STRING_LENGTH {
        return OVER_LIMIT_MESSAGE.into();
    }
    let mut out = String::new();
    for p in parts {
        match *p {
            Part::Text(t) => out.push_str(t),
            Part::Key(k) => out.push_str(&key_text(k)),
        }
    }
    out
}

/// `text`, or [`OVER_LIMIT_MESSAGE`] if it has more than `limit` UTF-16 code units. The exact
/// count runs only when the UTF-8 length is over the limit (UTF-16 length <= UTF-8 length).
fn bounded_text(text: &str, limit: usize) -> &str {
    if text.len() > limit && text.encode_utf16().count() > limit {
        OVER_LIMIT_MESSAGE
    } else {
        text
    }
}

/// A new error of `kind` with `message` (a string value or `None`), with a stack.
pub(crate) fn new_error_value(cx: &mut Cx, proto: u64, message: Option<u64>) -> u64 {
    let rt = cx.rt();
    let e = object::new_error_object(rt, proto);
    if let Some(m) = message {
        own_define(rt, e, rt.atom(Atom::Message), m, attr::HIDDEN);
    }
    install_stack(cx, e);
    e
}

pub(crate) fn new_error(cx: &mut Cx, kind: ErrorKind, msg: &str) -> u64 {
    let rt = cx.rt();
    let m = message_string(rt, msg);
    new_error_value(cx, rt.root(kind.prototype_root()), Some(m))
}

/// Creates an error and makes it the pending exception.
pub(crate) fn throw_error(cx: &mut Cx, kind: ErrorKind, msg: &str) -> Throw {
    let e = new_error(cx, kind, msg);
    cx.rt().throw_value(e);
    Throw
}

/// The "not implemented" error of v0 (a catchable `Error`, never UB).
pub(crate) fn not_implemented(cx: &mut Cx, what: &str) -> Throw {
    throw_error(
        cx,
        ErrorKind::Error,
        &format!("not implemented in ep9 v0: {what}"),
    )
}

/// `Err(Throw)` with a RangeError for stack overflow.
pub(crate) fn stack_overflow<T>(cx: &mut Cx) -> Res<T> {
    Err(throw_error(
        cx,
        ErrorKind::RangeError,
        "Maximum call stack size exceeded",
    ))
}

/// The source site and stack text of an exception value, for the embedder report.
pub(crate) fn describe_exception(rt: &Runtime, exc: u64) -> (Option<Site>, Option<String>) {
    if is_error(exc) {
        let site = object::error_site(exc).map(|(unit, offset)| Site { unit, offset });
        let stack = object::own_get(exc, rt.atom(Atom::Stack))
            .filter(|(v, _)| is_string(*v))
            .map(|(v, _)| crate::string::to_rust_string(v));
        (site, stack)
    } else {
        (rt.shared.throw_site.get(), None)
    }
}

/// Finds the handler for a throw at `pc` in the frame `fp` or below it (see module docs).
/// Returns the accumulator for the dispatch: the exception (a handler runs next) or 0 (the run
/// exits to Rust with the exception pending). Sets `cx.out_fp` and `cx.out_pc`.
pub(crate) fn unwind(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    let rt = cx.rt();
    let exc = rt.pending();
    debug_assert!(exc != 0, "unwind without a pending exception");
    let terminating = rt.terminating.get();
    let mut f = FrameRef(fp);
    let mut off = f.info().offset_of(pc);
    loop {
        let fi = f.info();
        if !terminating
            && let Some(o) = off
            && let Some(h) = find_handler(fi, o)
        {
            // SAFETY: the handler's context register is a local of this live frame (verifier).
            unsafe {
                let ctx = *f.0.add(h.0 as usize);
                *f.0.offset(slot::CONTEXT) = ctx;
            }
            rt.take_pending();
            cx.out_fp = f.0;
            cx.out_pc = fi.code.wrapping_add(h.1 as usize);
            return exc;
        }
        let ret = f.return_pc();
        let caller = f.caller();
        if core::ptr::eq(ret, EXIT_STUB.as_ptr()) {
            cx.out_fp = caller.0;
            cx.out_pc = EXIT_STUB.as_ptr();
            return 0;
        }
        f = caller;
        off = f.info().offset_of(ret.wrapping_sub(1));
    }
}

/// (context register, target offset) of the innermost handler that covers `off`.
fn find_handler(fi: &FunctionInfo, off: u32) -> Option<(u32, u32)> {
    let u = fi.loaded_unit()?;
    let f = &u.unit.functions[fi.function_id as usize];
    f.handlers
        .iter()
        .rev()
        .find(|h| h.start <= off && off < h.end)
        .map(|h| {
            let r = h
                .context
                .local_index()
                .expect("handler context is a local (verifier)");
            (r, h.target)
        })
}

const _: () = assert!(HEADER == 8);

#[cfg(test)]
pub(crate) mod testing {
    //! Hooks for tests/strings.rs (H19-Q5).
    pub(crate) fn bounded_text(text: &str, limit: usize) -> &str {
        super::bounded_text(text, limit)
    }
}
