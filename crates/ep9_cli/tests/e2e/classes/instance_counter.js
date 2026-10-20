var count = 0;
class Counted { constructor() { count++; this.id = count; } }
var a = new Counted(), b = new Counted(), c = new Counted();
print(a.id, b.id, c.id, count);
