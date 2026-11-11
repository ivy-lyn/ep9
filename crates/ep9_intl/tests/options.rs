//! GetBooleanOrStringNumberFormatOption (ECMA-402 9.2.13) through a host function: no W1
//! builtin reads an option with it yet (Intl.NumberFormat `useGrouping` is a later wave), so
//! the test calls it from JS with the stringValues and fallback of `useGrouping` («
//! "min2", "auto", "always", "true", "false" », "auto"). The expected values are node
//! v26.13.0's `new Intl.NumberFormat("en", { useGrouping: v }).resolvedOptions().useGrouping`
//! for the same values, except `true`: the operation gives true, and NumberFormat then
//! makes it "always" (node prints "always").

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_intl::options::{BoolOrString, get_boolean_or_string_number_format_option};
use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};

/// `useGrouping(options)`: the result as a string ("true!" and "false!" for the booleans).
fn use_grouping<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let Some(o) = args.get(0).as_object() else {
        return Err(cx.throw_type_error("an object"));
    };
    let r = get_boolean_or_string_number_format_option(
        cx,
        o,
        "useGrouping",
        &["min2", "auto", "always", "true", "false"],
        BoolOrString::Str("auto".to_string()),
    )?;
    let text = match r {
        BoolOrString::True => "true!".to_string(),
        BoolOrString::False => "false!".to_string(),
        BoolOrString::Str(s) => s,
    };
    Ok(cx.new_string(&text)?.into())
}

#[test]
fn the_strings_true_and_false_give_the_fallback() {
    let src = r#"var r = [];
for (var v of ["true", "false", { toString() { return "false"; } }, "min2", "always", true, false, 0, "", undefined]) r.push(useGrouping({ useGrouping: v }));
try { useGrouping({ useGrouping: "x" }); } catch (e) { r.push(e.name); }
var out = r.join(" ");"#;
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("compiles");
    let mut iso = Isolate::new(IsolateConfig {
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.register_host_function("useGrouping", 1, use_grouping);
    let got = iso
        .with_cx(|cx| {
            cx.run_script(unit, "options.js")?;
            let v = cx.global_binding("out")?;
            Ok(v.map(|v| cx.display(v)))
        })
        .map_err(|e| e.message);
    assert_eq!(
        got,
        Ok(Some(
            "auto auto auto min2 always true! false! false! false! auto RangeError".to_string()
        ))
    );
}
