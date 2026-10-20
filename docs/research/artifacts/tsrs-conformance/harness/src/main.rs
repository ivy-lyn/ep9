//! test262 parse-conformance worker: tsrs_parser vs oxc_parser.
//!
//! usage: t262h worker <parsers> < tasks
//!   parsers: comma list of `tsrs`, `oxc` (default both)
//!   each stdin line: `id\tpath\truns` where runs is a string over {s,S,m}
//!     s = Script goal, sloppy; S = Script goal, "use strict" prefix; m = Module goal
//! stdout: `B\tid\tparser` before each parse (crash attribution), then one JSON result line per test.
//! Exit code 3 = a tsrs panic happened; the orchestrator restarts after that test (arena state may be poisoned).

use std::fmt::Write as _;
use std::io::{BufRead, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use serde_json::{Value, json};

#[cfg(feature = "tsrs")]
use tsrs_ast::{ExternalModuleIndicatorOptions, Kind, Node, SourceFileParseOptions};
#[cfg(feature = "tsrs")]
use tsrs_core::{P, ScriptKind, tspath::Path};
#[cfg(feature = "tsrs")]
use tsrs_diagnostics::{self as d, Category};

const STRICT_PREFIX: &str = "'use strict';\n";

#[cfg(feature = "tsrs")]
fn plain_js_binder_codes() -> Vec<i32> {
    // The binder half of tsrs_compiler/src/program.rs `plain_js_errors()`: the only binder diagnostics tsc reports
    // for a plain .js file (no checkJs).
    [
        &d::Cannot_redeclare_block_scoped_variable_0,
        &d::A_module_cannot_have_multiple_default_exports,
        &d::Another_export_default_is_here,
        &d::The_first_export_default_is_here,
        &d::Identifier_expected_0_is_a_reserved_word_at_the_top_level_of_a_module,
        &d::Identifier_expected_0_is_a_reserved_word_in_strict_mode_Modules_are_automatically_in_strict_mode,
        &d::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here,
        &d::X_constructor_is_a_reserved_word,
        &d::X_delete_cannot_be_called_on_an_identifier_in_strict_mode,
        &d::Code_contained_in_a_class_is_evaluated_in_JavaScript_s_strict_mode_which_does_not_allow_this_use_of_0_For_more_information_see_https_Colon_Slash_Slashdeveloper_mozilla_org_Slashen_US_Slashdocs_SlashWeb_SlashJavaScript_SlashReference_SlashStrict_mode,
        &d::Invalid_use_of_0_Modules_are_automatically_in_strict_mode,
        &d::Invalid_use_of_0_in_strict_mode,
        &d::A_label_is_not_allowed_here,
        &d::X_with_statements_are_not_allowed_in_strict_mode,
    ]
    .iter()
    .map(|m| m.code())
    .collect()
}

fn panic_msg(e: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

#[cfg(feature = "tsrs")]
fn tsrs_diag_str(diag: &tsrs_ast::Diagnostic) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "TS{}@{}: {}",
        diag.code(),
        diag.pos().map_or(-1_i64, i64::from),
        d::localize(diag.message(), diag.message_key(), diag.message_args())
    );
    s
}

#[cfg(feature = "tsrs")]
fn collect_regexes(n: P<Node>, out: &mut Vec<P<Node>>) {
    if n.kind() == Kind::RegularExpressionLiteral {
        out.push(n);
    }
    n.for_each_child(&mut |c| {
        collect_regexes(c, out);
        false
    });
}

#[cfg(feature = "tsrs")]
fn run_tsrs(text: &str, module: bool, bind_codes: &[i32]) -> Value {
    let t0 = Instant::now();
    let opts = SourceFileParseOptions {
        file_name: if module { "/t262/test.mjs".to_string() } else { "/t262/test.js".to_string() },
        path: Path::new(if module { "/t262/test.mjs".to_string() } else { "/t262/test.js".to_string() }),
        external_module_indicator_options: ExternalModuleIndicatorOptions { jsx: false, force: module },
    };
    let file = tsrs_parser::parse_source_file(opts, text, ScriptKind::JS);
    let parse_us = t0.elapsed().as_micros() as u64;

    let errs = |ds: &[P<tsrs_ast::Diagnostic>]| -> Vec<String> {
        ds.iter().filter(|x| x.category() == Category::Error).map(|x| tsrs_diag_str(x)).collect()
    };
    let perr = errs(file.diagnostics());
    let jserr = errs(file.js_diagnostics());
    let jsdoc_n = file.jsdoc_diagnostics().len();
    let non_error_n = file.diagnostics().iter().filter(|x| x.category() != Category::Error).count();

    // Regex validation, as tsrs_checker grammarchecks.rs check_grammar_regular_expression_literal does it.
    let mut rx_errs: Vec<String> = Vec::new();
    let mut rx_panic: Option<String> = None;
    {
        let mut regexes = Vec::new();
        collect_regexes(file.as_node(), &mut regexes);
        let src: &'static str = file.text();
        for r in regexes {
            let res = catch_unwind(AssertUnwindSafe(|| {
                let mut s = tsrs_scanner::Scanner::new();
                s.set_on_error(true);
                s.set_text(src);
                s.reset_token_state(r.pos().expect_source("regex pos"));
                s.scan();
                let mut v: Vec<String> = Vec::new();
                for e in s.take_errors() {
                    if e.message.category() == Category::Error {
                        v.push(format!("TS{}@{}: {}", e.message.code(), e.start, e.message.text()));
                    }
                }
                let _ = s.re_scan_slash_token(true);
                for e in s.take_errors() {
                    if e.message.category() == Category::Error {
                        v.push(format!("TS{}@{}: {}", e.message.code(), e.start, e.message.text()));
                    }
                }
                v
            }));
            match res {
                Ok(v) => rx_errs.extend(v),
                Err(e) => rx_panic = Some(panic_msg(&e)),
            }
        }
    }

    // Binder (standalone; always-strict in TS 7).
    let t1 = Instant::now();
    let bind_res = catch_unwind(AssertUnwindSafe(|| {
        tsrs_binder::bind_source_file(file);
        file.bind_diagnostics()
            .iter()
            .filter(|x| x.category() == Category::Error)
            .map(|x| (x.code(), tsrs_diag_str(x)))
            .collect::<Vec<_>>()
    }));
    let bind_us = t1.elapsed().as_micros() as u64;
    let (bind_all, bind_panic): (Vec<(i32, String)>, Option<String>) = match bind_res {
        Ok(v) => (v, None),
        Err(e) => (Vec::new(), Some(panic_msg(&e))),
    };
    let bind_f: Vec<&String> =
        bind_all.iter().filter(|(c, _)| bind_codes.contains(c)).map(|(_, s)| s).collect();

    json!({
        "perr": perr.len(),
        "jserr": jserr.len(),
        "jsdoc": jsdoc_n,
        "nonerr": non_error_n,
        "rx": rx_errs.len(),
        "bind": bind_all.len(),
        "bindf": bind_f.len(),
        "first": perr.first().or(jserr.first()),
        "first_js": jserr.first(),
        "first_rx": rx_errs.first(),
        "first_bind": bind_all.first().map(|x| &x.1),
        "first_bindf": bind_f.first(),
        "bind_codes": bind_all.iter().map(|x| x.0).collect::<Vec<_>>(),
        "rx_panic": rx_panic,
        "bind_panic": bind_panic,
        "us": parse_us,
        "bind_us": bind_us,
    })
}

#[cfg(feature = "oxc")]
fn run_oxc(text: &str, module: bool) -> Value {
    use oxc_allocator::Allocator;
    use oxc_parser::{ParseOptions, Parser};
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let st = if module { SourceType::mjs() } else { SourceType::script() };
    let t0 = Instant::now();
    let ret = Parser::new(&allocator, text, st)
        .with_options(ParseOptions { parse_regular_expression: true, ..ParseOptions::default() })
        .parse();
    let parse_us = t0.elapsed().as_micros() as u64;
    let perr: Vec<String> = ret.diagnostics.errors().map(|e| e.message.to_string()).collect();
    let fatal = ret.fatal_error;
    let (serr, sem_panic): (Vec<String>, Option<String>) = if perr.is_empty() {
        let program = &ret.program;
        match catch_unwind(AssertUnwindSafe(|| {
            let sret = SemanticBuilder::new().with_check_syntax_error(true).build(program);
            sret.diagnostics.errors().map(|e| e.message.to_string()).collect::<Vec<_>>()
        })) {
            Ok(v) => (v, None),
            Err(e) => (Vec::new(), Some(panic_msg(&e))),
        }
    } else {
        (Vec::new(), None)
    };
    json!({
        "perr": perr.len(),
        "serr": serr.len(),
        "fatal": fatal,
        "first": perr.first(),
        "first_sem": serr.first(),
        "sem_panic": sem_panic,
        "us": parse_us,
    })
}

fn worker(parsers: &str) -> i32 {
    let want_tsrs = parsers.split(',').any(|p| p == "tsrs");
    let want_oxc = parsers.split(',').any(|p| p == "oxc");
    #[cfg(feature = "tsrs")]
    let bind_codes = plain_js_binder_codes();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split('\t');
        let id: u64 = it.next().unwrap().parse().unwrap();
        let path = it.next().unwrap();
        let runs = it.next().unwrap();
        let Ok(bytes) = std::fs::read(path) else {
            let _ = writeln!(out, "R\t{}", json!({"id": id, "read_error": true}));
            continue;
        };
        let code = String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let mut run_results = Vec::new();
        let mut tsrs_panicked = false;
        for mode in runs.chars() {
            let (text, module) = match mode {
                's' => (code.clone(), false),
                'S' => (format!("{STRICT_PREFIX}{code}"), false),
                'm' => (code.clone(), true),
                _ => panic!("bad mode {mode}"),
            };
            let mut r = serde_json::Map::new();
            r.insert("mode".into(), json!(mode.to_string()));
            #[cfg(feature = "tsrs")]
            if want_tsrs {
                let _ = writeln!(out, "B\t{id}\ttsrs\t{mode}");
                let _ = out.flush();
                match catch_unwind(AssertUnwindSafe(|| run_tsrs(&text, module, &bind_codes))) {
                    Ok(v) => {
                        r.insert("tsrs".into(), v);
                    }
                    Err(e) => {
                        tsrs_panicked = true;
                        r.insert("tsrs".into(), json!({"panic": panic_msg(&e)}));
                    }
                }
            }
            #[cfg(feature = "oxc")]
            if want_oxc {
                let _ = writeln!(out, "B\t{id}\toxc\t{mode}");
                let _ = out.flush();
                match catch_unwind(AssertUnwindSafe(|| run_oxc(&text, module))) {
                    Ok(v) => {
                        r.insert("oxc".into(), v);
                    }
                    Err(e) => {
                        r.insert("oxc".into(), json!({"panic": panic_msg(&e)}));
                    }
                }
            }
            run_results.push(Value::Object(r));
            if tsrs_panicked {
                break;
            }
        }
        let _ = writeln!(out, "R\t{}", json!({"id": id, "runs": run_results}));
        let _ = out.flush();
        if tsrs_panicked {
            return 3;
        }
    }
    0
}

#[cfg(feature = "tsrs")]
fn dump_node(n: P<Node>, depth: usize, src: &str, out: &mut String) {
    let pos = n.pos().expect_source("pos") as usize;
    let end = n.end().expect_source("end") as usize;
    let txt: String = src.get(pos..end).unwrap_or("").trim().chars().take(40).collect::<String>().replace('\n', " ");
    let _ = writeln!(out, "{}{:?} [{}..{}] {:?}", "  ".repeat(depth), n.kind(), pos, end, txt);
    n.for_each_child(&mut |c| {
        dump_node(c, depth + 1, src, out);
        false
    });
}

/// `t262h dump <file> [m]`: tsrs AST kinds + all diagnostics, for manual inspection.
#[cfg(feature = "tsrs")]
fn dump(path: &str, module: bool) {
    let text = std::fs::read_to_string(path).unwrap();
    let opts = SourceFileParseOptions {
        file_name: "/t262/test.js".to_string(),
        path: Path::new("/t262/test.js".to_string()),
        external_module_indicator_options: ExternalModuleIndicatorOptions { jsx: false, force: module },
    };
    let file = tsrs_parser::parse_source_file(opts, &text, ScriptKind::JS);
    let mut out = String::new();
    dump_node(file.as_node(), 0, file.text(), &mut out);
    for x in file.diagnostics() { let _ = writeln!(out, "D  {}", tsrs_diag_str(x)); }
    for x in file.js_diagnostics() { let _ = writeln!(out, "DJS {}", tsrs_diag_str(x)); }
    tsrs_binder::bind_source_file(file);
    for x in file.bind_diagnostics() { let _ = writeln!(out, "DB {}", tsrs_diag_str(x)); }
    print!("{out}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(feature = "tsrs")]
    if args.get(1).map(String::as_str) == Some("dump") {
        let a = args.clone();
        std::thread::Builder::new().stack_size(1 << 30).spawn(move || dump(&a[2], a.get(3).is_some_and(|m| m == "m"))).unwrap().join().unwrap();
        return;
    }
    std::panic::set_hook(Box::new(|_| {}));
    let parsers = args.get(2).cloned().unwrap_or_else(|| "tsrs,oxc".to_string());
    let h = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || worker(&parsers))
        .unwrap();
    let code = h.join().unwrap_or(4);
    std::process::exit(code);
}
