// exit: 1
// error-at: 7:9
// stderr: Uncaught TypeError: bad thing
// stderr:     at inner (
function outer() { return inner(); }
function inner() {
  throw new TypeError("bad thing");
}
outer();
