print(hoisted(), typeof laterVar, laterVar);
function hoisted() { return "hoisted"; }
var laterVar = 1;
print(laterVar);
function outer() { return inner(); function inner() { return "inner hoisted"; } }
print(outer());
