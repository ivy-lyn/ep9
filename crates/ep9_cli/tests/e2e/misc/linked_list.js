function Node(v, next) { this.v = v; this.next = next; }
var list = null;
for (var i = 1; i <= 10; i++) list = new Node(i, list);
var rev = null;
while (list) { var n = list.next; list.next = rev; rev = list; list = n; }
var out = [];
for (var p = rev; p; p = p.next) out.push(p.v);
print(out.join(" "));
