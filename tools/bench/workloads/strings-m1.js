// H19 string speed record (plan v9 V9-D17): a named micro set, recorded as a value, not a gate.
// Each micro prints "<name> <checksum>" and throws on a wrong checksum (node gives the values). `var ONLY = "<name>";`
// before this file runs one micro (the per-micro counts of the H19 record).
if (typeof print !== "function") globalThis.print = console.log;

const micros = {
  // s += part: CONS cells, one flatten at the end (H19 ropes, V9-D13).
  "concat-rope"() {
    let s = "";
    for (let i = 0; i < 20000; i++) s += "abcdefgh" + i;
    let h = 0;
    for (let i = 0; i < s.length; i += 97) h = (h * 31 + s.charCodeAt(i)) | 0;
    return s.length + ":" + h;
  },
  // s[i] and charCodeAt over a flat string: the single-character atoms.
  "index-chars"() {
    const s = "The quick brown fox jumps over the lazy dog 0123456789".repeat(20);
    let n = 0;
    for (let r = 0; r < 20; r++) {
      for (let i = 0; i < s.length; i++) {
        if (s[i] === "o") n++;
        n += s.charCodeAt(i) & 1;
      }
    }
    return n;
  },
  // o[rope] twice per key: ToPropertyKey atomizes the rope, which then forwards (THIN).
  "rope-keys"() {
    const o = {};
    for (let i = 0; i < 200; i++) o["property-name-" + i + "-suffix"] = i;
    let sum = 0;
    for (let r = 0; r < 50; r++) {
      for (let i = 0; i < 200; i++) {
        const k = "property-name-" + i + "-suffix";
        sum += o[k];
        o[k] = o[k] + 1;
      }
    }
    return sum;
  },
  // o[i] on a plain object: number keys (the number-string cache, H19-Q6).
  "number-keys"() {
    const o = {};
    for (let i = 0; i < 500; i++) o[i * 3] = i;
    let sum = 0;
    for (let r = 0; r < 40; r++) {
      for (let i = 0; i < 500; i++) sum += o[i * 3];
    }
    return sum;
  },
  // "" + n and String(n): ToString of numbers (the number-string cache, H19-Q6).
  "number-strings"() {
    let len = 0;
    for (let i = 0; i < 20000; i++) {
      len += ("" + (i % 300)).length;
      len += String((i % 50) + 0.5).length;
    }
    return len;
  },
  // === and < on strings built at run time (atoms compare by bits; ropes by a walk).
  "compare"() {
    const words = [];
    for (let i = 0; i < 300; i++) words.push("word-" + ((i * 7919) % 300) + "-tail-of-the-word");
    let lt = 0, eq = 0;
    for (let r = 0; r < 10; r++) {
      for (let i = 1; i < words.length; i++) {
        if (words[i - 1] < words[i]) lt++;
        if (words[i] === "word-" + ((i * 7919) % 300) + "-tail-of-the-word") eq++;
      }
    }
    return lt + ":" + eq;
  },
  // substring, indexOf and slice over a long text (flat copies today).
  "substrings"() {
    let s = "";
    for (let i = 0; i < 200; i++) s += "line " + i + " of the text;";
    let n = 0;
    for (let i = 0; i < 2000; i++) {
      const at = (i * 37) % (s.length - 40);
      const sub = s.substring(at, at + 30);
      n += sub.indexOf("of") + sub.slice(5, 9).length;
    }
    return n;
  },
};

// Expected results (node v24.22.1). A wrong result throws, so it cannot pass as a fast run.
const want = {
  "concat-rope": "248890:-1229793094",
  "index-chars": "11200",
  "rope-keys": "1240000",
  "number-keys": "4990000",
  "number-strings": "128630",
  "compare": "1740:2990",
  "substrings": "27475",
};

const only = typeof ONLY === "string" ? ONLY : null;
for (const name of Object.keys(micros)) {
  if (only !== null && name !== only) continue;
  const got = String(micros[name]());
  if (got !== want[name]) throw new Error("strings-m1 " + name + ": got " + got + ", want " + want[name]);
  print(name + " " + got);
}
