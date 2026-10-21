//! Symbol (§20.4): `Symbol(d)`, `new Symbol`, `Symbol.for`/`keyFor`, the well-known
//! symbols, and `Symbol.prototype.toString`, `valueOf`, `[@@toPrimitive]`, `[@@toStringTag]`.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn acceptance_symbol_for_and_new_symbol() {
    assert_eq!(
        common::run_both(
            r#"print(Symbol.for('a')===Symbol.for('a')); try { new Symbol(); print('no throw'); } catch (e) { print(e instanceof TypeError); }"#
        ),
        lines(&["true", "true"])
    );
}

#[test]
fn symbol_called_makes_new_symbols() {
    assert_eq!(
        common::run_both(
            r#"print(typeof Symbol(), Symbol() === Symbol(), Symbol('x') === Symbol('x'), Symbol('x').toString(), Symbol().toString(), Symbol(undefined).toString(), Symbol(null).toString(), Symbol(1).toString(), Symbol('').toString())"#
        ),
        lines(&["symbol false false Symbol(x) Symbol() Symbol() Symbol(null) Symbol(1) Symbol()"])
    );
}

#[test]
fn symbol_converts_its_description_with_to_string() {
    assert_eq!(
        common::run_both(
            r#"print(Symbol({ toString: function () { return 'z'; } }).toString()); try { Symbol(Symbol()); } catch (e) { print(e.name); }"#
        ),
        lines(&["Symbol(z)", "TypeError"])
    );
}

#[test]
fn symbol_is_a_constructor_that_throws() {
    assert_eq!(
        common::run_both(
            r#"class S extends Symbol {} try { new S(); print('no throw'); } catch (e) { print(e.name); } print(Symbol.length, Symbol.name)"#
        ),
        lines(&["TypeError", "0 Symbol"])
    );
}

#[test]
fn symbol_for_and_key_for() {
    assert_eq!(
        common::run_both(
            r#"var k = Symbol.for('k'); print(k.toString(), Symbol.keyFor(k), Symbol.keyFor(Symbol('k')), Symbol.keyFor(Symbol.iterator), Symbol.for(1) === Symbol.for('1'), Symbol.for() === Symbol.for('undefined'), Symbol.for.length, Symbol.keyFor.length)"#
        ),
        lines(&["Symbol(k) k undefined undefined true true 1 1"])
    );
}

#[test]
fn key_for_needs_a_symbol() {
    assert_eq!(
        common::run_both(
            r#"var r = []; for (var v of ['x', 1, undefined, {}, Object(Symbol())]) { try { Symbol.keyFor(v); r.push('ok'); } catch (e) { r.push(e.name); } } print(r.join())"#
        ),
        lines(&["TypeError,TypeError,TypeError,TypeError,TypeError"])
    );
}

#[test]
fn well_known_symbols() {
    assert_eq!(
        common::run_both(
            r#"print(Symbol.asyncIterator.toString(), Symbol.hasInstance.toString(), Symbol.isConcatSpreadable.toString(), Symbol.iterator.toString(), Symbol.match.toString(), Symbol.matchAll.toString(), Symbol.replace.toString(), Symbol.search.toString(), Symbol.species.toString(), Symbol.split.toString(), Symbol.toPrimitive.toString(), Symbol.toStringTag.toString(), Symbol.unscopables.toString(), Symbol.dispose.toString(), Symbol.asyncDispose.toString())"#
        ),
        lines(&[
            "Symbol(Symbol.asyncIterator) Symbol(Symbol.hasInstance) Symbol(Symbol.isConcatSpreadable) Symbol(Symbol.iterator) Symbol(Symbol.match) Symbol(Symbol.matchAll) Symbol(Symbol.replace) Symbol(Symbol.search) Symbol(Symbol.species) Symbol(Symbol.split) Symbol(Symbol.toPrimitive) Symbol(Symbol.toStringTag) Symbol(Symbol.unscopables) Symbol(Symbol.dispose) Symbol(Symbol.asyncDispose)"
        ])
    );
}

#[test]
fn well_known_symbols_are_read_only() {
    assert_eq!(
        common::run_both(
            r#"var i = Symbol.iterator; Symbol.iterator = 1; delete Symbol.iterator; var keys = ''; for (var k in Symbol) keys += k; print(Symbol.iterator === i, typeof Symbol.iterator, keys === '', Symbol.keyFor(i))"#
        ),
        lines(&["true symbol true undefined"])
    );
}

#[test]
fn symbol_prototype_methods() {
    assert_eq!(
        common::run_both(
            r#"var s = Symbol('p'); print(Object(s).toString(), s.toString(), Object(s).valueOf() === s, s.valueOf() === s, s[Symbol.toPrimitive]() === s, Object(s)[Symbol.toPrimitive]('number') === s, Symbol.prototype[Symbol.toStringTag], s[Symbol.toStringTag])"#
        ),
        lines(&["Symbol(p) Symbol(p) true true true true Symbol Symbol"])
    );
}

#[test]
fn symbol_wrappers_convert_to_their_symbol() {
    assert_eq!(
        common::run_both(
            r#"var s = Symbol('w'); var o = Object(s); print(typeof o, o == s, o === s, String(o.valueOf())); try { o + ''; } catch (e) { print(e.name); } try { +o; } catch (e) { print(e.name); }"#
        ),
        lines(&["object true false Symbol(w)", "TypeError", "TypeError"])
    );
}

#[test]
fn symbol_prototype_methods_check_this() {
    assert_eq!(
        common::run_both(
            r#"function callOn(t, f, a) { Object.prototype.__f = f; try { return t.__f(a); } finally { delete Object.prototype.__f; } } var r = []; for (var t of ['x', {}, 1, new String('s'), Symbol.prototype]) { try { r.push(callOn(t, Symbol.prototype.toString)); } catch (e) { r.push(e.name); } try { r.push(callOn(t, Symbol.prototype.valueOf)); } catch (e) { r.push(e.name); } try { r.push(callOn(t, Symbol.prototype[Symbol.toPrimitive])); } catch (e) { r.push(e.name); } } print(r.join())"#
        ),
        lines(&[
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError"
        ])
    );
}

#[test]
fn symbol_prototype_function_names() {
    assert_eq!(
        common::run_both(
            r#"var tp = Symbol.prototype[Symbol.toPrimitive]; print(tp.name, tp.length, Symbol.prototype.toString.name, Symbol.prototype.valueOf.length, Symbol.prototype.constructor === Symbol)"#
        ),
        lines(&["[Symbol.toPrimitive] 1 toString 0 true"])
    );
}

/// `get Symbol.prototype.description` is definition 6 of src/symbol.rs. An accessor property
/// needs P3-ACC: after the ACC merge, `PropDef::accessor(PropKey::Str("description"), Some(6),
/// None)` goes on `%Symbol.prototype%` after `valueOf`, and this test runs. The expected lines
/// are node 24's.
#[test]
#[ignore = "needs P3-ACC accessors: install `description` in src/symbol.rs after the ACC merge"]
fn description_getter() {
    assert_eq!(
        common::run_both(
            r#"print(Symbol('x').description, Symbol().description, Symbol.iterator.description, Object(Symbol('w')).description, Symbol('').description === '', Symbol.for('f').description); try { Symbol.prototype.description; print('no throw'); } catch (e) { print(e.name); } var s = Symbol('q'); s.description = 1; print(s.description)"#
        ),
        lines(&["x undefined Symbol.iterator w true f", "TypeError", "q"])
    );
    assert_eq!(
        common::run_both(
            r#"'use strict'; var s = Symbol('q'); try { s.description = 1; print('no throw'); } catch (e) { print(e.name); }"#
        ),
        lines(&["TypeError"])
    );
}
