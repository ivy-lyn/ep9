//! `Math.expm1`: the correctly rounded e^x - 1 (H36), a port of CORE-MATH `cr_expm1`.
//!
//! Ported for ep9 from CORE-MATH `src/binary64/expm1/expm1.c` (commit
//! 1f876c544231f310f483f0a523e163850141dd7e, 2026-10-26):
//!
//! ```text
//! Correctly rounded expm1 function for binary64 values.
//! Copyright (c) 2024 Alexei Sibidanov.
//! This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//! SPDX-License-Identifier: MIT
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("CORE-MATH"). The tables, constants and steps are the
//! C ones; the double tables keep their C text through [`crate::hf`]. The tables `t0` and `t1`
//! have the same C text as in sinh.c, so this file uses the `crate::sinh` copies. ep9
//! changes: rounding to nearest only (JS); no `errno`; every `__builtin_fma` is
//! [`FmaOps::fma`] (hardware or exact software FMA, the same bits); no FMA contraction;
//! `roundeven_finite` of a value below 2^51 in magnitude is the 1.5 * 2^52 trick; `as_ldexp`
//! adds to the exponent bits (the C non-x86 branch; the SSE branch gives the same bits); the
//! `volatile` overflow product is its round-to-nearest value, +∞.

use crate::asin::roundeven_small;
use crate::exact::FmaOps;
use crate::hf;
use crate::sinh::{T0, T1};

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

#[inline(always)]
fn muldd<F: FmaOps>(xh: f64, xl: f64, ch: f64, cl: f64) -> (f64, f64) {
    let ahhh = ch * xh;
    (ahhh, (ch * xl + cl * xh) + F::fma(ch, xh, -ahhh))
}

#[inline(always)]
fn mulddd<F: FmaOps>(xh: f64, xl: f64, c: f64) -> (f64, f64) {
    let h = c * xh;
    (h, c * xl + F::fma(c, xh, -h))
}

#[inline(always)]
fn opolydd<F: FmaOps>(xh: f64, xl: f64, c: &[[f64; 2]]) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, mut cl) = (c[i][0], c[i][1]);
    while i > 0 {
        i -= 1;
        (ch, cl) = muldd::<F>(xh, xl, ch, cl);
        (ch, cl) = fastsum(c[i][0], c[i][1], ch, cl);
    }
    (ch, cl)
}

#[inline(always)]
fn opolyddd<F: FmaOps>(x: f64, c: &[[f64; 2]], l: f64) -> (f64, f64) {
    let mut i = c.len() - 1;
    let (mut ch, l) = fasttwosum(c[i][0], l);
    let mut cl = c[i][1] + l;
    while i > 0 {
        i -= 1;
        (ch, cl) = mulddd::<F>(ch, cl, x);
        (ch, cl) = fastsum(c[i][0], c[i][1], ch, cl);
    }
    (ch, cl)
}

#[inline(always)]
fn as_ldexp(x: f64, i: i64) -> f64 {
    f64::from_bits(x.to_bits().wrapping_add((i as u64) << 52))
}

/// The exceptional cases, sorted by their bits (as u64).
static DB: [f64; 38] = [
    hf("0x1.e923c188ea79bp-4"),
    hf("0x1.1a0408712e00ap-2"),
    hf("0x1.1c38132777b26p-2"),
    hf("0x1.27f4980d511ffp-2"),
    hf("0x1.8172a0e02f90ep-2"),
    hf("0x1.8bbe2fb45c151p-2"),
    hf("0x1.bcab27d05abdep-2"),
    hf("0x1.005ae04256babp-1"),
    hf("0x1.accfbe46b4efp-1"),
    hf("0x1.d086543694c5ap-1"),
    hf("0x1.273c188aa7b14p+2"),
    hf("0x1.83d4bcdebb3f4p+2"),
    hf("0x1.08f51434652c3p+4"),
    hf("0x1.1d5c2daebe367p+4"),
    hf("0x1.c44ce0d716a1ap+4"),
    hf("0x1.2ee70220fb1c5p+5"),
    hf("0x1.89d56a0c38e6fp+5"),
    hf("0x1.7a60ee15e3e9dp+6"),
    hf("0x1.1f0da93354198p+7"),
    hf("0x1.54cd1fea7663ap+7"),
    hf("0x1.556c678d5e976p+7"),
    hf("0x1.2da9e5e6af0bp+8"),
    hf("0x1.9e7b643238a14p+8"),
    hf("0x1.d6479eba7c971p+8"),
    hf("0x1.0bc04af1b09f5p+9"),
    hf("-0x1.ab86cb1743b75p-4"),
    hf("-0x1.119aae6072d39p-2"),
    hf("-0x1.175693a03b59p-2"),
    hf("-0x1.474d4de7c14bbp-2"),
    hf("-0x1.789d025948efap-2"),
    hf("-0x1.82b5dfaf59b4cp-2"),
    hf("-0x1.9d871e078ebcep-2"),
    hf("-0x1.1397add4538acp-1"),
    hf("-0x1.22e24fa3d5cf9p-1"),
    hf("-0x1.dc2b5df1f7d3dp-1"),
    hf("-0x1.0a54d87783d6fp+0"),
    hf("-0x1.2a9cad9998262p+0"),
    hf("-0x1.e42a2abb1bf0fp+0"),
];

#[inline(never)]
fn as_expm1_database(x: f64, f: f64) -> f64 {
    let ix = x.to_bits();
    let (mut a, mut b) = (0i32, DB.len() as i32 - 1);
    let mut m = (a + b) / 2;
    while a <= b {
        let c = DB[m as usize].to_bits();
        if c < ix {
            a = m + 1;
        } else if c == ix {
            const S2: [u64; 2] = [0x76f58b0d65bd5553, 0xc06];
            const S: u64 = 0x300e81651c;
            let jf = f.to_bits();
            let dr =
                f64::from_bits(((S >> m) << 63) | ((((jf >> 52) & 0x7ff).wrapping_sub(54)) << 52));
            let t = (S2[(m >> 5) as usize] >> ((m << 1) & 63)) & 3;
            for k in -1i64..=1 {
                let r = jf.wrapping_add(k as u64);
                if r & 3 == t {
                    return f64::from_bits(r) + dr;
                }
            }
            break;
        } else {
            b = m - 1;
        }
        m = (a + b) >> 1;
    }
    f
}

/// e^(i/128) - 1 for -32 <= i <= 32, at index i + 32, as (low, high).
static TZ: [[f64; 2]; 65] = [
    [hf("-0x1.797d4686c5393p-57"), hf("-0x1.c5041854df7d4p-3")],
    [hf("-0x1.ea1cb9d163339p-55"), hf("-0x1.b881a23aebb48p-3")],
    [hf("0x1.f483a3e8cd60fp-55"), hf("-0x1.abe60e1f21838p-3")],
    [hf("0x1.dffd920f493dbp-56"), hf("-0x1.9f3129931fabp-3")],
    [hf("-0x1.51bfdbb129094p-55"), hf("-0x1.9262c1c3430ap-3")],
    [hf("0x1.cd3e5225e2206p-55"), hf("-0x1.857aa375db4e4p-3")],
    [hf("0x1.e3a6bdaece8f9p-58"), hf("-0x1.78789b0a5e0cp-3")],
    [hf("-0x1.daf2ae0c2d3d4p-55"), hf("-0x1.6b5c7478983d8p-3")],
    [hf("-0x1.fd36226fadd44p-56"), hf("-0x1.5e25fb4fde21p-3")],
    [hf("0x1.d887cd0341abp-56"), hf("-0x1.50d4fab639758p-3")],
    [hf("-0x1.676a52a1a618bp-55"), hf("-0x1.43693d679612cp-3")],
    [hf("0x1.9776b420ad283p-56"), hf("-0x1.35e28db4ecd9cp-3")],
    [hf("0x1.3d5fd7d70a5edp-56"), hf("-0x1.2840b5836cf68p-3")],
    [hf("0x1.a94ad2c8fa0bfp-58"), hf("-0x1.1a837e4ba376p-3")],
    [hf("0x1.6ad4c353465bp-61"), hf("-0x1.0caab118a1278p-3")],
    [hf("-0x1.8bba170e59b65p-56"), hf("-0x1.fd6c2d0e3d91p-4")],
    [hf("-0x1.e1e0a76cb0685p-55"), hf("-0x1.e14aed893eefp-4")],
    [hf("0x1.fe131f55e75f8p-55"), hf("-0x1.c4f1331d22d4p-4")],
    [hf("-0x1.b5beee8bcee31p-55"), hf("-0x1.a85e8c62d9c1p-4")],
    [hf("-0x1.7fe9b02c25e9bp-56"), hf("-0x1.8b92870fa2b58p-4")],
    [hf("-0x1.32ae7bdaf1116p-55"), hf("-0x1.6e8caff341fe8p-4")],
    [hf("0x1.a6cfe58cbd73bp-56"), hf("-0x1.514c92f634788p-4")],
    [hf("0x1.8798de3138a56p-57"), hf("-0x1.33d1bb17df2e8p-4")],
    [hf("-0x1.589321a7ef10bp-60"), hf("-0x1.161bb26cbb59p-4")],
    [hf("-0x1.8d0e700fcfb65p-56"), hf("-0x1.f0540438fd5cp-5")],
    [hf("0x1.473ef07d5dd3bp-55"), hf("-0x1.b3f864c08p-5")],
    [hf("-0x1.38e62149c16e2p-55"), hf("-0x1.77239501304p-5")],
    [hf("-0x1.08bb6309bd394p-58"), hf("-0x1.39d4a1a77e05p-5")],
    [hf("-0x1.bad3fd501a227p-55"), hf("-0x1.f8152aee945p-6")],
    [hf("0x1.3d27ac39ed253p-57"), hf("-0x1.7b88f290230ep-6")],
    [hf("-0x1.b60bbd08aac55p-55"), hf("-0x1.fc055004416cp-7")],
    [hf("-0x1.a00d03b3359dep-59"), hf("-0x1.fe0154aaeed8p-8")],
    [hf("0x0p+0"), hf("0x0p+0")],
    [hf("0x1.861931c15e39bp-55"), hf("0x1.0100ab00222cp-7")],
    [hf("0x1.7ab864b3e9045p-56"), hf("0x1.0202ad5778e4p-6")],
    [hf("0x1.4e5659d75e95bp-56"), hf("0x1.84890d904374p-6")],
    [hf("0x1.8e0bd083aba81p-56"), hf("0x1.040ac0224fd9p-5")],
    [hf("0x1.45cc1cf959b1bp-60"), hf("0x1.465509d383ebp-5")],
    [hf("-0x1.eb6980ce14da7p-55"), hf("0x1.89246d053d18p-5")],
    [hf("0x1.7324137d6c342p-56"), hf("0x1.cc79f4f5613ap-5")],
    [hf("-0x1.5272ff30eed1bp-59"), hf("0x1.082b577d34ed8p-4")],
    [hf("-0x1.1280f19dace1cp-55"), hf("0x1.2a5dd543ccc5p-4")],
    [hf("-0x1.d550af31c8ec3p-55"), hf("0x1.4cd4fc989cd68p-4")],
    [hf("0x1.7923b72aa582dp-55"), hf("0x1.6f9157587069p-4")],
    [hf("-0x1.76c2e732457f1p-56"), hf("0x1.92937074e0cd8p-4")],
    [hf("0x1.81f5c92a5200fp-55"), hf("0x1.b5dbd3f68122p-4")],
    [hf("0x1.e8ac7a4d3206cp-55"), hf("0x1.d96b0eff0e79p-4")],
    [hf("-0x1.12db6f4bbe33bp-56"), hf("0x1.fd41afcba45e8p-4")],
    [hf("-0x1.8c4a5df1ec7e5p-58"), hf("0x1.10b022db7ae68p-3")],
    [hf("-0x1.bd4b1c37ea8a2p-57"), hf("0x1.22e3b09dc54d8p-3")],
    [hf("0x1.5aeb9860044dp-55"), hf("0x1.353bc9fb00b2p-3")],
    [hf("-0x1.4c26602c63fdap-57"), hf("0x1.47b8b853aafecp-3")],
    [hf("-0x1.7f644c1f9d314p-55"), hf("0x1.5a5ac59b963ccp-3")],
    [hf("0x1.f5aa8ec61fc2dp-55"), hf("0x1.6d223c5b10638p-3")],
    [hf("0x1.7ab912c69ffebp-61"), hf("0x1.800f67b00d7b8p-3")],
    [hf("-0x1.b3564bc0ec9cdp-58"), hf("0x1.9322934f54148p-3")],
    [hf("0x1.6a7062465be33p-55"), hf("0x1.a65c0b85ac1a8p-3")],
    [hf("-0x1.85718d2ff1bf4p-55"), hf("0x1.b9bc1d3910094p-3")],
    [hf("-0x1.045cb0c685e08p-55"), hf("0x1.cd4315e9e0834p-3")],
    [hf("-0x1.6e7fb859d5055p-62"), hf("0x1.e0f143b41a554p-3")],
    [hf("0x1.51bbdee020603p-55"), hf("0x1.f4c6f5508ee5cp-3")],
    [hf("0x1.e17611afc42c5p-57"), hf("0x1.04623d0b0f8c8p-2")],
    [hf("-0x1.1c5b2e8735a43p-56"), hf("0x1.0e7510fd7c564p-2")],
    [hf("-0x1.25fe139c4cffdp-55"), hf("0x1.189c1ecaeb084p-2")],
    [hf("-0x1.89843c4964554p-56"), hf("0x1.22d78f0fa061ap-2")],
];

#[inline(always)]
fn as_expm1_accurate_with<F: FmaOps>(x: f64) -> f64 {
    if x.abs() < 0.25 {
        const CL: [f64; 6] = [
            hf("0x1.93974a8ca5354p-37"),
            hf("0x1.ae7f3e71e4908p-41"),
            hf("0x1.ae7f357341648p-45"),
            hf("0x1.952c7f96664cbp-49"),
            hf("0x1.686f8ce633aaep-53"),
            hf("0x1.2f49b2fbfb5b6p-57"),
        ];
        const CH: [[f64; 2]; 11] = [
            [hf("0x1.5555555555555p-3"), hf("0x1.5555555555554p-57")],
            [hf("0x1.5555555555555p-5"), hf("0x1.5555555555123p-59")],
            [hf("0x1.1111111111111p-7"), hf("0x1.1111111118167p-63")],
            [hf("0x1.6c16c16c16c17p-10"), hf("-0x1.f49f49e220ceap-65")],
            [hf("0x1.a01a01a01a01ap-13"), hf("0x1.a019eff6f919cp-73")],
            [hf("0x1.a01a01a01a01ap-16"), hf("0x1.9fcff48a75b41p-76")],
            [hf("0x1.71de3a556c734p-19"), hf("-0x1.c14f73758cd7fp-73")],
            [hf("0x1.27e4fb7789f5cp-22"), hf("0x1.dfce97931018fp-76")],
            [hf("0x1.ae64567f544e3p-26"), hf("0x1.c513da9e4c9c5p-80")],
            [hf("0x1.1eed8eff8d831p-29"), hf("0x1.ca00af84f2b6p-83")],
            [hf("0x1.6124613a86e8fp-33"), hf("0x1.f27ac6000898fp-87")],
        ];
        let fl = x * (CL[0] + x * (CL[1] + x * (CL[2] + x * (CL[3] + x * (CL[4] + x * CL[5])))));
        let (fh, fl) = opolyddd::<F>(x, &CH, fl);
        let (fh, fl) = mulddd::<F>(fh, fl, x);
        let (fh, fl) = mulddd::<F>(fh, fl, x);
        let (fh, fl) = mulddd::<F>(fh, fl, x);
        let hx = 0.5 * x;
        let x2h = x * hx;
        let x2l = F::fma(x, hx, -x2h);
        let (fh, fl) = fastsum(x2h, x2l, fh, fl);
        let (v0, v2) = fasttwosum(x, fh);
        let (v1, v2) = fasttwosum(v2, fl);
        let (v0, v1) = fasttwosum(v0, v1);
        let (mut v1, v2) = fasttwosum(v1, v2);
        let mut ix = v1.to_bits();
        if ix & (!0u64 >> 12) == 0 {
            if ix << 1 == 0 {
                return as_expm1_database(x, v0);
            }
            let v = v2.to_bits();
            let d = ((((ix as i64) >> 63) ^ ((v as i64) >> 63)) as u64) << 1;
            ix = ix.wrapping_add(d.wrapping_add(1));
            v1 = f64::from_bits(ix);
        }
        v0 + v1
    } else {
        const CH: [[f64; 2]; 7] = [
            [hf("0x1p+0"), 0.0],
            [hf("0x1p-1"), hf("0x1.712f72ecec2cfp-99")],
            [hf("0x1.5555555555555p-3"), hf("0x1.5555555554d07p-57")],
            [hf("0x1.5555555555555p-5"), hf("0x1.55194d28275dap-59")],
            [hf("0x1.1111111111111p-7"), hf("0x1.12faa0e1c0f7bp-63")],
            [hf("0x1.6c16c16da6973p-10"), hf("-0x1.4ba45ab25d2a3p-64")],
            [hf("0x1.a01a019eb7f31p-13"), hf("-0x1.9091d845ecd36p-67")],
        ];
        const S: f64 = hf("0x1.71547652b82fep+12");
        let t = roundeven_small(x * S);
        let jt = t as i64;
        let (i0, i1, ie) = (((jt >> 6) & 0x3f) as usize, (jt & 0x3f) as usize, jt >> 12);
        let (t0h, t0l) = (T0[i0][1], T0[i0][0]);
        let (t1h, t1l) = (T1[i1][1], T1[i1][0]);
        let (th, tl) = muldd::<F>(t0h, t0l, t1h, t1l);
        const L2H: f64 = hf("0x1.62e42ffp-13");
        const L2L: f64 = hf("0x1.718432a1b0e26p-47");
        const L2LL: f64 = hf("0x1.9ff0342542fc3p-102");
        let dx = x - L2H * t;
        let dxl = L2L * t;
        let dxll = L2LL * t + F::fma(L2L, t, -dxl);
        let dxh = dx + dxl;
        let dxl = ((dx - dxh) + dxl) + dxll;
        let (fh, fl) = opolydd::<F>(dxh, dxl, &CH);
        let (fh, fl) = muldd::<F>(dxh, dxl, fh, fl);
        let (fh, fl) = muldd::<F>(fh, fl, th, tl);
        let (mut fh, mut fl) = fastsum(th, tl, fh, fl);
        let off = f64::from_bits(((2048 + 1023 - ie) as u64) << 52);
        let e;
        if ie < 53 {
            (fh, e) = fasttwosum(off, fh);
        } else if ie < 104 {
            (fh, e) = fasttwosum(fh, off);
        } else {
            e = 0.0;
        }
        fl += e;
        (fh, fl) = fasttwosum(fh, fl);
        let d = fl.to_bits().wrapping_add(8) & (!0u64 >> 12);
        if d <= 8 {
            fh = as_expm1_database(x, fh);
        }
        as_ldexp(fh, ie)
    }
}

multiversion! {
    fn as_expm1_accurate(x: f64) -> f64 = as_expm1_accurate_with;
}

#[inline(always)]
fn expm1_with<F: FmaOps>(x: f64) -> f64 {
    let ix = x.to_bits();
    let aix = ix & (!0u64 >> 1);
    if aix < 0x3fd0000000000000 {
        // |x| < 0.25
        if aix < 0x3ca0000000000000 {
            // |x| < 2^-53
            if aix == 0 {
                return x;
            }
            return F::fma(const { hf("0x1p-54") }, x.abs(), x);
        }
        let sx = const { hf("0x1p7") } * x;
        let fx = roundeven_small(sx);
        let z = sx - fx;
        let z2 = z * z;
        let i = fx as i64;
        let th = TZ[(i + 32) as usize][1];
        let tl = TZ[(i + 32) as usize][0];
        const C: [f64; 6] = [
            hf("0x1p-7"),
            hf("0x1p-15"),
            hf("0x1.55555555551adp-24"),
            hf("0x1.555555555599cp-33"),
            hf("0x1.11111ad1ad69dp-42"),
            hf("0x1.6c16c168b1fb5p-52"),
        ];
        let fh = z * C[0];
        let fl = z2 * ((C[1] + z * C[2]) + z2 * (C[3] + z * (C[4] + z * C[5])));
        const E0: f64 = hf("0x1.ap-65");
        let eps = z2 * E0 + const { hf("0x1p-104") };
        let (rh, mut rl) = fasttwosum(th, fh);
        rl += tl + fl;
        let (fh, fl) = muldd::<F>(th, tl, fh, fl);
        let (fh, fl) = fastsum(rh, rl, fh, fl);
        let ub = fh + (fl + eps);
        let lb = fh + (fl - eps);
        if ub != lb {
            return as_expm1_accurate(x);
        }
        lb
    } else {
        // |x| >= 0.25
        if aix >= 0x40862e42fefa39f0 {
            // |x| >= 0x1.62e42fefa39fp+9
            if aix > 0x7ff0000000000000 {
                return x + x; // nan
            }
            if aix == 0x7ff0000000000000 {
                return if ix >> 63 != 0 { -1.0 } else { x }; // -inf gives -1, +inf gives +inf
            }
            if ix >> 63 == 0 {
                return f64::INFINITY; // x >= 0x1.62e42fefa39fp+9: overflow
            }
        }
        if ix >= 0xc0425e4f7b2737fa {
            // x <= -0x1.25e4f7b2737fap+5
            if ix >= 0xc042b708872320e2 {
                // x <= -0x1.2b708872320e2p+5
                return -1.0 + const { hf("0x1p-55") };
            }
            return ((const { hf("0x1.25e4f7b2737fap+5") } + x)
                + const { hf("0x1.8486612173c69p-51") })
                * const { hf("0x1.71547652b82fep-54") }
                - const { hf("0x1.fffffffffffffp-1") };
        }
        const S: f64 = hf("0x1.71547652b82fep+12"); // 2^12/log(2)
        let t = roundeven_small(x * S);
        let jt = t as i64;
        let (i0, i1, ie) = (((jt >> 6) & 0x3f) as usize, (jt & 0x3f) as usize, jt >> 12);
        let (t0h, t0l) = (T0[i0][1], T0[i0][0]);
        let (t1h, t1l) = (T1[i1][1], T1[i1][0]);
        let (th, tl) = muldd::<F>(t0h, t0l, t1h, t1l);
        const L2H: f64 = hf("0x1.62e42ffp-13");
        const L2L: f64 = hf("0x1.718432a1b0e26p-47");
        let dx = (x - L2H * t) + L2L * t;
        let dx2 = dx * dx;
        const CH: [f64; 4] = [
            hf("0x1p+0"),
            hf("0x1p-1"),
            hf("0x1.55555557e54ffp-3"),
            hf("0x1.55555553a12f4p-5"),
        ];
        let p = (CH[0] + dx * CH[1]) + dx2 * (CH[2] + dx * CH[3]);
        let mut fh = th;
        let tx = th * dx;
        let mut fl = tl + tx * p;
        let eps = 1.64e-19 * th;
        let off = f64::from_bits(((2048 + 1023 - ie) as u64) << 52);
        let e;
        if ie < 53 {
            (fh, e) = fasttwosum(off, fh);
        } else if ie < 75 {
            (fh, e) = fasttwosum(fh, off);
        } else {
            e = 0.0;
        }
        fl += e;
        let ub = fh + (fl + eps);
        let lb = fh + (fl - eps);
        if ub != lb {
            return as_expm1_accurate(x);
        }
        as_ldexp(lb, ie)
    }
}

multiversion! {
    /// e^x - 1, correctly rounded (round to nearest, ties to even): CORE-MATH `cr_expm1`. The
    /// same bits on every target. `Math.expm1` (§21.3.2.15): NaN, ±0 and +∞ give themselves,
    /// -∞ gives -1.
    pub fn expm1(x: f64) -> f64 = expm1_with;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expm1_special_values() {
        assert!(expm1(f64::NAN).is_nan());
        assert_eq!(expm1(0.0).to_bits(), 0);
        assert_eq!(expm1(-0.0).to_bits(), 1 << 63);
        assert_eq!(expm1(f64::INFINITY), f64::INFINITY);
        assert_eq!(expm1(f64::NEG_INFINITY), -1.0);
        assert_eq!(expm1(710.0), f64::INFINITY);
        assert_eq!(expm1(-40.0), -1.0);
    }
}
