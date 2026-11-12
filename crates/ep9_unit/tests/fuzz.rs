//! Random corruption: 10^5 cases (10^4 of them on the native unit) of byte changes, truncations
//! and extensions. The check never panics, and its outcome is consistent with what changed:
//! - another length: LOAD ERROR (total_size no longer matches);
//! - changed bytes outside the signature section, hash not recomputed: LOAD ERROR;
//! - hash recomputed and not a LOAD ERROR: the input is the canonical encoding of its unit.

mod common;

use common::*;
use ep9_unit::*;

const CASES: u64 = 100_000;
/// One case in this many uses the native unit. Container v3 aligns the text to 64 KiB, so a
/// native unit is 64 KiB and more, and each hash of it takes about 3 ms in a debug build (the
/// small unit: under 1 KB). One in 10 keeps 10^4 native cases and the test near 10 s of CPU
/// time, which the batches spread over every CPU.
const NATIVE_EVERY: u64 = 10;
/// Cases per batch of `random_flips_and_truncations_never_panic` (each batch holds about
/// 6.5 MB of native units).
const BATCH: u64 = 1000;

#[test]
fn random_flips_and_truncations_never_panic() {
    let host = synthetic_host();
    let bases = [
        TestUnit::small(&host).encode(),
        TestUnit::native(&host).encode(),
    ];
    let signature_start = {
        let u = TestUnit::parse(&bases[1]).unwrap();
        let sig_len = u.sections.last().unwrap().1.len();
        bases[1].len() - sig_len
    };
    let mut rng = Rng::new(0x5eed);
    let mut counts = [0usize; 4];
    // The random choices of each case come from one sequence, in case order; the hashes and
    // the checks of a batch run on every CPU (route UNIT-TEST-TIME).
    let mut case = 0;
    while case < CASES {
        let mut batch = Vec::new();
        for case in case..(case + BATCH).min(CASES) {
            let which = usize::from(case % NATIVE_EVERY == 0);
            let mut m = bases[which].clone();
            // 1-4 byte changes; half of them in the header and table, where the rules are. A
            // native unit is 64 KiB and more (container v3 aligns the text to 64 KiB), so the
            // test keeps the changed positions instead of comparing every byte.
            let mut changed = Vec::new();
            for _ in 0..1 + rng.below(4) {
                let span = if rng.below(2) == 0 {
                    (SECTION_TABLE_OFFSET + 3 * SECTION_ENTRY_SIZE).min(m.len())
                } else {
                    m.len()
                };
                let at = rng.below(span as u64) as usize;
                m[at] = rng.next() as u8;
                changed.push(at);
            }
            let rehashed = rng.below(2) == 0;
            let end = match rng.below(10) {
                0 => End::Truncate(rng.below(m.len() as u64) as usize),
                1 => End::Extend((0..1 + rng.below(16)).map(|_| rng.next() as u8).collect()),
                _ => End::Keep,
            };
            batch.push(Case {
                case,
                which,
                m,
                changed,
                rehashed,
                end,
            });
        }
        case += BATCH;
        for outcome in par_map(&batch, |c| run(c, &bases, signature_start, &host)) {
            counts[outcome as usize] += 1;
        }
    }
    println!("{CASES} cases: load/bytecode/source/load-error = {counts:?}");
}

#[test]
fn every_truncation_is_a_load_error() {
    let host = synthetic_host();
    for base in [
        TestUnit::small(&host).encode(),
        TestUnit::native(&host).encode(),
    ] {
        for n in 0..base.len() {
            assert_eq!(check(&base[..n], &host), Outcome::LoadError, "length {n}");
        }
        assert_eq!(check(&base, &host), Outcome::Load);
    }
}

/// The end of a case: as changed, cut to a length, or with bytes added.
enum End {
    Keep,
    Truncate(usize),
    Extend(Vec<u8>),
}

/// One case of `random_flips_and_truncations_never_panic`: the changed unit before the
/// rehash and the end change.
struct Case {
    case: u64,
    which: usize,
    m: Vec<u8>,
    changed: Vec<usize>,
    rehashed: bool,
    end: End,
}

/// Runs one case (the rehash, the end change, the check) and checks its outcome.
fn run(c: &Case, bases: &[Vec<u8>; 2], signature_start: usize, host: &HostInfo) -> Outcome {
    let (case, which, base) = (c.case, c.which, &bases[c.which]);
    let mut m = c.m.clone();
    if c.rehashed {
        rehash(&mut m);
    }
    match &c.end {
        End::Truncate(n) => m.truncate(*n),
        End::Extend(more) => m.extend_from_slice(more),
        End::Keep => {}
    }
    let outcome = check(&m, host);
    if m.len() != base.len() {
        assert_eq!(outcome, Outcome::LoadError, "case {case}: length changed");
        return outcome;
    }
    // Without a rehash only the changed positions can differ from the base.
    let outside_signature = c
        .changed
        .iter()
        .filter(|&&i| m[i] != base[i])
        .any(|&i| which == 0 || i < signature_start);
    if !c.rehashed && outside_signature {
        assert_eq!(
            outcome,
            Outcome::LoadError,
            "case {case}: hash not recomputed"
        );
    }
    if c.rehashed && outcome != Outcome::LoadError {
        let u = TestUnit::parse(&m).expect("passes step 1, so it parses");
        assert_eq!(u.encode(), m, "case {case}: accepted a non-canonical unit");
    }
    outcome
}
