function kind(v) { if (typeof v === "number") return "n"; if (typeof v === "string") return "s"; if (typeof v === "object") return v === null ? "null" : "o"; return "?"; }
print(kind(1), kind("a"), kind(null), kind({}), kind(undefined), kind(kind));
