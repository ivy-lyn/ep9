class MathX {
  static square(x) { return x * x; }
  static twice(x) { return MathX.square(x) * 2; }
}
print(MathX.square(4), MathX.twice(3), typeof MathX.prototype.square);
