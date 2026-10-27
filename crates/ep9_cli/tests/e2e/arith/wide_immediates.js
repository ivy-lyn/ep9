// Immediates that need the Wide (16-bit) or ExtraWide (32-bit) prefix: LdaSmi and the Smi
// binary ops. T0 runs the int32 cases in the prefix handlers and everything else in the
// cold stepper (M3-P4 FIXES); both must give the JS result.
function f(x) {
  return [
    x + 1000, x + 100000, x - 1000, x - 100000, x % 1000, x % 1000003,
    x | 4096, x ^ 70000, x & 1023, x & 65535, x * 1000, x / 1000, x ** 2,
    x << 1000, x >> 1000, x >>> 1000, 1000, -1000, 100000, -100000,
  ];
}
const inputs = [0, -0, 1, -1, 7, -7, 999, 1000, -1000, 2147483647, -2147483648, 2147483000,
  -2147483000, 1.5, -2.5, NaN, Infinity, "12", "x", null, undefined, true, 123456789];
function show(v) { return Object.is(v, -0) ? "-0" : String(v); }
// Run it often enough for the feedback vector and the warm paths.
let out;
for (let round = 0; round < 12; round++) {
  out = inputs.map((x) => f(x).map(show).join(","));
}
for (const line of out) print(line);
// An object accumulator: valueOf runs once per operation, in order.
let log = [];
const o = { valueOf() { log.push("v"); return 5000; } };
print(f(o).map(show).join(","), log.length);
// A loop that counts with wide immediates.
let s = 0;
for (let i = 0; i < 3000; i++) s = (s + (i & 1023) * 31) % 1000003;
print(s);
