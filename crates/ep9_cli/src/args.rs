//! Command-line flags.

use std::path::PathBuf;

use ep9_vm::{Dispatch, IsolateConfig};

pub const VERSION_LINE: &str = concat!("ep9 ", env!("CARGO_PKG_VERSION"));

/// The `--version` text: [`VERSION_LINE`], then the build line `features <list>; preset <name>`
/// of `ep9_vm::build_info()` (MC-w17-5; "features none" when no ep9_vm feature is on).
/// tools/bench (kernels.mjs, run.mjs, ep9-bench) copies the second line into its JSON as
/// `engine_build`.
pub fn version_text() -> String {
    format!("{VERSION_LINE}\n{}\n", ep9_vm::build_info())
}

pub const USAGE: &str = "\
usage: ep9 [options] <file.js>
       ep9 [options] -e <code>
       ep9 --version

options:
  -e, --eval <code>    run <code> as a script named [eval]
  --module             run the source as a module, with the modules it imports (./ ../ /)
  --strict             run the script as strict code
  --gc-stress[=<n>]    collect garbage before every n-th allocation (default 1) and run the
                       heap verifier after every collection
  --verify-heap        run the heap verifier after every collection
  --gc-stats           print the number of collections to stderr at exit
  --dispatch <kind>    interpreter dispatch: match, sibling (default: the build default)
  --feedback-threshold <n>
                       calls of a function before it gets a feedback vector (default 8;
                       0 = at the first call, 4294967295 = never)
  --print-bytecode     print the disassembly of the unit before it runs
  --facts-out <file>   after the run, write the feedback facts of the script (or of the entry
                       module) to <file> as an encoded FactSnapshot v0 (H16)
  --stack-size <MiB>   native stack of the isolate thread (default 64)
  --reprl              Fuzzilli REPRL mode (not built yet: exits 2)
  -h, --help           print this text
  --version            print the version

exit status: 0 = ok, 1 = compile error or uncaught exception, 2 = usage or I/O error.
host functions: print(...args) writes ToString of each argument, joined by a space, and a
newline to stdout; gc() runs a full garbage collection.
";

/// Where the source comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Eval(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub source: Source,
    pub module: bool,
    pub strict: bool,
    /// 0 = off.
    pub gc_stress: u32,
    pub verify_heap: bool,
    pub dispatch: Dispatch,
    /// `IsolateConfig::feedback_threshold`.
    pub feedback_threshold: u32,
    pub print_bytecode: bool,
    /// `--facts-out`: where the FactSnapshot of the main unit goes after the run.
    pub facts_out: Option<PathBuf>,
    pub gc_stats: bool,
    pub stack_mib: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Options),
    Version,
    Help,
    /// `--reprl`: the Fuzzilli REPRL loop (reprl.rs). The other arguments are ignored.
    Reprl,
}

fn feedback_threshold(v: &str) -> Result<u32, String> {
    v.parse::<u32>()
        .map_err(|_| format!("bad --feedback-threshold value {v:?} (0..=4294967295)"))
}

fn facts_out(v: &str) -> Result<PathBuf, String> {
    if v.is_empty() {
        return Err("--facts-out needs a file name".into());
    }
    Ok(PathBuf::from(v))
}

fn dispatch(name: &str) -> Result<Dispatch, String> {
    Dispatch::ALL
        .iter()
        .copied()
        .find(|d| d.name() == name)
        .ok_or_else(|| {
            let names: Vec<&str> = Dispatch::ALL.iter().map(|d| d.name()).collect();
            format!(
                "bad --dispatch value {name:?} (one of: {})",
                names.join(", ")
            )
        })
}

pub fn parse(argv: &[String]) -> Result<Command, String> {
    let mut source = None;
    let mut opts = Options {
        source: Source::Eval(String::new()),
        module: false,
        strict: false,
        gc_stress: 0,
        verify_heap: false,
        dispatch: Dispatch::DEFAULT,
        feedback_threshold: IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD,
        print_bytecode: false,
        facts_out: None,
        gc_stats: false,
        stack_mib: 64,
    };
    let mut it = argv.iter();
    let set_source = |source: &mut Option<Source>, s: Source| {
        if source.is_some() {
            return Err("more than one source (file or -e)".to_string());
        }
        *source = Some(s);
        Ok(())
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--version" | "-v" => return Ok(Command::Version),
            "--reprl" => return Ok(Command::Reprl),
            "-e" | "--eval" => {
                let code = it.next().ok_or("-e needs a value")?;
                set_source(&mut source, Source::Eval(code.clone()))?;
            }
            "--module" => opts.module = true,
            "--strict" => opts.strict = true,
            "--gc-stress" => opts.gc_stress = 1,
            "--verify-heap" => opts.verify_heap = true,
            "--print-bytecode" => opts.print_bytecode = true,
            "--gc-stats" => opts.gc_stats = true,
            "--dispatch" => opts.dispatch = dispatch(it.next().ok_or("--dispatch needs a value")?)?,
            "--feedback-threshold" => {
                let v = it.next().ok_or("--feedback-threshold needs a value")?;
                opts.feedback_threshold = feedback_threshold(v)?;
            }
            "--facts-out" => {
                opts.facts_out = Some(facts_out(it.next().ok_or("--facts-out needs a value")?)?);
            }
            "--stack-size" => {
                let v = it.next().ok_or("--stack-size needs a value")?;
                opts.stack_mib = v
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=4096).contains(n))
                    .ok_or_else(|| format!("bad --stack-size value {v:?} (1..=4096)"))?;
            }
            a if a.starts_with("--gc-stress=") => {
                let v = &a["--gc-stress=".len()..];
                opts.gc_stress = v
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("bad --gc-stress value {v:?}"))?;
            }
            a if a.starts_with("--dispatch=") => {
                opts.dispatch = dispatch(&a["--dispatch=".len()..])?;
            }
            a if a.starts_with("--facts-out=") => {
                opts.facts_out = Some(facts_out(&a["--facts-out=".len()..])?);
            }
            a if a.starts_with("--feedback-threshold=") => {
                opts.feedback_threshold = feedback_threshold(&a["--feedback-threshold=".len()..])?;
            }
            a if a.starts_with('-') && a != "-" => return Err(format!("unknown flag {a:?}")),
            file => set_source(&mut source, Source::File(PathBuf::from(file)))?,
        }
    }
    if opts.module && opts.strict {
        return Err("--module and --strict do not go together (module code is strict)".into());
    }
    opts.source = source.ok_or("no file and no -e")?;
    if opts.gc_stress > 0 {
        opts.verify_heap = true;
    }
    Ok(Command::Run(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Options, String> {
        let argv: Vec<String> = args.iter().map(ToString::to_string).collect();
        match parse(&argv)? {
            Command::Run(o) => Ok(o),
            other => Err(format!("{other:?}")),
        }
    }

    #[test]
    fn sources_and_flags() {
        let o = run(&["a.js"]).unwrap();
        assert_eq!(o.source, Source::File("a.js".into()));
        assert_eq!((o.gc_stress, o.verify_heap), (0, false));
        let o = run(&["--gc-stress", "-e", "1"]).unwrap();
        assert_eq!(o.source, Source::Eval("1".into()));
        assert_eq!((o.gc_stress, o.verify_heap), (1, true));
        assert_eq!(run(&["--gc-stress=7", "x.js"]).unwrap().gc_stress, 7);
        assert_eq!(
            run(&["--dispatch", "sibling", "x.js"]).unwrap().dispatch,
            Dispatch::Sibling
        );
        assert!(run(&["--strict", "x.js"]).unwrap().strict);
        assert!(run(&["--module", "x.js"]).unwrap().module);
        assert_eq!(run(&["x.js"]).unwrap().facts_out, None);
        assert_eq!(
            run(&["--facts-out", "f.bin", "x.js"]).unwrap().facts_out,
            Some(PathBuf::from("f.bin"))
        );
        assert_eq!(
            run(&["-e", "1", "--facts-out=f.bin"]).unwrap().facts_out,
            Some(PathBuf::from("f.bin"))
        );
        assert_eq!(
            run(&["x.js"]).unwrap().feedback_threshold,
            IsolateConfig::default().feedback_threshold
        );
        assert_eq!(
            run(&["--feedback-threshold", "0", "x.js"])
                .unwrap()
                .feedback_threshold,
            0
        );
        assert_eq!(
            run(&["--feedback-threshold=4294967295", "x.js"])
                .unwrap()
                .feedback_threshold,
            u32::MAX
        );
    }

    #[test]
    fn errors() {
        assert!(run(&[]).is_err());
        assert!(run(&["a.js", "b.js"]).is_err());
        assert!(run(&["-e"]).is_err());
        assert!(run(&["--gc-stress=0", "a.js"]).is_err());
        assert!(run(&["--dispatch", "jit", "a.js"]).is_err());
        assert!(run(&["--feedback-threshold", "-1", "a.js"]).is_err());
        assert!(run(&["--feedback-threshold=x", "a.js"]).is_err());
        assert!(run(&["--feedback-threshold"]).is_err());
        assert!(run(&["--bogus", "a.js"]).is_err());
        assert!(run(&["a.js", "--facts-out"]).is_err());
        assert!(run(&["--facts-out=", "a.js"]).is_err());
        assert!(run(&["--module", "--strict", "a.js"]).is_err());
        assert_eq!(parse(&["--version".into()]).unwrap(), Command::Version);
        assert_eq!(parse(&["-h".into()]).unwrap(), Command::Help);
        assert_eq!(parse(&["--reprl".into()]).unwrap(), Command::Reprl);
    }

    /// Two lines: the version, then the build (the default ep9 has the jit feature and
    /// preset; the jitless one has neither).
    #[test]
    fn version_text_names_the_build() {
        let t = version_text();
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 2, "{t}");
        assert_eq!(lines[0], VERSION_LINE);
        if cfg!(feature = "jit") {
            assert!(lines[1].starts_with("features jit"), "{t}");
            assert!(lines[1].ends_with("; preset jit"), "{t}");
        } else {
            assert_eq!(lines[1], "features none; preset jitless");
        }
    }
}
