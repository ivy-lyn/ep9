class Point {
  constructor(x, y) { this.x = x; this.y = y; }
  norm2() { return this.x * this.x + this.y * this.y; }
  toString() { return "P(" + this.x + "," + this.y + ")"; }
}
var p = new Point(3, 4);
print(p.norm2(), "" + p, p instanceof Point, typeof Point);
