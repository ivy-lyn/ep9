class Shape { area() { return 0; } describe() { return this.constructor.name + ":" + this.area(); } }
class Sq extends Shape { constructor(s) { super(); this.s = s; } area() { return this.s * this.s; } }
class Rect extends Shape { constructor(w, h) { super(); this.w = w; this.h = h; } area() { return this.w * this.h; } }
var shapes = [new Sq(3), new Rect(2, 5), new Shape()];
var out = [];
for (var i = 0; i < shapes.length; i++) out.push(shapes[i].describe());
print(out.join(" "));
