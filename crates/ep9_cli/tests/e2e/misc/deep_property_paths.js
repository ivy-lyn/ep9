var root = {};
var cur = root;
for (var i = 0; i < 50; i++) { cur.next = { depth: i }; cur = cur.next; }
var n = 0;
for (var p = root.next; p; p = p.next) n++;
print(n, root.next.next.next.depth);
