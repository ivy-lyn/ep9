//! BigInt values (ECMA-262 §6.1.6.2): the BIGINT cell (kind 9, OBJECT_LAYOUT 7) and the
//! operations on it. The digit arithmetic is `ep9_bigint` (dashu now, the V8 `src/bigint` port
//! at M8); this module moves digits between cells and that crate.
//!
//! ```text
//!  +0  word0: ShapeId 0 (as strings and symbols: every named-load IC case misses), kind 9,
//!      flags 0 (no word0 flag has a BigInt meaning)
//!  +8  u32: bits 0-30 the digit count n, bit 31 the sign (1 = negative)
//!  +12 u32: the hash (0 = not computed yet; a computed 0 is stored as 1)
//!  +16 n x u64 digits, least significant first (the order of `ep9_bytecode::BigIntConst`)
//! ```
//!
//! - Canonical: the top digit is not 0; `0n` has n = 0 and sign 0 (no `-0n`).
//! - Immutable after allocation, except the hash cache. No word is a Value: the GC traces none.
//! - Size: a computed result above `ep9_bigint::MAX_BITS` (2^30) bits is a RangeError "Maximum
//!   BigInt size exceeded" (H23 "BigInt size"). Every operator result and every `asIntN` and
//!   `asUintN` result passes one check, [`from_result`], before its cell is allocated (some
//!   operations also have an early guard before the work). A literal from a unit is not
//!   checked ([`alloc_literal`]: the loader cannot throw): a literal above the limit loads, and an
//!   operator, asIntN or asUintN result above the limit throws, also one that does not grow (`x / 1n`, `-x`, `x >> 0n`); a result that shrinks (`x * 0n`, `asIntN(0, x)`) is a value.
//! - GC rule: an operation reads its operands into an `ep9_bigint::Big` first, then allocates
//!   the result once; no [`View`] is held across an allocation.
//! - Interrupt poll (H23 "Interrupt poll", MC-SPEC-BIGPOLL): BigInt work takes one interrupt
//!   budget unit per `ep9_bigint::POLL_WORDS` digits, through `Cx::poll_interrupt` ([`poller`]).
//!   An operator takes the units of its operands before the work and of its result after it
//!   ([`charge`], [`from_result`]); `**`, StringToBigInt and toString also poll inside the work
//!   (`ep9_bigint` runs them in steps). A poll allocates nothing and runs no collection, so a
//!   [`View`] may be held across it.

use core::cmp::Ordering;

use ep9_bigint::{self as digits, Big, View};
use ep9_heap::{CellKind, CellPtr, CellSize, Heap, Tracer};
use ep9_value::Throw;

use crate::conv::{self, BinOp, Res};
use crate::error::throw_error;
use crate::frame::{FALSE, TRUE};
use crate::interp::Cx;
use crate::kinds::{self, is_bigint};
use crate::object::cell;
use crate::runtime::{ErrorKind, Runtime};

/// BIGINT cell offsets.
pub(crate) mod bf {
    /// u32: digit count (bits 0-30) and sign (bit 31).
    pub const LEN: usize = 8;
    /// u32: the hash cache.
    pub const HASH: usize = 12;
    /// The digits.
    pub const DATA: usize = 16;
    /// The sign bit of the +8 word.
    pub const SIGN: u32 = 1 << 31;
}

fn len_word(c: CellPtr) -> u32 {
    // SAFETY: `c` is a live BIGINT cell (callers check the kind or hold a BigInt Value); +8 is
    // inside its 16-byte minimum size.
    unsafe { c.field::<u32>(bf::LEN).read() }
}

fn cell_size(c: CellPtr) -> usize {
    bf::DATA + 8 * (len_word(c) & !bf::SIGN) as usize
}

/// The digits are raw u64 words, not Values: the GC visits nothing.
fn trace(_c: CellPtr, _t: &mut Tracer<'_>) {}

/// Registers the BIGINT kind (9). `object::register_kinds` calls it.
pub(crate) fn register_kind(heap: &Heap) {
    heap.register_kind(
        kinds::BIGINT,
        CellKind {
            name: "bigint",
            size: CellSize::Dynamic(cell_size),
            trace,
        },
    )
    .expect("register BIGINT");
}

/// A new BIGINT cell with the canonical `neg` and `digits`, with no size check (the unit
/// loader: a literal). GC point.
pub(crate) fn alloc_literal(rt: &Runtime, neg: bool, digits: &[u64]) -> u64 {
    debug_assert!(digits.last() != Some(&0) && !(neg && digits.is_empty()));
    assert!(
        digits.len() < (1 << 31),
        "a BigInt of {} digits",
        digits.len()
    );
    let c = rt.alloc(kinds::BIGINT, bf::DATA + 8 * digits.len());
    let word = digits.len() as u32 | if neg { bf::SIGN } else { 0 };
    // SAFETY: the new cell has room for the +8 and +12 words and `digits.len()` digits at +16;
    // nothing else reads it before this returns.
    unsafe {
        c.field::<u32>(bf::LEN).write(word);
        c.field::<u32>(bf::HASH).write(0);
        core::ptr::copy_nonoverlapping(digits.as_ptr(), c.field::<u64>(bf::DATA), digits.len());
    }
    c.to_value().to_bits()
}

/// A new BIGINT cell for `b`. GC point.
pub(crate) fn alloc(rt: &Runtime, b: &Big) -> u64 {
    alloc_literal(rt, b.neg, &b.digits)
}

/// The cell for a result of `ep9_bigint`, or the RangeError of its error. The one size check
/// of every operation result (H23 "BigInt size"): a unit literal can be above `MAX_BITS`, so a
/// result that does not grow (`x / 1n`, `-x`, `x >> 0n`) can be above it too. A result takes
/// its interrupt budget units first ([`charge`]). GC point.
pub(crate) fn from_result(cx: &mut Cx, r: Result<Big, digits::Error>) -> Res<u64> {
    match r {
        Ok(b) if b.view().bit_len() <= digits::MAX_BITS => {
            charge(cx, b.digits.len())?;
            Ok(alloc(cx.rt(), &b))
        }
        Ok(_) => Err(range_error(cx, digits::Error::TooBig)),
        Err(e) => Err(range_error(cx, e)),
    }
}

/// The poll callback that `ep9_bigint` calls in long work: one interrupt budget unit per call
/// through `Cx::poll_interrupt` (H23 "Interrupt poll"); `false` when the isolate must stop
/// (the termination is then pending). Not a GC point.
pub(crate) fn poller(cx: &mut Cx) -> impl FnMut() -> bool + '_ {
    move || crate::abi::Cx::new(cx).poll_interrupt().is_ok()
}

/// Takes the interrupt budget units of `words` digits of BigInt work: one per
/// `ep9_bigint::POLL_WORDS` (none below it), through [`poller`]. `Err` with the termination
/// pending when the isolate must stop. Not a GC point.
pub(crate) fn charge(cx: &mut Cx, words: usize) -> Res<()> {
    digits::Work::new(&mut poller(cx))
        .add(words)
        .map_err(|_| Throw)
}

/// The exception of an `ep9_bigint` error: the RangeError (V8's messages), or for
/// `Interrupted` no new error (the poll left the termination pending).
pub(crate) fn range_error(cx: &mut Cx, e: digits::Error) -> Throw {
    let msg = match e {
        digits::Error::TooBig => "Maximum BigInt size exceeded",
        digits::Error::DivByZero => "Division by zero",
        digits::Error::NegExponent => "Exponent must be non-negative",
        digits::Error::Interrupted => return Throw,
    };
    throw_error(cx, ErrorKind::RangeError, msg)
}

/// The value of the BigInt `*b`. The view borrows the local that holds the bits, which keeps
/// the cell alive (the conservative scan) on a non-moving heap; still, no caller holds a view
/// across an allocation (module rule).
pub(crate) fn view(b: &u64) -> View<'_> {
    debug_assert!(is_bigint(*b));
    let c = cell(*b);
    let w = len_word(c);
    let n = (w & !bf::SIGN) as usize;
    // SAFETY: `c` is a live BIGINT cell (kind checked in debug builds; callers hold a BigInt
    // Value); its `n` digits start at +16 and are never written after the allocation.
    let d = unsafe { core::slice::from_raw_parts(c.field::<u64>(bf::DATA).cast_const(), n) };
    View::new(w & bf::SIGN != 0, d)
}

/// `0n`?
pub(crate) fn is_zero(b: u64) -> bool {
    view(&b).is_zero()
}

/// SameValue of two BigInts: the same sign and digits.
pub(crate) fn equals(a: u64, b: u64) -> bool {
    a == b || view(&a) == view(&b)
}

/// The content hash of the BigInt `b` (H12-Q2: sign and digits, no address), cached at +12.
/// Not a GC point.
pub(crate) fn hash(b: u64) -> u32 {
    let c = cell(b);
    // SAFETY: as in `len_word`; +12 is the hash word.
    let cached = unsafe { c.field::<u32>(bf::HASH).read() };
    if cached != 0 {
        return cached;
    }
    let h = digits::hash(view(&b), 0).max(1);
    // SAFETY: as above. The cell is immutable except this cache word, which no view covers
    // (views start at +16), and an isolate runs on one thread.
    unsafe { c.field::<u32>(bf::HASH).write(h) };
    h
}

/// Compares two BigInts.
pub(crate) fn compare(a: u64, b: u64) -> Ordering {
    digits::cmp(view(&a), view(&b))
}

/// Compares the BigInt `a` with the Number `x` exactly; `None` for NaN.
pub(crate) fn cmp_number(a: u64, x: f64) -> Option<Ordering> {
    digits::cmp_f64(view(&a), x)
}

/// The most digits that [`decimal`] converts (V8's `BigInt::NoSideEffectsToString` limit): the
/// text of a larger value is not useful in a message, and its conversion would need polls.
const DECIMAL_MAX_WORDS: usize = 100;

/// The decimal text of `b` (no `n`), for messages and `describe`. A value of more than
/// [`DECIMAL_MAX_WORDS`] digits gives `<a very large BigInt>` (V8's text), so a message never
/// runs long work. Not a GC point.
pub(crate) fn decimal(b: u64) -> String {
    let v = view(&b);
    if v.digits.len() > DECIMAL_MAX_WORDS {
        return "<a very large BigInt>".to_string();
    }
    // At most 100 digits: no poll (the work is below one per call of `poller`).
    digits::to_string_radix(v, 10, &mut || true).expect("a poll that never stops")
}

/// BigInt::toString(b, radix) as a heap string. RangeError "Invalid string length" when the
/// text could pass `string::MAX_STRING_LENGTH` (checked from the bit length, before the
/// work; exact for a power-of-2 radix). Polls ([`poller`]). GC point.
pub(crate) fn to_string(cx: &mut Cx, b: u64, radix: u32) -> Res<u64> {
    let bound = digits::string_len_bound(view(&b), radix);
    if bound > crate::string::MAX_STRING_LENGTH as u64 {
        return Err(throw_error(
            cx,
            ErrorKind::RangeError,
            "Invalid string length",
        ));
    }
    let r = digits::to_string_radix(view(&b), radix, &mut poller(cx));
    let text = r.map_err(|e| range_error(cx, e))?;
    Ok(crate::string::new_string_latin1(cx.rt(), text.as_bytes()))
}

/// StringToBigInt (§7.1.14) of the string `s` as digits: `Ok(None)` when the text is not a
/// StringIntegerLiteral; RangeError above the size limit. A long text polls ([`poller`]). Not
/// a GC point (a rope is flattened first: that is the one GC point).
pub(crate) fn string_to_big(cx: &mut Cx, s: u64) -> Res<Option<Big>> {
    let s = crate::string::flatten(cx, s);
    let units = crate::string::view(cx, s).to_utf16();
    // The scans poll as the digit work does: one poll per SCAN_UNITS code units (H23).
    const SCAN_UNITS: usize = 64 * digits::POLL_WORDS as usize;
    let ws = |u: &u16| crate::numconv::is_js_whitespace(*u);
    let mut start = 0;
    while start < units.len() && ws(&units[start]) {
        start += 1;
        if start.is_multiple_of(SCAN_UNITS) {
            charge(cx, digits::POLL_WORDS as usize)?;
        }
    }
    let mut end = units.len();
    while end > start && ws(&units[end - 1]) {
        end -= 1;
        if (units.len() - end).is_multiple_of(SCAN_UNITS) {
            charge(cx, digits::POLL_WORDS as usize)?;
        }
    }
    // The grammar is ASCII: any other code unit makes the text invalid.
    let mut ascii = Vec::with_capacity(end - start);
    for chunk in units[start..end].chunks(SCAN_UNITS) {
        for &u in chunk {
            match u8::try_from(u).ok().filter(u8::is_ascii) {
                Some(c) => ascii.push(c),
                None => return Ok(None),
            }
        }
        if chunk.len() == SCAN_UNITS {
            charge(cx, digits::POLL_WORDS as usize)?;
        }
    }
    let r = digits::parse_literal(&ascii, &mut poller(cx));
    r.map_err(|e| range_error(cx, e))
}

/// `a OP b` on two BigInts (§6.1.6.2). `>>>` is a TypeError. GC point (the result).
/// It takes the budget units of the operands first, and `**` polls in its steps.
pub(crate) fn binary(cx: &mut Cx, op: BinOp, a: u64, b: u64) -> Res<u64> {
    let (x, y) = (view(&a), view(&b));
    charge(cx, x.digits.len() + y.digits.len())?;
    let r = match op {
        BinOp::Add => digits::add(x, y),
        BinOp::Sub => digits::sub(x, y),
        BinOp::Mul => digits::mul(x, y),
        BinOp::Div => digits::div_trunc(x, y),
        BinOp::Mod => digits::rem_trunc(x, y),
        BinOp::Exp => digits::pow(x, y, &mut poller(cx)),
        BinOp::BitOr => digits::or(x, y),
        BinOp::BitXor => digits::xor(x, y),
        BinOp::BitAnd => digits::and(x, y),
        BinOp::Shl => digits::shl(x, y),
        BinOp::Sar => digits::shr(x, y),
        BinOp::Shr => {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "BigInts have no unsigned right shift, use >> instead",
            ));
        }
    };
    from_result(cx, r)
}

/// The BigInt unary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Inc,
    Dec,
    Negate,
    BitNot,
}

/// `OP a` on a BigInt: `a + 1n`, `a - 1n`, `-a`, `~a`. Takes the budget units of the operand
/// first. GC point (the result).
pub(crate) fn unary(cx: &mut Cx, op: UnaryOp, a: u64) -> Res<u64> {
    let x = view(&a);
    charge(cx, x.digits.len())?;
    let one = Big {
        neg: false,
        digits: vec![1],
    };
    let r = match op {
        UnaryOp::Inc => digits::add(x, one.view()),
        UnaryOp::Dec => digits::sub(x, one.view()),
        UnaryOp::Negate => Ok(digits::neg(x)),
        UnaryOp::BitNot => digits::not(x),
    };
    from_result(cx, r)
}

/// `%BigInt.prototype%` of the current realm (the ToObject prototype of a BigInt), or `None`
/// when no builtin table made it (an isolate without `ep9_builtins`: a slot intrinsic is
/// undefined then, and undefined is never a prototype). Not a GC point.
pub(crate) fn prototype(rt: &Runtime) -> Option<u64> {
    let p = crate::builtins::intrinsic(rt, crate::builtins::IntrinsicId::BigIntPrototype);
    kinds::is_object(p).then_some(p)
}

// ---- conversions for the builtin ABI (spec-temporal B4) ---------------------------------

/// ToBigInt (§7.1.13): ToPrimitive (hint number), then by type: a BigInt as it is; true and
/// false give 1n and 0n; a String gives StringToBigInt or a SyntaxError; undefined, null, a
/// Number and a Symbol are a TypeError. GC point.
pub(crate) fn to_bigint(cx: &mut Cx, v: u64) -> Res<u64> {
    let p = conv::to_primitive(cx, v, conv::Hint::Number)?;
    if is_bigint(p) {
        return Ok(p);
    }
    if p == TRUE || p == FALSE {
        return Ok(alloc(cx.rt(), &digits::from_i64(i64::from(p == TRUE))));
    }
    if crate::object::is_string(p) {
        return match string_to_big(cx, p)? {
            Some(b) => Ok(alloc(cx.rt(), &b)),
            None => {
                let msg = format!(
                    "Cannot convert {} to a BigInt",
                    crate::string::to_rust_string(p)
                );
                Err(throw_error(cx, ErrorKind::SyntaxError, &msg))
            }
        };
    }
    let msg = format!("Cannot convert {} to a BigInt", conv::describe(p));
    Err(throw_error(cx, ErrorKind::TypeError, &msg))
}

/// NumberToBigInt (§21.2.1.1.1): RangeError if `n` is not an integral Number. GC point.
pub(crate) fn number_to_bigint(cx: &mut Cx, n: f64) -> Res<u64> {
    match digits::from_f64_exact(n) {
        Some(b) => Ok(alloc(cx.rt(), &b)),
        None => {
            let msg = format!(
                "The number {} cannot be converted to a BigInt because it is not an integer",
                conv::describe(conv::num(n))
            );
            Err(throw_error(cx, ErrorKind::RangeError, &msg))
        }
    }
}

/// 𝔽(ℝ(b)): the Number nearest to the BigInt `b` (round half to even; Infinity past the
/// largest finite Number). Not a GC point.
pub(crate) fn to_f64(b: u64) -> f64 {
    digits::to_f64(view(&b))
}

/// BigInt.asIntN (`signed`) or BigInt.asUintN of the BigInt `b` with `bits` (already
/// ToIndex). RangeError above the size limit. Takes the budget units of `b` first. GC point.
pub(crate) fn as_n(cx: &mut Cx, b: u64, bits: u64, signed: bool) -> Res<u64> {
    charge(cx, view(&b).digits.len())?;
    let r = if signed {
        digits::as_int_n(view(&b), bits)
    } else {
        digits::as_uint_n(view(&b), bits)
    };
    from_result(cx, r)
}

/// The BigInt of `n`. GC point.
pub(crate) fn from_i128(rt: &Runtime, n: i128) -> u64 {
    alloc(rt, &digits::from_i128(n))
}

/// The BigInt `b` as an i128, if it fits. Not a GC point.
pub(crate) fn to_i128(b: u64) -> Option<i128> {
    digits::to_i128(view(&b))
}
