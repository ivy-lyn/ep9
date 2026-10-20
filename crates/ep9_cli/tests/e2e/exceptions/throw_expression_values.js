function t(v) { try { throw v; } catch (e) { return e === v; } }
var o = {};
print(t(1), t("s"), t(o), t(null), t(undefined), t(NaN) === false);
