function a(x) {
  return function b(y) {
    return function c(z) {
      return function d(w) { return x + y + z + w; };
    };
  };
}
print(a(1)(2)(3)(4), a("a")("b")("c")("d"));
