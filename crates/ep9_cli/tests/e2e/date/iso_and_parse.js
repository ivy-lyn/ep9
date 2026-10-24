// toISOString, toUTCString, toJSON and Date.parse of the ISO and UTC forms (no local time).
var values = [0, 1, -1, 951782400000, 1.7e12 + 5, 8.64e15, -8.64e15, -62198755200000, 253402300800000];
for (var i = 0; i < values.length; i++) {
  var d = new Date(values[i]);
  print(d.toISOString(), d.toUTCString(), Date.parse(d.toISOString()) === values[i]);
}
print(new Date(0).toJSON(), new Date(NaN).toJSON());
print(Date.parse('2024-02-29'), Date.parse('2024-02'), Date.parse('2024'), Date.parse('+002024-02-29T12:00:00Z'));
print(Date.parse('2024-02-29T12:30:45.5+05:30'), Date.parse('2024-02-29T24:00Z'), Date.parse('2024-02-29T23:59:59.999Z'));
print(Date.parse('Thu, 29 Feb 2024 12:00:00 GMT'), Date.parse(new Date(1.7e12).toUTCString()));
print(Date.parse('-000000-01-01T00:00Z'), Date.parse('2024-02-29T25:00Z'), Date.parse('+275760-09-13T00:00:00.001Z'));
print(new Date(8.64e15 + 1).getTime(), new Date('2024-02-29T00:00:00Z').getTime(), new Date(new Date(77)).getTime());
var bad;
try { new Date(NaN).toISOString(); } catch (e) { bad = e.name; }
print(bad, String(new Date(NaN)), new Date(NaN).toUTCString());
print(Object.prototype.toString.call(new Date(0)), typeof Date.now(), Date.prototype.toGMTString === Date.prototype.toUTCString);
