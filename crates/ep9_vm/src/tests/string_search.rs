//! The code-unit search and bulk copy (OXC-PERF wave 2 STR-SEARCH-SIMD; `string/search.rs`,
//! `abi/strings.rs`). Every first-unit scan body that this CPU has (scalar, SSE2, AVX2, NEON)
//! is compared with the scalar body, and every search with a plain per-position search, on
//! edge and random inputs: lengths 0..=64 and long ones, every alignment, every match
//! position, no match, a match at the last unit, Latin-1 bytes 0x00, 0x7F, 0x80 and 0xFF, and
//! UTF-16 surrogates. A body the CPU does not have is reported and skipped (the x86 bodies run
//! on an x86-64 machine: `cargo test -p ep9_vm --lib string_search`).

use super::{config, isolate};
use crate::string::search::{self, Cursor, Kernel, Path, Step};
use crate::string::{self, Dir};
use crate::{Dispatch, JsString, Local};

/// A string Local from raw bits (the bits are live: the test holds them).
type S<'s> = Local<'s, JsString>;

/// xorshift64*: reproducible test data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The kernels this CPU has, scalar first; prints the ones it lacks. The base body of the
/// target (SSE2 on x86-64, NEON on aarch64) must be there, and so must each path that
/// `EP9_REQUIRE_SIMD_PATHS` names (a comma list, for the x86 run: `sse2,avx2`).
fn kernels() -> Vec<Kernel> {
    let mut v = Vec::new();
    for p in Path::ALL {
        match Kernel::new(p) {
            Some(k) => v.push(k),
            None => eprintln!("string_search: path {} not on this CPU or build", p.name()),
        }
    }
    assert_eq!(v[0], Kernel::scalar());
    let need = |p: Path| {
        assert!(
            v.iter().any(|k| k.path() == p),
            "path {} is missing",
            p.name()
        )
    };
    if cfg!(target_arch = "x86_64") {
        need(Path::Sse2);
    }
    if cfg!(target_arch = "aarch64") {
        need(Path::Neon);
    }
    for name in std::env::var("EP9_REQUIRE_SIMD_PATHS")
        .unwrap_or_default()
        .split(',')
    {
        if !name.is_empty() {
            let p = Path::ALL.into_iter().find(|p| p.name() == name);
            need(p.unwrap_or_else(|| panic!("EP9_REQUIRE_SIMD_PATHS: no path {name}")));
        }
    }
    v
}

#[test]
fn detect_picks_the_best_body_or_the_named_one() {
    let best = search::pick(None);
    assert!(best.available());
    for p in [Path::Avx2, Path::Sse2, Path::Neon, Path::Simd128] {
        if p.available() {
            assert_eq!(best, p, "the widest body this CPU has");
            break;
        }
    }
    if std::env::var_os("EP9_SIMD_SEARCH").is_none() {
        assert_eq!(Kernel::detect().path(), best);
    }
    assert_eq!(search::pick(Some("scalar")), Path::Scalar);
    assert_eq!(search::pick(Some("no-such-path")), best);
    for p in Path::ALL {
        let want = if p.available() { p } else { best };
        assert_eq!(search::pick(Some(p.name())), want, "{}", p.name());
    }
    // The test-only override.
    assert_eq!(
        search::with_path(Path::Scalar, || Kernel::detect().path()),
        Some(Path::Scalar)
    );
}

/// Long lengths past the vector widths and the poll window.
const LONG: [usize; 8] = [100, 127, 128, 255, 256, 257, 1000, 4099];

fn lengths() -> impl Iterator<Item = usize> {
    (0..=64).chain(LONG)
}

/// Every body gives the scalar result for one Latin-1 haystack and byte.
fn check_u8(ks: &[Kernel], hay: &[u8], b: u8) {
    let want = search::scalar_find_u8(hay, b);
    assert_eq!(want, hay.iter().position(|&c| c == b));
    for &k in ks {
        assert_eq!(
            k.find_u8(hay, b),
            want,
            "{:?} len {} byte {b:#x}",
            k.path(),
            hay.len()
        );
    }
}

/// Every body gives the scalar result for one UTF-16 haystack and unit.
fn check_u16(ks: &[Kernel], hay: &[u16], u: u16) {
    let want = search::scalar_find_u16(hay, u);
    assert_eq!(want, hay.iter().position(|&c| c == u));
    for &k in ks {
        assert_eq!(
            k.find_u16(hay, u),
            want,
            "{:?} len {} unit {u:#x}",
            k.path(),
            hay.len()
        );
    }
}

#[test]
fn every_scan_body_finds_each_position_at_every_alignment() {
    let ks = kernels();
    // One buffer, read at offsets 0..32: every alignment of the first load.
    for &(fill, b) in &[
        (b'a', b'z'),
        (0x00, 0xFF),
        (0xFF, 0x00),
        (0x7F, 0x80),
        (0x80, 0x7F),
    ] {
        let mut buf8 = vec![fill; 4099 + 32];
        for off in 0..32 {
            for n in lengths() {
                let hay = &mut buf8[off..off + n];
                check_u8(&ks, hay, b); // no match
                for at in 0..n {
                    hay[at] = b;
                    check_u8(&ks, hay, b); // a match at `at` (the last unit too)
                    if at + 1 < n {
                        hay[n - 1] = b; // a second match after it: the first one wins
                        check_u8(&ks, hay, b);
                        hay[n - 1] = fill;
                    }
                    hay[at] = fill;
                }
            }
        }
    }
    for &(fill, u) in &[
        (0x0061, 0x007A),
        (0xD800, 0xDC00),
        (0xDC00, 0xD800),
        (0xDFFF, 0xFFFF),
        (0xFFFF, 0x0000),
        (0x00FF, 0xFF00),
        (0xFF00, 0x00FF),
    ] {
        let mut buf16 = vec![fill; 4099 + 16];
        for off in 0..16 {
            for n in lengths() {
                let hay = &mut buf16[off..off + n];
                check_u16(&ks, hay, u);
                for at in 0..n {
                    hay[at] = u;
                    check_u16(&ks, hay, u);
                    if at + 1 < n {
                        hay[n - 1] = u;
                        check_u16(&ks, hay, u);
                        hay[n - 1] = fill;
                    }
                    hay[at] = fill;
                }
            }
        }
    }
}

#[test]
fn every_scan_body_matches_scalar_on_random_input() {
    let ks = kernels();
    let mut r = Rng(0x5eed_0001);
    for _ in 0..20_000 {
        let n = if r.below(8) == 0 {
            65 + r.below(600)
        } else {
            r.below(65)
        };
        // A small alphabet, so matches are common; sometimes the whole byte range.
        let wide = r.below(4) == 0;
        let hay8: Vec<u8> = (0..n)
            .map(|_| {
                if wide {
                    r.next() as u8
                } else {
                    b'a' + r.below(4) as u8
                }
            })
            .collect();
        let b = if wide {
            r.next() as u8
        } else {
            b'a' + r.below(5) as u8
        };
        let off = r.below(n + 1);
        check_u8(&ks, &hay8[off..], b);
        // Surrogates, Latin-1 range and the top of the BMP.
        let pick =
            |r: &mut Rng| [0x61, 0x62, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xFFFF, 0xFF][r.below(8)];
        let hay16: Vec<u16> = (0..n).map(|_| pick(&mut r)).collect();
        let u = pick(&mut r);
        check_u16(&ks, &hay16[off..], u);
    }
}

/// The plain search: the first (or, backward, the last) position whose substring is `needle`.
fn naive(hay: &[u16], needle: &[u16], from: usize, dir: Dir) -> Option<usize> {
    let n = needle.len();
    let last = hay.len().checked_sub(n)?;
    let ok = |i: usize| hay[i..i + n] == *needle;
    match dir {
        Dir::Forward => (from..=last).find(|&i| ok(i)),
        Dir::Backward => (0..=from.min(last)).rev().find(|&i| ok(i)),
    }
}

/// Runs the step machine to its end; gives the result and the poll count.
fn run_steps<H: search::Unit, N: search::Unit>(
    k: Kernel,
    hay: &[H],
    needle: &[N],
    start: usize,
    dir: Dir,
) -> (Option<usize>, usize) {
    let mut c = Cursor::new(start);
    let mut polls = 0;
    loop {
        let s = match dir {
            Dir::Forward => search::index_of_step(k, hay, needle, &mut c),
            Dir::Backward => search::last_index_of_step(hay, needle, &mut c),
        };
        match s {
            Step::Found(i) => return (Some(i), polls),
            Step::NotFound => return (None, polls),
            Step::Poll => polls += 1,
        }
    }
}

/// Every combination of encodings and every kernel against [`naive`].
fn check_search(ks: &[Kernel], hay: &[u16], needle: &[u16]) {
    assert!(!needle.is_empty() && needle.len() <= hay.len());
    let last = hay.len() - needle.len();
    let latin =
        |s: &[u16]| -> Option<Vec<u8>> { s.iter().map(|&u| u8::try_from(u).ok()).collect() };
    let (h8, n8) = (latin(hay), latin(needle));
    for start in 0..=last {
        for dir in [Dir::Forward, Dir::Backward] {
            let want = naive(hay, needle, start, dir);
            for &k in ks {
                let msg = || format!("{:?} {dir:?} start {start} {hay:x?} {needle:x?}", k.path());
                assert_eq!(run_steps(k, hay, needle, start, dir).0, want, "{}", msg());
                if let Some(h8) = &h8 {
                    assert_eq!(
                        run_steps(k, h8.as_slice(), needle, start, dir).0,
                        want,
                        "{}",
                        msg()
                    );
                    if let Some(n8) = &n8 {
                        assert_eq!(
                            run_steps(k, h8.as_slice(), n8.as_slice(), start, dir).0,
                            want,
                            "{}",
                            msg()
                        );
                    }
                }
                if let Some(n8) = &n8 {
                    assert_eq!(
                        run_steps(k, hay, n8.as_slice(), start, dir).0,
                        want,
                        "{}",
                        msg()
                    );
                }
            }
        }
    }
}

#[test]
fn searches_match_the_plain_search_in_every_encoding() {
    let ks = kernels();
    let mut r = Rng(0x5eed_0002);
    let alpha: [u16; 6] = [0x61, 0x62, 0x63, 0xFF, 0xD800, 0xDC00];
    for round in 0..3_000 {
        let n = if round % 10 == 0 {
            65 + r.below(300)
        } else {
            1 + r.below(64)
        };
        // Latin-1 only in half the rounds, so the u8 forms run too.
        let k = if round % 2 == 0 { 4 } else { 6 };
        let hay: Vec<u16> = (0..n).map(|_| alpha[r.below(k)]).collect();
        let m = 1 + r.below(n.min(6));
        let needle: Vec<u16> = if r.below(2) == 0 {
            let at = r.below(n - m + 1);
            hay[at..at + m].to_vec() // a needle that occurs
        } else {
            (0..m).map(|_| alpha[r.below(k)]).collect()
        };
        check_search(&ks, &hay, &needle);
    }
    // Fixed edges: a one-unit search, a match at the last unit, the whole string, a long
    // repeated prefix (many candidates), a search unit above 0xFF in a Latin-1 string.
    let a: Vec<u16> = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"
        .encode_utf16()
        .collect();
    check_search(&ks, &a, &[0x62]);
    check_search(&ks, &a, &a);
    check_search(&ks, &a, &a[20..]);
    check_search(&ks, &a, &[0x61, 0x61, 0x62]);
    check_search(&ks, &a, &[0x161]);
    let sur: Vec<u16> = vec![0xD83D, 0xDE00, 0x61, 0xD83D, 0xDE01, 0xDE00, 0xD83D, 0xDE00];
    check_search(&ks, &sur, &[0xDE00]);
    check_search(&ks, &sur, &[0xD83D, 0xDE00]);
    check_search(&ks, &sur, &[0xDE00, 0xD83D]);
}

#[test]
fn a_long_search_polls_once_per_256_units_of_work() {
    let k = Kernel::detect();
    // 10,000 positions, no match: one unit of work per position.
    let hay = vec![b'a'; 10_000];
    let (r, polls) = run_steps(k, hay.as_slice(), b"b".as_slice(), 0, Dir::Forward);
    assert_eq!((r, polls), (None, 10_000 / 256));
    let (r, polls) = run_steps(k, hay.as_slice(), b"b".as_slice(), 9_999, Dir::Backward);
    assert_eq!((r, polls), (None, 10_000 / 256));
    // A 1,000-unit search that matches only at the end: every position is a candidate whose
    // compare fails at its last unit; the work is about len x search length.
    let hay = vec![0x61u16; 3_000];
    let mut needle = vec![0x61u16; 1_000];
    needle[999] = 0x62;
    let mut hay2 = hay.clone();
    hay2[2_999] = 0x62;
    let (r, polls) = run_steps(k, hay2.as_slice(), needle.as_slice(), 0, Dir::Forward);
    assert_eq!(r, Some(2_000));
    // 2,000 failed candidates of 1,000 units each, then the match.
    assert!(polls >= 2_000 * 1_000 / 256, "{polls} polls");
    let (r, _) = run_steps(k, hay2.as_slice(), needle.as_slice(), 2_000, Dir::Backward);
    assert_eq!(r, Some(2_000));
}

/// The strings of each kind with the text `t`: SEQ (Latin-1 or UTF-16 by the text), a SEQ
/// UTF-16 cell of Latin-1 text, a CONS, a flattened CONS, a SLICED string and a THIN one.
fn kinds_of(rt: &crate::runtime::Runtime, t: &[u16]) -> Vec<(&'static str, u64)> {
    let seq = string::new_string_utf16(rt, t);
    let mut v = vec![("seq", seq)];
    if t.iter().all(|&u| u <= 0xFF) {
        v.push(("seq-utf16", string::testing::new_utf16_cell(rt, t)));
    }
    if t.len() >= 2 * string::MIN_CONS_LENGTH {
        let h = t.len() / 2;
        let (a, b) = (
            string::new_string_utf16(rt, &t[..h]),
            string::new_string_utf16(rt, &t[h..]),
        );
        v.push(("cons", string::concat(rt, a, b).unwrap()));
        let c = string::concat(rt, a, b).unwrap();
        string::flatten_rt(rt, c);
        v.push(("cons-flat", c));
        let thin = string::concat(rt, a, b).unwrap();
        crate::atoms::atomize(rt, thin);
        v.push(("thin-or-atom", thin));
    }
    if t.len() >= string::MIN_SLICE_LENGTH {
        let mut wide = vec![0x78u16; 5];
        wide.extend_from_slice(t);
        wide.extend_from_slice(&[0x79; 7]);
        let p = string::new_string_utf16(rt, &wide);
        v.push(("sliced", string::substring(rt, p, 5, 5 + t.len())));
    }
    v
}

#[test]
fn cx_searches_and_copies_read_every_string_kind_on_every_path() {
    // No GC stress here: the strings are in Rust Vecs, which the conservative scan does not
    // read (the next test runs a search under stress with its strings in Rust locals).
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let texts: Vec<Vec<u16>> = [
        "the quick brown fox jumps over the lazy dog; the end",
        "abcabcabcabcabcabcabcabcabcabcabd",
        "caf\u{e9} na\u{ef}ve r\u{e9}sum\u{e9} caf\u{e9} na\u{ef}ve",
        "\u{1F600}smile\u{1F601}grin\u{1F600}smile again \u{1F600}",
    ]
    .iter()
    .map(|t| t.encode_utf16().collect())
    .collect();
    let mut searches: Vec<Vec<u16>> = [
        "the",
        "abd",
        "caf\u{e9}",
        "\u{1F600}",
        "e",
        "",
        "zz",
        "\u{e9} ",
        "abcabcabd",
    ]
    .iter()
    .map(|t| t.encode_utf16().collect())
    .collect();
    searches.push(vec![0xDE00]); // a lone trail surrogate: matches inside a pair
    searches.push(vec![0xD83D]); // a lone lead surrogate
    let froms = |len: usize| [0, 1, 5, len / 2, len, len + 3];
    for t in &texts {
        let rt: &crate::runtime::Runtime = &iso.rt;
        let hays = kinds_of(rt, t);
        let needles: Vec<(Vec<u16>, u64)> = searches
            .iter()
            .map(|n| (n.clone(), string::new_string_utf16(rt, n)))
            .collect();
        for p in Path::ALL {
            for &(kind, h) in &hays {
                for (n, nb) in &needles {
                    let r = search::with_path(p, || {
                        iso.with_cx(|cx| {
                            let (hl, nl) = (S::from_bits(h), S::from_bits(*nb));
                            let mut got = Vec::new();
                            for from in froms(t.len()) {
                                got.push(cx.string_index_of(hl, nl, from)?);
                                got.push(cx.string_last_index_of(hl, nl, from)?);
                            }
                            let flat = cx.flatten(hl)?;
                            let mut out = vec![7];
                            cx.push_code_units(flat, 0, t.len(), &mut out);
                            cx.push_code_units(flat, 3, 3, &mut out);
                            cx.push_code_units(flat, 2, 9, &mut out);
                            Ok((got, out))
                        })
                        .unwrap()
                    });
                    let Some((got, out)) = r else { continue };
                    let mut want = Vec::new();
                    for from in froms(t.len()) {
                        want.push(if n.is_empty() {
                            (from <= t.len()).then_some(from)
                        } else {
                            naive(t, n, from, Dir::Forward)
                        });
                        want.push(if n.is_empty() {
                            Some(from.min(t.len()))
                        } else {
                            naive(t, n, from, Dir::Backward)
                        });
                    }
                    assert_eq!(got, want, "{} {kind} {t:x?} {n:x?}", p.name());
                    let mut want_out = vec![7];
                    want_out.extend_from_slice(t);
                    want_out.extend_from_slice(&t[2..9]);
                    assert_eq!(out, want_out, "{} {kind}", p.name());
                }
            }
        }
    }
}

#[test]
fn a_cx_search_of_a_rope_flattens_it_under_gc_stress() {
    // GC stress: a collection before every allocation, then the verifier. The strings are in
    // Rust locals (conservative scan); the flatten inside the search is the GC point.
    let mut iso = isolate(config(Dispatch::DEFAULT, true));
    let rt: &crate::runtime::Runtime = &iso.rt;
    let a = string::new_string(rt, "0123456789abcdef");
    let b = string::new_string_utf16(
        rt,
        &"ghij\u{263A}klmnopqrstu".encode_utf16().collect::<Vec<_>>(),
    );
    let h = string::concat(rt, a, b).unwrap();
    let c = string::new_string(rt, "rst");
    let d = string::new_string(rt, "uvw");
    let n = string::concat(rt, c, d).unwrap(); // "rstuvw": 6 units, a flat string
    let n2 = string::concat(rt, a, c).unwrap(); // a CONS search string, not in `h`
    let r = iso
        .with_cx(|cx| {
            let (hl, nl, n2l) = (S::from_bits(h), S::from_bits(n), S::from_bits(n2));
            let k = cx.new_string("j\u{263A}k")?;
            let frs = cx.new_string("frs")?;
            Ok((
                cx.string_index_of(hl, k, 0)?,
                cx.string_last_index_of(hl, k, 99)?,
                cx.string_index_of(hl, nl, 0)?,
                cx.string_index_of(hl, n2l, 0)?,
                cx.string_index_of(n2l, frs, 0)?,
            ))
        })
        .unwrap();
    assert_eq!(r, (Some(19), Some(19), None, None, Some(15)));
    std::hint::black_box((a, b, h, c, d, n, n2));
}
