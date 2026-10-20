// gc-stress: off
var s = 0;
for (var i = 0; i < 300000; i++) s = (s + i * 7) % 1000003;
print(s);
