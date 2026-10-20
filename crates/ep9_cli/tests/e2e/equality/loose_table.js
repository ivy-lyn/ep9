var vals = [null, undefined, 0, "", "0", false, NaN, "1", 1, true];
var names = ["null", "undef", "0", "''", "'0'", "false", "NaN", "'1'", "1", "true"];
for (var i = 0; i < vals.length; i++) {
  var row = names[i] + ":";
  for (var j = 0; j < vals.length; j++) row += vals[i] == vals[j] ? " T" : " .";
  print(row);
}
