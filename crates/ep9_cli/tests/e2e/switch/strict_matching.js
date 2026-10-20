function f(v) { switch (v) { case 1: return "num"; case "1": return "str"; case true: return "bool"; case null: return "null"; case undefined: return "undef"; default: return "other"; } }
print(f(1), f("1"), f(true), f(null), f(undefined), f(NaN), f(0));
