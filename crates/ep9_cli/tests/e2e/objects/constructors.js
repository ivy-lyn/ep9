function Point(x, y) { this.x = x; this.y = y; }
Point.prototype.add = function (o) { return new Point(this.x + o.x, this.y + o.y); };
Point.prototype.toString = function () { return "(" + this.x + "," + this.y + ")"; };
var p = new Point(1, 2).add(new Point(3, 4));
print(p.x, p.y, "" + p, p instanceof Point, p.constructor === Point);
