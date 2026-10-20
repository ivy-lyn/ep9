var s = 0;
for (var i = 1; i <= 10; i++) s += i;
var w = 0, j = 0;
while (j < 5) { w += j; j++; }
var d = 0, k = 10;
do { d += k; k--; } while (k > 7);
print(s, w, d);
var once = 0;
do { once++; } while (false);
print(once);
