// Breaker probes for "verified region" facts. Each probe prints one line: id | fact attacked | observed.
// Run: node probes.mjs   (ESM => strict mode everywhere, like class / module code)
const out = [];
const P = (id, fact, observed) => out.push(`${id.padEnd(4)} | ${fact.padEnd(58)} | ${observed}`);

// B01 Reflect.construct with a foreign newTarget: `this` in C's ctor gets F.prototype (layout/proto fact)
{
  class C { x = 1; constructor() { this.y = 2; } }
  function F() {}
  F.prototype = { tag: 'F' };
  const o = Reflect.construct(C, [], F);
  P('B01', 'C instance has proto C.prototype', `proto===F.prototype:${Object.getPrototypeOf(o) === F.prototype} keys=${Object.keys(o)}`);
}
// B02 base-ctor return override: derived fields land on a foreign object (incl. Proxy, incl. #private)
{
  class Base { constructor(o) { return o; } }
  class Stamp extends Base { #brand = 1; x = 7; static has(o) { return #brand in o; } }
  const foreign = { a: 1 };
  new Stamp(foreign);
  const px = new Proxy({}, {});
  new Stamp(px);
  P('B02', 'Stamp fields live only on Stamp-shaped objects', `foreign keys=${Object.keys(foreign)} #brand on plain=${Stamp.has(foreign)} on proxy=${Stamp.has(px)} proto plain=${Object.getPrototypeOf(foreign) === Object.prototype}`);
}
// B03 setPrototypeOf(D, X) changes what super() calls (GetSuperConstructor reads D.[[Prototype]])
{
  class A { constructor() { this.from = 'A'; } }
  class X { constructor() { this.from = 'X'; this.extra = 1; } }
  class D extends A { constructor() { super(); } }
  Object.setPrototypeOf(D, X);
  const d = new D();
  P('B03', 'super() in D calls A', `from=${d.from} keys=${Object.keys(d)} proto=D.prototype:${Object.getPrototypeOf(d) === D.prototype}`);
}
// B04 [[Set]] field init (useDefineForClassFields=false lowering: this.x = v) hits proto setter / Proxy in chain; [[Define]] does not
{
  let hits = [];
  class SetStyle { constructor() { this.x = 1; } }          // what tsc emits for `x = 1` with [[Set]]
  class DefStyle { x = 1; }                                 // [[Define]]
  Object.defineProperty(Object.prototype, 'x', { set(v) { hits.push('setter'); }, get() { return 'proto-get'; }, configurable: true });
  const s = new SetStyle(), d = new DefStyle();
  P('B04a', 'field x is an own data slot after ctor', `[[Set]]: own=${Object.hasOwn(s, 'x')} s.x=${s.x} hits=${hits}; [[Define]]: own=${Object.hasOwn(d, 'x')} d.x=${d.x}`);
  delete Object.prototype.x;
  hits = [];
  class Q { constructor() { this.q = 1; } }
  Object.setPrototypeOf(Q.prototype, new Proxy({}, { set(t, k, v, r) { hits.push('trap:' + String(k)); return true; } }));
  const q = new Q();
  P('B04b', 'this.q = 1 in ctor creates own slot', `own=${Object.hasOwn(q, 'q')} hits=${hits}`);
}
// B05 class C.prototype is non-writable, non-configurable (S fact); its CONTENTS are not
{
  class C { m() { return 1; } }
  const d = Object.getOwnPropertyDescriptor(C, 'prototype');
  let threw = false; try { C.prototype = {}; } catch { threw = true; }
  const mDesc = Object.getOwnPropertyDescriptor(C.prototype, 'm');
  C.prototype.m = () => 'patched';
  P('B05', 'C.prototype identity fixed; C.prototype.m fixed', `prototype writable=${d.writable} configurable=${d.configurable} assignThrows(strict)=${threw}; m writable=${mDesc.writable} configurable=${mDesc.configurable} after patch m()=${new C().m()}`);
}
// B06 method called with a foreign this
{
  class Counter { n = 0; inc() { return ++this.n; } }
  const r1 = Counter.prototype.inc.call({ n: 'a' });
  const r2 = Reflect.apply(Counter.prototype.inc, { n: 1.5 }, []);
  P('B06', 'this in Counter.inc is a Counter with int n', `call({n:'a'})=${r1} apply({n:1.5})=${r2}`);
}
// B07 ToPrimitive in an operator runs user code that mutates a prototype mid-expression
{
  class V { get() { return 1; } }
  const v = new V();
  const evil = { valueOf() { V.prototype.get = () => 'changed'; return 1; } };
  const a = v.get();
  const s = a + evil;           // user code runs here
  const b = v.get();
  P('B07', 'v.get target unchanged across `a + evil`', `before=${a} after=${b} sum=${s}`);
}
// B08 holes read through Array.prototype / Object.prototype indexed props
{
  const arr = [1, , 3];
  Array.prototype[1] = 'from-proto';
  const r = arr[1];
  delete Array.prototype[1];
  const a2 = [1, 2, 3]; a2.length = 5;
  P('B08', 'arr[i] for 0<=i<length is a number (packed)', `hole via proto=${r}; after length=5: a2[4]=${a2[4]} 4 in a2=${4 in a2}`);
}
// B09 Symbol.species: map() on a region array allocates via user constructor
{
  class MyArr extends Array { static get [Symbol.species]() { return function () { return { custom: true }; }; } }
  const m = MyArr.from([1, 2]);
  let r; try { r = m.map(x => x); } catch (e) { r = 'threw ' + e.constructor.name; }
  const plain = [1, 2];
  plain.constructor = { [Symbol.species]: function () { return { viaOwnCtor: true }; } };
  let r2; try { r2 = plain.map(x => x); } catch (e) { r2 = 'threw ' + e.constructor.name; }
  P('B09', 'arr.map returns a packed Array', `subclass species -> ${JSON.stringify(r)}; own constructor prop -> ${JSON.stringify(r2)}`);
}
// B10 arguments aliasing: sloppy mapped arguments alias formals; strict does not
{
  const sloppy = new Function('a', 'arguments[0] = "str"; return a;');
  function strict(a) { arguments[0] = 'str'; return a; }   // module code is strict
  P('B10', 'param a keeps its int value', `sloppy -> ${sloppy(1)} ; strict -> ${strict(1)}`);
}
// B11 generator suspension: facts before yield are void after resume
{
  const box = { v: 1 };
  function* g() { const before = typeof box.v; yield; return `${before}->${typeof box.v}`; }
  const it = g(); it.next(); box.v = 'str';
  P('B11', 'box.v int across yield', it.next().value);
}
// B12 preventExtensions during construction: [[Define]] field init throws (JS semantics to KEEP)
{
  class Base { constructor() { Object.preventExtensions(this); } }
  class D extends Base { x = 1; }
  let r; try { new D(); r = 'no throw'; } catch (e) { r = 'TypeError:' + (e instanceof TypeError); }
  P('B12', 'field define on new object always succeeds', r);
}
// B13 exception mid-construction leaves an escaped, partly built object
{
  let leaked;
  class C { a = 1; b = (leaked = this, (() => { throw new Error('x'); })()); c = 3; }
  try { new C(); } catch {}
  P('B13', 'every C instance has fields a,b,c', `leaked keys=${Object.keys(leaked)} instanceof C=${leaked instanceof C}`);
}
// B14 structuredClone drops the prototype, keeps own data props
{
  class Pt { constructor() { this.x = 1; this.y = 2; } len() { return 0; } }
  const c = structuredClone(new Pt());
  P('B14', 'value from outside is a Pt', `clone proto===Pt.prototype:${Object.getPrototypeOf(c) === Pt.prototype} keys=${Object.keys(c)}`);
}
// B15 ArrayBuffer detach / resize inside a callee voids hoisted bounds checks
{
  const ta = new Float64Array(8);
  const n0 = ta.length;
  (function callee() { ta.buffer.transfer(); })();   // ES2024 ArrayBuffer.prototype.transfer detaches
  const rab = new ArrayBuffer(64, { maxByteLength: 64 });
  const tb = new Float64Array(rab);
  const m0 = tb.length; rab.resize(8);
  P('B15', 'typed array length fixed inside the loop', `transfer: ${n0}->${ta.length} ta[0]=${ta[0]} ; resize: ${m0}->${tb.length}`);
}
// B16 monkey-patching builtins
{
  const s0 = Math.sqrt(4);
  const orig = Math.sqrt; Math.sqrt = () => 'patched';
  const s1 = Math.sqrt(4); Math.sqrt = orig;
  P('B16', 'Math.sqrt is the builtin', `${s0} -> ${s1}`);
}
// B17 subclass from outside overrides a method and still passes instanceof
{
  class Shape { area() { return 1; } }
  const Outside = class extends Shape { area() { return 'override'; } };
  const o = new Outside();
  P('B17', 'x instanceof Shape => x.area is Shape.prototype.area', `instanceof=${o instanceof Shape} area()=${o.area()}`);
}
// B18 cross-realm array: Array.isArray true, instanceof false, different proto
{
  const vm = await import('node:vm');
  const a = vm.runInNewContext('[1,2,3]');
  P('B18', 'an Array has proto %Array.prototype% of this realm', `isArray=${Array.isArray(a)} instanceof Array=${a instanceof Array} proto same=${Object.getPrototypeOf(a) === Array.prototype}`);
}
// B19 super.m() reads HomeObject.[[Prototype]] at call time
{
  class A { m() { return 'A'; } }
  class B extends A { m() { return super.m(); } }
  Object.setPrototypeOf(B.prototype, { m() { return 'swapped'; } });
  P('B19', 'super.m in B is A.prototype.m', new B().m());
}
// B20 Error.prepareStackTrace runs user code on .stack read; CallSite.getThis leaks receivers of SLOPPY frames only
{
  let seen = [];
  const prev = Error.prepareStackTrace;
  Error.prepareStackTrace = (e, cs) => { for (const c of cs) { const t = c.getThis(); if (t && t.secret) seen.push('leak'); } return 'custom'; };
  const sloppyHolder = new Function('o', 'return (function () { return new Error("x").stack; }).call(o);');
  const r1 = sloppyHolder({ secret: 1 });
  const strictHolder = function (o) { return (function () { return new Error('x').stack; }).call(o); };
  const r2 = strictHolder({ secret: 1 });
  Error.prepareStackTrace = prev;
  P('B20', 'a non-escaping object stays unreachable', `stack=${r1}/${r2} getThis leaks=${seen.length} (sloppy frames only)`);
}
// B21 delete / defineProperty / freeze on an instance
{
  class C { x = 1; y = 2; }
  const a = new C(), b = new C(), c = new C();
  delete a.x; Object.defineProperty(b, 'x', { get() { return 'acc'; } }); Object.freeze(c);
  let w; try { c.x = 5; w = 'no throw'; } catch (e) { w = 'TypeError'; }
  P('B21', 'C instances keep data slots x,y writable', `after delete a.x=${a.x}; accessor b.x=${b.x}; frozen write ${w}`);
}
// B22 using / Symbol.dispose: hidden call at scope exit (ES2026 explicit resource management, if supported)
{
  let r = 'n/a (no `using` support)';
  try {
    const f = new Function('box', '"use strict"; { using h = { [Symbol.dispose]() { box.v = "str"; } }; } return typeof box.v;');
    r = f({ v: 1 });
  } catch (e) { r = 'unsupported: ' + e.constructor.name; }
  P('B22', 'box.v int across a block end', r);
}
// B23 Object.prototype pollution changes `in` and missing-property reads
{
  class C { x = 1; }
  const c = new C();
  const before = 'z' in c;
  Object.prototype.z = 'polluted';
  const after = 'z' in c; const val = c.z;
  delete Object.prototype.z;
  P('B23', "'z' in c is false (absence fact)", `${before} -> ${after} c.z=${val}`);
}
// B24 Symbol.hasInstance overrides instanceof
{
  class C {}
  Object.defineProperty(C, Symbol.hasInstance, { value: () => true });
  P('B24', 'x instanceof C => x has C layout', `({}) instanceof C = ${({}) instanceof C}`);
}

console.log(out.join('\n'));
