function flat(a, out) { for (var i = 0; i < a.length; i++) { var x = a[i]; if (typeof x === "object" && x !== null && "length" in x) flat(x, out); else out.push(x); } return out; }
print(flat([1, [2, [3, [4, [5]]]], 6, [[7]]], []).join(" "));
