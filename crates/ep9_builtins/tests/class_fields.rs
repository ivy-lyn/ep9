//! JS-level tests of class fields, private names, private methods and static blocks
//! (owner P2-CLS-VM), run through BytecodeGen (P2-CLS-CG). The expected lines come from node
//! v24 (`print` = `console.log`), except where a comment names the spec text that node does
//! not follow. Every check runs with and without GC stress (`common::run_both`).

mod common;

use common::run_both;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Public and private fields, a computed field key, static fields. The class-fields
/// initializer key on `C` and the private names are not listed by
/// `Object.getOwnPropertyNames`, `Object.keys` or for-in (P2-CLS-VM acceptance).
#[test]
fn fields_are_hidden_from_key_lists() {
    assert_eq!(
        run_both(
            r#"class C {
  a = 1; #b = 2; static s = 3; static #t = 4;
  ['c' + 'd'] = 5;
  getB() { return this.#b; } setB(v) { this.#b = v; }
  static has(o) { return #b in o; } static t() { return C.#t; }
}
var o = new C();
print(o.a, o.getB(), o.cd, C.s, C.t());
o.setB(9); print(o.getB());
print(C.has(o), C.has({}), C.has(C));
print(Object.getOwnPropertyNames(o).join());
print(Object.getOwnPropertyNames(C).join());
print(Object.keys(o).join(), Object.keys(C).join());
var ks = []; for (var k in o) ks.push(k); print(ks.join());
ks = []; for (var k in C) ks.push(k); print(ks.join());
var d = Object.getOwnPropertyDescriptor(o, 'a');
print(d.value, d.writable, d.enumerable, d.configurable);"#
        ),
        lines(&[
            "1 2 5 3 4",
            "9",
            "true false false",
            "a,cd",
            "length,name,prototype,has,t,s",
            "a,cd s",
            "a,cd",
            "s",
            "1 true true true",
        ])
    );
}

/// PrivateGet, PrivateSet and `#x in` errors, and a second add through a constructor
/// return override (node v24 texts).
#[test]
fn private_name_errors() {
    assert_eq!(
        run_both(
            r#"class C {
  #x = 1;
  static get(o) { return o.#x; } static set(o) { o.#x = 2; } static has(o) { return #x in o; }
}
var tries = [() => C.get({}), () => C.set({}), () => C.has(1), () => C.get(1), () => C.get(undefined)];
for (var f of tries) { try { f(); print('no error'); } catch (e) { print(e.constructor.name + ': ' + e.message); } }
class Base { constructor(o) { return o; } }
class D extends Base { #y = 1; static y(o) { return o.#y; } }
var obj = {};
new D(obj); print(D.y(obj));
try { new D(obj); } catch (e) { print(e.constructor.name + ': ' + e.message); }
class M extends Base { #m() { return 'm'; } static m(o) { return o.#m(); } }
var obj2 = {};
new M(obj2); print(M.m(obj2));
try { new M(obj2); } catch (e) { print(e.constructor.name); }
try { M.m({}); } catch (e) { print(e.constructor.name); }"#
        ),
        lines(&[
            "TypeError: Cannot read private member #x from an object whose class did not declare it",
            "TypeError: Cannot write private member #x to an object whose class did not declare it",
            "TypeError: Cannot use 'in' operator to search for '#x' in 1",
            "TypeError: Cannot read private member #x from an object whose class did not declare it",
            "TypeError: Cannot read properties of undefined (reading '#x')",
            "1",
            "TypeError: Cannot initialize #y twice on the same object",
            "m",
            "TypeError",
            "TypeError",
        ])
    );
}

/// Private methods, a getter and a static private method: the brand, `#m in o`, the brand
/// check of a call on an object without it; private methods are not own properties.
#[test]
fn private_methods_and_accessors() {
    assert_eq!(
        run_both(
            r#"class C {
  #v = 10;
  #m() { return this.#v + 1; }
  get #g() { return this.#v * 2; }
  static #sm() { return 'sm'; }
  run() { return [this.#m(), this.#g, C.#sm()].join(); }
  static isC(o) { return #m in o; }
  static callM(o) { return o.#m(); }
}
var c = new C();
print(c.run(), C.isC(c), C.isC({}), C.isC(C));
print(Object.getOwnPropertyNames(c).length, Object.getOwnPropertyNames(C.prototype).join());
try { C.callM({}); } catch (e) { print(e.constructor.name); }"#
        ),
        lines(&[
            "11,20,sm true false false",
            "0 constructor,run",
            "TypeError"
        ])
    );
}

/// A private accessor as an assignment target (plain and compound): the setter runs.
#[test]
fn private_setter_assignment() {
    assert_eq!(
        run_both(
            r#"class C {
  #v = 10;
  get #g() { return this.#v; }
  set #g(x) { this.#v = x; }
  run() { this.#g = 3; this.#g += 4; return this.#g; }
}
print(new C().run());"#
        ),
        lines(&["7"])
    );
}

/// Static fields and static blocks run in source order with `this` = the class.
#[test]
fn static_fields_and_blocks_run_in_source_order() {
    assert_eq!(
        run_both(
            r#"var log = [];
class C {
  static a = (log.push('a'), 1);
  static { log.push('block1:' + this.a); }
  static b = (log.push('b'), this.a + 1);
  static { log.push('block2:' + C.b); }
  static #p = 7;
  static { log.push('p:' + C.#p); }
}
print(log.join());
print(C.a, C.b);"#
        ),
        lines(&["a,block1:1,b,block2:2,p:7", "1 2"])
    );
}

/// Fields of a derived class run when `super()` returns; `super.x` and arrows in field
/// initializers; a field initializer runs before parameter defaults (§10.2.2 step 6.b).
#[test]
fn derived_fields_run_after_super() {
    assert_eq!(
        run_both(
            r#"var order = [];
class B { constructor() { order.push('B ctor'); } get v() { return 'super-v'; } }
class D extends B {
  x = (order.push('D field x'), 1);
  y = super.v;
  z = (() => this.x + 1)();
  constructor() { order.push('D before super'); super(); order.push('D after super ' + this.x); }
}
var d = new D();
print(order.join('|'));
print(d.x, d.y, d.z);
class E extends B { w = 2; }
var e = new E();
print(e.w, Object.getOwnPropertyNames(e).join());
class F { #p = 1; constructor(a = this.#p) { this.a = a; } }
print(new F().a);
class G { f = this; }
var g = new G(); print(g.f === g);"#
        ),
        lines(&[
            "D before super|B ctor|D field x|D after super 1",
            "1 super-v 2",
            "2 w",
            "1",
            "true",
        ])
    );
}

/// Computed class field keys name anonymous functions (DefineKeyedOwnProperty bit 0; a
/// private field's name is its description `#f`); a named function and a non-computed key
/// keep the closure's own name.
#[test]
fn computed_field_keys_name_anonymous_functions() {
    assert_eq!(
        run_both(
            r#"var k = 'kk', s = Symbol('sd'), e = Symbol();
class C {
  [k] = function () {};
  [s] = () => 0;
  [e] = function () {};
  #f = function () {};
  n = function () {};
  named = function nm() {};
  static [k] = class {};
  getF() { return this.#f; }
}
var c = new C();
print(c.kk.name, c[s].name, '"' + c[e].name + '"', c.getF().name, c.n.name, c.named.name, C.kk.name);"#
        ),
        lines(&[r#"kk [sd] "" #f n nm kk"#])
    );
}

/// Bit 0 of DefineKeyedOwnPropertyInLiteral (P2-CLS-VM task 4, D15): computed object literal
/// keys name anonymous functions, classes and methods (no class-field codegen needed). The
/// last line follows ECMA-262 (ClassDefinitionEvaluation sets the name before the static
/// `name` method replaces it; test262 `*-init-fn-name-class.js` checks the same for
/// destructuring defaults); node v24 prints "string" there.
#[test]
fn computed_literal_keys_name_anonymous_functions() {
    assert_eq!(
        run_both(
            r#"var k = 'kk', s = Symbol('sd'), e = Symbol();
var lit = { [k]: function () {}, [s]: () => 1, [e]: function () {}, [1 + 1]: function () {},
  [k + 'm']() {}, x: function () {}, [k + 'n']: function nn() {}, [k + 'c']: class {} };
print(lit.kk.name, lit[s].name, '"' + lit[e].name + '"', lit[2].name, lit.kkm.name, lit.x.name,
  lit.kkn.name, lit.kkc.name);
var d = Object.getOwnPropertyDescriptor(lit.kk, 'name');
print(d.writable, d.enumerable, d.configurable);
var withStatic = { [k]: class { static name() {} } };
print(typeof withStatic.kk.name);"#
        ),
        lines(&[
            r#"kk [sd] "" 2 kkm x nn kkc"#,
            "false false true",
            "function"
        ])
    );
}

/// ClassDefinitionEvaluation step 8 (ECMA-262 §15.7.14): the heritage runs with the outer
/// PrivateEnvironment, so a `#x` there means the enclosing class's `#x` even when the class
/// being defined declares its own `#x` (a field, `#x in`, a method, a closure made in the
/// heritage and called later, a class nested in the heritage, two levels). The inner class
/// name stays in its TDZ during the heritage (last line).
#[test]
fn heritage_uses_the_outer_private_environment() {
    assert_eq!(
        run_both(
            r#"class B {} class Base1 {} class Base2 {}
class O {
  #x = B;
  #m() { return B; }
  field() { return class I extends this.#x { #x; }; }
  pick() { return class I extends (#x in this ? Base1 : Base2) { #x; }; }
  method() { return class I extends this.#m() { #m() {} }; }
  arrow() { const self = this; return class I extends (() => self.#x)() { #x; }; }
  nested() {
    return class I extends (class J { #x = 1; static has(o) { return #x in o; } }, this.#x) { #x; };
  }
  twoLevels() { return class H extends (class I extends this.#x { #x; }) { #x; }; }
  late() { let f; class I extends (f = () => this.#x, Object) { #x = 2; } return f() === B; }
  inner() { return class I extends this.#x { #x = 7; static read(o) { return o.#x; } }; }
}
var o = new O();
print(new (o.field())() instanceof B);
print(new (o.pick())() instanceof Base1);
print(new (o.method())() instanceof B);
print(new (o.arrow())() instanceof B);
print(new (o.nested())() instanceof B);
var H = o.twoLevels();
print(new H() instanceof B, H.prototype instanceof B);
var I = o.inner();
print(I.read(new I()), o.late());
try { I.read(o); print('no error'); } catch (e) { print(e.constructor.name); }
try { class I extends I {} } catch (e) { print(e.constructor.name); }"#
        ),
        lines(&[
            "true",
            "true",
            "true",
            "true",
            "true",
            "true true",
            "7 true",
            "TypeError",
            "ReferenceError",
        ])
    );
}
