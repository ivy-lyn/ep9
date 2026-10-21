//! Random corruption: 10^5 cases of byte changes, truncations and extensions. The check never
//! panics, and its outcome is consistent with what changed:
//! - another length: LOAD ERROR (total_size no longer matches);
//! - changed bytes outside the signature section, hash not recomputed: LOAD ERROR;
//! - hash recomputed and not a LOAD ERROR: the input is the canonical encoding of its unit.

mod common;

use common::*;
use ep9_unit::*;

const CASES: u64 = 100_000;

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
    for case in 0..CASES {
        let which = (case % 2) as usize;
        let base = &bases[which];
        let mut m = base.clone();
        // 1-4 byte changes; half of them in the header and table, where the rules are.
        for _ in 0..1 + rng.below(4) {
            let span = if rng.below(2) == 0 {
                (SECTION_TABLE_OFFSET + 3 * SECTION_ENTRY_SIZE).min(m.len())
            } else {
                m.len()
            };
            let at = rng.below(span as u64) as usize;
            m[at] = rng.next() as u8;
        }
        let rehashed = rng.below(2) == 0;
        if rehashed {
            rehash(&mut m);
        }
        match rng.below(10) {
            0 => m.truncate(rng.below(m.len() as u64) as usize),
            1 => m.extend((0..1 + rng.below(16)).map(|_| rng.next() as u8)),
            _ => {}
        }
        let outcome = check(&m, &host);
        counts[outcome as usize] += 1;
        if m.len() != base.len() {
            assert_eq!(outcome, Outcome::LoadError, "case {case}: length changed");
            continue;
        }
        let outside_signature = (0..m.len())
            .filter(|&i| m[i] != base[i])
            .any(|i| which == 0 || i < signature_start);
        if !rehashed && outside_signature {
            assert_eq!(
                outcome,
                Outcome::LoadError,
                "case {case}: hash not recomputed"
            );
        }
        if rehashed && outcome != Outcome::LoadError {
            let u = TestUnit::parse(&m).expect("passes step 1, so it parses");
            assert_eq!(u.encode(), m, "case {case}: accepted a non-canonical unit");
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
