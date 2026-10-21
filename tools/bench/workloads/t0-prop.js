// T0 dispatch kernel "prop" (H15 ADR): named property loads and stores, method calls on class
// instances and array element access, on a small fixed set of objects. It checks its output.
class Vec {
  constructor(x, y) {
    this.x = x;
    this.y = y;
  }
  dot(o) {
    return this.x * o.x + this.y * o.y;
  }
  bump(d) {
    this.x = (this.x + d) & 1023;
    return this;
  }
}

function run(n) {
  const pts = [];
  for (let i = 0; i < 64; i++) pts.push({ x: i, y: i * 2, z: 0, v: new Vec(i, 64 - i) });
  let s = 0;
  for (let k = 0; k < n; k++) {
    const p = pts[k & 63];
    const q = pts[(k * 7) & 63];
    p.z = (p.z + q.x) & 1023;
    s = (s + p.x * p.y + p.z + p.v.dot(q.v)) % 1000003;
    q.v.bump(p.z & 3);
  }
  return s;
}

let total = 0;
for (let r = 0; r < 4; r++) total = (total * 31 + run(600000 + r)) % 1000003;
const want = 354392;
if (total !== want) throw new Error("t0-prop: got " + total + ", want " + want);
print(total);
