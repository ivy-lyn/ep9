// T0 dispatch kernel "call" (H15 ADR): JS-to-JS calls with 0 to 3 arguments, a recursive
// function, a closure and a function passed as a value. It checks its own output.
function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}
function add3(a, b, c) {
  return a + b + c;
}
function twice(f, x) {
  return f(x, x, 1) + f(x, 1, x);
}
function counter(k) {
  let n = k;
  return function () {
    n = (n + 1) & 1023;
    return n;
  };
}
function zero() {
  return 0;
}

function run(n) {
  const next = counter(n & 7);
  let s = 0;
  for (let i = 0; i < n; i++) {
    s = (s + add3(i & 15, s & 7, 1)) % 1000003;
    s = (s + twice(add3, i & 3) + next() + zero()) % 1000003;
  }
  return s;
}

let total = fib(25);
for (let r = 0; r < 4; r++) total = (total * 31 + run(400000 + r)) % 1000003;
const want = 577789;
if (total !== want) throw new Error("t0-call: got " + total + ", want " + want);
print(total);
