// Writes number_tostring.golden: Number::toString (ECMA-262 §6.1.6.1.20) as node prints it,
// for ep9_builtins/tests/json.rs (`number_to_string_matches_the_node_golden`), which makes the
// same doubles from the same generator and compares ep9's strings.
//
//   node crates/ep9_cli/tests/e2e/json/number_tostring_golden.mjs \
//     > crates/ep9_cli/tests/e2e/json/number_tostring.golden
//   node crates/ep9_cli/tests/e2e/json/number_tostring_golden.mjs --block <n>  (one block, in full)
//
// Part 1: 100,000 doubles from xorshift64 (seed below), in 1,000 blocks of 100. One line per
// block: "block <n> <FNV-1a 32 of the 100 strings, each followed by LF> <the last string>".
// Part 2: 200 doubles that lie exactly halfway between two shortest digit strings (step 5: the
// even one wins), found by a fixed search: "tie <bits as 16 hex digits> <string>".
// Not a .js file, so the e2e runner does not run it.
const MASK = (1n << 64n) - 1n;
let state = 0x9e3779b97f4a7c15n;
function next() {
  state ^= (state << 13n) & MASK;
  state ^= state >> 7n;
  state ^= (state << 17n) & MASK;
  return state;
}
const view = new DataView(new ArrayBuffer(8));
function fromBits(b) {
  view.setBigUint64(0, b);
  return view.getFloat64(0);
}
function toBits(x) {
  view.setFloat64(0, x);
  return view.getBigUint64(0);
}
// Value i of part 1 (ep9_builtins/tests/json.rs `golden_double` is the same).
function goldenDouble(i) {
  const r = next();
  const s = next();
  switch (i % 4) {
    case 0: // any bits: NaN, infinities, subnormals, the whole range
      return fromBits(r);
    case 1: { // binary exponents -80..59: where two shortest strings can tie
      const biased = 1023n - 80n + (s % 140n);
      return fromBits((r & (1n << 63n)) | (biased << 52n) | (r & ((1n << 52n) - 1n)));
    }
    case 2: { // q / 2^t: exact values with few decimal digits
      const q = r >> (11n + (s % 40n));
      const t = Number((s >> 8n) % 27n);
      return Number(q) / 2 ** t;
    }
    default: { // integers up to 2^53, scaled by 2^0..2^15
      const q = r >> (11n + (s % 53n));
      return Number(q) * 2 ** Number((s >> 8n) % 16n);
    }
  }
}
function fnv(strings) {
  let h = 0x811c9dc5;
  for (const str of strings) {
    for (const c of str + '\n') {
      h ^= c.charCodeAt(0);
      h = Math.imul(h, 0x01000193) >>> 0;
    }
  }
  return h >>> 0;
}
const only = process.argv[2] === '--block' ? Number(process.argv[3]) : -1;
const out = [];
for (let b = 0; b < 1000; b++) {
  const strings = [];
  for (let i = 0; i < 100; i++) strings.push(String(goldenDouble(b * 100 + i)));
  if (b === only) {
    strings.forEach((s) => console.log(s));
  }
  out.push(`block ${b} ${fnv(strings).toString(16).padStart(8, '0')} ${strings[99]}`);
}
if (only >= 0) process.exit(0);
// Part 2: x = q / 2^t (q odd, t 1..26) is a tie when its exact decimal value has exactly one
// digit more than String(x) and ends in 5, and both neighbours with that many digits read
// back as x.
state = 0x2545f4914f6cdd1dn;
let ties = 0;
while (ties < 200) {
  const t = 1n + (next() % 26n);
  const q = (next() >> (11n + (next() % 40n))) | 1n;
  const x = Number(q) / 2 ** Number(t);
  let exact = (q * 5n ** t).toString();
  const str = String(x);
  const k = str.replace(/e.*/, '').replace(/[-.]/g, '').replace(/^0+/, '').length;
  if (exact.length !== k + 1 || !exact.endsWith('5')) continue;
  const lower = BigInt(exact.slice(0, k));
  const upper = lower + 1n;
  if (upper.toString().length !== k) continue;
  // x = exact * 10^-t: the neighbours are lower and upper times 10^(1 - t).
  const back = (d) => Number(`${d}e${1n - t}`);
  if (back(lower) !== x || back(upper) !== x) continue;
  out.push(`tie ${toBits(x).toString(16).padStart(16, '0')} ${str}`);
  ties++;
}
console.log(out.join('\n'));
