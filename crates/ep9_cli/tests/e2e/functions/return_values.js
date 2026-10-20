function none() {}
function early(x) { if (x) return "early"; return "late"; }
function obj() { return { a: 1 }; }
print(none(), early(1), early(0), obj().a);
