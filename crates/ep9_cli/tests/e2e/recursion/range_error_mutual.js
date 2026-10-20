function ping(n) { return pong(n + 1); }
function pong(n) { return ping(n + 1); }
var r;
try { ping(0); r = "no"; } catch (e) { r = e instanceof RangeError; }
print(r);
