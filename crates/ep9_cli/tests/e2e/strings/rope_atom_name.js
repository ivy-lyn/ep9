// A rope that becomes an atom without a flatten (a String object in a JSON.stringify
// replacer list interns its rope), then the same text as a name that eval and with look up.
// The name lookup reads the first unit of the atom to test for a private name (#x); that read
// must work on a rope (M4-P1 STR1 F3).
var a = "abcdefgh", b = "ijklmnoz";
var w = new String(a + b);
print(JSON.stringify({}, [w]));
function f() { eval("var " + a + b + " = 5;"); with ({}) { return eval(a + b); } }
print(f());
var o = {};
o[a + b] = 1;
print(JSON.stringify(o, [new String(a + b)]));
with (o) { print(eval(a + b)); }
