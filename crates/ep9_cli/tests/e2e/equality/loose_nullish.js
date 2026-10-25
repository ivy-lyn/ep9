// `x == null`, `x != undefined` and the other loose compares with null or undefined on one
// side: T0's TestEqual answers them without the cold path (M3-P4 FIXES). No conversion runs:
// an object's valueOf and toString are never called.
let calls = 0;
const obj = { valueOf() { calls++; return 0; }, toString() { calls++; return ""; } };
const values = [null, undefined, 0, -0, 1, NaN, "", "0", "null", false, true, obj, [], [0],
  Symbol("s"), function f() {}, 1.5, -1];
const names = ["null", "undefined", "0", "-0", "1", "NaN", "''", "'0'", "'null'", "false",
  "true", "obj", "[]", "[0]", "sym", "fn", "1.5", "-1"];
function eq(a, b) { return a == b; }
function ne(a, b) { return a != b; }
function eqNull(a) { return a == null; }
function nullEq(a) { return null == a; }
function neUndef(a) { return a != undefined; }
// Enough calls for the feedback vector and the warm paths.
for (let round = 0; round < 3; round++) {
  const out = [];
  for (let i = 0; i < values.length; i++) {
    const v = values[i];
    out.push(names[i] + ":" + [eq(v, null), eq(null, v), eq(v, undefined), eq(undefined, v),
      ne(v, null), eqNull(v), nullEq(v), neUndef(v)].map(Number).join(""));
  }
  if (round === 2) print(out.join(" "));
}
print("conversions:", calls);
// A hole read as undefined, and void.
const arr = [1, , 3];
print(arr[1] == null, void 0 == null, arr[5] != undefined);
