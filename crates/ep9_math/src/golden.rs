//! Golden tests (H36 Rules row 5, H36-Q6): for each function, 1,000,000 deterministic inputs
//! and the SHA-256 of the result bits, pinned here. Every target (CI: 4 targets, x86-64 with
//! and without FMA) must give the same digests. The inputs come from an integer PRNG and exact
//! bit operations only, so they are the same everywhere. NaN results hash as one NaN (JS has
//! one NaN; the bits of a NaN may differ between targets).
//!
//! Also here: `soft_equals_hw_*` (the whole function, fast and accurate paths, with the hardware
//! FMA and with [`crate::exact::SoftFma`] on the same inputs) and the CORE-MATH worst cases
//! with their MPFR results.

use crate::exact::with_soft_fma;
use crate::hf;
use ep9_bytecode::sha256::{Sha256, to_hex};

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Any bit pattern (NaN, infinities, subnormals included).
    fn bits(&mut self) -> f64 {
        f64::from_bits(self.next())
    }

    /// A random double with exponent in `emin..=emax` (each binade equally likely; a binade
    /// below -1022 gives a subnormal of that size), positive or with a random sign.
    fn logu(&mut self, emin: i32, emax: i32, any_sign: bool) -> f64 {
        let e = emin + self.below((emax - emin + 1) as u64) as i32;
        let r = self.next();
        let sign = if any_sign { r & (1 << 63) } else { 0 };
        let mant = r & 0xfffffffffffff;
        let b = if e >= -1022 {
            (((e + 1023) as u64) << 52) | mant
        } else {
            // 2^e <= value < 2^(e+1) in the subnormal range: 2^(e+1074) units of 2^-1074
            let k = (e + 1074) as u32;
            (1u64 << k) | (mant >> (52 - k))
        };
        f64::from_bits(sign | b)
    }

    /// `c` moved by a random number of ulps in `-2^k..=2^k` (bit arithmetic).
    fn near(&mut self, c: f64, k: u32) -> f64 {
        let d = self.below((2u64 << k) + 1) as i64 - (1i64 << k);
        f64::from_bits(c.to_bits().wrapping_add(d as u64))
    }

    /// `near` with a random sign.
    fn near_any(&mut self, c: f64, k: u32) -> f64 {
        let v = self.near(c, k);
        if self.next() & 1 == 0 { v } else { -v }
    }
}

const N: usize = 1_000_000;

/// The special inputs added to every list.
const SPECIAL: [f64; 22] = [
    0.0,
    -0.0,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NAN,
    1.0,
    -1.0,
    2.0,
    -2.0,
    0.5,
    -0.5,
    5e-324,
    -5e-324,
    2.2250738585072014e-308,
    f64::MAX,
    -f64::MAX,
    std::f64::consts::PI,
    -std::f64::consts::PI,
    std::f64::consts::FRAC_PI_2,
    std::f64::consts::E,
    10.0,
    1e-300,
];

/// A weighted generator of one input.
type Gen1<'a> = (u32, &'a dyn Fn(&mut Rng) -> f64);
/// A weighted generator of one input pair.
type Gen2<'a> = (u32, &'a dyn Fn(&mut Rng) -> (f64, f64));

/// `n` inputs from `gen` (a draw picks one of its generators by weight), then [`SPECIAL`].
fn unary_inputs(seed: u64, n: usize, gens: &[Gen1]) -> Vec<f64> {
    let mut r = Rng(seed);
    let total: u32 = gens.iter().map(|g| g.0).sum();
    let mut v = Vec::with_capacity(n + SPECIAL.len());
    for _ in 0..n {
        let mut k = r.below(u64::from(total)) as u32;
        for (w, g) in gens {
            if k < *w {
                v.push(g(&mut r));
                break;
            }
            k -= w;
        }
    }
    v.extend_from_slice(&SPECIAL);
    v
}

/// Like [`unary_inputs`] for pairs; the specials are every pair of [`SPECIAL`].
fn binary_inputs(seed: u64, n: usize, gens: &[Gen2]) -> Vec<(f64, f64)> {
    let mut r = Rng(seed);
    let total: u32 = gens.iter().map(|g| g.0).sum();
    let mut v = Vec::with_capacity(n + SPECIAL.len() * SPECIAL.len());
    for _ in 0..n {
        let mut k = r.below(u64::from(total)) as u32;
        for (w, g) in gens {
            if k < *w {
                v.push(g(&mut r));
                break;
            }
            k -= w;
        }
    }
    for a in SPECIAL {
        for b in SPECIAL {
            v.push((a, b));
        }
    }
    v
}

/// The inputs of each one-argument function.
fn inputs_of(name: &str, n: usize) -> Vec<f64> {
    let seed = name.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    let bits: &dyn Fn(&mut Rng) -> f64 = &|r| r.bits();
    match name {
        "sin" | "cos" | "tan" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (4, &|r| r.logu(-30, 1023, true)),
                (2, &|r| r.logu(-1074, -27, true)),
                (2, &|r| {
                    // near k*pi/2
                    let k = (r.below(2_000_001) as i64 - 1_000_000) as f64;
                    r.near(k * std::f64::consts::FRAC_PI_2, 20)
                }),
            ],
        ),
        "asin" | "acos" | "atanh" => unary_inputs(
            seed,
            n,
            &[
                (1, bits),
                (6, &|r| r.logu(-60, -1, true)),
                (2, &|r| r.near_any(1.0, 40)),
                (1, &|r| r.logu(-1074, -61, true)),
            ],
        ),
        "atan" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (6, &|r| r.logu(-60, 60, true)),
                (2, &|r| r.logu(-1074, 1023, true)),
            ],
        ),
        "sinh" | "cosh" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (6, &|r| r.logu(-60, 10, true)),
                (2, &|r| r.near_any(710.0, 50)),
            ],
        ),
        "tanh" | "expm1" | "exp" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (6, &|r| r.logu(-60, 10, true)),
                (2, &|r| r.logu(-1074, -61, true)),
            ],
        ),
        "asinh" | "cbrt" => unary_inputs(
            seed,
            n,
            &[
                (3, bits),
                (5, &|r| r.logu(-60, 60, true)),
                (2, &|r| r.logu(-1074, 1023, true)),
            ],
        ),
        "acosh" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (4, &|r| r.logu(0, 1023, false)),
                (4, &|r| r.near(1.0, 48)),
            ],
        ),
        "log" | "log2" | "log10" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (4, &|r| r.logu(-1074, 1023, false)),
                (4, &|r| r.near(1.0, 48)),
            ],
        ),
        "log1p" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (4, &|r| r.logu(-60, 1023, true)),
                (2, &|r| r.near(-1.0, 48)),
                (2, &|r| r.logu(-1074, -61, true)),
            ],
        ),
        "round_js" => unary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (4, &|r| r.logu(-5, 60, true)),
                (4, &|r| {
                    let k = r.below(1 << 40) as f64;
                    r.near_any(k + 0.5, 2)
                }),
            ],
        ),
        "fround" => unary_inputs(seed, n, &[(5, bits), (5, &|r| r.logu(-160, 130, true))]),
        _ => panic!("no inputs for {name}"),
    }
}

/// The inputs of each two-argument function.
fn pairs_of(name: &str, n: usize) -> Vec<(f64, f64)> {
    let seed = name.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    let bits: &dyn Fn(&mut Rng) -> (f64, f64) = &|r| (r.bits(), r.bits());
    match name {
        "atan2" => binary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (5, &|r| {
                    (r.logu(-1074, 1023, true), r.logu(-1074, 1023, true))
                }),
                (3, &|r| {
                    let x = r.logu(-500, 500, true);
                    (r.near_any(x, 30), x)
                }),
            ],
        ),
        "pow_js" => binary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (3, &|r| (r.logu(-1074, 1023, false), r.logu(-30, 10, true))),
                (3, &|r| (r.near(1.0, 40), r.logu(0, 60, true))),
                (2, &|r| {
                    (r.logu(-20, 20, true), (r.below(200) as i64 - 100) as f64)
                }),
            ],
        ),
        "fmod" => binary_inputs(
            seed,
            n,
            &[
                (3, bits),
                (5, &|r| {
                    (r.logu(-1074, 1023, true), r.logu(-1074, 1023, true))
                }),
                (2, &|r| (r.logu(-10, 60, true), r.logu(-10, 10, true))),
            ],
        ),
        "hypot2" => binary_inputs(
            seed,
            n,
            &[
                (2, bits),
                (5, &|r| {
                    (r.logu(-1074, 1023, true), r.logu(-1074, 1023, true))
                }),
                (3, &|r| {
                    let x = r.logu(-500, 500, true);
                    (r.near_any(x, 50), x)
                }),
            ],
        ),
        _ => panic!("no inputs for {name}"),
    }
}

/// The triples of `fma_exact` and the three-argument `hypot_js`.
fn triples_of(name: &str, n: usize) -> Vec<[f64; 3]> {
    let seed = name.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    let mut r = Rng(seed);
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(match r.below(4) {
            0 => [r.bits(), r.bits(), r.bits()],
            1 => [
                r.logu(-60, 60, true),
                r.logu(-60, 60, true),
                r.logu(-120, 120, true),
            ],
            2 => {
                // cancellation: c near -(a*b)
                let a = r.logu(-500, 500, true);
                let b = r.logu(-500, 500, true);
                [a, b, -r.near(a * b, 3)]
            }
            _ => [
                r.logu(-1074, 1023, true),
                r.logu(-1074, 1023, true),
                r.logu(-1074, 1023, true),
            ],
        });
    }
    for a in SPECIAL {
        for b in SPECIAL {
            v.push([a, b, 1.0]);
        }
    }
    v
}

/// SHA-256 of the result bits (little-endian), every NaN as 0x7ff8000000000000.
fn digest(results: impl Iterator<Item = f64>) -> String {
    let mut h = Sha256::new();
    for x in results {
        let b = if x.is_nan() {
            0x7ff8000000000000
        } else {
            x.to_bits()
        };
        h.update(&b.to_le_bytes());
    }
    to_hex(&h.finish())
}

/// The digest of `name` on its 1,000,000 inputs (plus the specials).
fn golden_digest(name: &str) -> String {
    match name {
        "atan2" => digest(
            pairs_of(name, N)
                .into_iter()
                .map(|(y, x)| crate::atan2(y, x)),
        ),
        "pow_js" => digest(
            pairs_of(name, N)
                .into_iter()
                .map(|(x, y)| crate::pow_js(x, y)),
        ),
        "fmod" => digest(
            pairs_of(name, N)
                .into_iter()
                .map(|(x, y)| crate::fmod(x, y)),
        ),
        "hypot2" => digest(
            pairs_of(name, N)
                .into_iter()
                .map(|(a, b)| crate::hypot_js(&[a, b])),
        ),
        "hypot3" => digest(triples_of(name, N).into_iter().map(|t| crate::hypot_js(&t))),
        "fma_exact" => digest(
            triples_of(name, N)
                .into_iter()
                .map(|[a, b, c]| crate::fma_exact(a, b, c)),
        ),
        _ => {
            let f = unary(name);
            digest(inputs_of(name, N).into_iter().map(f))
        }
    }
}

fn unary(name: &str) -> fn(f64) -> f64 {
    match name {
        "sin" => crate::sin,
        "cos" => crate::cos,
        "tan" => crate::tan,
        "asin" => crate::asin,
        "acos" => crate::acos,
        "atan" => crate::atan,
        "sinh" => crate::sinh,
        "cosh" => crate::cosh,
        "tanh" => crate::tanh,
        "asinh" => crate::asinh,
        "acosh" => crate::acosh,
        "atanh" => crate::atanh,
        "exp" => crate::exp,
        "expm1" => crate::expm1,
        "log" => crate::log,
        "log2" => crate::log2,
        "log10" => crate::log10,
        "log1p" => crate::log1p,
        "cbrt" => crate::cbrt,
        "round_js" => crate::round_js,
        "fround" => crate::fround,
        _ => panic!("no function {name}"),
    }
}

macro_rules! golden {
    ($($test:ident: $name:literal => $hash:literal,)*) => {
        $(
            #[test]
            #[cfg_attr(miri, ignore)]
            fn $test() {
                let got = golden_digest($name);
                // `EP9_GOLDEN_PRINT=1 cargo test -p ep9_math golden_ -- --nocapture` prints
                // the digests of this target.
                if std::env::var_os("EP9_GOLDEN_PRINT").is_some() {
                    println!("GOLDEN {} {}", $name, got);
                    return;
                }
                assert_eq!(got, $hash, "{}: golden SHA-256 changed (H36 Rules row 5)", $name);
            }
        )*
    };
}

golden! {
    golden_sin: "sin" => "49248bcbb3c599b5d26f37ea18e249aa48b1a2eced012ad8f51cdaa5bb08e9bb",
    golden_cos: "cos" => "288fd42cfe8584887835a5820cfa911caac4b191cd6670d9a1bde64684cf41bd",
    golden_tan: "tan" => "3fb6046180752ef7ebc21d7d2d36b9d0b54230954828910690207e379655558b",
    golden_asin: "asin" => "d4a5e18251cdaff14ba84f337d3b9c79452cb216cb713f4d1ef009946c4657c9",
    golden_acos: "acos" => "708bd8ab4a89d35a5e1a5e7bb322ddfeb1baa10837a579c73a57cf9d1306eab1",
    golden_atan: "atan" => "127b69499691c942e136ffd78e49a837ca054d63748111931ed1e493d6b8256d",
    golden_atan2: "atan2" => "d104c45b15bc958e10d5363feb093662af1e56422d72abdd087ab2085b20b0d9",
    golden_sinh: "sinh" => "affdd164e30e65d8ca408341fba80d0a647d58f21e3aab152fa595623fa2e028",
    golden_cosh: "cosh" => "7e8e1224aeae3bf9bf63a53fcbc24849cdb05c4fd025a422d7d0ddfc5c781e86",
    golden_tanh: "tanh" => "df3468da1cb7911fa3de65dafac9e909b404d0603120ad76c792396737fc3fc2",
    golden_asinh: "asinh" => "0edff9816213cf46b17d4499365778392eb787ed27c31f4e6df5399498fda17b",
    golden_acosh: "acosh" => "de1816ecd48bf4b902edfc608f9ab4d51d2773c8afc60199f6deed8a0444ecd0",
    golden_atanh: "atanh" => "e48caa76c76db6c75dcf2f5cdf6be9f2e14159355f22126b69e8c590e9547dff",
    golden_exp: "exp" => "3f7f92a6e16b5e837782d9259b726cc9a429d564763888bfd42e995c36d124de",
    golden_expm1: "expm1" => "9fa9391574eccf6391963a61a4bd37ec04d2028070c1f8fefd389f859f13460d",
    golden_log: "log" => "eafb7d2342dbe92f2c25befb454c3796611d95280e377bc25ab1c89f446b204a",
    golden_log2: "log2" => "bd5c4683a5c87fbd05906f9a1d8de5cdb1c0451b20b632984cc35ffe22f8b662",
    golden_log10: "log10" => "03255d14632de51134f1dfcf40c185fc056ce9b5f9c2f6ad6f73da9b175cc9e2",
    golden_log1p: "log1p" => "849aa450384600d74c458ac2a3cbf93e71011cac827fa5f616f409048e2128f0",
    golden_cbrt: "cbrt" => "b9546647912cc542c0478902a8e5e0411727737845cc700f2fa2078f48fb2ea8",
    golden_pow_js: "pow_js" => "af5eddabddc60f798782624a4e8f1bfa2e92d52b1a7f00e76a7937db46427297",
    golden_hypot2: "hypot2" => "cd59a988f41561fb8d98b2c102018676da9e7b6a58c5d9a4fd9930dd4fb3b5a8",
    golden_hypot3: "hypot3" => "57e06fca0d67d13d7e5aee3d50933a5a5a1057c3a8639e35da64043717dc9f32",
    golden_fmod: "fmod" => "0f1ed16e1e657593a6dbb0511d333448c21fc4715425d3217ed548ea7fca1c3b",
    golden_round_js: "round_js" => "392f53ca5160f9dd2322531397c8904631740027d0a5d36e59091e2d3125bc64",
    golden_fround: "fround" => "afb93baec45f0abc777fe4e4e5d0684559fbb470f5e33eee061e3f1ec5612ef2",
    golden_fma_exact: "fma_exact" => "ed0b1b4b3e27103685edc9611a53524d9dfc6b8f2e283b9e14e00120cbf25a68",
}

/// The same bits with the hardware FMA and with `SoftFma`, on the first `n` golden inputs of
/// `name` (the whole function: the `multiversion!` entry points of its fast and accurate paths
/// all take `SoftFma` inside [`with_soft_fma`]).
fn assert_soft_equals_hw(name: &str, n: usize) {
    let same = |a: f64, b: f64| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
    match name {
        "atan2" | "pow_js" => {
            let f: fn(f64, f64) -> f64 = if name == "atan2" {
                crate::atan2
            } else {
                crate::pow_js
            };
            for (a, b) in pairs_of(name, n) {
                let hw = f(a, b);
                let sw = with_soft_fma(|| f(a, b));
                assert!(same(hw, sw), "{name}({a:e}, {b:e}): hw {hw:e}, soft {sw:e}");
            }
        }
        _ => {
            let f = unary(name);
            for x in inputs_of(name, n) {
                let hw = f(x);
                let sw = with_soft_fma(|| f(x));
                assert!(same(hw, sw), "{name}({x:e}): hw {hw:e}, soft {sw:e}");
            }
        }
    }
}

macro_rules! soft_equals_hw {
    ($($test:ident: $name:literal,)*) => {
        $(
            #[test]
            #[cfg_attr(miri, ignore)]
            fn $test() {
                assert_soft_equals_hw($name, 100_000);
            }
        )*
    };
}

soft_equals_hw! {
    soft_equals_hw_sin: "sin",
    soft_equals_hw_cos: "cos",
    soft_equals_hw_tan: "tan",
    soft_equals_hw_asin: "asin",
    soft_equals_hw_acos: "acos",
    soft_equals_hw_atan: "atan",
    soft_equals_hw_atan2: "atan2",
    soft_equals_hw_sinh: "sinh",
    soft_equals_hw_cosh: "cosh",
    soft_equals_hw_tanh: "tanh",
    soft_equals_hw_asinh: "asinh",
    soft_equals_hw_acosh: "acosh",
    soft_equals_hw_atanh: "atanh",
    soft_equals_hw_expm1: "expm1",
    soft_equals_hw_log2: "log2",
    soft_equals_hw_log10: "log10",
    soft_equals_hw_log1p: "log1p",
    soft_equals_hw_cbrt: "cbrt",
}

/// The CI `SoftFma vs hardware FMA` step (`cargo test -p ep9_math --release soft_vs_hw --
/// --ignored`): every CR function on all its golden inputs.
#[test]
#[ignore = "about 2 * 10^7 calls; run with --release -- --ignored"]
fn soft_vs_hw_cr_functions() {
    for name in [
        "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh",
        "acosh", "atanh", "exp", "expm1", "log", "log2", "log10", "log1p", "cbrt", "pow_js",
    ] {
        assert_soft_equals_hw(name, N);
    }
}

/// CORE-MATH worst cases (`src/binary64/<fn>/<fn>.wc`: the first 4 and 4 drawn at random),
/// with the MPFR 4.2 results (gmpy2, binary64 context, round to nearest).
#[test]
fn core_math_worst_cases() {
    let rows: &[(&str, &[&str], &str)] = &[
        ("sin", &["0x1.005023d32fee5p+1"], "0x1.d109ad145c88fp-1"),
        ("sin", &["0x1.00874a5f9fc33p-13"], "0x1.00874a54e4265p-13"),
        ("sin", &["0x1.009ee184a02e7p-8"], "0x1.009eb68a55928p-8"),
        ("sin", &["0x1.00aeab577dc9ap-11"], "0x1.00aeaaab74d9ep-11"),
        ("sin", &["0x1.921fb54442d3ep+0"], "0x1.0000000000000p+0"),
        ("sin", &["0x1.a9ab8756c7affp+813"], "0x1.6ead7f6f7bb9ap-2"),
        ("sin", &["0x1.a2e69f31a3fbbp+343"], "0x1.f28f6e2ceca0ap-1"),
        ("sin", &["0x1.4dc37dcd45a23p+563"], "0x1.8973a0ec28d4bp-3"),
        ("cos", &["0x1.0009effd4bedap-12"], "0x1.fffffeffec1fcp-1"),
        ("cos", &["0x1.00147eec5cfa5p+0"], "0x1.148001d3ec044p-1"),
        ("cos", &["0x1.0066bbcf2cf68p-7"], "0x1.fffbfccad4182p-1"),
        ("cos", &["0x1.00786b0bf4210p-2"], "0x1.f006612beb9a8p-1"),
        ("cos", &["0x1.6092795aec658p+258"], "-0x1.e15b1a9e97985p-1"),
        ("cos", &["0x1.0420cc1ff4e65p+281"], "0x1.3c80f21a6f8ebp-1"),
        ("cos", &["0x1.c96de05025609p+400"], "-0x1.8e3402726c5adp-2"),
        ("cos", &["0x1.827d4440ee2dbp+505"], "-0x1.a7b56f83b0859p-2"),
        ("tan", &["0x1.0004b89dcb553p-11"], "0x1.0004b9f3338f8p-11"),
        ("tan", &["0x1.00452f0e0134dp-13"], "0x1.00452f2367da9p-13"),
        ("tan", &["0x1.005785112e4cap-12"], "0x1.00578566db452p-12"),
        ("tan", &["0x1.005c4ae44e6c0p+0"], "0x1.8fef1e1cdf9afp+0"),
        ("tan", &["0x1.9561a8e52acd6p+677"], "-0x1.66d7ef25b2366p+2"),
        ("tan", &["0x1.f802f33ed5b94p+44"], "-0x1.772525119e155p+2"),
        ("tan", &["0x1.20d3274620c25p+1013"], "-0x1.4ec2764dcd982p+1"),
        ("tan", &["0x1.fba9599f8a7b0p+92"], "0x1.7e5c2880b72eep-3"),
        ("asin", &["0x1.fffffffffffffp-7"], "0x1.0002aabdde94cp-6"),
        ("asin", &["0x1.0000000000000p-6"], "0x1.0002aabdde94cp-6"),
        (
            "asin",
            &["0x0.fffffffffffffp-1022"],
            "0x0.fffffffffffffp-1022",
        ),
        ("asin", &["0x1.7137449123e7bp-26"], "0x1.7137449123e7bp-26"),
        ("asin", &["0x1.1c6fe96290bb8p-4"], "0x1.1caa8feb783dbp-4"),
        ("asin", &["0x1.1c7e48baafd3ep-4"], "0x1.1cb8f82b5f9aap-4"),
        ("asin", &["0x1.1dafeba5569edp-4"], "0x1.1deb594cbfe8bp-4"),
        ("asin", &["0x1.1e08fa38ee787p-4"], "0x1.1e449f9a352aep-4"),
        ("acos", &["0x1.ffffffffffdc0p-1"], "0x1.8000000000024p-22"),
        ("acos", &["0x1.dffff776c7505p-1"], "0x1.6bf3ba258567bp-2"),
        ("acos", &["0x1.fffffffffffffp-1"], "0x1.0000000000000p-26"),
        ("acos", &["0x1.022efc1ed056dp-1"], "0x1.0ad1f27f8943fp+0"),
        ("acos", &["-0x1.d1abd5d08e203p-7"], "0x1.95c314f688b1ep+0"),
        ("acos", &["0x1.17ce59d9ccebap-42"], "0x1.921fb544428b9p+0"),
        ("acos", &["0x1.d19b5764a989fp-4"], "0x1.74f5dc8740ffdp+0"),
        ("acos", &["0x1.5539676733ae8p-44"], "0x1.921fb54442bc3p+0"),
        ("atan", &["0x1.000321dec01a8p-10"], "0x1.00031c8938d9ep-10"),
        ("atan", &["0x1.00046968c6c71p+11"], "0x1.91ffb5d198240p+0"),
        ("atan", &["0x1.0007ba6fbd19ap-15"], "0x1.0007ba6e67a56p-15"),
        ("atan", &["0x1.00094221b47e5p+7"], "0x1.901fca72350b9p+0"),
        ("atan", &["0x1.e6c6fee6c421dp+6"], "0x1.900530caa20cdp+0"),
        ("atan", &["0x1.3f6ca93669bcdp-11"], "0x1.3f6ca69f56528p-11"),
        ("atan", &["0x1.517b5cc536a7cp-7"], "0x1.51784ed05d1cfp-7"),
        ("atan", &["0x1.9ba636ecdefb4p-14"], "0x1.9ba636d6b2379p-14"),
        (
            "atan2",
            &["0x0.0000000000001p-1022", "0x0.0000000000001p-1022"],
            "0x1.921fb54442d18p-1",
        ),
        (
            "atan2",
            &["0x0.0000000000001p-1022", "0x0.fffffffffffffp-1022"],
            "0x1.0000000000001p-52",
        ),
        (
            "atan2",
            &["0x0.0000000000001p-1022", "0x1.0000000000000p-1022"],
            "0x1.0000000000000p-52",
        ),
        (
            "atan2",
            &["0x0.0000000000001p-1022", "0x1.0000000000000p+0"],
            "0x0.0000000000001p-1022",
        ),
        (
            "atan2",
            &["0x1.ff0fe58722328p+50", "0x1.11ccfe43d6b7ep+51"],
            "0x1.80759907a8c64p-1",
        ),
        (
            "atan2",
            &["0x1.f78bd17c18c55p+53", "0x1.5a0ee4d4dfa69p+53"],
            "0x1.eff75b305248dp-1",
        ),
        (
            "atan2",
            &["0x1.68377e5d8523dp+52", "0x1.b8d9ba152296bp+52"],
            "0x1.5ec291d2eb479p-1",
        ),
        (
            "atan2",
            &["0x1.259f6d6f68a70p+52", "0x1.887184cbac68bp+52"],
            "0x1.48e11f041398cp-1",
        ),
        ("sinh", &["0x1.002ee87ae6797p-8"], "0x1.002f133d0bd22p-8"),
        ("sinh", &["0x1.003a9d3e03c20p-20"], "0x1.003a9d3e03eccp-20"),
        ("sinh", &["0x1.00a0a44cc45b0p-3"], "0x1.014cb39ade240p-3"),
        ("sinh", &["0x1.00b191ef23e3cp-5"], "0x1.00bc52fe0ec34p-5"),
        ("sinh", &["0x1.024a1f51b1a50p-2"], "0x1.050980e316e6cp-2"),
        ("sinh", &["0x1.08871cf6a5906p-2"], "0x1.0b7ad12a59756p-2"),
        ("sinh", &["0x1.3a85358c137e1p+5"], "0x1.a5912b198de70p+55"),
        ("sinh", &["0x1.050e8f95e1e5dp-2"], "0x1.07e4d7d791035p-2"),
        ("cosh", &["0x1.003c1d82e8bb4p+0"], "0x1.8b4e05c82a049p+0"),
        ("cosh", &["0x1.0051f2e2330acp-19"], "0x1.0000000002014p+0"),
        ("cosh", &["0x1.007fe00ff6065p-22"], "0x1.0000000000081p+0"),
        ("cosh", &["0x1.00bf99263dd9bp-1"], "0x1.20de183e41450p+0"),
        ("cosh", &["0x1.c17d1d5829729p+7"], "0x1.2dd3d2cd8fd0fp+323"),
        ("cosh", &["0x1.21fec0d44e09ep-6"], "0x1.000a441af798bp+0"),
        ("cosh", &["0x1.225e61eccca65p-6"], "0x1.000a4ae16cff4p+0"),
        ("cosh", &["0x1.13a7cedbf8ac7p-10"], "0x1.000009468fb1cp+0"),
        ("tanh", &["0x1.000b47bdb383fp-9"], "0x1.000b32658e41fp-9"),
        ("tanh", &["0x1.0017d47cec425p-12"], "0x1.0017d4277f166p-12"),
        ("tanh", &["0x1.002b19915b12bp-7"], "0x1.0029c391a65afp-7"),
        ("tanh", &["0x1.004524207fad9p-3"], "0x1.fde1b6fff6ef9p-4"),
        ("tanh", &["0x1.a10467ff22c82p-10"], "0x1.a10450f1648e6p-10"),
        ("tanh", &["0x1.1275b24c8b4b2p-10"], "0x1.1275abba0bc1bp-10"),
        ("tanh", &["0x1.46fcc14d78d73p-3"], "0x1.443ca2d068665p-3"),
        ("tanh", &["0x1.03c88e0465a5ep+1"], "0x1.ee9f14be571c2p-1"),
        ("asinh", &["0x1.d12ed0af1a239p-26"], "0x1.d12ed0af1a238p-26"),
        ("asinh", &["0x1.d12ed0af1a241p-26"], "0x1.d12ed0af1a240p-26"),
        ("asinh", &["0x1.d12ed0af1a249p-26"], "0x1.d12ed0af1a248p-26"),
        ("asinh", &["0x1.d12ed0af1a251p-26"], "0x1.d12ed0af1a250p-26"),
        ("asinh", &["0x1.ad44f8145a511p+589"], "0x1.9979474620b3dp+8"),
        ("asinh", &["0x1.b3a527168a87fp+104"], "0x1.253f98525f2f7p+6"),
        ("asinh", &["0x1.3cc12df7ed4cep+697"], "0x1.e407993c85e0ep+8"),
        ("asinh", &["0x1.0f1486d8dc82ep+119"], "0x1.4cf08954c367dp+6"),
        ("acosh", &["0x1.099318a43ac80p+0"], "0x1.17326ffc09f68p-2"),
        ("acosh", &["0x1.1c0ef964a4640p+0"], "0x1.db286408b9018p-2"),
        ("acosh", &["0x1.1f0de946abb17p+0"], "0x1.f369e55660740p-2"),
        ("acosh", &["0x1.35344613601b6p+0"], "0x1.44a11154b7850p-1"),
        ("acosh", &["0x1.465fc5bcb1feap+204"], "0x1.1cad136e640bfp+7"),
        ("acosh", &["0x1.2a716cbcbeee8p+23"], "0x1.0c9f795452197p+4"),
        ("acosh", &["0x1.04be653f790a9p+964"], "0x1.4e73e3d08fc9dp+9"),
        ("acosh", &["0x1.58538eec967e3p+183"], "0x1.ff578bb147e0ap+6"),
        ("atanh", &["0x1.d12ed0af1a27fp-27"], "0x1.d12ed0af1a280p-27"),
        ("atanh", &["0x1.d12ed0af1a280p-27"], "0x1.d12ed0af1a281p-27"),
        ("atanh", &["0x1.d12ed0af1a281p-27"], "0x1.d12ed0af1a282p-27"),
        ("atanh", &["0x1.d12ed0af1a282p-27"], "0x1.d12ed0af1a283p-27"),
        ("atanh", &["0x1.b75fd11d6bb4cp-2"], "0x1.d5c647095b82ep-2"),
        ("atanh", &["0x1.184a0aa581920p-24"], "0x1.184a0aa581927p-24"),
        ("atanh", &["0x1.7cc6976083123p-1"], "0x1.eadc8aedee0bep-1"),
        ("atanh", &["0x1.658ec1c0a5b61p-2"], "0x1.7541388ad9b33p-2"),
        (
            "log2",
            &["0x0.0000000010000p-1022"],
            "-0x1.0880000000000p+10",
        ),
        (
            "log2",
            &["0x0.0000000020000p-1022"],
            "-0x1.0840000000000p+10",
        ),
        (
            "log2",
            &["0x0.0000000040000p-1022"],
            "-0x1.0800000000000p+10",
        ),
        (
            "log2",
            &["0x0.0000000080000p-1022"],
            "-0x1.07c0000000000p+10",
        ),
        ("log2", &["0x1.fc098fc35379bp-1"], "-0x1.6f4df6e0a0b6dp-7"),
        ("log2", &["0x1.2744fe8ca03f4p-4"], "-0x1.e5a57174b0456p+1"),
        ("log2", &["0x1.6c97c9e0e7489p-32"], "-0x1.f7d6748da57a7p+4"),
        ("log2", &["0x1.5b9851e7de4a8p-32"], "-0x1.f8f095aa28150p+4"),
        (
            "log10",
            &["0x0.0449fb5c8a96ep-1022"],
            "-0x1.356db4d19cab0p+8",
        ),
        (
            "log10",
            &["0x0.1553c114cff2bp-1022"],
            "-0x1.34bb61e364c88p+8",
        ),
        (
            "log10",
            &["0x0.27ac359afdc72p-1022"],
            "-0x1.34766019516fdp+8",
        ),
        (
            "log10",
            &["0x0.2ae3d19d69e4cp-1022"],
            "-0x1.346db4d19cab0p+8",
        ),
        (
            "log10",
            &["0x1.fb941a7a58951p+1012"],
            "0x1.30f08add10dcdp+8",
        ),
        (
            "log10",
            &["0x1.2d25840798c91p-436"],
            "-0x1.065b699e9e349p+7",
        ),
        ("log10", &["0x1.1155ee14e5f6ep-48"], "-0x1.cd78b24c16c51p+3"),
        (
            "log10",
            &["0x1.4b4f34dc8bc5fp-446"],
            "-0x1.0c4b75f39106fp+7",
        ),
        (
            "log1p",
            &["0x0.0000000000001p-1022"],
            "0x0.0000000000001p-1022",
        ),
        (
            "log1p",
            &["-0x0.0000000000001p-1022"],
            "-0x0.0000000000001p-1022",
        ),
        (
            "log1p",
            &["0x0.fffffffffffffp-1022"],
            "0x0.fffffffffffffp-1022",
        ),
        (
            "log1p",
            &["-0x0.fffffffffffffp-1022"],
            "-0x0.fffffffffffffp-1022",
        ),
        (
            "log1p",
            &["0x1.08c6c21dbf9e9p-762"],
            "0x1.08c6c21dbf9e9p-762",
        ),
        (
            "log1p",
            &["0x1.f91d809950250p-901"],
            "0x1.f91d809950250p-901",
        ),
        ("log1p", &["0x1.d53e670ba68b1p-26"], "0x1.d53e66a022d49p-26"),
        ("log1p", &["0x1.471b32d9f1b47p-7"], "0x1.457bff242fbacp-7"),
        ("expm1", &["0x1.00091a4a0dae5p+2"], "0x1.ad0726fd1ccb3p+5"),
        ("expm1", &["0x1.000bffb8030a7p-39"], "0x1.000bffb8040a8p-39"),
        ("expm1", &["0x1.002881d0cbecfp-12"], "0x1.00308483dbda3p-12"),
        ("expm1", &["0x1.002efbb049433p-37"], "0x1.002efbb04d44bp-37"),
        ("expm1", &["0x1.667b34adef708p-15"], "0x1.667d2aac96b4bp-15"),
        ("expm1", &["0x1.654973f06a5a9p-5"], "0x1.6d315cbba9366p-5"),
        (
            "expm1",
            &["-0x1.975b3a57b1e1ep-30"],
            "-0x1.975b3a52a17b7p-30",
        ),
        ("expm1", &["0x1.f1f9feed03c8ep-30"], "0x1.f1f9fef495222p-30"),
        ("cbrt", &["0x1.00152f57068b7p-1"], "0x1.966b1fb0afe60p-1"),
        ("cbrt", &["0x1.006509cda9886p-1"], "0x1.969559c48c40fp-1"),
        ("cbrt", &["0x1.0082b35be0924p-1"], "0x1.96a5070b791e7p-1"),
        ("cbrt", &["0x1.00ef471361446p-1"], "0x1.96de5f15ae4b2p-1"),
        ("cbrt", &["0x1.cdb350418ecdcp+2"], "0x1.eea7000000000p+0"),
        ("cbrt", &["0x1.3b082606ce4a0p+2"], "0x1.b37a000000000p+0"),
        ("cbrt", &["0x1.a830abb4ca600p+1"], "0x1.7dac000000000p+0"),
        ("cbrt", &["0x1.cef3802cb8c32p+0"], "0x1.37e4800000000p+0"),
    ];
    for (name, args, want) in rows {
        let a: Vec<f64> = args.iter().map(|s| hf(s)).collect();
        let got = if *name == "atan2" {
            crate::atan2(a[0], a[1])
        } else {
            unary(name)(a[0])
        };
        assert_eq!(got.to_bits(), hf(want).to_bits(), "{name}{args:?}");
    }
}
