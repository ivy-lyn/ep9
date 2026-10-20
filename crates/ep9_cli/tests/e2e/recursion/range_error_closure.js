var depth = 0;
var rec = function () { depth++; return rec(); };
try { rec(); } catch (e) { print(e instanceof RangeError, depth > 1000); }
depth = 0;
try { rec(); } catch (e) { print("again", e instanceof RangeError); }
