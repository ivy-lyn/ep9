// A workload whose check fails: the realm rejects (top.currentReject).
class Benchmark {
  runIteration() {
    return 1;
  }
  check() {
    throw new RangeError("bad result 42");
  }
}
