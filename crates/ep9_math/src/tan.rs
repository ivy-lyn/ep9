//! `Math.tan`: the correctly rounded tangent (H36), a port of CORE-MATH `cr_tan`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/tan/tan.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded tangent function for binary64 values.
//! Copyright (c) 2022-2025 Paul Zimmermann and Tom Hubrecht
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The parts that `tan.c` shares with
//! `cos.c` are in `trig.rs`. ep9 changes: rounding to nearest only (JS); no `errno` and no
//! floating-point flags; every `__builtin_fma` is [`FmaOps::fma`] (hardware or exact software
//! FMA, the same bits); no FMA contraction. |x| <= 0x1.d12ed0af1a27ep-27 is
//! `fma(x, 2^-54, x)`; otherwise the double-double path `tan_fast` (sin2pi/cos2pi from the SC
//! table, then a division) runs, and its rounding test sends the rest to the 128-bit path
//! `tan_accurate`, a separate entry point.

use crate::exact::FmaOps;
use crate::hf;
use crate::trig::{
    C, Dint, MAGIC, S, SC, T, a_mul, add_dint, dint_fromd, dint_tod, eval_pc, eval_pc_fast,
    eval_ps, eval_ps_fast, fast_two_sum, mul_dint, mul_dint_21, reduce, reduce2, s_mul, set_dd,
};

/// 1 as a `Dint`.
const ONE: Dint = Dint {
    hi: 0x8000000000000000,
    lo: 0,
    ex: 1,
    sgn: 0,
};

/// For 0 <= i < 256, `TINV[i] = floor(2^127/(2^63+i*2^55+2^55-1))`.
static TINV: [u64; 256] = [
    0xff00ff00ff00ff02,
    0xfe03f80fe03f80ff,
    0xfd08e5500fd08e56,
    0xfc0fc0fc0fc0fc11,
    0xfb18856506ddaba7,
    0xfa232cf252138ac1,
    0xf92fb2211855a867,
    0xf83e0f83e0f83e11,
    0xf74e3fc22c700f76,
    0xf6603d980f6603db,
    0xf57403d5d00f5742,
    0xf4898d5f85bb3952,
    0xf3a0d52cba872338,
    0xf2b9d6480f2b9d66,
    0xf1d48bcee0d399fc,
    0xf0f0f0f0f0f0f0f2,
    0xf00f00f00f00f010,
    0xef2eb71fc4345239,
    0xee500ee500ee5010,
    0xed7303b5cc0ed731,
    0xec979118f3fc4da3,
    0xebbdb2a5c1619c8d,
    0xeae56403ab959010,
    0xea0ea0ea0ea0ea10,
    0xe939651fe2d8d35d,
    0xe865ac7b7603a198,
    0xe79372e225fe30da,
    0xe6c2b4481cd8568a,
    0xe5f36cb00e5f36cc,
    0xe525982af70c880f,
    0xe45932d7dc52100f,
    0xe38e38e38e38e38f,
    0xe2c4a6886a4c2e11,
    0xe1fc780e1fc780e3,
    0xe135a9c97500e137,
    0xe070381c0e070383,
    0xdfac1f74346c5760,
    0xdee95c4ca037ba58,
    0xde27eb2c41f3d9d2,
    0xdd67c8a60dd67c8b,
    0xdca8f158c7f91ab9,
    0xdbeb61eed19c5959,
    0xdb2f171df770291a,
    0xda740da740da740f,
    0xd9ba4256c0366e92,
    0xd901b2036406c80f,
    0xd84a598ec9151f44,
    0xd79435e50d79435f,
    0xd6df43fca482f00e,
    0xd62b80d62b80d62c,
    0xd578e97c3f5fe552,
    0xd4c77b03531dec0e,
    0xd4173289870ac52e,
    0xd3680d3680d3680e,
    0xd2ba083b445250ac,
    0xd20d20d20d20d20e,
    0xd161543e28e50275,
    0xd0b69fcbd2580d0c,
    0xd00d00d00d00d00e,
    0xcf6474a8819ec8ea,
    0xcebcf8bb5b4169cc,
    0xce168a7725080ce2,
    0xcd712752a886d243,
    0xccccccccccccccce,
    0xcc29786c7607f99f,
    0xcb8727c065c393e1,
    0xcae5d85f1bbd6c96,
    0xca4587e6b74f032a,
    0xc9a633fcd967300d,
    0xc907da4e871146ad,
    0xc86a78900c86a78a,
    0xc7ce0c7ce0c7ce0d,
    0xc73293d789b9f839,
    0xc6980c6980c6980d,
    0xc5fe740317f9d00d,
    0xc565c87b5f9d4d1c,
    0xc4ce07b00c4ce07c,
    0xc4372f855d824ca6,
    0xc3a13de60495c774,
    0xc30c30c30c30c30d,
    0xc2780613c0309e02,
    0xc1e4bbd595f6e948,
    0xc152500c152500c2,
    0xc0c0c0c0c0c0c0c1,
    0xc0300c0300c0300d,
    0xbfa02fe80bfa02ff,
    0xbf112a8ad278e8de,
    0xbe82fa0be82fa0bf,
    0xbdf59c91700bdf5a,
    0xbd69104707661aa3,
    0xbcdd535db1cc5b7c,
    0xbc52640bc52640bd,
    0xbbc8408cd63069a1,
    0xbb3ee721a54d880c,
    0xbab656100bab6562,
    0xba2e8ba2e8ba2e8c,
    0xb9a7862a0ff46588,
    0xb92143fa36f5e02f,
    0xb89bc36ce3e0453b,
    0xb81702e05c0b8171,
    0xb79300b79300b794,
    0xb70fbb5a19be3659,
    0xb68d31340e4307d9,
    0xb60b60b60b60b60c,
    0xb58a485518d1e7e4,
    0xb509e68a9b948220,
    0xb48a39d44685fe97,
    0xb40b40b40b40b40c,
    0xb38cf9b00b38cf9b,
    0xb30f63528917c80c,
    0xb2927c29da5519d0,
    0xb21642c8590b2165,
    0xb19ab5c45606f00c,
    0xb11fd3b80b11fd3c,
    0xb0a59b418d749d54,
    0xb02c0b02c0b02c0b,
    0xafb321a1496fdf0f,
    0xaf3addc680af3ade,
    0xaec33e1f671529a5,
    0xae4c415c9882b931,
    0xadd5e6323fd48a87,
    0xad602b580ad602b6,
    0xaceb0f891e6551bc,
    0xac7691840ac76919,
    0xac02b00ac02b00ac,
    0xab8f69e28359cd12,
    0xab1cbdd3e2970f60,
    0xaaaaaaaaaaaaaaab,
    0xaa392f35dc17f00b,
    0xa9c84a47a07f5638,
    0xa957fab5402a55ff,
    0xa8e83f5717c0a8e9,
    0xa87917088e262b70,
    0xa80a80a80a80a80b,
    0xa79c7b16ea64d422,
    0xa72f05397829cbc2,
    0xa6c21df6e1625c80,
    0xa655c4392d7b73a8,
    0xa5e9f6ed347f0721,
    0xa57eb50295fad40b,
    0xa513fd6bb00a5140,
    0xa4a9cf1d96833751,
    0xa44029100a440291,
    0xa3d70a3d70a3d70b,
    0xa36e71a2cb033129,
    0xa3065e3fae7cd0e0,
    0xa29ecf163bb6500a,
    0xa237c32b16cfd772,
    0xa1d139855f7268ee,
    0xa16b312ea8fc377d,
    0xa105a932f2ca891f,
    0xa0a0a0a0a0a0a0a1,
    0xa03c1688732b3032,
    0x9fd809fd809fd80a,
    0x9f747a152d7836d0,
    0x9f1165e7254813e2,
    0x9eaecc8d53ae2ddf,
    0x9e4cad23dd5f3a20,
    0x9deb06c9194aa416,
    0x9d89d89d89d89d8a,
    0x9d2921c3d6411308,
    0x9cc8e160c3fb19b9,
    0x9c69169b30446dfa,
    0x9c09c09c09c09c0a,
    0x9baade8e4a2f6e10,
    0x9b4c6f9ef03a3caa,
    0x9aee72fcf957c10f,
    0x9a90e7d95bc609a9,
    0x9a33cd67009a33ce,
    0x99d722dabde58f06,
    0x997ae76b50efd00a,
    0x991f1a515885fb37,
    0x98c3bac74f5db00a,
    0x9868c809868c8099,
    0x980e4156201301c8,
    0x97b425ed097b425f,
    0x975a750ff68a58af,
    0x97012e025c04b80a,
    0x96a850096a850097,
    0x964fda6c0964fda7,
    0x95f7cc72d1b887e9,
    0x95a02568095a0257,
    0x9548e4979e0829fd,
    0x94f2094f2094f209,
    0x949b92ddc02526e5,
    0x9445809445809446,
    0x93efd1c50e726b7c,
    0x939a85c40939a85c,
    0x93459be6b009345a,
    0x92f113840497889c,
    0x929cebf48bbd90e5,
    0x9249249249249249,
    0x91f5bcb8bb02d9cd,
    0x91a2b3c4d5e6f809,
    0x9150091500915009,
    0x90fdbc090fdbc091,
    0x90abcc0242af3009,
    0x905a38633e06c43b,
    0x9009009009009009,
    0x8fb823ee08fb823f,
    0x8f67a1e3fdc26179,
    0x8f1779d9fdc3a219,
    0x8ec7ab397255e41d,
    0x8e78356d1408e783,
    0x8e2917e0e702c6ce,
    0x8dda520237694809,
    0x8d8be33f95d71590,
    0x8d3dcb08d3dcb08d,
    0x8cf008cf008cf009,
    0x8ca29c046514e023,
    0x8c55841c815ed5ca,
    0x8c08c08c08c08c09,
    0x8bbc50c8deb420c0,
    0x8b70344a139bc75b,
    0x8b246a87e19008b2,
    0x8ad8f2fba9386823,
    0x8a8dcd1feeae465c,
    0x8a42f8705669db46,
    0x89f87469a23920e0,
    0x89ae4089ae4089ae,
    0x89645c4f6e055dec,
    0x891ac73ae9819b50,
    0x88d180cd3a4133d7,
    0x8888888888888889,
    0x883fddf00883fddf,
    0x87f78087f78087f8,
    0x87af6fd5992d0d40,
    0x8767ab5f34e47ef1,
    0x872032ac13008720,
    0x86d905447a34acc6,
    0x869222b1acf1ce96,
    0x864b8a7de6d1d608,
    0x86053c345a0b8473,
    0x85bf37612cee3c9b,
    0x85797b917765ab89,
    0x8534085340853408,
    0x84eedd357c1b0085,
    0x84a9f9c8084a9f9d,
    0x84655d9bab2f1008,
    0x8421084210842108,
    0x83dcf94dc7570ce1,
    0x839930523fbe3368,
    0x8355ace3c897db10,
    0x83126e978d4fdf3b,
    0x82cf750393ac3319,
    0x828cbfbeb9a020a3,
    0x824a4e60b3262bc5,
    0x8208208208208208,
    0x81c635bc123fdf8e,
    0x81848da8faf0d277,
    0x814327e3b94f462f,
    0x8102040810204081,
    0x80c121b28bd1ba98,
    0x8080808080808081,
    0x8040201008040201,
    0x8000000000000000,
];

/// An approximation of 1/a, for `a` not zero, with relative error < 2^-124.999.
#[inline(always)]
fn inv_dint(a: &Dint) -> Dint {
    let h = a.hi; // 2^63 <= h < 2^64
    // a 64-bit inverse t of h: t*h ~ 2^127 (Newton's iteration, three times)
    let i = ((h >> 55) & 0xff) as usize;
    let mut t = TINV[i];
    let mut e = (1u128 << 127) - u128::from(h) * u128::from(t); // exact
    e = u128::from(t) * (e >> 55);
    t += (e >> 72) as u64;
    e = (1u128 << 127) - u128::from(h) * u128::from(t); // 0 <= e < 2^111
    e = u128::from(t) * (e >> 47);
    t += (e >> 80) as u64;
    e = (1u128 << 127) - u128::from(h) * u128::from(t); // 0 <= e < 2^95
    e = u128::from(t) * (e >> 31);
    t += (e >> 96) as u64;
    let mut r = Dint {
        hi: t,
        lo: 0,
        ex: 1 - a.ex,
        sgn: 1,
    };
    // Newton's iteration: r -> r + r*(1-a*r)
    let mut q = mul_dint_21(a, &r); // -a*r
    r.sgn = 0; // restore the sign
    q = add_dint(&ONE, &q); // 1-a*r
    q = mul_dint(&r, &q); // r*(1-a*r)
    add_dint(&r, &q)
}

/// An approximation of b/a, for `a` not zero, with relative error < 2^-123.67.
#[inline(always)]
fn div_dint(b: &Dint, a: &Dint) -> Dint {
    let r = inv_dint(a);
    mul_dint(&r, b)
}

/// For 0x1.7137449123ef6p-26 < x < +Inf, return `(i, h, l)` such that i/2^11 + h + l
/// approximates frac(x/(2pi)), with absolute error < 2^-104.815.
#[inline(always)]
fn reduce_fast<F: FmaOps>(x: f64) -> (usize, f64, f64) {
    let (mut h, l);
    if x <= const { hf("0x1.921fb54442d17p+2") } {
        // x < 2*pi; | CH+CL - 1/(2pi) | < 2^-110.523
        const CH: f64 = hf("0x1.45f306dc9c883p-3");
        const CL: f64 = hf("-0x1.6b01ec5417056p-57");
        let (hh, ll) = a_mul::<F>(CH, x); // exact
        h = hh;
        l = F::fma(CL, x, ll);
    } else {
        // x > 0x1.921fb54442d17p+2
        let tu = x.to_bits();
        let mut e = ((tu >> 52) & 0x7ff) as i64; // 1025 <= e <= 2046
        let m = (1u64 << 52) | (tu & 0xfffffffffffff);
        let mut c = [0u64; 3];
        let mm = u128::from(m);
        // x = m/2^53 * 2^(e-1022)
        if e <= 1074 {
            // 1025 <= e <= 1074: 2^2 <= x < 2^52
            let v = mm * u128::from(T[2]);
            let mut u = mm * u128::from(T[1]);
            c[0] = (u as u64).wrapping_add((v >> 64) as u64);
            c[1] = ((u >> 64) as u64) + u64::from(c[0] < u as u64);
            u = mm * u128::from(T[0]);
            c[1] = c[1].wrapping_add(u as u64);
            c[2] = ((u >> 64) as u64) + u64::from(c[1] < u as u64);
            e = 1075 - e; // 1 <= e <= 50
        } else {
            // 1075 <= e <= 2046, 2^52 <= x < 2^1024
            let i = ((e - 1138 + 63) / 64) as usize; // ceil((e-1138)/64), 0 <= i <= 15
            let v = mm * u128::from(T[i + 3]);
            let mut u = mm * u128::from(T[i + 2]);
            c[0] = (u as u64).wrapping_add((v >> 64) as u64);
            c[1] = ((u >> 64) as u64) + u64::from(c[0] < u as u64);
            u = mm * u128::from(T[i + 1]);
            c[1] = c[1].wrapping_add(u as u64);
            c[2] = ((u >> 64) as u64) + u64::from(c[1] < u as u64);
            u = mm * u128::from(T[i]);
            c[2] = c[2].wrapping_add(u as u64);
            e = 1139 + ((i as i64) << 6) - e; // 1 <= e <= 64
        }
        // e is the number of bits of c[2] to ignore
        if e == 64 {
            c[0] = c[1];
            c[1] = c[2];
        } else {
            c[0] = (c[1] << (64 - e)) | c[0] >> e;
            c[1] = (c[2] << (64 - e)) | c[1] >> e;
        }
        (h, l) = set_dd(c[1], c[0]);
    }
    let i = (h * const { hf("0x1p11") }).floor();
    h = F::fma(i, const { hf("-0x1p-11") }, h);
    (i as usize, h, l)
}

/// `(bh + bl) / (ah + al)` as `h + l` (Karp-Markstein), relative error < 2^-96.99, for
/// |bl|, |al| < 2^-49.47.
#[inline(always)]
fn fast_div<F: FmaOps>(bh: f64, bl: f64, ah: f64, al: f64) -> (f64, f64) {
    let y = 1.0 / ah;
    let h = bh * y;
    let eh = F::fma(ah, -h, bh);
    let el = F::fma(al, -h, bl);
    (h, y * (eh + el))
}

/// For finite x with |x| > 0x1.d12ed0af1a27ep-27: `(h, l, err)` with | h + l - tan(x) | < err.
#[inline(always)]
fn tan_fast<F: FmaOps>(x: f64) -> (f64, f64, f64) {
    let mut neg = usize::from(x < 0.0);
    let mut is_tan = 1usize;
    let absx = if neg != 0 { -x } else { x };
    let (mut i, mut h, mut l) = reduce_fast::<F>(absx);
    // if i >= 2^10: pi <= x <= 2pi, tan(pi+x) = tan(x)
    i &= 0x3ff;
    // if i >= 2^9: pi/2 <= x <= pi, tan(pi/2+x) = -cot(x)
    is_tan ^= i >> 9;
    neg ^= i >> 9;
    i &= 0x1ff;
    // if i >= 2^8: tan(pi/2-x) = cot(x)
    if i & 0x100 != 0 {
        // pi/4 <= x_red <= pi/2; 0x1p-11 - h is exact
        is_tan ^= 1;
        i = 0x1ff - i;
        h = const { hf("0x1p-11") } - h;
        l = -l;
    }
    // For i=0 and h<2^-37, the slow path.
    if i == 0 && h < const { hf("0x1p-37") } {
        return (h, l, 1.0);
    }
    // SC[] evaluates at i/2^11 + SC[i][0]; h - SC[i][0] is exact
    h -= SC[i][0];
    // |l| <= ulp(h) after this
    (h, l) = fast_two_sum(h, l);
    let (uh, ul) = a_mul::<F>(h, h);
    let ul = F::fma(h + h, l, ul);
    // uh+ul approximates (h+l)^2
    let (sh, sl) = eval_ps_fast::<F>(h, l, uh, ul);
    let (ch, cl) = eval_pc_fast::<F>(uh, ul);
    let (sh0, sl0) = s_mul::<F>(SC[i][2], sh, sl);
    let (ch0, cl0) = s_mul::<F>(SC[i][1], ch, cl);
    let (h1, mut l1) = fast_two_sum(ch0, sh0);
    l1 += sl0 + cl0;
    // | h1 + l1 - sin2pi (R) | < 2^-67.777 * |sin2pi(R)|
    let (ch, cl) = s_mul::<F>(SC[i][2], ch, cl);
    let (sh, sl) = s_mul::<F>(SC[i][1], sh, sl);
    let (h2, mut l2) = fast_two_sum(ch, -sh);
    l2 += cl - sl;
    // | h2 + l2 - cos2pi (R) | < 2^-68.073 * |cos2pi(R)|
    (h, l) = if is_tan != 0 {
        fast_div::<F>(h1, l1, h2, l2)
    } else {
        fast_div::<F>(h2, l2, h1, l1)
    };
    // h+l = tan(x mod pi/2) * (1+eps) with |eps| < 2^-65.864
    let sg = if neg != 0 { -1.0 } else { 1.0 };
    h *= sg;
    l *= sg;
    (h, l, h * const { hf("0x1.1ap-66") })
}

/// For x regular with |x| > 0x1.d12ed0af1a27ep-27.
#[inline(never)]
fn tan_accurate(x: f64) -> f64 {
    let absx = if x > 0.0 { x } else { -x };
    let mut xd = dint_fromd(absx);
    reduce(&mut xd);
    // now |X - x/(2pi) mod 1| < 2^-126.67*X, with 0 <= X < 1.
    let mut is_tan = true;
    let mut neg = x < 0.0;
    let mut i = reduce2(&mut xd); // exact; X = i/2^11 + r
    i &= 0x3ff; // if pi <= x < 2*pi, tan(x) = tan(x-pi)
    if i & 0x200 != 0 {
        // pi/2 <= x < pi: tan(x) = -1/tan(x-pi/2)
        is_tan = false;
        neg = !neg;
        i &= 0x1ff;
    }
    if i & 0x100 != 0 {
        // pi/4 <= x < pi/2: tan(x) = 1/tan(pi/2-x)
        is_tan = !is_tan;
        xd.sgn = 1; // negate X
        xd = add_dint(&MAGIC, &xd); // X -> 2^-11 - X
        i = 0x1ff - i;
    }
    // now 0 <= i < 256 and 0 <= X < 2^-11
    let x2 = mul_dint(&xd, &xd);
    let mut u = eval_pc(&x2); // cos2pi(X)
    let mut v = eval_ps(&xd, &x2); // sin2pi(X)
    // sin2pi(R) ~ sin2pi(i/2^11)*cos2pi(X)+cos2pi(i/2^11)*sin2pi(X)
    let uu = mul_dint(&S[i], &u);
    let vv = mul_dint(&C[i], &v);
    let sin = add_dint(&uu, &vv);
    // cos2pi(R) ~ cos2pi(i/2^11)*cos2pi(X)-sin2pi(i/2^11)*sin2pi(X)
    u = mul_dint(&C[i], &u);
    v = mul_dint(&S[i], &v);
    v.sgn = 1 - v.sgn; // negate V
    let cos = add_dint(&u, &v);
    // S/C when is_tan, C/S otherwise
    u = if is_tan {
        div_dint(&sin, &cos)
    } else {
        div_dint(&cos, &sin)
    };
    // total relative error < 2^-121.578: less than 86 ulps
    let err = 86u64;
    let lo0 = u.lo.wrapping_sub(err);
    let hi0 = u.hi.wrapping_sub(u64::from(lo0 > u.lo));
    let lo1 = u.lo.wrapping_add(err);
    let hi1 = u.hi.wrapping_add(u64::from(lo1 < u.lo));
    // check that the upper 54 bits are equal
    if (hi0 >> 10) != (hi1 >> 10) {
        const EXCEPTIONS: [[f64; 3]; 2] = [
            // 78 identical bits after the round bit
            [
                hf("0x1.dffffffffff1fp-22"),
                hf("0x1.e000000000151p-22"),
                hf("0x1.fffffffffffffp-76"),
            ],
            // 72 identical bits after the round bit
            [
                hf("0x1.dfffffffffc7cp-21"),
                hf("0x1.e000000000546p-21"),
                hf("-0x1.658bcedb6e1d4p-147"),
            ],
        ];
        for ex in &EXCEPTIONS {
            if x.abs() == ex[0] {
                return if x > 0.0 {
                    ex[1] + ex[2]
                } else {
                    -ex[1] - ex[2]
                };
            }
        }
        // a hard-to-round case; all of them are known and pass (C comment)
    }
    if neg {
        u.sgn = 1 - u.sgn;
    }
    dint_tod(&u)
}

#[inline(always)]
fn tan_with<F: FmaOps>(x: f64) -> f64 {
    let tu = x.to_bits();
    let e = (tu >> 52) & 0x7ff;
    if e == 0x7ff {
        // NaN, +Inf and -Inf
        if tu << 1 == 0x7ff << 53 {
            return f64::NAN; // the C code returns x - x
        }
        return x + x;
    }
    // For |x| <= 0x1.d12ed0af1a27ep-27, tan(x) rounds to x + tiny (to nearest).
    if tu & 0x7fffffffffffffff <= 0x3e4d12ed0af1a27e {
        if x == 0.0 {
            return x;
        }
        return F::fma(x, const { hf("0x1p-54") }, x);
    }
    let (h, l, err) = tan_fast::<F>(x);
    let left = h + (l - err);
    let right = h + (l + err);
    if left == right {
        return left;
    }
    tan_accurate(x)
}

multiversion! {
    /// The tangent, correctly rounded (round to nearest, ties to even): CORE-MATH `cr_tan`. The
    /// same bits on every target. `Math.tan` (§21.3.2.33): NaN and ±∞ give NaN, ±0 gives ±0.
    pub fn tan(x: f64) -> f64 = tan_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tan_special_values() {
        assert!(tan(f64::NAN).is_nan());
        assert!(tan(f64::INFINITY).is_nan());
        assert!(tan(f64::NEG_INFINITY).is_nan());
        assert_eq!(tan(0.0).to_bits(), 0);
        assert_eq!(tan(-0.0).to_bits(), 1 << 63);
        assert_eq!(tan(std::f64::consts::FRAC_PI_4), hf("0x1.fffffffffffffp-1"));
        assert_eq!(
            tan(std::f64::consts::FRAC_PI_2),
            hf("0x1.d02967c31cdb5p+53")
        );
    }

    /// The 128-bit path agrees with the fast path where both give an answer.
    #[test]
    fn tan_accurate_agrees() {
        for &x in &[
            0.5,
            1.0,
            -3.0,
            100.0,
            1e10,
            -1e22,
            1e300,
            hf("0x1.fffffffffffffp+1023"),
        ] {
            assert_eq!(tan_accurate(x).to_bits(), tan(x).to_bits(), "{x}");
        }
    }
}
