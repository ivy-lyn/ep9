function abc(a, b, c) {}
function dflt(a, b = 1, c) {}
function rest(a, ...r) {}
var anon = function () {};
var arrow = (x) => x;
print(abc.length, dflt.length, rest.length, abc.name, anon.name, arrow.name, arrow.length);
