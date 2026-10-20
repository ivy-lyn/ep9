var vals = [null, undefined, 0, -0, "", "0", false, NaN, 1, "1"];
for (var i = 0; i < vals.length; i++) {
  var row = "";
  for (var j = 0; j < vals.length; j++) row += vals[i] === vals[j] ? "T" : ".";
  print(row);
}
