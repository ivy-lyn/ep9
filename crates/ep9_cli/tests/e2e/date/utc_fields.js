// Date.UTC, getTime and the UTC getters and setters: values that do not depend on the
// local time zone.
var t = Date.UTC(2024, 1, 29, 23, 59, 58, 999);
var d = new Date(t);
print(t, d.getTime(), d.valueOf() === t);
print(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), d.getUTCDay());
print(d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(), d.getUTCMilliseconds());
// Overflowing fields carry into the next unit.
print(Date.UTC(2023, 12, 1) === Date.UTC(2024, 0, 1), Date.UTC(2024, 0, 0) === Date.UTC(2023, 11, 31));
print(Date.UTC(99), Date.UTC(100), Date.UTC(-1, 0, 1), Date.UTC(275760, 8, 13), Date.UTC(275760, 8, 14));
d.setUTCMilliseconds(1000);
print(d.toISOString());
d.setUTCHours(48, 30);
print(d.toISOString(), d.getUTCDay());
d.setUTCMonth(1, 29);
print(d.toISOString());
d.setUTCFullYear(2023);
print(d.toISOString());
var n = new Date(NaN);
print(n.getUTCDate(), n.setUTCSeconds(5), n.setUTCFullYear(1999), n.toISOString());
// Day numbers around the epoch and the leap rules of 1900, 2000 and 2100.
var days = [];
for (var y = 1899; y <= 2101; y += 101) {
  days.push((Date.UTC(y + 1, 0, 1) - Date.UTC(y, 0, 1)) / 864e5);
}
print(days.join(), Date.UTC(1900, 1, 29) === Date.UTC(1900, 2, 1), Date.UTC(2000, 1, 29) === Date.UTC(2000, 2, 1));
print(new Date(-1).getUTCFullYear(), new Date(-1).getUTCMilliseconds(), new Date(-864e5).getUTCDay());
