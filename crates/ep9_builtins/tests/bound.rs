//! Calls through long chains of bound functions (§10.4.1) with bound arguments: the argument
//! list is built in one walk of the chain (`bound::copy_chain_args`), and the walk polls for
//! termination (`bound::unwrap`). The expected values were checked against node 24.

mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::run_both;
use ep9_bytecode::VerifiedUnit;
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_vm::Local;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

fn compile(src: &str) -> VerifiedUnit {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("compiles")
}

/// Makes `b` (over `f`) and `bk` (over the constructor `K`): `levels` binds each, one bound
/// argument per level (the level's index). Deleting each `name` keeps the names short.
const CHAIN: &str = "function f() { return arguments.length + ':' + arguments[0] + ',' + arguments[1] + ','
                         + arguments[arguments.length - 2] + ',' + arguments[arguments.length - 1]; }
                     function K() { this.s = f.apply(null, arguments); }
                     var b = f, bk = K;
                     for (var i = 0; i < levels; i++) {
                         b = b.bind(null, i); delete b.name;
                         bk = bk.bind(null, i); delete bk.name;
                     }";

#[test]
fn a_long_bind_chain_with_an_argument_at_every_level_is_linear() {
    // 100000 levels: a walk from `f` for each bound argument took about N^2/2 steps (12.7 s for
    // one call in a release build); one walk takes N (milliseconds, even in a debug build). The
    // three calls must take less than 20 s. The 300 s wait only stops a hung test.
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let mut iso = common::isolate(false);
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile(&format!("var levels = 100000; {CHAIN}")),
                    "chain.js",
                )
                .map(|_| ())
            })
            .expect("the chain is built");
            let t = Instant::now();
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile("var r = [b('x'), b.call(null, 'y'), new bk('z').s].join(' ');"),
                    "calls.js",
                )
                .map(|_| ())
            });
            let took = t.elapsed();
            let r = r
                .map(|_| common::global_text(&mut iso, "r"))
                .map_err(|e| e.message);
            let _ = tx.send((r, took));
        })
        .expect("a thread");
    let (r, took) = rx
        .recv_timeout(Duration::from_secs(300))
        .expect("the test finishes");
    eprintln!("3 calls through a 100000-level chain: {took:?}");
    assert_eq!(
        r.as_ref().map(|v| v.as_deref()),
        Ok(Some(
            "100001:0,1,99999,x 100001:0,1,99999,y 100001:0,1,99999,z"
        ))
    );
    assert!(took < Duration::from_secs(20), "the calls took {took:?}");
}

#[test]
fn bound_arguments_keep_their_order_across_empty_and_nonempty_levels() {
    // Bytecode calls (`slow::call`), Rust calls (`call`, `apply`: `call::call_with`) and
    // bytecode constructs, through levels with and without bound arguments.
    assert_eq!(
        run_both(
            "function f() { return Array.prototype.join.call(arguments, ','); }
             var b1 = f.bind(null);
             var b2 = b1.bind(null, 'a', 'b');
             var b3 = b2.bind(null);
             var b4 = b3.bind(null, 'c');
             var b5 = b4.bind(null, 'd', 'e', 'f');
             var b6 = b5.bind(null);
             print(b6('x', 'y'), b6(), b5('z'), b4(), b2('q'), b1('r'));
             print(b6.call(null, 1), b6.apply(null, [2, 3]), b3.call(), b1.apply());
             var o = {m: b6};
             print(o.m(9), [b6][0](8));
             function K() { this.a = Array.prototype.join.call(arguments, ','); this.nt = new.target === K; }
             var k1 = K.bind(null, 1).bind(null).bind(null, 2, 3).bind(null);
             var r = new k1(4);
             print(r.a, r.nt, r instanceof K, new k1().a);"
        ),
        lines(&[
            "a,b,c,d,e,f,x,y a,b,c,d,e,f a,b,c,d,e,f,z a,b,c a,b,q r",
            "a,b,c,d,e,f,1 a,b,c,d,e,f,2,3 a,b ",
            "a,b,c,d,e,f,9 a,b,c,d,e,f,8",
            "1,2,3,4 true true 1,2,3",
        ])
    );
}

#[test]
fn new_target_through_a_chain_with_bound_arguments() {
    // §10.4.1.2 step 5 at each level: new.target is the innermost target when it was the
    // bound function, and is kept otherwise (a subclass's `super`).
    assert_eq!(
        run_both(
            "function K() { this.a = Array.prototype.join.call(arguments, ','); this.nt = new.target; }
             var k1 = K.bind(null, 1).bind(null).bind(null, 2, 3).bind(null);
             k1.prototype = K.prototype;
             class D extends k1 { constructor() { super(5, 6); } }
             var d = new D(), k = new k1(4);
             print(d.a, d.nt === D, d instanceof D, d instanceof K);
             print(k.a, k.nt === K);"
        ),
        lines(&["1,2,3,5,6 true true true", "1,2,3,4 true"])
    );
}

/// `Cx::construct` and `Cx::call` from Rust (`call::construct_with`, `call::call_with`) through
/// a chain with bound arguments at some levels. The bound arguments are new objects that only
/// the chain holds, so under GC stress they must survive the allocation of `this` between the
/// copy and the frame push.
#[test]
fn rust_calls_through_a_chain_with_bound_arguments() {
    for stress in [false, true] {
        let r = common::on_big_stack(move || {
            let mut iso = common::isolate(stress);
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile(
                        "function v(x) { return {v: x}; }
                     function C() {
                         var s = [];
                         for (var i = 0; i < arguments.length; i++) s[i] = arguments[i].v;
                         this.s = s.join(); this.nt = new.target;
                     }
                     var BC = C.bind(null, v('a')).bind(null).bind(null, v('b'), v('c'));
                     function Other() {}
                     Other.prototype.tag = 'other';
                     function g() { return Array.prototype.join.call(arguments, ''); }
                     var bg = g.bind(null).bind(null, 'p', 'q').bind(null).bind(null, 'r');",
                    ),
                    "rust.js",
                )
                .map(|_| ())
            })
            .map_err(|e| e.message)?;
            iso.with_cx(|cx| {
                let g = cx.global();
                let (c, bc, other, bg, vf) = (
                    cx.get(g, "C")?,
                    cx.get(g, "BC")?,
                    cx.get(g, "Other")?,
                    cx.get(g, "bg")?,
                    cx.get(g, "v")?,
                );
                let v7 = cx.call(vf, Local::undefined(), &[Local::int32(7)])?;
                // new.target == F: the target C.
                let o = cx.construct(bc, &[v7], bc)?;
                let s = cx.get(o, "s")?;
                assert_eq!(cx.to_rust_string(s)?, "a,b,c,7");
                let nt = cx.get(o, "nt")?;
                assert!(nt.is_same_bits(&c));
                // new.target != F: kept.
                let o = cx.construct(bc, &[], other)?;
                let s = cx.get(o, "s")?;
                assert_eq!(cx.to_rust_string(s)?, "a,b,c");
                let nt = cx.get(o, "nt")?;
                assert!(nt.is_same_bits(&other));
                let r = cx.call(bg, Local::undefined(), &[Local::int32(1)])?;
                assert_eq!(cx.to_rust_string(r)?, "pqr1");
                Ok(())
            })
            .map_err(|e| e.message)?;
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            Ok::<(), String>(())
        });
        assert_eq!(r, Ok(()), "stress {stress}");
    }
}

/// Builds the chains of `CHAIN` (100000 binds, with or without a bound argument per level) in a
/// first script, requests termination, then runs `call` in a second one. Nothing polls
/// between the start of `call.js` and the walk of the chain, so only a poll in the walk stops
/// the call. Returns whether the run was terminated and how far `call.js` got.
fn terminate_in_chain(bind_args: bool, call: &'static str) -> (bool, String) {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let chain = if bind_args {
                CHAIN.to_string()
            } else {
                CHAIN.replace("bind(null, i)", "bind(null)")
            };
            let mut iso = common::isolate(false);
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile(&format!("var levels = 100000; {chain}")),
                    "chain.js",
                )
                .map(|_| ())
            })
            .expect("the chain is built");
            iso.terminate_handle().terminate();
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile(&format!(
                        "var stage = 'before'; var r = {call}; stage = 'reached ' + r;"
                    )),
                    "call.js",
                )
                .map(|_| ())
            });
            let terminated = r.is_err_and(|e| e.terminated);
            let stage = common::global_text(&mut iso, "stage");
            // The isolate keeps working.
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(compile("stage = 'again'"), "again.js")
                    .map(|_| ())
            })
            .expect("runs");
            let again = common::global_text(&mut iso, "stage");
            assert_eq!(again.as_deref(), Some("again"));
            (terminated, stage.unwrap_or_default())
        })
        .expect("a thread")
        .join()
        .expect("no panic")
}

#[test]
fn termination_stops_a_call_in_a_long_bind_chain() {
    // Bytecode call, Rust call (`call::call_with`), bytecode construct; then a chain of
    // empty binds, which has no bound arguments to copy.
    let cases = [
        (true, "b('x')"),
        (true, "b.call(null, 'x')"),
        (true, "new bk('x').s"),
        (false, "b('x')"),
    ];
    let got: Vec<_> = cases
        .iter()
        .map(|&(bind_args, call)| (call, terminate_in_chain(bind_args, call)))
        .collect();
    let want: Vec<_> = cases
        .iter()
        .map(|&(_, call)| (call, (true, "before".to_string())))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn termination_stops_a_rust_construct_in_a_long_bind_chain() {
    let r = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            let mut iso = common::isolate(false);
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile(&format!("var levels = 100000; {CHAIN}")),
                    "chain.js",
                )
                .map(|_| ())
            })
            .expect("the chain is built");
            iso.terminate_handle().terminate();
            iso.with_cx(|cx| {
                let g = cx.global();
                let bk = cx.get(g, "bk")?;
                cx.construct(bk, &[], bk)?;
                Ok(())
            })
            .map_err(|e| e.terminated)
        })
        .expect("a thread")
        .join()
        .expect("no panic");
    assert_eq!(r, Err(true));
}
