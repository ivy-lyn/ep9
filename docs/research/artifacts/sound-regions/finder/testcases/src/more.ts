declare function sinkAll(...xs: any[]): void;
declare function sink2(x: any): void;
export class Reach { x = 1; }
export class Holder { r = new Reach(); }
export class SpreadEsc { x = 1; }
export class CloneHole { x = 1; m(): number { return this.x; } }
export class KeysEsc { x = 1; }
export function more(): number {
  sink2(new Holder());                    // Holder escapes; Reach reachable through its field
  const arr = [new SpreadEsc()]; sinkAll(...arr);   // spread argument (known gap)
  const c: CloneHole = structuredClone(new CloneHole()); // clone typed as CloneHole has no prototype (known gap)
  return Object.keys(new KeysEsc()).length + c.x;     // Object.keys(o: {}) -> escape to {}
}
