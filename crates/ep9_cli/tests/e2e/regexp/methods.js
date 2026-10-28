// RegExp methods (M4-P3 REGEXP1; plan v9 §5.4 acceptance): a literal is an object, and test
// runs the match.
print(typeof /a+/g, /a+/g.test("aa"));
