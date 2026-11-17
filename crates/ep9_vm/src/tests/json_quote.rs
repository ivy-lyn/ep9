//! The JSON quote bodies and the JSON text (OXC-PERF wave 3 W3-JSON-QUOTE;
//! `string/json_quote.rs`, `abi/json.rs`). Every body that this CPU has (SWAR, SSE2, AVX2, NEON)
//! is compared with the reference body (the per-unit loop of the old `quote`, `Path::Scalar`)
//! on edge and random inputs: lengths 0..=64 and long ones (255, 256, 4095, 4096, 4097,
//! 100_000), every start alignment 0..16, every byte 0x00-0xFF at every position, no escape,
//! an escape at the last unit, all units escaped, runs of escapes, UTF-16 pairs, lone leads
//! and trails and units above 0xFF at every position (the widen point), pairs across a vector
//! end and across the 4096-unit chunk end, and an output that is already one-byte or two-byte
//! with earlier text. A raw chunk run checks the reserve bound with a canary. A body the CPU
//! does not have is reported and skipped (the x86 bodies run on an x86-64 machine:
//! `cargo test -p ep9_vm --lib json_quote`).

use super::{config, isolate};
use crate::string::json_quote::{self, CHUNK, Kernel, Path, testing};
use crate::string::{self, JsonText};
use crate::{Dispatch, FlatStr, JsString, Local};

/// A string Local from raw bits (the bits are live: the test holds them).
type StrLocal<'s> = Local<'s, JsString>;

/// A fixed-seed LCG (Knuth MMIX constants): reproducible test data.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The kernels this CPU has (not the reference); prints the ones it lacks. SWAR must be
/// there, and so must the base body of the target (SSE2 on x86-64, NEON on aarch64) and each
/// path that `EP9_REQUIRE_SIMD_PATHS` names (a comma list, for the x86 run: `sse2,avx2`).
fn kernels() -> Vec<Kernel> {
    let mut v = Vec::new();
    for p in Path::ALL {
        if p == Path::Scalar {
            continue;
        }
        match Kernel::new(p) {
            Some(k) => v.push(k),
            None => eprintln!("json_quote: path {} not on this CPU or build", p.name()),
        }
    }
    let need = |p: Path| {
        assert!(
            v.iter().any(|k| k.path() == p),
            "path {} is missing",
            p.name()
        )
    };
    need(Path::Swar);
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

/// The start states of the output: empty one-byte, one-byte with text, two-byte with text.
fn starts() -> Vec<JsonText> {
    let mut one = JsonText::new();
    one.push_ascii(b"[1,\"x\",");
    let mut two = JsonText::new();
    two.push_ascii(b"{\"k\":");
    two.push_units(&[0x2603, 0x3A]);
    assert!(two.is_two_byte());
    vec![JsonText::new(), one, two]
}

fn copy(t: &JsonText) -> JsonText {
    let mut c = JsonText::new();
    c.push_units(&t.to_utf16());
    c
}

/// The quote of a Latin-1 source by `k` and by the reference, from each start state: the
/// same units and the same width.
fn check_latin1(ks: &[Kernel], src: &[u8], what: &str) {
    for start in starts() {
        let mut want = copy(&start);
        json_quote::quote_latin1(Kernel::scalar(), src, &mut want);
        // A Latin-1 source never widens the text.
        assert_eq!(want.is_two_byte(), start.is_two_byte());
        for &k in ks {
            let mut got = copy(&start);
            json_quote::quote_latin1(k, src, &mut got);
            assert_eq!(
                got.to_utf16(),
                want.to_utf16(),
                "{} {what} {src:x?}",
                k.path().name()
            );
            assert_eq!(got.is_two_byte(), want.is_two_byte(), "{}", k.path().name());
        }
    }
}

/// [`check_latin1`] for a UTF-16 source.
fn check_utf16(ks: &[Kernel], src: &[u16], what: &str) {
    for start in starts() {
        let mut want = copy(&start);
        json_quote::quote_utf16(Kernel::scalar(), src, &mut want);
        for &k in ks {
            let mut got = copy(&start);
            json_quote::quote_utf16(k, src, &mut got);
            assert_eq!(
                got.to_utf16(),
                want.to_utf16(),
                "{} {what} {src:x?}",
                k.path().name()
            );
            assert_eq!(got.is_two_byte(), want.is_two_byte(), "{}", k.path().name());
        }
    }
}

/// The quote of a text as node prints `JSON.stringify(text)` (the expected strings are
/// node v26.13.1 output).
fn quoted(k: Kernel, text: &[u16]) -> Vec<u16> {
    let mut t = JsonText::new();
    json_quote::quote_utf16(k, text, &mut t);
    t.to_utf16()
}

#[test]
fn detect_picks_the_best_body_or_the_named_one() {
    let best = json_quote::pick(None);
    assert!(best.available());
    assert_ne!(best, Path::Scalar);
    for p in [Path::Avx2, Path::Sse2, Path::Neon, Path::Simd128] {
        if p.available() {
            assert_eq!(best, p, "the widest body this CPU has");
            break;
        }
    }
    for p in Path::ALL {
        let got = json_quote::pick(Some(p.name()));
        if p.available() {
            assert_eq!(got, p);
        } else {
            assert_eq!(got, best, "a path the CPU lacks gives the best one");
        }
    }
    assert_eq!(json_quote::pick(Some("avx512")), best);
    // The forced path wins on this thread, then the old one comes back.
    assert_eq!(
        json_quote::with_path(Path::Swar, || Kernel::detect().path()),
        Some(Path::Swar)
    );
    // Off the forced thread, detect gives the `EP9_SIMD_JSON` pick (a test run can force any
    // path, `scalar` too); with the variable unset, that is the best body, not the reference.
    let env = std::env::var("EP9_SIMD_JSON").ok();
    assert_eq!(Kernel::detect().path(), json_quote::pick(env.as_deref()));
    if env.is_none() {
        assert_ne!(Kernel::detect().path(), Path::Scalar);
    }
    // Every path that this CPU has is a kernel, and the required ones are there.
    assert!(!kernels().is_empty());
}

#[test]
fn the_spec_edge_cases_match_node() {
    let cases: &[(&[u16], &str)] = &[
        (&[], r#""""#),
        (&[0x22, 0x5C, 0x2F], r#""\"\\/""#),
        (
            &[0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D],
            r#""\b\t\n\u000b\f\r""#,
        ),
        (
            &[0x00, 0x1F, 0x7F, 0x80, 0xFF],
            "\"\\u0000\\u001f\u{7f}\u{80}\u{ff}\"",
        ),
        (&[0xD800, 0x78], r#""\ud800x""#),
        (&[0xDC00], r#""\udc00""#),
        (&[0x61, 0xDBFF], r#""a\udbff""#),
        (&[0xDFFF, 0xD800], r#""\udfff\ud800""#),
        (&[0xD83D, 0xDE00], "\"\u{1F600}\""),
        (&[0xD83D, 0xD83D, 0xDE00], "\"\\ud83d\u{1F600}\""),
        (&[0x2603, 0x1A], "\"\u{2603}\\u001a\""),
    ];
    let mut ks = kernels();
    ks.push(Kernel::scalar());
    for &(text, want) in cases {
        let want: Vec<u16> = want.encode_utf16().collect();
        for &k in &ks {
            assert_eq!(quoted(k, text), want, "{} {text:x?}", k.path().name());
        }
    }
}

#[test]
fn latin1_lengths_and_alignments() {
    let ks = kernels();
    let mut big = vec![b'a'; 100_000 + 16];
    for (i, b) in big.iter_mut().enumerate() {
        *b = b"abcdefghijklmnopqrstuvwxyz 0123456789\x80\xe9\xff\x7f"[i % 41];
    }
    let mut lens: Vec<usize> = (0..=64).collect();
    lens.extend([255, 256, 4095, 4096, 4097, 8193, 100_000]);
    for &n in &lens {
        for start in 0..16 {
            if n > 4097 && start > 1 {
                continue;
            }
            check_latin1(&ks, &big[start..start + n], "clean");
        }
    }
    // The same lengths with an escape at the last unit and every unit escaped.
    for &n in &lens {
        let mut s = big[3..3 + n].to_vec();
        if let Some(l) = s.last_mut() {
            *l = b'"';
        }
        check_latin1(&ks, &s, "escape at the last unit");
        let all: Vec<u8> = (0..n).map(|i| [0x01, b'\\', b'"', 0x1F][i % 4]).collect();
        check_latin1(&ks, &all, "all units escape");
    }
}

#[test]
fn latin1_every_byte_at_every_position() {
    let ks = kernels();
    for n in (1..=40).chain([47, 48, 49, 63, 64, 65, 70]) {
        for pos in 0..n {
            for b in 0..=255u8 {
                let mut s = vec![b'm'; n];
                s[pos] = b;
                check_latin1(&ks, &s, "one byte");
            }
        }
    }
    // Runs of escapes of every length, at every position of a 48-byte field.
    for run in 1..=20 {
        for pos in 0..48 {
            let mut s = vec![b'z'; 48 + run];
            for (j, c) in s[pos..pos + run].iter_mut().enumerate() {
                *c = [b'\n', b'"', 0x00, b'\\', 0x1B][j % 5];
            }
            check_latin1(&ks, &s, "a run");
        }
    }
}

#[test]
fn utf16_pairs_lone_surrogates_and_wide_units_at_every_position() {
    let ks = kernels();
    for n in (1..=40).chain([47, 48, 49, 63, 64, 65]) {
        for pos in 0..n {
            let specials: &[&[u16]] = &[
                &[0xD83D, 0xDE00], // a valid pair (the widen point for a one-byte output)
                &[0xD800],         // a lone lead
                &[0xDFFF],         // a lone trail
                &[0xD800, 0x41],   // a lead, then not a trail
                &[0xDC00, 0xD800], // a trail, then a lead
                &[0x100],          // the lowest unit above 0xFF
                &[0x2603],
                &[0xFFFF],
                &[0x22],
                &[0x1F],
                &[0xFF],
                &[0x7F],
            ];
            for sp in specials {
                let mut s = vec![u16::from(b'q'); n];
                for (j, &u) in sp.iter().enumerate() {
                    if pos + j < n {
                        s[pos + j] = u;
                    }
                }
                check_utf16(&ks, &s, "a special unit");
            }
        }
    }
    // Every unit 0x00-0x1FF and every surrogate region boundary at the vector positions.
    for u in (0..=0x1FFu16).chain([0xD7FF, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xE000]) {
        for pos in [0, 1, 7, 8, 15, 16, 17, 31, 32, 33] {
            let mut s = vec![0x30u16; 40];
            s[pos] = u;
            check_utf16(&ks, &s, "one unit");
        }
    }
}

#[test]
fn utf16_lengths_alignments_and_chunk_ends() {
    let ks = kernels();
    let mut big: Vec<u16> = (0..100_016u32)
        .map(|i| u16::from(b"abc xyz 019\xe9"[i as usize % 12]))
        .collect();
    let mut lens: Vec<usize> = (0..=64).collect();
    lens.extend([255, 256, 4095, 4096, 4097, 100_000]);
    for &n in &lens {
        for start in 0..16 {
            if n > 4097 && start > 1 {
                continue;
            }
            check_utf16(&ks, &big[start..start + n], "clean");
        }
    }
    // A pair, a lone lead and a widen across the chunk end (units CHUNK - 1 and CHUNK), and
    // at the vector ends before it.
    for at in [
        CHUNK - 1,
        CHUNK - 2,
        CHUNK - 8,
        CHUNK - 16,
        CHUNK - 17,
        CHUNK - 32,
    ] {
        let n = CHUNK + 40;
        let mut s = big[..n].to_vec();
        s[at] = 0xD83D;
        s[at + 1] = 0xDE00;
        check_utf16(&ks, &s, "a pair across an end");
        s[at + 1] = 0x41;
        check_utf16(&ks, &s, "a lone lead before an end");
        s[at] = 0x4E2D;
        check_utf16(&ks, &s, "a wide unit at an end");
        // An escape run across the chunk end.
        for c in &mut s[at - 3..at + 5] {
            *c = 0x0A;
        }
        check_utf16(&ks, &s, "a run across an end");
    }
    // A two-byte text of a long string: a wide unit first, then a long clean run with
    // escapes.
    big[0] = 0x4E2D;
    big[50_000] = 0x22;
    big[99_999] = 0xD800;
    check_utf16(&ks, &big[..100_000], "long two-byte");
}

#[test]
fn random_inputs_match_the_reference() {
    let ks = kernels();
    let mut r = Lcg(0x00E9_150A);
    let alpha8: &[u8] = b"aZ 9\"\\\n\t\x00\x1f\x7f\x80\xe9\xff";
    let alpha16: &[u16] = &[
        0x61, 0x5A, 0x20, 0x22, 0x5C, 0x0A, 0x00, 0x1F, 0x7F, 0xE9, 0xFF, 0x100, 0x2603, 0xD83D,
        0xDE00, 0xD800, 0xDFFF, 0xFFFF,
    ];
    for t in 0..10_000 {
        let n = if r.below(10) == 0 {
            65 + r.below(600)
        } else {
            r.below(65)
        };
        // Mostly clean text, with a rate of special units that changes per case.
        let rate = 1 + r.below(16);
        if t % 2 == 0 {
            let s: Vec<u8> = (0..n)
                .map(|_| {
                    if r.below(rate) == 0 {
                        alpha8[r.below(alpha8.len())]
                    } else {
                        b'k'
                    }
                })
                .collect();
            check_latin1(&ks, &s, "random");
        } else {
            let s: Vec<u16> = (0..n)
                .map(|_| {
                    if r.below(rate) == 0 {
                        alpha16[r.below(alpha16.len())]
                    } else {
                        0x6B
                    }
                })
                .collect();
            check_utf16(&ks, &s, "random");
        }
    }
}

#[test]
fn a_chunk_body_stays_inside_the_reserve_bound() {
    // The worst case: every unit takes 6 output units (`\u00xx`), at every length and offset
    // of a chunk, on every body, for each source and output kind.
    let ks = kernels();
    let worst8 = vec![0x01u8; 200];
    let worst16 = vec![0x01u16; 200];
    let lone16: Vec<u16> = (0..200).map(|i| [0xD800, 0x01][i % 2]).collect();
    for &k in &ks {
        for from in 0..8 {
            for end in from + 1..=from + 120 {
                for two in [false, true] {
                    let (i, w, stop) = testing::chunk_latin1(k, two, &worst8, from, end);
                    assert_eq!((i, w.len(), stop), (end, 6 * (end - from), false));
                    let (i, w, stop) = testing::chunk_utf16(k, two, &worst16, from, end);
                    assert_eq!((i, w.len(), stop), (end, 6 * (end - from), false));
                    let (i, _, stop) = testing::chunk_utf16(k, two, &lone16, from, end);
                    assert_eq!((i, stop), (end, false));
                }
            }
        }
        // A widen stop gives the unit that needs it, with the units before it written.
        let mut s = vec![0x41u16; 50];
        s[37] = 0x2603;
        let (i, w, stop) = testing::chunk_utf16(k, false, &s, 0, 50);
        assert_eq!((i, w.len(), stop), (37, 37, true), "{}", k.path().name());
        // A pair across the chunk end: the body takes the trail past the end.
        s[37] = 0x41;
        s[39] = 0xD83D;
        s[40] = 0xDE00;
        let (i, w, stop) = testing::chunk_utf16(k, true, &s, 0, 40);
        assert_eq!((i, w.len(), stop), (41, 41, false), "{}", k.path().name());
    }
}

#[test]
fn the_text_widens_once_and_keeps_its_order() {
    let mut t = JsonText::new();
    t.push_ascii(b"ab");
    t.push_units(&[0x63, 0xE9]);
    assert!(!t.is_two_byte());
    t.push_units(&[0x64, 0x2603, 0x65]);
    assert!(t.is_two_byte());
    t.push_ascii(b"f");
    json_quote::quote_latin1(Kernel::detect(), b"g\xff", &mut t);
    assert_eq!(
        t.to_utf16(),
        [
            0x61, 0x62, 0x63, 0xE9, 0x64, 0x2603, 0x65, 0x66, 0x22, 0x67, 0xFF, 0x22
        ]
    );
    assert_eq!(t.len(), 12);
    // A lone surrogate does not widen; a pair does, from its own unit.
    let mut t = JsonText::new();
    json_quote::quote_utf16(Kernel::detect(), &[0x61, 0xD800, 0x62], &mut t);
    assert!(!t.is_two_byte());
    json_quote::quote_utf16(Kernel::detect(), &[0x61, 0xD83D, 0xDE00], &mut t);
    assert!(t.is_two_byte());
    let want: Vec<u16> = "\"a\\ud800b\"\"a\u{1F600}\"".encode_utf16().collect();
    assert_eq!(t.to_utf16(), want);
}

/// The strings of each kind with the text `t`: SEQ, a SEQ UTF-16 cell of Latin-1 text, a
/// CONS, a flattened CONS and a SLICED string.
fn kinds_of(rt: &crate::runtime::Runtime, t: &[u16]) -> Vec<(&'static str, u64)> {
    let mut v = vec![("seq", string::new_string_utf16(rt, t))];
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
fn cx_quotes_every_string_kind_on_every_path_and_finishes_one_byte() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let texts: Vec<Vec<u16>> = [
        "the \"quick\"\nbrown fox \\ jumps over the lazy dog\t",
        "caf\u{e9} na\u{ef}ve r\u{e9}sum\u{e9} \u{7f} caf\u{e9} na\u{ef}ve",
        "\u{1F600}smile\u{1F601}grin \u{2603} smile again \u{1F600}",
    ]
    .iter()
    .map(|t| t.encode_utf16().collect())
    .collect();
    let mut lone: Vec<u16> = "lone lead and trail: ".encode_utf16().collect();
    lone.extend_from_slice(&[0xD800, 0x20, 0xDC00]);
    for t in texts.iter().chain([&lone]) {
        let rt: &crate::runtime::Runtime = &iso.rt;
        let strs = kinds_of(rt, t);
        let want = quoted(Kernel::scalar(), t);
        let wide = want.iter().any(|&u| u > 0xFF);
        for p in Path::ALL {
            for &(kind, s) in &strs {
                let r = json_quote::with_path(p, || {
                    iso.with_cx(|cx| {
                        let flat = cx.flatten(StrLocal::from_bits(s))?;
                        let mut out = JsonText::new();
                        cx.json_quote_into(flat, &mut out);
                        let raw: Local<'_, FlatStr> = flat;
                        cx.json_push_flat(raw, &mut out);
                        let r = cx.json_text_finish(out)?;
                        let kind = crate::kinds::cell_kind(r.bits());
                        let flat_r = cx.flatten(r)?;
                        let units: Vec<u16> = (0..cx.string_len(r))
                            .map(|i| cx.code_unit_at(flat_r, i).unwrap())
                            .collect();
                        Ok((units, kind))
                    })
                    .unwrap()
                });
                let Some((units, cell)) = r else { continue };
                let mut w = want.clone();
                w.extend_from_slice(t);
                assert_eq!(units, w, "{} {kind}", p.name());
                // A text with no unit above 0xFF is a SEQ_LATIN1 cell (1), else SEQ_UTF16 (2).
                let two = wide || t.iter().any(|&u| u > 0xFF);
                assert_eq!(cell, if two { 2 } else { 1 }, "{} {kind}", p.name());
            }
        }
    }
}
