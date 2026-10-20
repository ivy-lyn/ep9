var r = "none";
try { throw 1; } catch { r = "caught without binding"; }
print(r);
