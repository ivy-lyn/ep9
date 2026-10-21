// Statistics of the PERF-NOISE speed-gate rule (docs/plan/10-quality.md section 3), shared by
// tools/bench/workloads/t0-dispatch.mjs (class S) and tools/bench/cross/run.mjs (class M).
//
//   median of the paired ratios          -> the gate value
//   order-statistic interval (ranks)     -> 95% interval of the median (n = 10: ranks 2 and 9)
//   side of a threshold                  -> "above" | "below" | "marginal" (interval straddles it)
//   IQR > 5% of the median               -> void (the host was not quiet)
//   geomean                              -> summary over kernels
//
// Tests: node --test tools/bench/cross/run.test.mjs tools/bench/workloads/t0-dispatch.test.mjs

/** The void rule: IQR of the paired ratios above this share of the median (classes S and M). */
export const VOID_IQR = 0.05;

export function sorted(xs) {
  return [...xs].sort((a, b) => a - b);
}

export function median(xs) {
  const s = sorted(xs);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

/** Quantile with linear interpolation between order statistics (type 7). */
export function quantile(xs, q) {
  const s = sorted(xs);
  const h = (s.length - 1) * q;
  const lo = Math.floor(h);
  return s[lo] + (h - lo) * ((s[Math.min(lo + 1, s.length - 1)] ?? s[lo]) - s[lo]);
}

/**
 * 1-based ranks (k, n + 1 - k) of the order-statistic interval for the median with coverage
 * >= 95%: the largest k with 1 - 2 P(Bin(n, 1/2) <= k - 1) >= 0.95. n = 31 gives (10, 22),
 * n = 15 (4, 12), n = 10 (2, 9), n = 7 (1, 7), as in the PERF-NOISE table.
 * null when no such interval exists: for n < 6 even (1, n) covers only 1 - 2^(1-n) (n = 5: 93.75%).
 */
export function medianRanks(n) {
  // P(Bin(n, 1/2) <= j), summed in floating point (n is small).
  const cdf = (j) => {
    let c = 1;
    let sum = 0;
    for (let i = 0; i <= j; i++) {
      sum += c;
      c = (c * (n - i)) / (i + 1);
    }
    return sum / 2 ** n;
  };
  if (1 - 2 * cdf(0) < 0.95) return null;
  let k = 1;
  while (k + 1 <= n && 1 - 2 * cdf(k) >= 0.95) k++;
  return [k, n + 1 - k];
}

/**
 * The verdict of a list of paired ratios against `threshold` (10-quality section 3):
 * - side "above": the whole interval is above the threshold; "below": the whole interval is
 *   below it; "marginal": the interval contains it; "insufficient data": n < 6 (no interval).
 * - void: the IQR is more than 5% of the median, so the host was not quiet.
 */
export function verdict(ratios, threshold = 1) {
  const s = sorted(ratios);
  const ranks = medianRanks(s.length);
  const med = median(s);
  const iqr = quantile(s, 0.75) - quantile(s, 0.25);
  // No 95% interval (n < 6): no interval and no side.
  const interval = ranks && [s[ranks[0] - 1], s[ranks[1] - 1]];
  const side = !interval
    ? "insufficient data"
    : interval[0] > threshold ? "above" : interval[1] < threshold ? "below" : "marginal";
  return {
    n: s.length,
    median: med,
    interval,
    ranks,
    iqrPct: (100 * iqr) / med,
    void: iqr / med > VOID_IQR,
    threshold,
    side,
  };
}

export function geomean(xs) {
  return Math.exp(xs.reduce((a, x) => a + Math.log(x), 0) / xs.length);
}
