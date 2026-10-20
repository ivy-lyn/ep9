function build(d) { return d === 0 ? null : { left: build(d - 1), right: build(d - 1), v: d }; }
function count(t) { return t === null ? 0 : 1 + count(t.left) + count(t.right); }
function sum(t) { return t === null ? 0 : t.v + sum(t.left) + sum(t.right); }
var t = build(10);
print(count(t), sum(t));
