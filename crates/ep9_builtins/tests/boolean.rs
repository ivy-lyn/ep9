//! Boolean (§20.3): the constructor, `Boolean.prototype.toString` and `valueOf`.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn boolean_called_converts() {
    assert_eq!(
        common::run_both(
            r#"print(Boolean(), Boolean(0), Boolean(-0), Boolean(''), Boolean('a'), Boolean({}), Boolean(NaN), Boolean(null), Boolean(Symbol()), Boolean(new Boolean(false)))"#
        ),
        lines(&["false false false false true true false false true true"])
    );
}

#[test]
fn boolean_constructed_is_an_object() {
    assert_eq!(
        common::run_both(
            r#"var b = new Boolean(false); print(typeof b, b ? 'truthy' : 'falsy', b.valueOf(), b == false, b === false, b instanceof Boolean)"#
        ),
        lines(&["object truthy false true false true"])
    );
}

#[test]
fn boolean_to_string_and_value_of() {
    assert_eq!(
        common::run_both(
            r#"print(true.toString(), false.toString(), new Boolean(true).toString(), new Boolean(1).valueOf(), Boolean.prototype.valueOf(), Boolean.prototype.toString(), new Boolean(1) + '')"#
        ),
        lines(&["true false true true false false true"])
    );
}

#[test]
fn boolean_methods_check_this() {
    assert_eq!(
        common::run_both(
            r#"function callOn(t, f, a) { Object.prototype.__f = f; try { return t.__f(a); } finally { delete Object.prototype.__f; } } var r = []; for (var t of [1, 'true', {}, new Number(1), false, new Boolean(true)]) { try { r.push(callOn(t, Boolean.prototype.toString)); } catch (e) { r.push(e.name); } try { r.push(callOn(t, Boolean.prototype.valueOf)); } catch (e) { r.push(e.name); } } print(r.join())"#
        ),
        lines(&[
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,false,false,true,true"
        ])
    );
}

#[test]
fn boolean_subclass() {
    assert_eq!(
        common::run_both(
            r#"class B extends Boolean {} var b = new B(0); print(b instanceof B, b instanceof Boolean, b.valueOf(), b.toString())"#
        ),
        lines(&["true true false false"])
    );
}

#[test]
fn boolean_functions_have_length_and_name() {
    assert_eq!(
        common::run_both(
            r#"print(Boolean.length, Boolean.name, Boolean.prototype.toString.length, Boolean.prototype.valueOf.name, Boolean.prototype.constructor === Boolean)"#
        ),
        lines(&["1 Boolean 0 valueOf true"])
    );
}
