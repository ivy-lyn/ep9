// Each class has exactly one known hazard (name says which). Good* classes must qualify.
export interface Shared { v: number }
export interface Closed { w: number }
declare function sink(x: any): void;
declare function sinkUnknown(x: unknown): void;
declare function sinkObject(x: object): void;
function keep<T>(x: T): T { return x; }
function keepD<T extends Shared>(x: T): T { return x; }

export class Good { a = 1; b: string; constructor(b: string) { this.b = b; } sum(): number { return this.a + this.b.length; } }
export class GoodClosed implements Closed { w = 2; }
export class GoodChild extends Good { c = true; }

export class BadCastAny { x = 1; }
export class BadNonNull { x = 1; }
export class BadAnyFlow { x = 1; }
export class BadDynKey { x = 1; y = 2; }
export class BadDelete { x?: number = 1; }
export class BadDefine { x = 1; }
export class BadProto { x = 1; }
export class BadLeak { x: number; constructor() { register(this); this.x = 1; } }
export class BadLeakMethod { x: number; constructor() { this.init(); this.x = 1; } init() { } }
export class BadEscapeAny { x = 1; }
export class BadEscapeUnknown { x = 1; }
export class BadEscapeObject { x = 1; }
export class BadEscapeShared implements Shared { v = 1; }
export class BadGeneric { x = 1; }
export class BadGenericShared implements Shared { v = 3; }
export class BadThrow { x = 1; }
export class BadBase extends Error { x = 1; }
export class BadIndex { [k: string]: number; x = 1; }
export class BadAccessorBase { get x(): number { return 1; } }
export class BadAccessorChild extends BadAccessorBase { override get x(): number { return 2; } }
export class BadOptional { x?: number; }
export class BadObjLitEscape { x = 1; }
export class GoodCasted { x = 1; }
export class BadStructural { x = 1; }

function register(o: object) { sink(o); }

export function use(s: string, k: string, cl: Closed) {
  const a = JSON.parse(s) as BadCastAny;                 // r1 as-unchecked (any -> C)
  const m: BadNonNull | null = Math.random() ? new BadNonNull() : null;
  const n = m!;                                          // r1 nonnull
  const f: BadAnyFlow = JSON.parse(s);                   // r1 any-flows-in
  const d = new BadDynKey(); const dv = (d as any as Record<string, number>)[k]; // casts too
  const d2 = new BadDynKey(); const dv2 = d2[k as "x" | "y"]; // constant-typed key (ok)
  const del = new BadDelete(); delete del.x;              // r3 delete
  const def = new BadDefine(); Object.defineProperty(def, "x", { value: 2 }); // r3
  (BadProto.prototype as any).y = 1;                     // r3 (via cast; prototype assign)
  sink(new BadEscapeAny());                              // r5 any
  sinkUnknown(new BadEscapeUnknown());                   // r5 unknown
  sinkObject(new BadEscapeObject());                     // r5 object
  const sh: Shared = { v: 2 }; const sh2: Shared = new BadEscapeShared(); // r5 shared interface
  const cl2: Closed = new GoodClosed();                  // closed interface: ok
  keep(new BadGeneric());                                // r5 generic-T
  keepD(new BadGenericShared());                         // r5 via constraint Shared (shared)
  if (s === "t") throw new BadThrow();                   // r5 throw
  const o = { inner: new BadObjLitEscape() }; sink(o);   // escape through object literal (nested)
  const g = new Good("x"); const gc = new GoodChild("y"); const up = gc as Good; // upcast: r1 as-up on Good/GoodChild
  const gcast = new GoodCasted(); const back = gcast as GoodCasted;   // no-op cast
  const st: BadStructural = { x: 5 };                    // object literal typed as a class (structural)
  return [a, n, f, dv, dv2, def, sh, sh2, cl2, o, g, up, back, st, cl];
}

export function goodFn(a: number, b: string): number { let c = a + b.length; const g = new Good(b); return c + g.a; }
export function badParamFn(a: any): number { return 1; }
export function badLocalFn(a: number): number { const m = new Map<string, number>(); return a + m.size; }
export function badCastFn(a: number): number { return (a as unknown as string).length; }
export function nnFn(a: number | null): number { return a!; }
