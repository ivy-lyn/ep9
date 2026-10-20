// T0 dispatch kernel "branchy" (H15 ADR): data-dependent branches on a xorshift32 stream, so
// about half of them mispredict. It checks its own output.
function run(n, seed) {
  let x = seed | 0;
  let a = 0;
  let b = 0;
  let c = 0;
  for (let i = 0; i < n; i++) {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    if (x & 1) a = (a + 3) | 0;
    else b = (b + 5) | 0;
    if (x & 2) c = c ^ i;
    if ((x & 12) === 4) a = (a - b) | 0;
    else if ((x & 12) === 8) b = (b + c) | 0;
    else c = (c + 1) | 0;
  }
  return (a ^ b ^ c) | 0;
}

let total = 0;
for (let r = 1; r <= 8; r++) total = (total * 31 + run(1500000, r * 2654435761)) | 0;
const want = 1090221937;
if (total !== want) throw new Error("t0-branchy: got " + total + ", want " + want);
print(total);
