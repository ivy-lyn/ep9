// A sloppy workload: no prologue, so the realm runs as sloppy code (--script-mode auto).
// Outside the class: a class body is always strict code.
function thisOfPlainCall() {
  return this;
}
class Benchmark {
  runIteration(i) {
    return 10 * (i + 1);
  }
  check() {
    if (thisOfPlainCall() !== globalThis) throw new Error("not sloppy");
  }
}
