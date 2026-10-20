var base = { greet: function () { return "base " + this.id; }, kind: "base" };
function Derived(id) { this.id = id; }
Derived.prototype = base;
var d = new Derived(7);
print(d.greet(), d.kind, "kind" in d);
d.kind = "own";
print(d.kind, base.kind);
delete d.kind;
print(d.kind);
