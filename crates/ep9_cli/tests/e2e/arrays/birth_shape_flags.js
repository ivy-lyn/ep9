// An array-literal site keeps the shape its arrays reach (allocation-site feedback, M5-P2
// ELEM2). A shape that is not extensible, sealed, frozen or a dictionary shape must never
// become the birth shape of the site's next arrays (Codex M5-P2 r1 elem2 1).
function check(label, make, spoil) {
  for (var i = 0; i < 50; i++) make();
  var a = make();
  spoil(a);
  var bad = 0;
  for (var i = 0; i < 50; i++) {
    var b = make();
    b.x = 1;
    if (!Object.isExtensible(b) || Object.isSealed(b) || b.x !== 1) bad++;
    try {
      if (b.push(2) !== b.length || b[b.length - 1] !== 2) bad++;
      b[0] = 0.25;
      if (b[0] !== 0.25) bad++;
    } catch (e) {
      bad++;
    }
  }
  var c = make();
  print(label, Object.isExtensible(c), Object.isSealed(c), Object.isFrozen(c), bad);
}

check("preventExtensions", function () { return [1]; }, function (a) {
  Object.preventExtensions(a);
  a[0] = 0.5;
});
check("seal", function () { return [1, 2]; }, function (a) {
  Object.seal(a);
  a[0] = 1.5;
});
check("freeze", function () { return [1, 2, 3]; }, function (a) {
  Object.freeze(a);
});
check("read-only length", function () { return [4, 5]; }, function (a) {
  Object.defineProperty(a, "length", { writable: false });
  a[0] = 0.5;
});
check("accessor element", function () { return [6]; }, function (a) {
  Object.defineProperty(a, "0", { get: function () { return 1; }, configurable: true });
});
check("empty, preventExtensions", function () { return []; }, function (a) {
  Object.preventExtensions(a);
});
check("named property deleted", function () { return [7]; }, function (a) {
  a.y = 1;
  delete a.y;
  a[0] = 0.5;
});
check("many named properties", function () { return [8]; }, function (a) {
  for (var k = 0; k < 40; k++) a["p" + k] = k;
  for (var k = 0; k < 40; k++) delete a["p" + k];
  a[0] = 0.5;
});

// A prototype's unique shape is not a birth shape either (Codex M5-P2 r2 1): the site gave
// it to fresh literals, which were not in prototype mode and kept the prototype alive; a
// later Object.create of one of them failed a debug assert.
(function () {
  function f() { return [1]; }
  for (var i = 0; i < 50; i++) f();
  var p = f();
  var child = Object.create(p);
  p[0] = 0.5;
  var bad = 0;
  for (var i = 0; i < 50; i++) {
    var q = f();
    q.x = 1;
    var c = Object.create(q);
    if (Object.getPrototypeOf(c) !== q || c.x !== 1 || c[0] !== 1 || c.length !== 1) bad++;
    q.x = 2;
    delete q[0];
    if (c.x !== 2 || c[0] !== undefined || child[0] !== 0.5) bad++;
  }
  print("prototype", child[0], p.length, bad);
})();
