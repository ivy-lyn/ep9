//! Encoding tests for SP-Value. They also run under Miri (`cargo +nightly miri test`).

use core::hint::black_box;
use core::ptr::NonNull;
use std::format;
use std::vec::Vec;

use crate::encoding::*;
use crate::{JsResult, ShapeId, Throw, Unpacked, Value, Word0};

const ENC_OFFSET: u64 = DOUBLE_ENCODE_OFFSET;
const PURE_NAN_ENCODED: u64 = PURE_NAN_BITS + ENC_OFFSET;

/// Exactly one kind test is true for every valid value.
fn assert_one_kind(v: Value) {
    let kinds = [
        v.is_cell(),
        v.is_int32(),
        v.is_double(),
        v.is_bool(),
        v.is_undefined(),
        v.is_null(),
        v.is_empty(),
    ];
    assert_eq!(
        kinds.iter().filter(|k| **k).count(),
        1,
        "{v:?} bits {:#x}",
        v.to_bits()
    );
    assert_eq!(v.is_number(), v.is_int32() || v.is_double());
    assert_eq!(v.is_undefined_or_null(), v.is_undefined() || v.is_null());
    assert!(Value::is_valid_bits(v.to_bits()), "{:#x}", v.to_bits());
}

/// Checks a double-form value against the f64 it was built from.
fn assert_double_round_trip(d: f64) {
    let v = Value::double(d);
    assert_one_kind(v);
    assert!(v.is_double() && !v.is_cell() && !v.is_int32());
    let top16 = v.to_bits() >> 48;
    // Purified doubles use top 16 bits 0x0002 (+0) ..= 0xFFF2 (-inf); JSC allows up to 0xFFFC.
    assert!((0x0002..=0xFFF2).contains(&top16), "{d:?} top16 {top16:#x}");
    let back = v.as_double().unwrap();
    if d.is_nan() {
        assert!(back.is_nan());
        assert_eq!(back.to_bits(), PURE_NAN_BITS);
        assert_eq!(v.to_bits(), PURE_NAN_ENCODED);
    } else {
        assert_eq!(back.to_bits(), d.to_bits(), "{d:?}");
        assert_eq!(v.to_bits(), d.to_bits().wrapping_add(ENC_OFFSET));
    }
    assert_eq!(v.as_int32(), None);
    assert_eq!(v.as_bool(), None);
    assert_eq!(v.as_cell_ptr(), None);
}

/// Checks `Value::number`: int32 exactly when `d` is an i32 and not -0.
fn assert_number(d: f64) {
    let v = Value::number(d);
    assert_one_kind(v);
    let fits = d.fract() == 0.0
        && d >= i32::MIN as f64
        && d <= i32::MAX as f64
        && !(d == 0.0 && d.is_sign_negative());
    assert_eq!(v.is_int32(), fits, "{d:?}");
    let n = v.as_number().unwrap();
    if d.is_nan() {
        assert_eq!(n.to_bits(), PURE_NAN_BITS);
    } else {
        assert_eq!(n.to_bits(), d.to_bits(), "{d:?}");
    }
}

#[test]
fn immediates_have_the_documented_bits() {
    assert_eq!(Value::NULL.to_bits(), 0x02);
    assert_eq!(Value::FALSE.to_bits(), 0x06);
    assert_eq!(Value::TRUE.to_bits(), 0x07);
    assert_eq!(Value::UNDEFINED.to_bits(), 0x0a);
    assert_eq!(Value::EMPTY.to_bits(), 0x22);
    assert_eq!(NOT_CELL_MASK, 0xFFFE_0000_0000_0002);
    for v in [
        Value::NULL,
        Value::FALSE,
        Value::TRUE,
        Value::UNDEFINED,
        Value::EMPTY,
    ] {
        assert_one_kind(v);
        assert!(!v.is_cell() && !v.is_number());
        assert_ne!(v.to_bits() & OTHER_TAG, 0, "every immediate has OtherTag");
        assert!(v.to_bits() < 0x100, "below the first heap page");
    }
    assert!(Value::NULL.is_undefined_or_null() && Value::UNDEFINED.is_undefined_or_null());
    assert!(!Value::EMPTY.is_undefined_or_null() && !Value::FALSE.is_undefined_or_null());
    assert_eq!(Value::bool(true).as_bool(), Some(true));
    assert_eq!(Value::bool(false).as_bool(), Some(false));
    assert_eq!(Value::EMPTY.as_bool(), None);
    assert!(Value::EMPTY.is_empty() && !Value::UNDEFINED.is_empty());
}

#[test]
fn int32_edges() {
    for i in [0, 1, -1, 42, i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1] {
        let v = Value::int32(i);
        assert_one_kind(v);
        assert_eq!(v.to_bits(), NUMBER_TAG | i as u32 as u64);
        assert_eq!(v.to_bits() >> 32, 0xFFFE_0000);
        assert_eq!(v.as_int32(), Some(i));
        assert_eq!(v.as_number(), Some(i as f64));
        assert_eq!(v.as_double(), None);
        // `number` gives the same int32 bits.
        assert_eq!(Value::number(i as f64).to_bits(), v.to_bits());
    }
}

#[test]
fn double_edges() {
    let cases = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::EPSILON,
        f64::from_bits(1),                     // smallest subnormal
        f64::from_bits(0x000F_FFFF_FFFF_FFFF), // largest subnormal
        -f64::from_bits(1),
        9007199254740992.0,  // 2^53
        -9007199254740992.0, // -2^53
        9007199254740994.0,  // 2^53 + 2
        2147483648.0,        // i32::MAX + 1
        -2147483649.0,       // i32::MIN - 1
        i32::MAX as f64,
        i32::MIN as f64,
        1e308,
        5e-324,
    ];
    for d in cases {
        assert_double_round_trip(d);
        assert_number(d);
    }
}

#[test]
fn negative_zero_stays_a_double_with_its_sign() {
    let v = Value::number(-0.0);
    assert!(v.is_double());
    assert!(v.as_double().unwrap().is_sign_negative());
    assert_eq!(v.to_bits(), 0x8000_0000_0000_0000 + ENC_OFFSET);
    let z = Value::number(0.0);
    assert_eq!(z.as_int32(), Some(0));
    // The double form of +0 is the lowest encoded double.
    assert_eq!(Value::double(0.0).to_bits(), 0x0002_0000_0000_0000);
}

#[test]
fn infinities_and_extremes_encode_to_the_documented_range() {
    assert_eq!(
        Value::double(f64::INFINITY).to_bits(),
        0x7FF2_0000_0000_0000
    );
    assert_eq!(
        Value::double(f64::NEG_INFINITY).to_bits(),
        0xFFF2_0000_0000_0000
    );
    assert_eq!(Value::double(f64::MIN).to_bits(), 0xFFF1_FFFF_FFFF_FFFF);
}

/// Without purification these NaNs would alias other kinds. This is why H6 purifies.
#[test]
fn unpurified_nans_would_alias_other_kinds() {
    let enc = |bits: u64| bits.wrapping_add(ENC_OFFSET);
    for bits in [
        0xFFFC_0000_0000_0000u64,
        0xFFFE_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFF8,
    ] {
        assert!(f64::from_bits(bits).is_nan());
    }
    // 0xFFFC... + 2^49 = 0xFFFE_0000_0000_0000: exactly the int32 0.
    assert_eq!(enc(0xFFFC_0000_0000_0000), Value::int32(0).to_bits());
    // 0xFFFE... + 2^49 wraps to 0: the niche of Option<Value> and JsResult.
    assert_eq!(enc(0xFFFE_0000_0000_0000), 0);
    // 0xFFFF_FFFF_FFFF_FFF8 + 2^49 = 0x0001_FFFF_FFFF_FFF8: passes the cell test.
    assert_eq!(enc(0xFFFF_FFFF_FFFF_FFF8) & NOT_CELL_MASK, 0);
    // With purification all three box to the one pure NaN.
    for bits in [
        0xFFFC_0000_0000_0000u64,
        0xFFFE_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFF8,
    ] {
        assert_eq!(
            Value::double(f64::from_bits(bits)).to_bits(),
            PURE_NAN_ENCODED
        );
    }
}

#[test]
fn nan_payloads_are_purified() {
    let nans = [
        PURE_NAN_BITS,
        0xFFF8_0000_0000_0000, // negative quiet NaN (x86 default NaN)
        0x7FF0_0000_0000_0001, // smallest signaling NaN
        0x7FF4_0000_0000_0000, // signaling NaN
        0x7FFF_FFFF_FFFF_FFFF,
        0xFFF0_0000_0000_0001,
        0xFFFC_0000_0000_0000, // would alias int32
        0xFFFD_1234_5678_9ABC,
        0xFFFE_0000_0000_0000, // would encode as 0
        0xFFFF_0000_0000_0000, // JSC ImpureNaNAsBits
        0xFFFF_FFFF_FFFF_FFFF,
    ];
    for bits in nans {
        let d = f64::from_bits(bits);
        assert!(d.is_nan());
        assert_double_round_trip(d);
        assert_number(d);
        assert_eq!(Value::number(d).to_bits(), PURE_NAN_ENCODED);
    }
    // NaNs made at run time: aarch64 and x86 produce different default NaNs.
    let zero = black_box(0.0f64);
    let inf = black_box(f64::INFINITY);
    let neg = black_box(-1.0f64);
    for d in [
        zero / zero,
        inf - inf,
        neg.sqrt(),
        f64::NAN,
        -f64::NAN,
        inf * zero,
    ] {
        assert!(d.is_nan());
        assert_eq!(Value::double(d).to_bits(), PURE_NAN_ENCODED);
        assert_eq!(Value::number(d).to_bits(), PURE_NAN_ENCODED);
    }
}

#[test]
fn number_picks_int32_only_for_exact_i32() {
    assert!(Value::number(1.0).is_int32());
    assert!(Value::number(-2147483648.0).is_int32());
    assert!(Value::number(2147483647.0).is_int32());
    assert!(Value::number(2147483648.0).is_double());
    assert!(Value::number(-2147483649.0).is_double());
    assert!(Value::number(1.5).is_double());
    assert!(Value::number(9007199254740992.0).is_double());
    assert!(Value::number(f64::INFINITY).is_double());
    assert!(Value::number(f64::from_bits(1)).is_double());
    // Both forms decode to the same JS number.
    assert_eq!(
        Value::double(7.0).as_number(),
        Value::number(7.0).as_number()
    );
    assert_eq!(Value::UNDEFINED.as_number(), None);
}

/// Zeroed, writable memory for a fake cell, below 2^47 by construction (H7). A `Box` is not
/// enough: on Linux arm64 the malloc heap and PIE images sit above 2^47 (0xaaaa_..., 0xffff_...).
/// - Native: one committed page of a small cage from `ep9_platform`.
/// - Miri: Miri cannot map a fixed address range, but it gives its own allocations low
///   addresses, so a plain allocation is below 2^47 there.
struct LowMem {
    #[cfg(not(miri))]
    _cage: ep9_platform::Cage,
    base: NonNull<u64>,
}

/// Size of the Miri buffer. The native page is at least 4 KiB.
#[cfg(miri)]
const MIRI_LAYOUT: std::alloc::Layout = match std::alloc::Layout::from_size_align(64, 8) {
    Ok(l) => l,
    Err(_) => panic!("bad layout"),
};

impl LowMem {
    #[cfg(not(miri))]
    fn new() -> LowMem {
        use ep9_platform::{Cage, CageConfig, allocation_granularity, page_size};
        let g = allocation_granularity();
        let config = CageConfig {
            cage_size: g,
            code_range_size: 0,
            ..CageConfig::default()
        };
        let cage = Cage::reserve(&config).expect("reserve a small cage below 2^47");
        cage.heap().commit(0, page_size()).expect("commit one page");
        // The cage got its base from mmap/VirtualAlloc as an exposed address.
        let base = core::ptr::with_exposed_provenance_mut::<u64>(cage.heap().base());
        LowMem {
            _cage: cage,
            base: NonNull::new(base).expect("cage base is not null"),
        }
    }

    #[cfg(miri)]
    fn new() -> LowMem {
        // SAFETY: the layout has a non-zero size.
        let p = unsafe { std::alloc::alloc_zeroed(MIRI_LAYOUT) };
        LowMem {
            base: NonNull::new(p.cast()).expect("allocation failed"),
        }
    }
}

#[cfg(miri)]
impl Drop for LowMem {
    fn drop(&mut self) {
        // SAFETY: `base` came from `alloc_zeroed(MIRI_LAYOUT)` in `new` and is freed only here.
        unsafe { std::alloc::dealloc(self.base.as_ptr().cast(), MIRI_LAYOUT) }
    }
}

#[test]
fn cell_pointer_round_trip_keeps_provenance() {
    let mem = LowMem::new();
    let p = mem.base;
    // SAFETY: `p` points to at least 8 zeroed, writable, 8-aligned words that `mem` owns.
    unsafe {
        p.write(0x1111);
        p.add(1).write(0x2222);
    }
    let addr = p.as_ptr() as usize as u64;
    assert!(
        addr < CELL_ADDRESS_LIMIT,
        "cell address {addr:#x} is above 2^47"
    );
    // SAFETY: `p` is live, 8-aligned and below 2^47 (checked above). It stands in for a GC
    // cell; `mem` outlives every use of this Value.
    let v = unsafe { Value::from_cell_ptr(p) };
    assert_one_kind(v);
    assert!(v.is_cell() && !v.is_number());
    assert_eq!(v.to_bits(), addr);
    let q = v.as_cell_ptr().unwrap().cast::<u64>();
    assert_eq!(q, p);
    // SAFETY: `q` has the provenance that `from_cell_ptr` exposed and points to `p`'s words.
    unsafe {
        assert_eq!(q.read(), 0x1111);
        q.add(1).write(0x3333);
    }
    assert!(matches!(v.unpack(), Unpacked::Cell(c) if c == p.cast()));
    // SAFETY: as above; `mem` is still alive.
    let back = unsafe { [p.read(), p.add(1).read()] };
    assert_eq!(back, [0x1111, 0x3333]);
}

#[test]
fn valid_bits_rejects_patterns_no_constructor_makes() {
    for bits in [
        0,                                                 // the niche
        0x03,                                              // unknown immediate
        0x1004,                                            // misaligned "pointer"
        0x0000_8000_0000_0000,                             // 2^47: outside the cage
        0x0001_0000_0000_0000,                             // 2^48
        0xFFFE_0001_0000_0000,                             // int32 tag with junk in bits 32-48
        0xFFFF_0000_0000_0000,                             // int32 tag with junk
        0x7FF0_0000_0000_0001u64.wrapping_add(ENC_OFFSET), // impure NaN
    ] {
        assert!(!Value::is_valid_bits(bits), "{bits:#x}");
    }
    for bits in [
        0x8,
        0x1000,
        CELL_ADDRESS_LIMIT - 8,
        PURE_NAN_ENCODED,
        0x0002_0000_0000_0000,
    ] {
        assert!(Value::is_valid_bits(bits), "{bits:#x}");
    }
}

#[test]
fn niches_are_zero() {
    let v = Value::int32(7);
    let ok: JsResult = Ok(v);
    let err: JsResult = Err(Throw);
    // SAFETY: all three types are 8 bytes (asserted at compile time) and fully initialized:
    // `Value` has no padding and a niche-encoded variant writes its niche bytes. So reading
    // them as u64 is sound. core guarantees None = 0 for Option<NonZero>; Err = 0 is rustc's
    // niche choice, which this test pins.
    let (ok_bits, err_bits, none_bits) = unsafe {
        (
            core::mem::transmute::<JsResult, u64>(ok),
            core::mem::transmute::<JsResult, u64>(err),
            core::mem::transmute::<Option<Value>, u64>(None),
        )
    };
    assert_eq!(ok_bits, v.to_bits());
    assert_eq!(err_bits, 0);
    assert_eq!(none_bits, 0);
}

#[test]
fn word0_layout_and_access() {
    let w = Word0::new(ShapeId::new(0xDEAD_BEEF), 0x12, 0x34, 0x5678);
    assert_eq!(w.shape_id(), ShapeId::new(0xDEAD_BEEF));
    assert_eq!((w.gc_state(), w.kind(), w.flags()), (0x12, 0x34, 0x5678));
    let packed = Word0::pack(ShapeId::new(0xDEAD_BEEF), 0x12, 0x34, 0x5678);
    assert_eq!(packed, 0x5678_3412_DEAD_BEEF);
    // SAFETY: Word0 is 8 bytes, repr(C), no padding, and no other thread touches `w`.
    let bytes: [u8; 8] = unsafe { core::ptr::read((&raw const w).cast::<[u8; 8]>()) };
    assert_eq!(bytes, packed.to_le_bytes());
    assert_eq!(bytes[Word0::GC_STATE_OFFSET], 0x12);
    assert_eq!(bytes[Word0::KIND_OFFSET], 0x34);

    w.set_gc_state(0xFF);
    w.set_kind(1);
    w.set_flags(0);
    w.set_shape_id(ShapeId::new(16));
    assert_eq!(w.gc_state(), 0xFF);
    assert_eq!(w.shape_id().get(), 16);
    // SAFETY: as above.
    let bytes: [u8; 8] = unsafe { core::ptr::read((&raw const w).cast::<[u8; 8]>()) };
    assert_eq!(
        u64::from_le_bytes(bytes),
        Word0::pack(ShapeId::new(16), 0xFF, 1, 0)
    );
}

#[test]
fn debug_output() {
    assert_eq!(format!("{:?}", Value::int32(-5)), "Value(int32 -5)");
    assert_eq!(format!("{:?}", Value::double(1.5)), "Value(double 1.5)");
    assert_eq!(
        format!("{:?}", Value::double(f64::NAN)),
        "Value(double NaN)"
    );
    assert_eq!(format!("{:?}", Value::UNDEFINED), "Value(undefined)");
    assert_eq!(format!("{:?}", Value::NULL), "Value(null)");
    assert_eq!(format!("{:?}", Value::TRUE), "Value(true)");
    assert_eq!(format!("{:?}", Value::EMPTY), "Value(empty)");
}

#[test]
fn constructors_are_const() {
    const N: Value = Value::number(-0.0);
    const I: Value = Value::int32(i32::MIN);
    const D: Value = Value::double(f64::NAN);
    assert!(N.is_double() && I.is_int32());
    assert_eq!(D.to_bits(), PURE_NAN_ENCODED);
}

// ---- property test with a local PRNG (no dependencies) ----

/// SplitMix64 (Steele, Lea, Flood 2014). Deterministic, so a failure is reproducible.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Iterations: 1M natively, fewer under Miri (it is about 1000x slower).
const ROUNDS: usize = if cfg!(miri) { 3_000 } else { 1_000_000 };

#[test]
fn property_random_bits_round_trip() {
    let mut rng = SplitMix64(0x0E99_5EED);
    let mut kinds = [0usize; 3]; // int32 / double / NaN inputs seen
    for _ in 0..ROUNDS {
        let r = rng.next();
        // 1) Any 64-bit pattern as an f64: covers NaN payloads, subnormals, both signs.
        let d = f64::from_bits(r);
        assert_double_round_trip(d);
        assert_number(d);
        kinds[if d.is_nan() { 2 } else { 1 }] += 1;

        // 2) Integer-valued doubles near the i32 and 2^53 edges.
        let shift = (r >> 58) as u32; // 0..63
        let n = ((r as i64) >> shift) as f64;
        assert_number(n);
        assert_double_round_trip(n);

        // 3) int32 round trip.
        let i = r as u32 as i32;
        let v = Value::int32(i);
        assert_one_kind(v);
        assert_eq!(v.as_int32(), Some(i));
        assert_eq!(Value::number(i as f64).to_bits(), v.to_bits());
        kinds[0] += 1;

        // 4) from_bits_unchecked accepts what to_bits gives.
        let w = Value::double(d);
        // SAFETY: `w.to_bits()` came from a constructor, so it is valid and not a cell.
        let w2 = unsafe { Value::from_bits_unchecked(w.to_bits()) };
        assert_eq!(w2.to_bits(), w.to_bits());
    }
    assert!(kinds.iter().all(|k| *k > 0), "{kinds:?}");
}

#[test]
fn property_kind_tests_partition_all_values() {
    let mut rng = SplitMix64(0xC0FF_EE00);
    let mut values: Vec<Value> = Vec::new();
    for _ in 0..ROUNDS / 10 {
        let r = rng.next();
        values.clear();
        values.extend([
            Value::double(f64::from_bits(r)),
            Value::number(f64::from_bits(r)),
            Value::int32(r as i32),
            Value::bool(r & 1 == 1),
            Value::UNDEFINED,
            Value::NULL,
            Value::EMPTY,
        ]);
        let addr = (r % CELL_ADDRESS_LIMIT) & !7;
        if addr != 0 {
            // SAFETY: only the bits are inspected; the address is never dereferenced. `addr`
            // is non-zero, 8-aligned and below 2^47, so it is a valid cell pattern.
            values.push(unsafe { Value::from_bits_unchecked(addr) });
        }
        for v in &values {
            assert_one_kind(*v);
        }
    }
}

// ---- H06-empty: 0x22 against the JSC patterns ----

/// JSC constants (WebKit `runtime/JSCJSValue.h`, commit e4be3948, lines 393-427).
mod jsc {
    pub const NUMBER_TAG: u64 = 0xFFFE_0000_0000_0000;
    pub const MISC_TAG: u64 = 0x2 | 0x4 | 0x8; // OtherTag | BoolTag | UndefinedTag
    pub const VALUE_EMPTY: u64 = 0x0;
    pub const VALUE_DELETED: u64 = 0x4;
    pub const BIGINT32_TAG: u64 = 0x12;
    pub const BIGINT32_MASK: u64 = NUMBER_TAG | BIGINT32_TAG;
    pub const NATIVE_CALLEE_TAG: u64 = 0x2 | 0x1;
    pub const NATIVE_CALLEE_MASK: u64 = NUMBER_TAG | 0x7;
}

/// 06-object-model §1 (decision H06-empty): empty/hole = 0x22. It is none of JSC's reserved
/// patterns, it fails every JSC kind test, and the one-compare "misc" test `v <= 0x0E` for
/// null, false, true and undefined stays valid.
#[test]
fn empty_avoids_jsc_reserved_patterns() {
    let e = Value::EMPTY.to_bits();
    assert_eq!(e, 0x22);
    // Not one of JSC's reserved patterns.
    for reserved in [
        jsc::VALUE_EMPTY,
        jsc::VALUE_DELETED,
        jsc::BIGINT32_TAG,
        jsc::NATIVE_CALLEE_TAG,
    ] {
        assert_ne!(e, reserved, "{reserved:#x}");
    }
    // Fails JSC's BigInt32 and NativeCallee tests.
    assert_ne!(e & jsc::BIGINT32_MASK, jsc::BIGINT32_TAG);
    assert_ne!(e & jsc::NATIVE_CALLEE_MASK, jsc::NATIVE_CALLEE_TAG);
    // Above MiscTag, so `v <= 0x0E` still means "null, bool or undefined" among immediates.
    assert!(e > jsc::MISC_TAG);
    for v in [Value::NULL, Value::FALSE, Value::TRUE, Value::UNDEFINED] {
        assert!(v.to_bits() <= jsc::MISC_TAG, "{v:?}");
    }
    // Fails every other ep9 kind test, and its tag bit is free in the other immediates.
    let v = Value::EMPTY;
    assert!(!v.is_cell() && !v.is_number() && !v.is_bool() && !v.is_undefined_or_null());
    assert_ne!(e & OTHER_TAG, 0);
    for other in [Value::NULL, Value::FALSE, Value::TRUE, Value::UNDEFINED] {
        assert_eq!(other.to_bits() & EMPTY_TAG, 0, "{other:?}");
    }
    // Not 0: the niche of Option<Value> and JsResult<Value> stays free.
    assert_ne!(e, 0);
}

// ---- ToInt32 / ToUint32 ----

/// Reference ToInt32/ToUint32 by a different method: truncate, then an exact `fmod` by 2^32.
/// IEEE-754 remainder is exact, and every intermediate is an integer below 2^53.
fn ref_to_uint32(d: f64) -> u32 {
    if !d.is_finite() {
        return 0;
    }
    let t = d.trunc();
    let mut r = t % 4_294_967_296.0;
    if r < 0.0 {
        r += 4_294_967_296.0;
    }
    assert!((0.0..4_294_967_296.0).contains(&r) && r.fract() == 0.0);
    r as u32
}

/// 2^k from its bits. `powi` is not exact everywhere (Miri adds error on purpose).
fn pow2(k: i32) -> f64 {
    f64::from_bits(((1023 + k) as u64) << 52)
}

#[test]
fn to_int32_matches_node() {
    let cases = crate::conv_node_cases::CASES;
    assert!(cases.len() > 1000, "{} cases", cases.len());
    for &(bits, want_i32, want_u32) in cases {
        let d = f64::from_bits(bits);
        assert_eq!(
            crate::to_int32(d),
            want_i32,
            "ToInt32({d:e}) bits {bits:#018x}"
        );
        assert_eq!(
            crate::to_uint32(d),
            want_u32,
            "ToUint32({d:e}) bits {bits:#018x}"
        );
        assert_eq!(ref_to_uint32(d), want_u32, "reference ToUint32({d:e})");
    }
}

#[test]
fn to_int32_spec_edges() {
    use crate::{to_int32, to_uint32};
    let p31 = 2_147_483_648.0f64;
    let p32 = 4_294_967_296.0f64;
    let cases: [(f64, i32, u32); 20] = [
        (0.0, 0, 0),
        (-0.0, 0, 0),
        (f64::NAN, 0, 0),
        (f64::INFINITY, 0, 0),
        (f64::NEG_INFINITY, 0, 0),
        (0.9, 0, 0),
        (-0.9, 0, 0),
        (-1.0, -1, u32::MAX),
        (-1.5, -1, u32::MAX),
        (p31 - 1.0, i32::MAX, 0x7FFF_FFFF),
        (p31, i32::MIN, 0x8000_0000),
        (-p31, i32::MIN, 0x8000_0000),
        (-p31 - 0.5, i32::MIN, 0x8000_0000),
        (-p31 - 1.0, i32::MAX, 0x7FFF_FFFF),
        (p32 - 1.0, -1, u32::MAX),
        (p32, 0, 0),
        (p32 + 1.5, 1, 1),
        (-p32 - 1.0, -1, u32::MAX),
        (f64::MAX, 0, 0),
        (9_007_199_254_740_993.0, 0, 0), // rounds to 2^53 when parsed
    ];
    for (d, i, u) in cases {
        assert_eq!(to_int32(d), i, "ToInt32({d:e})");
        assert_eq!(to_uint32(d), u, "ToUint32({d:e})");
    }
    // The shift rule at its two ends. One ulp of 2^83 is 2^31 (node: `x | 0` = -2147483648);
    // one ulp of 2^84 is 2^32, so every bit of the result is 0.
    assert_eq!(to_int32(9_007_199_254_740_994.0), 2);
    assert_eq!(to_int32(f64::from_bits(pow2(84).to_bits() + 1)), 0);
    assert_eq!(to_int32(f64::from_bits(pow2(83).to_bits() + 1)), i32::MIN);
    assert_eq!(to_uint32(f64::from_bits(pow2(52).to_bits() + 7)), 7);
    // Const evaluation works.
    const C: i32 = crate::to_int32(-4_294_967_297.0);
    assert_eq!(C, -1);
}

#[test]
fn property_to_int32_matches_fmod_reference() {
    let mut rng = SplitMix64(0x7011_7E32);
    for _ in 0..ROUNDS {
        let r = rng.next();
        // Raw bit patterns: every exponent, NaN payloads, subnormals.
        let d = f64::from_bits(r);
        let want = ref_to_uint32(d);
        assert_eq!(crate::to_uint32(d), want, "{d:e} bits {r:#018x}");
        assert_eq!(crate::to_int32(d), want as i32, "{d:e} bits {r:#018x}");
        // Magnitudes 2^0 .. 2^95, where the shift rule does its work.
        let e = (r >> 57) as i32 + (r >> 63) as i32 * 32; // 0 ..= 95
        let m = 1.0 + (r & ((1 << 52) - 1)) as f64 / (1u64 << 52) as f64;
        let d = m * pow2(e) * if r & 1 == 1 { -1.0 } else { 1.0 };
        let want = ref_to_uint32(d);
        assert_eq!(crate::to_uint32(d), want, "{d:e}");
        assert_eq!(crate::to_int32(d), want as i32, "{d:e}");
    }
}
