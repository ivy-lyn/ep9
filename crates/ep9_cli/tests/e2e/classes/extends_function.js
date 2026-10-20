function Old(x) { this.x = x; }
Old.prototype.getX = function () { return this.x; };
class New extends Old { constructor(x) { super(x * 2); } }
print(new New(5).getX(), new New(1) instanceof Old);
