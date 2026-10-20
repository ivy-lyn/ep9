function inf(n) { return inf(n + 1) + 1; }
var caught = 0;
try { inf(0); } catch (e) { caught++; print(e instanceof RangeError, e.name); }
try { inf(0); } catch (e) { caught++; print(e instanceof RangeError); }
print("alive", caught);
function ok(n) { return n === 0 ? "fine" : ok(n - 1); }
print(ok(1000));
