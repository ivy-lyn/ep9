//! `Math.hypot` (H36 row `Math.hypot`): a port of V8 `MathHypot`, IEEE operations only, so
//! the result has the same bits as node.
//!
//! Ported for ep9 from V8 `src/builtins/math.tq` (`MathHypot`, its general path; main at
//! f8008f466663464cf3f14474da2c8c91b1be1fcb, 2026-10-27):
//!
//! ```text
//! Copyright 2019 the V8 project authors. All rights reserved. Use of this
//! source code is governed by a BSD-style license that can be found in the
//! LICENSE file.
//! ```
//!
//! Licence text: `THIRD-PARTY-LICENSE` ("V8"). The general path normalizes by the largest
//! |value| and adds the squares with Kahan compensation. V8 also has `FastMathHypot` for up to
//! three arguments; it does the same operations in the same order, so it gives the same bits.
//! ep9 change: the caller does `ToNumber` on each argument (in order) and passes the numbers.

/// `Math.hypot(...values)` for the numbers `values` (already `ToNumber`ed): +∞ if one value is
/// ±∞, else NaN if one is NaN, else +0 if all are ±0 (also for no values), else
/// `sqrt(Σ (|v|/max)^2) * max` with Kahan summation, as V8 computes it.
#[inline(never)]
pub fn hypot_js(values: &[f64]) -> f64 {
    let mut one_arg_is_nan = false;
    let mut max = 0.0f64;
    for &value in values {
        if value.is_nan() {
            one_arg_is_nan = true;
        } else {
            let abs_value = value.abs();
            if abs_value > max {
                max = abs_value;
            }
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    } else if one_arg_is_nan {
        return f64::NAN;
    } else if max == 0.0 {
        return 0.0;
    }
    // Kahan summation, each value normalized by the largest one (no overflow).
    let mut sum = 0.0f64;
    let mut compensation = 0.0f64;
    for &value in values {
        // V8 keeps 0 for a NaN entry; there is none here.
        let n = value.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot_special_values() {
        assert_eq!(hypot_js(&[]).to_bits(), 0);
        assert_eq!(hypot_js(&[-0.0]).to_bits(), 0);
        assert_eq!(hypot_js(&[-0.0, -0.0]).to_bits(), 0);
        assert_eq!(hypot_js(&[f64::NAN, f64::NEG_INFINITY]), f64::INFINITY);
        assert!(hypot_js(&[f64::NAN, 1.0]).is_nan());
        assert_eq!(hypot_js(&[-3.0]), 3.0);
        assert_eq!(hypot_js(&[3.0, 4.0]), 5.0);
        assert_eq!(hypot_js(&[1e308, 1e308]), 1.4142135623730951e308);
        assert_eq!(hypot_js(&[5e-324, 5e-324]), 5e-324);
    }

    /// Values from node v24.22.1 (V8 13.6.233.17-node.58).
    #[test]
    fn hypot_matches_node() {
        // node -p "Math.hypot(1, 2, 3)" and so on
        assert_eq!(hypot_js(&[1.0, 2.0, 3.0]), 3.741657386773941);
        assert_eq!(hypot_js(&[0.1, 0.2, 0.3, 0.4]), 0.5477225575051662);
        assert_eq!(hypot_js(&[1.0, 1.0, 1.0]), 1.7320508075688772);
    }
}
