// B25: the program itself reaches a module-private, never-escaping object through node:inspector
// (no human debugger, no flags) and retypes its field. So "no alias / no escape" facts need a fuse
// on "an inspector session exists", not only static escape analysis.
import { Session } from 'node:inspector/promises';

const holder = (() => {
  class Secret { v = 1; }          // never exported, instance never passed anywhere
  const s = new Secret();
  return () => [typeof s.v, s.v];  // only a reader closure escapes
})();

const before = holder();
const session = new Session();
session.connect();
await session.post('Runtime.enable');
const { result: proto } = await session.post('Runtime.evaluate', { expression: 'Object.prototype' });
const { objects } = await session.post('Runtime.queryObjects', { prototypeObjectId: proto.objectId });
await session.post('Runtime.callFunctionOn', {
  objectId: objects.objectId,
  functionDeclaration: 'function () { for (const o of this) { if (o && o.constructor && o.constructor.name === "Secret") o.v = "str"; } }',
});
session.disconnect();
console.log(`B25  | non-escaping Secret instance keeps int field v           | before=${before} after=${holder()}`);
