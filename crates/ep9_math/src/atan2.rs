//! `Math.atan2`: the correctly rounded two-argument arc tangent (H36), a port of CORE-MATH
//! `cr_atan2`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/atan2/atan2.c` and `src/binary64/atan2/tint.h`
//! (commit 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly-rounded atan2 function for two binary64 values.
//! Copyright (c) 2024-2025 Paul Zimmermann and Alexei Sibidanov
//! Copyright (c) 2023-2024 Paul Zimmermann and Alexei Sibidanov (tint.h)
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. ep9 changes: rounding to
//! nearest only (JS); no `errno` and no floating-point flags (the `fenv` calls are gone); every
//! `__builtin_fma` is [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA
//! contraction. The fast path (a 65-point table and a double-double correction) runs first;
//! its rounding tests send the rest to `atan2_accurate`, a separate entry point that works on
//! 192-bit numbers (`Tint`, the C `tint_t`) with a \[29,29\] rational approximation. The C
//! `tint_tod` stops the program on an "unexpected worst case"; CORE-MATH knows of no such
//! input, and this port rounds it like any other value.

use crate::exact::FmaOps;
use crate::hf;

const PI_H: f64 = hf("0x1.921fb54442d18p+1");
const PI_L: f64 = hf("0x1.1a62633145c07p-53");
const PI_OVER2_H: f64 = hf("0x1.921fb54442d18p+0");
const PI_OVER2_L: f64 = hf("0x1.1a62633145c07p-54");

/// A 192-bit number: `(-1)^sgn * (h/2^64 + m/2^128 + l/2^192) * 2^ex`. Either `h = m = l = 0`
/// (zero) or the top bit of `h` is 1.
#[derive(Clone, Copy)]
struct Tint {
    h: u64,
    m: u64,
    l: u64,
    ex: i64,
    sgn: u64,
}

impl Tint {
    /// The C `_h` view: `h` and `m` as one 128-bit number.
    #[inline(always)]
    fn hi(&self) -> u128 {
        (u128::from(self.h) << 64) | u128::from(self.m)
    }

    #[inline(always)]
    fn set_hi(&mut self, v: u128) {
        self.h = (v >> 64) as u64;
        self.m = v as u64;
    }
}

const ZERO: Tint = Tint {
    h: 0,
    m: 0,
    l: 0,
    ex: -1076,
    sgn: 0,
};
const ONE: Tint = Tint {
    h: 0x8000000000000000,
    m: 0,
    l: 0,
    ex: 1,
    sgn: 0,
};
/// pi, with error < 2^-196.96.
const PI: Tint = Tint {
    h: 0xc90fdaa22168c234,
    m: 0xc4c6628b80dc1cd1,
    l: 0x29024e088a67cc74,
    ex: 2,
    sgn: 0,
};
/// pi/2, with error < 2^-197.96.
const PI2: Tint = Tint {
    h: 0xc90fdaa22168c234,
    m: 0xc4c6628b80dc1cd1,
    l: 0x29024e088a67cc74,
    ex: 1,
    sgn: 0,
};

/// `a * b`, with error < 10 ulps or 2^-187.67 * |r|.
#[inline(always)]
fn mul_tint(a: &Tint, b: &Tint) -> Tint {
    let mut r = Tint {
        h: 0,
        m: 0,
        l: 0,
        ex: a.ex + b.ex,
        sgn: a.sgn ^ b.sgn,
    };
    let (ah, am, al) = (u128::from(a.h), u128::from(a.m), u128::from(a.l));
    let (bh, bm, bl) = (u128::from(b.h), u128::from(b.m), u128::from(b.l));
    let rh = ah * bh;
    let rm1 = ah * bm;
    let rm2 = am * bh;
    let mut rl1 = ah * bl;
    let rl2 = am * bm;
    let rl3 = al * bh;
    r.h = (rh >> 64) as u64;
    r.m = rh as u64;
    // accumulate rm1
    r.l = rm1 as u64;
    let mut h = (rm1 >> 64) as u64;
    r.m = r.m.wrapping_add(h);
    r.h += u64::from(r.m < h); // no overflow possible
    // accumulate rm2
    let mut l = rm2 as u64;
    h = (rm2 >> 64) as u64;
    r.l = r.l.wrapping_add(l);
    let mut cm = u64::from(r.l < l); // carry at r.m
    r.m = r.m.wrapping_add(h);
    r.h += u64::from(r.m < h); // no overflow possible
    // accumulate rl1+rl2+rl3
    rl1 = (rl1 >> 64) + (rl2 >> 64) + (rl3 >> 64);
    l = rl1 as u64;
    cm += (rl1 >> 64) as u64;
    r.l = r.l.wrapping_add(l);
    cm += u64::from(r.l < l); // carry at r.m
    // accumulate cm
    r.m = r.m.wrapping_add(cm);
    r.h += u64::from(r.m < cm);
    // If one operand was zero, r.h = r.m = r.l = 0 and this keeps r = 0.
    if r.h >> 63 == 0 {
        r.h = (r.h << 1) | (r.m >> 63);
        r.m = (r.m << 1) | (r.l >> 63);
        r.l <<= 1;
        r.ex -= 1;
    }
    r
}

/// The sign of `|a| - |b|`: -1, 0 or +1.
#[inline(always)]
fn cmp_tint_abs(a: &Tint, b: &Tint) -> i32 {
    if a.h == 0 {
        return if b.h == 0 { 0 } else { -1 };
    }
    if b.h == 0 {
        return 1;
    }
    let c =
        a.ex.cmp(&b.ex)
            .then(a.hi().cmp(&b.hi()))
            .then(a.l.cmp(&b.l));
    c as i32
}

/// The significand of `b` shifted right by `k` bits (`k` as the C unsigned value: a huge `k`
/// gives zero).
#[inline(always)]
fn rshift(b: &Tint, k: u64) -> (u128, u64) {
    let bh = b.hi();
    if k == 0 {
        (bh, b.l)
    } else if k < 64 {
        (bh >> k, ((bh << (64 - k)) as u64) | (b.l >> k))
    } else if k == 64 {
        (bh >> k, bh as u64)
    } else if k < 128 {
        (bh >> k, (bh >> (k - 64)) as u64)
    } else if k < 192 {
        (0, (bh >> (k - 64)) as u64)
    } else {
        (0, 0)
    }
}

/// The 192-bit significand `(h, l)` shifted left by `k` bits.
#[inline(always)]
fn lshift(bh: u128, bl: u64, k: u64) -> (u128, u64) {
    if k == 0 {
        (bh, bl)
    } else if k < 64 {
        ((bh << k) | u128::from(bl >> (64 - k)), bl << k)
    } else if k == 64 {
        ((bh << k) | u128::from(bl), 0)
    } else if k < 128 {
        ((bh << k) | (u128::from(bl) << (k - 64)), 0)
    } else if k < 192 {
        (u128::from(bl) << (k - 64), 0)
    } else {
        (0, 0)
    }
}

/// The C `clz` of a 192-bit significand.
#[inline(always)]
fn clz192(h: u128, l: u64) -> u64 {
    let th = (h >> 64) as u64;
    u64::from(if th != 0 {
        th.leading_zeros()
    } else if h != 0 {
        64 + (h as u64).leading_zeros()
    } else {
        128 + l.leading_zeros()
    })
}

/// `a + b`, with error bounded by 2 ulps.
#[inline(always)]
fn add_tint(a: &Tint, b: &Tint) -> Tint {
    let (a, b) = match cmp_tint_abs(a, b) {
        0 => {
            // |a| = |b|: 0 or 2*a (exact)
            if a.sgn ^ b.sgn != 0 {
                return ZERO;
            }
            let mut r = *a;
            r.ex += 1;
            return r;
        }
        -1 => (b, a),
        _ => (a, b),
    };
    // From now on, |a| > |b| thus a.ex >= b.ex.
    let mut r = ZERO;
    let sh = a.ex.wrapping_sub(b.ex) as u64;
    let (mut th_, mut tl) = rshift(b, sh);
    if a.sgn ^ b.sgn != 0 {
        // opposite signs: a subtraction
        let al = a.l;
        tl = al.wrapping_sub(tl);
        th_ = a.hi().wrapping_sub(th_).wrapping_sub(u128::from(tl > al));
        let ex = clz192(th_, tl);
        if ex <= 1 || sh == 0 {
            let (h, l) = lshift(th_, tl, ex);
            r.set_hi(h);
            r.l = l;
            r.ex = a.ex - ex as i64;
        } else {
            // ex >= 2 and sh >= 1 (only sh = 1): redo the subtraction without dropping the
            // low bits of b; it is exact.
            let (bh, bl) = lshift(b.hi(), b.l, ex - sh);
            let (rh, rl) = lshift(a.hi(), a.l, ex);
            let tl = rl.wrapping_sub(bl);
            let th_ = rh.wrapping_sub(bh).wrapping_sub(u128::from(tl > rl));
            let ex1 = clz192(th_, tl);
            let (h, l) = lshift(th_, tl, ex1);
            r.set_hi(h);
            r.l = l;
            r.ex = a.ex - (ex + ex1) as i64;
        }
    } else {
        // same signs: an addition
        let ah = a.hi();
        let al = a.l;
        let mut rl = al.wrapping_add(tl);
        let cl = u64::from(rl < al);
        let mut rh = ah.wrapping_add(th_);
        let mut ch = u64::from(rh < ah);
        rh = rh.wrapping_add(u128::from(cl));
        ch += u64::from(rh < u128::from(cl));
        if ch != 0 {
            // at most 1
            r.ex = a.ex + 1;
            rl = ((rh << 63) as u64) | (rl >> 1);
            rh = (u128::from(ch) << 127) | (rh >> 1);
        } else {
            r.ex = a.ex;
        }
        r.set_hi(rh);
        r.l = rl;
    }
    r.sgn = a.sgn;
    r
}

/// `x` as a `Tint`, for `x` not NaN, Inf or 0. Exact.
#[inline(always)]
fn tint_fromd(x: f64) -> Tint {
    let u = x.to_bits();
    let ax = u & 0x7fffffffffffffff;
    let e = (ax >> 52) as i64;
    let (ex, h) = if e != 0 {
        (e - 0x3fe, (1u64 << 63) | (ax << 11))
    } else {
        let e = i64::from(ax.leading_zeros());
        (-0x3f2 - e, ax << e)
    };
    Tint {
        h,
        m: 0,
        l: 0,
        ex,
        sgn: u >> 63,
    }
}

/// `x * 2^i` by adding to the exponent field (the C `as_ldexp`); no overflow or underflow.
#[inline(always)]
fn as_ldexp(x: f64, i: i64) -> f64 {
    f64::from_bits(x.to_bits().wrapping_add((i as u64) << 52))
}

/// `a` rounded to a double. `err = 0`: `a` is a double (exact). Otherwise `err` bounds the
/// error on `a.l` in ulps.
#[inline(always)]
fn tint_tod<F: FmaOps>(a: &Tint, err: u64) -> f64 {
    if a.ex >= 1025 {
        // overflow: |a| >= 2^1024
        const B: f64 = hf("0x1p1023");
        return if a.sgn != 0 { -B - B } else { B + B };
    }
    if a.ex <= -1074 {
        // underflow: |a| < 2^-1074
        const TINY: f64 = hf("0x1p-1074");
        let t = if a.sgn != 0 { -TINY } else { TINY };
        if a.ex < -1074 {
            return t * 0.5;
        }
        // 2^-1075 <= |a| < 2^-1074; mid: |a| = 2^-1075
        let mid = a.h == 1u64 << 63 && a.m == 0 && a.l == 0;
        return t * if mid { 0.5 } else { 0.75 };
    }
    let (mut hh, mut mm, mut ll) = (a.h, a.m, a.l);
    let mut ex = a.ex;
    let mut low = hh & 0x7ff; // low 11 bits of a.h
    // The C code stops here when the rounding cannot be decided (a.m all zeros or all ones
    // next to a rounding boundary, within err of it). See the module doc.
    if ex <= -1022 {
        // subnormal
        let sh = (-1021 - ex) as u64; // 1 <= sh <= 52
        ll = (mm << (64 - sh)) | (ll >> sh) | u64::from(ll > 0);
        mm = (hh << (64 - sh)) | (mm >> sh);
        hh >>= sh;
        low = hh & 0x7ff;
        ex += sh as i64;
    }
    let h = (hh >> 11) as f64; // significant bits of a.h
    let l = if err == 0 {
        0.0
    } else if low < 0x400 {
        0.25 // round to zero
    } else if low > 0x400 {
        0.75 // round away
    } else if mm == 0 && ll == 0 {
        0.5 // middle case
    } else {
        0.75 // round away
    };
    let s = if a.sgn != 0 { -1.0 } else { 1.0 };
    let h = F::fma(l, s, s * h) * const { hf("0x1p-52") };
    // now -1021 <= ex <= 1024, thus 2^(ex-1) does not underflow or overflow
    h * as_ldexp(1.0, ex - 1)
}

/// An approximation of `1/a`, for `a` not zero; absolute error < 2^-103.9 for 1 <= r <= 2.
#[inline(always)]
fn inv_tint<F: FmaOps>(a: &Tint) -> Tint {
    let mut ad = tint_tod::<F>(a, 0); // exact
    let subnormal = ad.abs() < const { hf("0x1p-1022") };
    if subnormal {
        ad *= const { hf("0x1p53") };
    }
    let mut r = tint_fromd(1.0 / ad); // about 53 bits
    if subnormal {
        r.ex += 53;
    }
    // Newton: r1 = r0 + r0*(1-a*r0)
    let mut q = mul_tint(a, &r);
    q.sgn = 1 - q.sgn;
    q = add_tint(&ONE, &q);
    q = mul_tint(&r, &q);
    add_tint(&r, &q)
}

/// An approximation of `b/a`, for `a` not zero, with relative error < 2^-185.53.
#[inline(always)]
fn div_tint<F: FmaOps>(b: &Tint, a: &Tint) -> Tint {
    let y = inv_tint::<F>(a);
    let r = mul_tint(&y, b);
    // Karp-Markstein: r1 = r0 + y*(b-a*r0)
    let mut z = mul_tint(a, &r);
    z.sgn = 1 - z.sgn;
    z = add_tint(b, &z);
    z = mul_tint(&y, &z);
    add_tint(&r, &z)
}

/// `b/a` for doubles, the same error bound as [`div_tint`].
#[inline(always)]
fn div_tint_d<F: FmaOps>(b: f64, a: f64) -> Tint {
    div_tint::<F>(&tint_fromd(b), &tint_fromd(a))
}

/// `z*P(z)/Q(z)` approximates atan(z) on \[0,1\] (rminimax, relative error 3.99613e-59).
static P: [Tint; 30] = [
    Tint {
        h: 0x82703f8b53112eca,
        m: 0x71ce6279d467aa45,
        l: 0x19c9b28466b6fc0,
        ex: -10,
        sgn: 0,
    },
    Tint {
        h: 0xb3cf74b427d53e03,
        m: 0xee590b839e751890,
        l: 0xf2570b12bcb2381a,
        ex: -8,
        sgn: 0,
    },
    Tint {
        h: 0xb08498b17b88c39a,
        m: 0x11e37821d784d8fa,
        l: 0x2795b14b84b9d78c,
        ex: -6,
        sgn: 0,
    },
    Tint {
        h: 0xfd4baa7bb52b0d83,
        m: 0xb25425beb0a04dc3,
        l: 0x92008d9fb34b68ff,
        ex: -5,
        sgn: 0,
    },
    Tint {
        h: 0x96541948b85fb386,
        m: 0xe0b815ace425a859,
        l: 0xbeca17c69bb4f7a0,
        ex: -3,
        sgn: 0,
    },
    Tint {
        h: 0x96aaa5d4baffc590,
        m: 0xf23db783b6e4ce55,
        l: 0x2c54e054d3b69f7e,
        ex: -2,
        sgn: 0,
    },
    Tint {
        h: 0x83cd41b5fa862b11,
        m: 0xad8f9d6e3eb9d833,
        l: 0x4ef470d801fa03ca,
        ex: -1,
        sgn: 0,
    },
    Tint {
        h: 0xcbbc7752197cc106,
        m: 0xbc66cfb3edefa6e3,
        l: 0x568b2f9e4feb8910,
        ex: -1,
        sgn: 0,
    },
    Tint {
        h: 0x8cf8f7c585767042,
        m: 0xcd92a5b9ac13d8c8,
        l: 0x43bd4a023dbe04d,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xafdc36d6b36139bf,
        m: 0x11df63474e3859d4,
        l: 0x42b30b2c089a2b6c,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xc6f8fb1463822519,
        m: 0x2f9f04cf55c10377,
        l: 0x1c44675c6d483606,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xcce79f10f754e315,
        m: 0x91ce47edcbc8a4ee,
        l: 0xb2a6f721ac656b42,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xc090384dbd77132e,
        m: 0x1dfb4d9a1d595de7,
        l: 0xe506ca899a394f70,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xa556ac9eee25e867,
        m: 0xd84dafa25141d203,
        l: 0xf1c7c2a5156b3ec3,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0x81bd2677ddfcbc38,
        m: 0x1df33f27dbb60939,
        l: 0xcb4c2c73286a6c29,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xb9ed12169d72e8d9,
        m: 0xc81d95dcef13c227,
        l: 0x974b95796b93c332,
        ex: -1,
        sgn: 0,
    },
    Tint {
        h: 0xf2df6da875b666fa,
        m: 0xc29dd3a69c274c65,
        l: 0x83b9260976c8f0c5,
        ex: -2,
        sgn: 0,
    },
    Tint {
        h: 0x902d34d4db910ed8,
        m: 0x6f3ecddfd6af0a9f,
        l: 0x8e950d7e70a8e9b2,
        ex: -2,
        sgn: 0,
    },
    Tint {
        h: 0x9aefa41961ce1207,
        m: 0xacd0317666753751,
        l: 0x3eaf42a53fcc6534,
        ex: -3,
        sgn: 0,
    },
    Tint {
        h: 0x95db9977c5e6e263,
        m: 0x85367a63fab8a720,
        l: 0xfc2b63bd115998d8,
        ex: -4,
        sgn: 0,
    },
    Tint {
        h: 0x817cb6d8236d5f4d,
        m: 0x70c7815f28909d9b,
        l: 0xc54e97155d12158,
        ex: -5,
        sgn: 0,
    },
    Tint {
        h: 0xc5eec4228c352098,
        m: 0x789e56be1dee9b67,
        l: 0xba25bda81292ccc1,
        ex: -7,
        sgn: 0,
    },
    Tint {
        h: 0x840c38ccfcb5dc64,
        m: 0xb55a44095bf7f312,
        l: 0x6668b8792d4ecd08,
        ex: -8,
        sgn: 0,
    },
    Tint {
        h: 0x97074af7b286b19e,
        m: 0xc20aac21f3130f1d,
        l: 0x369e9fdbdd48528a,
        ex: -10,
        sgn: 0,
    },
    Tint {
        h: 0x9054011c32d4b7a5,
        m: 0xc68d3c5d525f4af4,
        l: 0xeeb3c98615dee188,
        ex: -12,
        sgn: 0,
    },
    Tint {
        h: 0xdddb4a2eaab2bb15,
        m: 0x753761a8f0e9aa8d,
        l: 0x3f20620cd3e075bb,
        ex: -15,
        sgn: 0,
    },
    Tint {
        h: 0x80ef9a1b3e59abe5,
        m: 0x18e6afe0651fdf49,
        l: 0xd1c0329e6a702d9d,
        ex: -17,
        sgn: 0,
    },
    Tint {
        h: 0xca16dd49cd30854c,
        m: 0xbad8ca379401027c,
        l: 0x1fc7f47985486f16,
        ex: -21,
        sgn: 0,
    },
    Tint {
        h: 0xa0ba01b8742ba801,
        m: 0xe6dc14a9516d7f04,
        l: 0x1f5303d878905d2f,
        ex: -25,
        sgn: 0,
    },
    Tint {
        h: 0x9c0440896ce6515c,
        m: 0xcf3470fa7f10ac3,
        l: 0xa9dc80682c111a9d,
        ex: -59,
        sgn: 1,
    },
];

static Q: [Tint; 30] = [
    Tint {
        h: 0x82703f8b53112eca,
        m: 0x71ce6279d467aa45,
        l: 0x19c9b28466b6fc0,
        ex: -10,
        sgn: 0,
    },
    Tint {
        h: 0xb3cf74b427d53e03,
        m: 0xee590b839e751890,
        l: 0xf2570b12bcb238cf,
        ex: -8,
        sgn: 0,
    },
    Tint {
        h: 0xb33c44af0d43c9e8,
        m: 0xf4426f84614701db,
        l: 0x979e49dc5b858442,
        ex: -6,
        sgn: 0,
    },
    Tint {
        h: 0x8264d256f0bf4d61,
        m: 0xee21431cc0f29764,
        l: 0xce0cc1b593ac9952,
        ex: -4,
        sgn: 0,
    },
    Tint {
        h: 0x9d97c4947687b2ee,
        m: 0xef4087c49dde5229,
        l: 0x742bc6e02bfb6df8,
        ex: -3,
        sgn: 0,
    },
    Tint {
        h: 0xa0f888dc7aab949e,
        m: 0xf623f19ac413db23,
        l: 0x854744a164f70ecd,
        ex: -2,
        sgn: 0,
    },
    Tint {
        h: 0x8fd9c7172ab0828e,
        m: 0x1a9f72fadafc3dfb,
        l: 0xaf96a180ea297178,
        ex: -1,
        sgn: 0,
    },
    Tint {
        h: 0xe38168b78a51ee15,
        m: 0x604a47815c8091eb,
        l: 0x9faaa42312e8eb30,
        ex: -1,
        sgn: 0,
    },
    Tint {
        h: 0xa164ccdbb65ee133,
        m: 0xe623433c5e086402,
        l: 0xcb228afd335a245f,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xced0c0cbe44fac2f,
        m: 0xd7c2adfc7c5dda9e,
        l: 0xa22b4a311cf5a5b1,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xf0e6660438c90371,
        m: 0x3dca1a0f58f0d943,
        l: 0x8a92b9de68c4488c,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0x8000000000000000,
        m: 0x0,
        l: 0x0,
        ex: 1,
        sgn: 0,
    },
    Tint {
        h: 0xf8ea68f9ad849103,
        m: 0xb257e4bfc606967f,
        l: 0x818fa2816928c4aa,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xddc64eeee3506234,
        m: 0x47a33ea40a2b22e4,
        l: 0xa6a2c129ace2470a,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xb52ddfba2d37c8e2,
        m: 0x24551a9b19fa064a,
        l: 0x3d78c68b54bc3850,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0x87ac113d26b683a1,
        m: 0x4e97756c38785b0b,
        l: 0xf75f0a4506931ba8,
        ex: 0,
        sgn: 0,
    },
    Tint {
        h: 0xba065070b900f3d5,
        m: 0xe49d8f9f238f1eeb,
        l: 0x6805d48d67f1dcd9,
        ex: -1,
        sgn: 0,
    },
    Tint {
        h: 0xe902d2a4bfc22441,
        m: 0x945cfe6c83a5b828,
        l: 0x519ebb457f8ccca0,
        ex: -2,
        sgn: 0,
    },
    Tint {
        h: 0x84e405c915f7769b,
        m: 0xfaaf34c3c51dd39,
        l: 0xa97fa0ae99382c93,
        ex: -2,
        sgn: 0,
    },
    Tint {
        h: 0x896da46c7e6cebb3,
        m: 0xe0afc0b6db3dbbf5,
        l: 0x61ccff6e78efff4,
        ex: -3,
        sgn: 0,
    },
    Tint {
        h: 0x801a0c1a0dfefc92,
        m: 0xf8e0a97902e08d47,
        l: 0x81898e0b7d8ef0bc,
        ex: -4,
        sgn: 0,
    },
    Tint {
        h: 0xd59d088930df07db,
        m: 0xa8cb00021849fbaf,
        l: 0xf80cc91aa0bddf89,
        ex: -6,
        sgn: 0,
    },
    Tint {
        h: 0x9db56a6984b77231,
        m: 0xcf7c2074e0480959,
        l: 0x3316ecaa6aa4604e,
        ex: -7,
        sgn: 0,
    },
    Tint {
        h: 0xcb7450d9d62d2e68,
        m: 0x9787b4265977d004,
        l: 0xba615cee00a841d7,
        ex: -9,
        sgn: 0,
    },
    Tint {
        h: 0xe12a52e02b64b23e,
        m: 0x97c9139a8ced716c,
        l: 0x70a176ad2b5f16d2,
        ex: -11,
        sgn: 0,
    },
    Tint {
        h: 0xd057b6d0900ed30d,
        m: 0x5e04bb17eb879a5e,
        l: 0x23ed135b60bea070,
        ex: -13,
        sgn: 0,
    },
    Tint {
        h: 0x9b1f1ca22fef7ae5,
        m: 0x19a687f0a4073eee,
        l: 0x5a9ccbc8c14578b,
        ex: -15,
        sgn: 0,
    },
    Tint {
        h: 0xaeba710f053aac3b,
        m: 0x30ea5d6fb116f775,
        l: 0x8a9bcfef5698f8f1,
        ex: -18,
        sgn: 0,
    },
    Tint {
        h: 0x84b9ada076cd007d,
        m: 0x13e55006073f76b1,
        l: 0x4cf4259968228f3e,
        ex: -21,
        sgn: 0,
    },
    Tint {
        h: 0xcca4b9046786937d,
        m: 0x233c1e691c127c1f,
        l: 0xf493e0b7a3de459a,
        ex: -26,
        sgn: 0,
    },
];

#[inline(always)]
fn atan2_accurate_with<F: FmaOps>(y: f64, x: f64) -> f64 {
    // First the case when t = y/x is small and exact and x > 0: for |t| <= 0x1.d12ed0af1a27fp-27,
    // atan(t) rounds to t (to nearest).
    let t = y / x;
    if t == 0.0 {
        if x > 0.0 {
            return t;
        }
        return if y > 0.0 { PI_H + PI_L } else { -PI_H - PI_L };
    }
    let corr = F::fma(t, x, -y);
    if corr == 0.0 && x > 0.0 && t.abs() <= const { hf("0x1.d12ed0af1a27fp-27") } {
        // t is exact. If |y| >= 2^-915, t*x - y does not underflow.
        if y.abs() >= const { hf("0x1p-915") } {
            return F::fma(t, const { hf("-0x1p-54") }, t);
        }
        // Now |y| < 2^-969 (C comment); scale y and t by 2^105 so t*x - y does not underflow.
        let corr = F::fma(t * const { hf("0x1p105") }, x, -y * const { hf("0x1p105") });
        if corr == 0.0 {
            return F::fma(t, const { hf("-0x1p-54") }, t);
        }
    }

    let inv = y.abs() > x.abs();
    let mut z = if inv {
        div_tint_d::<F>(x, y)
    } else {
        div_tint_d::<F>(y, x)
    };

    // When |y/x| < 2^-96 and x > 0, atan(y/x) rounds as y/x moved a little towards zero
    // (2 is taken from z.l). The C borrow tests are kept as written.
    if !inv && x > 0.0 && z.ex <= -96 {
        z.l = z.l.wrapping_sub(2);
        z.m = z.m.wrapping_sub(u64::from(z.l < 2));
        z.h = z.h.wrapping_sub(u64::from(z.m < 1));
        return tint_tod::<F>(&z, 1);
    }

    // Below, y/x reads x/y when inv. The rational approximation is for z > 0 only.
    let sz = z.sgn;
    z.sgn = 0;
    let mut p = P[29];
    let mut q = Q[29];
    for i in (0..29).rev() {
        p = mul_tint(&p, &z);
        q = mul_tint(&q, &z);
        p = add_tint(&p, &P[i]);
        q = add_tint(&q, &Q[i]);
    }
    p = mul_tint(&p, &z);
    z = div_tint::<F>(&p, &q);
    // z = atan(y/x) * (1 + eps) with |eps| < 2^-182.63: at most 662 ulps.
    let mut err = 662;
    z.sgn = sz;
    // Now z approximates atan(y/x) (or atan(x/y) when inv), with -pi/4 < z < pi/4.
    if inv {
        if z.sgn == 0 {
            // 0 < atan(x/y) < pi/4: pi/2 - atan(x/y)
            z.sgn = 1;
            z = add_tint(&PI2, &z);
        } else {
            // -pi/4 < atan(x/y) < 0: -pi/2 - atan(x/y)
            z = add_tint(&z, &PI2);
            z.sgn = 1;
        }
        err = 524;
    }
    // now -pi/2 < z < pi/2; for x < 0 go to the opposite quadrant
    if x < 0.0 {
        if z.sgn == 0 {
            // 1st quadrant -> 3rd quadrant (subtract pi)
            z.sgn = 1;
            z = add_tint(&PI, &z);
            z.sgn = 1;
        } else {
            // 4th quadrant -> 2nd quadrant (add pi)
            z = add_tint(&z, &PI);
        }
        err = 266;
    }
    tint_tod::<F>(&z, err)
}

multiversion! {
    fn atan2_accurate(y: f64, x: f64) -> f64 = atan2_accurate_with;
}

#[inline(always)]
fn fasttwosum(x: f64, y: f64) -> (f64, f64) {
    let s = x + y;
    let z = s - x;
    (s, y - z)
}

#[inline(always)]
fn fastsum(xh: f64, xl: f64, yh: f64, yl: f64) -> (f64, f64) {
    let (sh, sl) = fasttwosum(xh, yh);
    (sh, (xl + yl) + sl)
}

/// The zero, NaN and Inf inputs.
#[inline(never)]
fn as_atan2_special(y0: f64, x0: f64) -> f64 {
    let iy = y0.to_bits();
    let ix = x0.to_bits();
    let aiy = iy << 1;
    let aix = ix << 1;
    if aiy >= 0x7ff << 53 || aix >= 0x7ff << 53 {
        // NaN or Inf
        if aiy > 0x7ff << 53 || aix > 0x7ff << 53 {
            return y0 + x0;
        }
        // Neither is NaN; at least one is +/-Inf.
        if aiy == 0x7ff << 53 && aix == 0x7ff << 53 {
            // atan2(+/-Inf,-Inf) = +/-3pi/4, atan2(+/-Inf,+Inf) = +/-pi/4
            const FINF: [[f64; 2]; 2] = [
                [hf("0x1p-55"), hf("0x1.921fb54442d18p-1")],
                [hf("0x1p-54"), hf("0x1.2d97c7f3321d2p+1")],
            ];
            let f = FINF[(ix >> 63) as usize];
            return f[1].copysign(y0) + f[0].copysign(y0);
        }
        // only one of y and x is +/-Inf
        if aix == 0x7ff << 53 {
            if x0 < 0.0 {
                return PI_H.copysign(y0) + PI_L.copysign(y0);
            }
            // atan2(+/-y,+Inf) = +/-0 for finite y
            return 0.0f64.copysign(y0);
        }
        // atan2(+/-Inf,x) = +/-pi/2 for finite x
        return PI_OVER2_H.copysign(y0) + PI_OVER2_L.copysign(y0);
    }
    if aiy == 0 || aix == 0 {
        if aiy == 0 && aix == 0 {
            if ix == 0 {
                // atan2(+/-0, +0) = +/-0
                return y0;
            }
            // atan2(+/-0, -0) = +/-pi
            return if iy == 0 { PI_H + PI_L } else { -PI_H - PI_L };
        }
        if aiy == 0 {
            // atan2(+/-0,x) = +/-0 for x>0, +/-pi for x<0
            if x0 > 0.0 {
                return y0 / x0;
            }
            return if iy >> 63 == 0 {
                PI_H + PI_L
            } else {
                -PI_H - PI_L
            };
        }
        // only x is zero: atan2(y,+/-0) = +/-pi/2
        return if y0 > 0.0 {
            PI_OVER2_H + PI_OVER2_L
        } else {
            -PI_OVER2_H - PI_OVER2_L
        };
    }
    0.0
}

/// 65 entries `j/64`.
static T2: [f64; 65] = [
    hf("0x0p+0"),
    hf("0x1p-6"),
    hf("0x1p-5"),
    hf("0x1.8p-5"),
    hf("0x1p-4"),
    hf("0x1.4p-4"),
    hf("0x1.8p-4"),
    hf("0x1.cp-4"),
    hf("0x1p-3"),
    hf("0x1.2p-3"),
    hf("0x1.4p-3"),
    hf("0x1.6p-3"),
    hf("0x1.8p-3"),
    hf("0x1.ap-3"),
    hf("0x1.cp-3"),
    hf("0x1.ep-3"),
    hf("0x1p-2"),
    hf("0x1.1p-2"),
    hf("0x1.2p-2"),
    hf("0x1.3p-2"),
    hf("0x1.4p-2"),
    hf("0x1.5p-2"),
    hf("0x1.6p-2"),
    hf("0x1.7p-2"),
    hf("0x1.8p-2"),
    hf("0x1.9p-2"),
    hf("0x1.ap-2"),
    hf("0x1.bp-2"),
    hf("0x1.cp-2"),
    hf("0x1.dp-2"),
    hf("0x1.ep-2"),
    hf("0x1.fp-2"),
    hf("0x1p-1"),
    hf("0x1.08p-1"),
    hf("0x1.1p-1"),
    hf("0x1.18p-1"),
    hf("0x1.2p-1"),
    hf("0x1.28p-1"),
    hf("0x1.3p-1"),
    hf("0x1.38p-1"),
    hf("0x1.4p-1"),
    hf("0x1.48p-1"),
    hf("0x1.5p-1"),
    hf("0x1.58p-1"),
    hf("0x1.6p-1"),
    hf("0x1.68p-1"),
    hf("0x1.7p-1"),
    hf("0x1.78p-1"),
    hf("0x1.8p-1"),
    hf("0x1.88p-1"),
    hf("0x1.9p-1"),
    hf("0x1.98p-1"),
    hf("0x1.ap-1"),
    hf("0x1.a8p-1"),
    hf("0x1.bp-1"),
    hf("0x1.b8p-1"),
    hf("0x1.cp-1"),
    hf("0x1.c8p-1"),
    hf("0x1.dp-1"),
    hf("0x1.d8p-1"),
    hf("0x1.ep-1"),
    hf("0x1.e8p-1"),
    hf("0x1.fp-1"),
    hf("0x1.f8p-1"),
    hf("0x1p+0"),
];

/// 65 entries `{low, high}` of atan(j/64).
static F2: [[f64; 2]; 65] = [
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("-0x1.95220c39d4dffp-53"), hf("0x1.fff555bbb73p-7")],
    [hf("0x1.2542779d776dep-53"), hf("0x1.ffd55bba976p-6")],
    [hf("-0x1.6061bbe3de53cp-53"), hf("0x1.7fb818430da4p-5")],
    [hf("-0x1.639269b0da47ep-53"), hf("0x1.ff55bb72cfep-5")],
    [hf("-0x1.4a7663af440f7p-55"), hf("0x1.3f59f0e7c55ap-4")],
    [hf("0x1.d1824d59f9e13p-53"), hf("0x1.7ee182602f1p-4")],
    [hf("0x1.bef71e5340b31p-55"), hf("0x1.be39ebe6f07cp-4")],
    [hf("0x1.b8cb225e627dp-53"), hf("0x1.fd5ba9aac2f6p-4")],
    [hf("0x1.b92de9bac94c2p-53"), hf("0x1.1e1fafb04372p-3")],
    [hf("-0x1.d3cb89e62dafdp-54"), hf("0x1.3d6eee8c6627p-3")],
    [hf("-0x1.882a55960087ap-53"), hf("0x1.5c9811e3ec27p-3")],
    [hf("0x1.1347b0b4f881dp-54"), hf("0x1.7b97b4bce5bp-3")],
    [hf("0x1.873d8079ed0d2p-53"), hf("0x1.9a6a8e96c862p-3")],
    [hf("0x1.022f621a5c1cbp-54"), hf("0x1.b90d7529260ap-3")],
    [hf("0x1.9c648d1534598p-53"), hf("0x1.d77d5df20573p-3")],
    [hf("-0x1.4ea9238610a08p-54"), hf("0x1.f5b75f92c80ep-3")],
    [hf("0x1.2c5c8e721970dp-53"), hf("0x1.09dc597d8636p-2")],
    [hf("0x1.30ca4748b1bf9p-57"), hf("0x1.18bf5a30bf178p-2")],
    [hf("-0x1.20ef9ba6dbf9p-53"), hf("0x1.278372057ef48p-2")],
    [hf("0x1.e69c5abb498d2p-53"), hf("0x1.362773707ebc8p-2")],
    [hf("0x1.a8a86f0ea9311p-54"), hf("0x1.44aa436c2af08p-2")],
    [hf("0x1.db5336feef7fp-54"), hf("0x1.530ad9951cd48p-2")],
    [hf("0x1.9636a3aa3b84p-54"), hf("0x1.614840309cfep-2")],
    [hf("0x1.1ce2a8c848b74p-55"), hf("0x1.6f61941e4defp-2")],
    [hf("-0x1.4b1bbd1ea6db3p-55"), hf("0x1.7d5604b63b3f8p-2")],
    [hf("-0x1.4925e8b916e0bp-53"), hf("0x1.8b24d394a1b28p-2")],
    [hf("0x1.9e6c988fd0a77p-56"), hf("0x1.98cd5454d6b18p-2")],
    [hf("-0x1.a49bd836a17p-53"), hf("0x1.a64eec3cc24p-2")],
    [hf("-0x1.ca3cf09c6b5f8p-53"), hf("0x1.b3a911da65c7p-2")],
    [hf("-0x1.cc1ce70934c34p-56"), hf("0x1.c0db4c94ec9fp-2")],
    [hf("0x1.2e982ddf3872ap-55"), hf("0x1.cde53432c135p-2")],
    [hf("-0x1.2ea406ee84d0fp-55"), hf("0x1.dac670561bb5p-2")],
    [hf("-0x1.de35847c81979p-53"), hf("0x1.e77eb7f175a38p-2")],
    [hf("-0x1.a3992dc382a23p-57"), hf("0x1.f40dd0b541418p-2")],
    [hf("-0x1.b32c949c9d593p-55"), hf("0x1.0039c73c1a40cp-1")],
    [hf("-0x1.d5b495f6349e6p-56"), hf("0x1.0657e94db30dp-1")],
    [hf("-0x1.f34582f6255fep-53"), hf("0x1.0c6145b5b43dcp-1")],
    [hf("0x1.ed42511e3f11dp-54"), hf("0x1.1255d9bfbd2a8p-1")],
    [hf("-0x1.1cef189ff9e7fp-54"), hf("0x1.1835a88be7c14p-1")],
    [hf("-0x1.928df287a668fp-58"), hf("0x1.1e00babdefeb4p-1")],
    [hf("-0x1.e3bde360c7ddbp-53"), hf("0x1.23b71e2cc9e6cp-1")],
    [hf("0x1.bd86313ce4fdep-54"), hf("0x1.2958e59308e3p-1")],
    [hf("-0x1.8e8a85803cc1dp-53"), hf("0x1.2ee628406cbccp-1")],
    [hf("-0x1.77ef7641c777fp-54"), hf("0x1.345f01cce37bcp-1")],
    [hf("0x1.b73ef3389d02fp-53"), hf("0x1.39c391cd41718p-1")],
    [hf("0x1.ecf8b492644fp-56"), hf("0x1.3f13fb89e96f4p-1")],
    [hf("0x1.c1125fd3810c7p-53"), hf("0x1.445065b795b54p-1")],
    [hf("0x1.2483350fe548bp-53"), hf("0x1.4978fa3269eep-1")],
    [hf("0x1.4a33dbeb3796cp-55"), hf("0x1.4e8de5bb6ec04p-1")],
    [hf("-0x1.46edd2af69483p-53"), hf("0x1.538f57b89062p-1")],
    [hf("-0x1.2bcb93b18b52ap-53"), hf("0x1.587d81f732fbcp-1")],
    [hf("0x1.0028e4bc5e7cap-57"), hf("0x1.5d58987169b18p-1")],
    [hf("0x1.ed487acaf1174p-53"), hf("0x1.6220d115d7b8cp-1")],
    [hf("-0x1.2dd4dfd7d1777p-53"), hf("0x1.66d663923e088p-1")],
    [hf("0x1.2bfe3cf3b9d79p-54"), hf("0x1.6b798920b3d98p-1")],
    [hf("-0x1.8c34d25aadef6p-56"), hf("0x1.700a7c5784634p-1")],
    [hf("-0x1.f426acf4d3bdbp-54"), hf("0x1.748978fba8e1p-1")],
    [hf("-0x1.afe57dd9ff23p-53"), hf("0x1.78f6bbd5d316p-1")],
    [hf("-0x1.54fbef0e862abp-54"), hf("0x1.7d528289fa094p-1")],
    [hf("0x1.90227758b11bap-54"), hf("0x1.819d0b7158a4cp-1")],
    [hf("0x1.16b66e7fc8b8cp-53"), hf("0x1.85d69576cc2c4p-1")],
    [hf("-0x1.55b9a5e177a1bp-55"), hf("0x1.89ff5ff57f1f8p-1")],
    [hf("0x1.c27cfaa9f7a14p-53"), hf("0x1.8e17aa99cc05cp-1")],
    [hf("0x1.1a62633145c07p-55"), hf("0x1.921fb54442d18p-1")],
];

/// The quadrant offsets, indexed by `sx<<2 | sy<<1 | GT`.
static O: [[f64; 2]; 8] = [
    [0.0, 0.0],
    [hf("0x1.921fb54442d18p+0"), hf("0x1.1a62633145c07p-54")],
    [0.0, 0.0],
    [hf("-0x1.921fb54442d18p+0"), hf("-0x1.1a62633145c07p-54")],
    [hf("0x1.921fb54442d18p+1"), hf("0x1.1a62633145c07p-53")],
    [hf("0x1.921fb54442d18p+0"), hf("0x1.1a62633145c07p-54")],
    [hf("-0x1.921fb54442d18p+1"), hf("-0x1.1a62633145c07p-53")],
    [hf("-0x1.921fb54442d18p+0"), hf("-0x1.1a62633145c07p-54")],
];

#[inline(always)]
fn atan2_with<F: FmaOps>(y0: f64, x0: f64) -> f64 {
    const MASK: u64 = 0x7fffffffffffffff;
    let iy = y0.to_bits();
    let ix = x0.to_bits();
    let aiy = iy & MASK;
    if aiy == 0 || aiy >= 0x7ff << 52 {
        return as_atan2_special(y0, x0);
    }
    let aix = ix & MASK;
    if aix == 0 || aix >= 0x7ff << 52 {
        return as_atan2_special(y0, x0);
    }
    let ax = x0.abs();
    let ay = y0.abs();
    let mut x = ax.max(ay);
    let mut y = ax.min(ay);
    let sy = iy >> 63;
    let sx = ix >> 63;
    let gt = u64::from(aix < aiy);
    let dxy = aix.wrapping_sub(aiy) ^ gt.wrapping_neg();
    if dxy >= 53 << 52 {
        return atan2_accurate(y0, x0);
    }
    // copysign(1, asgn[GT^sx^sy]), asgn = {0.0, -0.0}
    let sg = if (gt ^ sx ^ sy) != 0 { -1.0 } else { 1.0 };
    let kw = (sx << 2 | sy << 1 | gt) as usize;
    let jj = (y / x + const { 2.0 + 1.0 / 128.0 }).to_bits();
    let jt = ((jj >> (52 - 7)) & 127) as usize;
    let mut fh = F2[jt][1] * sg;
    let mut fl = F2[jt][0] * sg;
    fh += O[kw][0];
    fl += O[kw][1];
    if x < const { hf("0x1p-920") } {
        x *= const { hf("0x1p920") };
        y *= const { hf("0x1p920") };
    }
    if x > const { hf("0x1p1022") } && jt != 0 {
        x *= 0.5;
        y *= 0.5;
    }
    let t0 = T2[jt];
    let zn = F::fma(-t0, x, y);
    let zd = F::fma(t0, y, x);
    let mut z = zn / zd;
    const B: [f64; 3] = [
        hf("-0x1.55555555554d2p-2"),
        hf("0x1.999999860e1cap-3"),
        hf("-0x1.248ad469844a1p-3"),
    ];
    let mut z2 = z * z;
    z *= sg;
    let dz = (z * z2) * (B[0] + z2 * (B[1] + z2 * B[2]));
    // From 1.1e10 random pairs in the round-up mode, plus 2.5%.
    let mut eps = z.abs() * const { hf("0x1.051p-51") } + const { hf("0x1p-90") };
    let (rh, zr) = fasttwosum(fh, z);
    let rl = (fl + dz) + zr;
    let lb = rh + (rl - eps);
    let ub = rh + (rl + eps);
    if lb == ub {
        return ub;
    }
    let dh = y * t0;
    let mut dl = F::fma(y, t0, -dh);
    let (dh, e) = fasttwosum(x, dh);
    let rdh = 1.0 / dh;
    dl += e;
    let nh = x * t0;
    let nl = F::fma(x, t0, -nh);
    let dt = y - nh;
    let y1 = dt + nh;
    let (nh, nl) = if y1 == y {
        fasttwosum(dt, -nl)
    } else {
        fasttwosum(dt, (y - y1) - nl)
    };
    let mut zh = nh * rdh;
    z2 = zh * zh;
    let mut zl = rdh * (F::fma(dh, -zh, nh) + (nl - (nh * rdh) * dl));
    const B2: [f64; 4] = [
        hf("-0x1.5555555555555p-2"),
        hf("0x1.999999999755ep-3"),
        hf("-0x1.24924883596f8p-3"),
        hf("0x1.c6f7d73531bc2p-4"),
    ];
    zl += zh * z2 * ((B2[0] + z2 * B2[1]) + (z2 * z2) * (B2[2] + z2 * B2[3]));
    zh *= sg;
    zl *= sg;
    eps = const { hf("0x1.4p-50") } * (zh.abs() * z2 + const { hf("0x1p-51") });
    let (fh, fl) = fastsum(fh, fl, zh, zl);
    let lb = fh + (fl - eps);
    let ub = fh + (fl + eps);
    if lb != ub {
        return atan2_accurate(y0, x0);
    }
    ub
}

multiversion! {
    /// The two-argument arc tangent `atan2(y, x)`, correctly rounded (round to nearest, ties
    /// to even): CORE-MATH `cr_atan2`. The same bits on every target. `Math.atan2`
    /// (§21.3.2.8) has the C99 special cases: NaN in gives NaN; `atan2(±0, +0) = ±0`,
    /// `atan2(±0, -0) = ±π`, `atan2(±∞, ±∞) = ±π/4` or `±3π/4`, and so on.
    pub fn atan2(y: f64, x: f64) -> f64 = atan2_with;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI as PI_F};

    #[test]
    fn atan2_special_values() {
        assert!(atan2(f64::NAN, 1.0).is_nan());
        assert!(atan2(1.0, f64::NAN).is_nan());
        assert_eq!(atan2(0.0, 0.0).to_bits(), 0);
        assert_eq!(atan2(-0.0, 0.0).to_bits(), 1 << 63);
        assert_eq!(atan2(0.0, -0.0), PI_F);
        assert_eq!(atan2(-0.0, -0.0), -PI_F);
        assert_eq!(atan2(0.0, -1.0), PI_F);
        assert_eq!(atan2(-0.0, -1.0), -PI_F);
        assert_eq!(atan2(-0.0, 1.0).to_bits(), 1 << 63);
        assert_eq!(atan2(1.0, 0.0), FRAC_PI_2);
        assert_eq!(atan2(-1.0, -0.0), -FRAC_PI_2);
        assert_eq!(atan2(f64::INFINITY, f64::INFINITY), FRAC_PI_4);
        assert_eq!(
            atan2(f64::NEG_INFINITY, f64::NEG_INFINITY),
            -3.0 * FRAC_PI_4
        );
        assert_eq!(atan2(1.0, f64::NEG_INFINITY), PI_F);
        assert_eq!(atan2(-1.0, f64::INFINITY).to_bits(), 1 << 63);
        assert_eq!(atan2(f64::INFINITY, 5.0), FRAC_PI_2);
        assert_eq!(atan2(1.0, 1.0), FRAC_PI_4);
        assert_eq!(atan2(1.0, -1.0), 3.0 * FRAC_PI_4);
    }

    /// The 192-bit path agrees with the fast path where both give an answer.
    #[test]
    fn atan2_accurate_agrees() {
        for &(y, x) in &[
            (1.0, 3.0),
            (-2.5, 7.0),
            (3.0, -0.1),
            (-1e-300, -2.0),
            (1e300, 1e-10),
        ] {
            assert_eq!(
                atan2_accurate(y, x).to_bits(),
                atan2(y, x).to_bits(),
                "{y} {x}"
            );
        }
    }
}
