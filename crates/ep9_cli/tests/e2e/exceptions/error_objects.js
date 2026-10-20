var e = new Error("boom");
print(e.message, e.name, e instanceof Error, typeof e);
var t = new TypeError("bad type");
print(t.message, t.name, t instanceof TypeError, t instanceof Error, t instanceof RangeError);
var r = RangeError("no new");
print(r.name, r.message, r instanceof RangeError);
print(new Error().message === "", new Error(undefined).message === "", new Error(5).message);
print(e.toString(), t.toString(), new SyntaxError("s").toString());
