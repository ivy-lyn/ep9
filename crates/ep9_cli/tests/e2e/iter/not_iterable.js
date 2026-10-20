var vals = [{}, 5, null, undefined, true];
for (var i = 0; i < vals.length; i++) {
  try { for (var x of vals[i]) {} print("iterated"); } catch (e) { print(i, e instanceof TypeError); }
}
try { var [a] = {}; } catch (e) { print("destructure", e instanceof TypeError); }
try { f(...5); } catch (e) { print("spread", e instanceof TypeError); }
function f() {}
