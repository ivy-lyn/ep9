var s = 0;
for (var i = 0; i < 100; i++) {
  s += i;
  if (i % 10 === 0) s += 0.5;
}
print(s);
var p = 1;
for (var j = 0; j < 40; j++) p *= 2;
print(p, p / 3, p | 0);
var t = 0.1;
for (var k = 0; k < 10; k++) t += 0.1;
print(t, t === 1.1);
