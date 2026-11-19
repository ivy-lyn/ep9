//! `cargo xtask disasm-check`: checks interpreter handlers in release machine code (H15,
//! docs/plan/08-repo.md section 7).
//!
//! Symbol contract (set by the dispatch DSL, spikes/sp-dispatch/src/dsl.rs): each handler is
//! exported as `ep9h__<expansion>__<class>__<op>`. Class `tail` must always continue to another
//! handler; class `exit` may return to the host. Mach-O adds a leading `_`.
//!
//! | Check | Rule |
//! |---|---|
//! | tail exits | A `tail` handler has 0 returns, 0 indirect calls, 0 calls to handlers, and at least one tail exit (indirect jump, or a direct jump to another handler). A direct jump to a symbol that is not a handler is followed: that code counts as part of the handler, and a target that cannot be disassembled fails. An `exit` handler has 0 indirect calls and 0 calls to handlers. |
//! | handler args | Every handler in the LLVM IR has at most 4 parameters (`--max-args`), and none is passed in memory (sret, byval, inalloca, preallocated). |
//! | match loop (`--match-loop`) | For SP-Dispatch, whose 20-opcode x86-64 match loop LLVM threads (`ep9loop__<expansion>__<fn>`). On x86-64 it has an indirect (dispatch) jump for at least half of the fast `tail` handlers, so LLVM copied the dispatch into the arms (threaded code), and it touches the stack only next to a call, in the entry block or in an exit sequence. On aarch64 the shape is printed only. T0 (`ep9_vm`) releases sibling calls on every target (H15 ADR, 2026-10-21), so for T0 the loop line is a report, not a gate. |
//! | frames (`--frames GOLDEN`) | Plan rule 81 (no new entry save on a hot x86 handler; docs/plan/10-quality.md, 2026-11-16 note). Its own check and CI step: it builds like the default mode and runs only this check. Every hot handler (every handler whose name does not end in `_cold`, and on x86-64 also the `HOT_COLD` list: `Wide_cold` and `ExtraWide_cold`, which run the `ops::prefix_set` fast bodies there through `interp::tails::wide`, M9-P1 X86-WIDE-FAST; MC-w43-2; and `CallProperty0_cold`..`CallProperty2_cold`, which run the host call of `slow::call_property` there, M9 OXC-W2-HOSTCALL; MC-w44-4) is scanned. The ones that build a frame on entry (the first instruction saves a register or grows the stack) or save a callee-saved register on entry (a red-zone `mov` save too; M9-P1-WPRE-3), with the callee-saved registers each one saves on entry (the leading prologue, where a push of another register, a stack alignment push, counts 0, then each callee-saved register that the entry code saves before its first conditional branch, call, return or tail jump, with an unconditional jump inside the handler followed; a store of a callee-saved register in a form that the check does not know fails it; `entry_saves`), must equal the golden file: one `<expansion> <handler> <saves>` row per handler and a `target` line. A new row, a lost row or a changed count fails; a change that is meant updates the golden in the same PR (`--bless` writes it). Report only: an `any-path` line per hot handler that stores a callee-saved register on any path of its block (a shrink-wrapped save too; `any_path_saves`). x86-64 alignment column (H15 x64 function alignment, M10-P1 X86-ALIGN; M10-PRE-ALIGN-FORM): one `align64 <expansion> <aligned>/<total>` line per expansion in the golden; each hot handler must start on a 64-byte line: the alignment of its section is 64 or more and its offset in that section is a multiple of 64 (`llvm-readobj --sections --symbols`: ELF `sh_addralign`, COFF `IMAGE_SCN_ALIGN_*`; the staticlib holds relocatable objects, so the symbol value alone is 0 with and without the flag; `align_rows`). A handler that does not fails, and so does a line that differs from the golden. The root `.cargo/config.toml` x86-64 `[target]` rustflags set the alignment; `RUSTFLAGS` in the environment replaces them, and this column then fails. aarch64 goldens have no `align64` line. |
//! | Mach-O data in code | x86-64 Mach-O keeps jump tables in `__text` (LC_DATA_IN_CODE). The check drops what `llvm-objdump -d` decodes from those bytes, so table data never counts as jumps, calls, returns or stack accesses, and decodes the code after each table again from the table end, so a decode that ran across the table end cannot hide an instruction there. |
//!
//! Build mode (default) runs `cargo rustc --lib --release --crate-type staticlib --target T`.
//! The crate's release profile applies (fat LTO). A staticlib needs no linker, so any target
//! with an installed rust-std works, Windows included. `--binary FILE` checks a file that is
//! already built (no IR there, so no argument check).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const USAGE: &str = "\
usage: cargo xtask disasm-check [options]
  --manifest-path PATH  crate to build (default: spikes/sp-dispatch/Cargo.toml)
  -p, --package NAME    package in that workspace
  --target TRIPLE       target to build for; repeat for more (default: host)
  --all-targets         aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu
                        aarch64-unknown-linux-gnu x86_64-pc-windows-msvc
  --features LIST       cargo features
  --toolchain NAME      build with `rustup run NAME cargo` (for example a nightly)
  --rustc-arg ARG       extra rustc argument for the checked crate; repeat for more
                        (negative tests, for example -Cllvm-args=-disable-tail-duplicate)
  --binary FILE         check this built file instead of building; repeat for more
  --expect N            every expansion must have exactly N handlers
  --max-args N          handler argument limit (default 4)
  --match-loop          also check the match loop (ep9loop__* symbols; rule on x86-64 only)
  --frames GOLDEN       only the frame check: the hot handlers that build a frame on entry and
                        their entry register saves must equal GOLDEN (one target, no --binary);
                        on x86-64 also every hot handler starts on a 64-byte line (align64)
  --bless               with --frames: write GOLDEN from this build instead of checking it
  --objdump PATH, --nm PATH   tools (default: rustup llvm-tools, then PATH)
  -v, --verbose         one line per handler";

/// The 5 targets of `--all-targets`; fp-check (fpcheck.rs) checks the same set.
pub(crate) const ALL_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

const PREFIX: &str = "ep9h__";
const LOOP_PREFIX: &str = "ep9loop__";
/// A stack access within this many instructions of a call is a spill or reload around that
/// call (a cold path), not traffic on every dispatch.
const LOOP_CALL_WINDOW: usize = 4;

struct Opts {
    manifest: PathBuf,
    package: Option<String>,
    targets: Vec<String>,
    features: Option<String>,
    toolchain: Option<String>,
    rustc_args: Vec<String>,
    binaries: Vec<PathBuf>,
    expect: Option<usize>,
    max_args: usize,
    match_loop: bool,
    /// `--frames`: the golden file of the frame check.
    frames: Option<PathBuf>,
    /// `--bless`: write the frame golden instead of checking it.
    bless: bool,
    objdump: Option<PathBuf>,
    nm: Option<PathBuf>,
    verbose: bool,
}

pub(crate) fn repo_root() -> PathBuf {
    // tools/xtask -> repo root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        manifest: repo_root().join("spikes/sp-dispatch/Cargo.toml"),
        package: None,
        targets: Vec::new(),
        features: None,
        toolchain: None,
        rustc_args: Vec::new(),
        binaries: Vec::new(),
        expect: None,
        max_args: 4,
        match_loop: false,
        frames: None,
        bless: false,
        objdump: None,
        nm: None,
        verbose: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--manifest-path" => o.manifest = PathBuf::from(val()?),
            "-p" | "--package" => o.package = Some(val()?),
            "--target" => o.targets.push(val()?),
            "--all-targets" => o.targets.extend(ALL_TARGETS.iter().map(|s| s.to_string())),
            "--features" => o.features = Some(val()?),
            "--toolchain" => o.toolchain = Some(val()?),
            "--rustc-arg" => o.rustc_args.push(val()?),
            "--binary" => o.binaries.push(PathBuf::from(val()?)),
            "--expect" => o.expect = Some(val()?.parse().map_err(|e| format!("--expect: {e}"))?),
            "--max-args" => o.max_args = val()?.parse().map_err(|e| format!("--max-args: {e}"))?,
            "--match-loop" => o.match_loop = true,
            "--frames" => o.frames = Some(PathBuf::from(val()?)),
            "--bless" => o.bless = true,
            "--objdump" => o.objdump = Some(PathBuf::from(val()?)),
            "--nm" => o.nm = Some(PathBuf::from(val()?)),
            "-v" | "--verbose" => o.verbose = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    if o.bless && o.frames.is_none() {
        return Err(format!("--bless needs --frames\n{USAGE}"));
    }
    if o.frames.is_some() && !o.binaries.is_empty() {
        return Err(format!(
            "--frames builds the crate; it takes no --binary\n{USAGE}"
        ));
    }
    if o.targets.is_empty() && o.binaries.is_empty() {
        o.targets.push(host_triple()?);
    }
    if o.frames.is_some() && o.targets.len() != 1 {
        return Err(format!(
            "--frames checks one target (a golden file is per target)\n{USAGE}"
        ));
    }
    Ok(o)
}

/// Runs `cmd` and returns its stdout; stderr goes to the log. A failed run is an error.
pub(crate) fn run_capture(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?} failed: {}", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{cmd:?}: {e}"))
}

/// Like `run_capture`, but keeps stderr out of the log unless the command fails.
fn run_quiet(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{cmd:?}: {e}"))
}

pub(crate) fn host_triple() -> Result<String, String> {
    let v = run_capture(Command::new("rustc").arg("-vV"))?;
    v.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_string)
        .ok_or_else(|| "rustc -vV: no host line".into())
}

/// Finds an LLVM tool: the override, then rustup's llvm-tools component, then PATH.
pub(crate) fn find_tool(name: &str, over: &Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = over {
        return Ok(p.clone());
    }
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let (Ok(sysroot), Ok(host)) = (
        run_capture(Command::new("rustc").args(["--print", "sysroot"])),
        host_triple(),
    ) {
        let p = Path::new(sysroot.trim())
            .join("lib/rustlib")
            .join(host)
            .join("bin")
            .join(&exe);
        if p.exists() {
            return Ok(p);
        }
    }
    if Command::new(&exe)
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_ok()
    {
        return Ok(PathBuf::from(exe));
    }
    Err(format!(
        "{name} not found. Install it with: rustup component add llvm-tools (or pass --{})",
        name.trim_start_matches("llvm-")
    ))
}

struct Tools {
    objdump: PathBuf,
    nm: PathBuf,
    ar: PathBuf,
}

pub fn main(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    let tools = Tools {
        objdump: find_tool("llvm-objdump", &o.objdump)?,
        nm: find_tool("llvm-nm", &o.nm)?,
        ar: find_tool("llvm-ar", &None)?,
    };
    if let Some(golden) = &o.frames {
        let t = &o.targets[0];
        let (art, _) = build(&o, t)?;
        let ok = check_frames(&tools, &art, t, golden, o.bless)?;
        println!();
        println!(
            "disasm-check --frames {}: {t}",
            if ok { "PASS" } else { "FAIL" }
        );
        return Ok(ok);
    }
    let mut results = Vec::new();
    if o.binaries.is_empty() {
        for t in &o.targets {
            let (art, ll) = build(&o, t)?;
            let ok = check_file(&o, &tools, &art, Some(&ll), t)?;
            results.push((t.clone(), ok));
        }
    } else {
        for b in &o.binaries {
            let ok = check_file(&o, &tools, b, None, "binary")?;
            results.push((b.display().to_string(), ok));
        }
    }
    let all = results.iter().all(|(_, ok)| *ok);
    println!();
    for (what, ok) in &results {
        println!("disasm-check {}: {what}", if *ok { "PASS" } else { "FAIL" });
    }
    Ok(all)
}

/// The IR file name (without `.ll`) for one build: the package (or the manifest's directory when
/// no `-p` is given), the target, the toolchain, the features, and a hash of extra rustc args.
/// Two crates checked from one target dir (ep9_vm, then the SP-Dispatch spike) must never share
/// an IR file: a cached build does not rewrite it, so the second check would read the first
/// crate's IR. Cargo fingerprints the extra args, so it rebuilds the staticlib when they change.
fn ir_tag(o: &Opts, target: &str) -> String {
    let package = o.package.clone().unwrap_or_else(|| {
        o.manifest
            .parent()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "crate".into())
    });
    let rargs = if o.rustc_args.is_empty() {
        String::new()
    } else {
        let h = o.rustc_args.join(" ").bytes().fold(0x811c_9dc5u32, |h, b| {
            (h ^ b as u32).wrapping_mul(0x0100_0193)
        });
        format!("-r{h:08x}")
    };
    format!(
        "{package}-{target}{}{}{rargs}",
        o.toolchain
            .as_deref()
            .map(|t| format!("-{t}"))
            .unwrap_or_default(),
        o.features
            .as_deref()
            .map(|f| format!("-{}", f.replace([',', ' '], "+")))
            .unwrap_or_default()
    )
}

/// Builds the crate as a staticlib for `target` and returns (staticlib, LLVM IR file).
fn build(o: &Opts, target: &str) -> Result<(PathBuf, PathBuf), String> {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target"));
    let tdir = base.join("xtask-disasm");
    fs::create_dir_all(&tdir).map_err(|e| e.to_string())?;
    let ll = tdir.join(format!("{}.ll", ir_tag(o, target)));
    let mut cmd = match &o.toolchain {
        Some(tc) => {
            let mut c = Command::new("rustup");
            c.args(["run", tc, "cargo"]);
            c
        }
        None => Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())),
    };
    cmd.arg("rustc").arg("--manifest-path").arg(&o.manifest);
    if let Some(p) = &o.package {
        cmd.args(["-p", p]);
    }
    if let Some(f) = &o.features {
        cmd.args(["--features", f]);
    }
    cmd.args([
        "--lib",
        "--release",
        "--crate-type",
        "staticlib",
        "--target",
        target,
    ]);
    cmd.args(["--message-format=json-render-diagnostics", "--target-dir"])
        .arg(&tdir);
    // Cargo fingerprints these extra rustc args, so a fresh build keeps a matching .ll file.
    cmd.arg("--")
        .arg(format!("--emit=llvm-ir={}", ll.display()))
        .args(&o.rustc_args);
    eprintln!(
        "disasm-check: building {} for {target}",
        o.manifest.display()
    );
    let out = run_capture(&mut cmd)?;
    let mut lib = None;
    for line in out
        .lines()
        .filter(|l| l.contains("\"reason\":\"compiler-artifact\""))
    {
        for f in json_string_array(line, "filenames") {
            if f.ends_with(".a") || f.ends_with(".lib") {
                lib = Some(PathBuf::from(f));
            }
        }
    }
    let lib = lib.ok_or("cargo reported no staticlib artifact")?;
    if !ll.exists() {
        return Err(format!(
            "{} is missing although cargo says the build is fresh; remove {} and run again",
            ll.display(),
            tdir.display()
        ));
    }
    Ok((lib, ll))
}

/// Reads the string array `"key":[...]` from one line of cargo JSON output.
fn json_string_array(line: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\":[");
    let Some(start) = line.find(&pat) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut chars = line[start + pat.len()..].chars();
    while let Some(c) = chars.next() {
        match cur.as_mut() {
            None => match c {
                '"' => cur = Some(String::new()),
                ']' => break,
                _ => {}
            },
            Some(s) => match c {
                '"' => out.extend(cur.take()),
                '\\' => match chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(e) => s.push(e),
                    None => break,
                },
                c => s.push(c),
            },
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arch {
    A64,
    X64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Tail,
    Exit,
}

#[derive(Clone, Debug)]
struct Handler {
    raw: String,
    name: String,
    exp: String,
    class: Class,
    member: String,
    addr: u64,
}

fn parse_handler(raw: &str) -> Option<(String, String, Class)> {
    let n = raw
        .strip_prefix('_')
        .filter(|s| s.starts_with(PREFIX))
        .unwrap_or(raw);
    let rest = n.strip_prefix(PREFIX)?;
    let mut it = rest.splitn(3, "__");
    let (exp, class, op) = (it.next()?, it.next()?, it.next()?);
    let class = match class {
        "tail" => Class::Tail,
        "exit" => Class::Exit,
        _ => return None,
    };
    if exp.is_empty() || op.is_empty() {
        return None;
    }
    Some((n.to_string(), exp.to_string(), class))
}

/// Handler symbols from `llvm-nm --defined-only`, with their archive member and address.
fn nm_handlers(nm: &Path, file: &Path) -> Result<Vec<Handler>, String> {
    let out = run_quiet(Command::new(nm).arg("--defined-only").arg(file))?;
    let mut member = String::new();
    let mut v = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.ends_with(':') && !t.contains(' ') {
            member = t.trim_end_matches(':').to_string();
            continue;
        }
        let parts: Vec<&str> = t.split_whitespace().collect();
        if parts.len() != 3 || !matches!(parts[1], "T" | "t") {
            continue;
        }
        let Some((name, exp, class)) = parse_handler(parts[2]) else {
            continue;
        };
        let addr = u64::from_str_radix(parts[0], 16).map_err(|e| format!("nm address: {e}"))?;
        v.push(Handler {
            raw: parts[2].to_string(),
            name,
            exp,
            class,
            member: member.clone(),
            addr,
        });
    }
    Ok(v)
}

/// One decoded instruction of a [`Block`].
#[derive(Debug)]
pub(crate) struct Insn {
    pub(crate) addr: u64,
    /// "mnemonic\toperands" as llvm-objdump prints it.
    pub(crate) text: String,
    /// The relocation target that llvm-objdump `-r` prints after this instruction, if any.
    pub(crate) reloc: Option<String>,
}

/// One symbol of `llvm-objdump -d -r` output.
#[derive(Debug)]
pub(crate) struct Block {
    /// The archive member that holds the symbol (empty outside an archive).
    pub(crate) member: String,
    /// The symbol name as the object file spells it (Mach-O adds a leading `_`).
    pub(crate) raw: String,
    pub(crate) addr: u64,
    pub(crate) insns: Vec<Insn>,
}

fn is_reloc(s: &str) -> bool {
    ["R_", "ARM64_RELOC_", "X86_64_RELOC_", "IMAGE_REL_"]
        .iter()
        .any(|p| s.starts_with(p))
}

/// Disassembles the named symbols. Returns the file format and one block per symbol.
fn disassemble(
    objdump: &Path,
    file: &Path,
    raws: &[String],
) -> Result<(String, Vec<Block>), String> {
    let out = run_capture(
        Command::new(objdump)
            .args(["-d", "-r", "--no-show-raw-insn"])
            .arg(format!("--disassemble-symbols={}", raws.join(",")))
            .arg(file),
    )?;
    let (format, mut blocks) = parse_disasm(&out);
    if format.is_empty() {
        return Err(format!(
            "llvm-objdump printed no file format for {}",
            file.display()
        ));
    }
    if format.to_ascii_lowercase().starts_with("mach-o") {
        let dice = run_quiet(
            Command::new(objdump)
                .args(["--macho", "--data-in-code"])
                .arg(file),
        )?;
        let data = parse_data_in_code(&dice);
        if !data.is_empty() {
            let headers = run_quiet(
                Command::new(objdump)
                    .args(["--macho", "--private-headers"])
                    .arg(file),
            )?;
            let base = macho_dice_base(&headers);
            redecode_after_data(&mut blocks, base, &data, |sym, start, stop| {
                let out = run_capture(
                    Command::new(objdump)
                        .args(["-d", "-r", "--no-show-raw-insn"])
                        .arg(format!("--disassemble-symbols={sym}"))
                        .arg(format!("--start-address={start:#x}"))
                        .args(stop.map(|s| format!("--stop-address={s:#x}")))
                        .arg(file),
                )?;
                Ok(parse_disasm(&out).1)
            })?;
            drop_data(&mut blocks, base, &data);
        }
    }
    Ok((format, blocks))
}

/// Decodes the code after each data range again, from the range end. `-d` decodes a symbol from
/// its start, data included, so on x86 an "instruction" decoded from the data can end past the
/// range: the code after it is then decoded from a wrong address, or swallowed (a `ret` hidden).
/// `dis(symbol, start, stop)` disassembles `symbol` from `start` up to `stop` (the next range;
/// `None`: the symbol end); its instructions replace the ones of the first pass there. fp-check
/// (fpcheck.rs) uses it too.
pub(crate) fn redecode_after_data(
    blocks: &mut [Block],
    base: u64,
    data: &[(u64, u64)],
    mut dis: impl FnMut(&str, u64, Option<u64>) -> Result<Vec<Block>, String>,
) -> Result<(), String> {
    // [start, end) addresses, sorted; overlapping or adjacent ranges merged.
    let mut sorted: Vec<(u64, u64)> = data
        .iter()
        .map(|&(off, len)| (base + off, base + off + len))
        .collect();
    sorted.sort_unstable();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for (s, e) in sorted {
        match ranges.last_mut() {
            Some(l) if s <= l.1 => l.1 = l.1.max(e),
            _ => ranges.push((s, e)),
        }
    }
    for (i, &(s, e)) in ranges.iter().enumerate() {
        let stop = ranges.get(i + 1).map(|r| r.0);
        for b in blocks.iter_mut() {
            // The range is in b's code if it starts at or after b and before the end of b's
            // last decoded instruction (an x86 instruction has at most 15 bytes).
            if !b
                .insns
                .last()
                .is_some_and(|l| (b.addr..l.addr + 15).contains(&s))
            {
                continue;
            }
            let again = dis(&b.raw, e, stop)?;
            b.insns
                .retain(|x| x.addr < e || stop.is_some_and(|t| x.addr >= t));
            if let Some(n) = again
                .into_iter()
                .find(|n| n.raw == b.raw && n.addr == b.addr)
            {
                b.insns.extend(n.insns);
                b.insns.sort_by_key(|x| x.addr);
            }
        }
    }
    Ok(())
}

/// Entries `(offset, length)` of `llvm-objdump --macho --data-in-code` output
/// ("0x0000417c    608 JUMP_TABLE32").
pub(crate) fn parse_data_in_code(out: &str) -> Vec<(u64, u64)> {
    out.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let off = u64::from_str_radix(w.next()?.strip_prefix("0x")?, 16).ok()?;
            let len = w.next()?.parse().ok()?;
            w.next()?;
            Some((off, len))
        })
        .collect()
}

/// The address of data-in-code offset 0: in a linked image the offsets count from the file
/// start, so the base is the vmaddr of the segment at file offset 0 (`__TEXT`); in an object
/// (one segment, not at file offset 0) the offsets are addresses already. Reads
/// `llvm-objdump --macho --private-headers` output.
fn macho_dice_base(headers: &str) -> u64 {
    let field = |l: &str, k: &str| {
        let v = l.trim().strip_prefix(k)?.trim();
        match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).ok(),
            None => v.parse().ok(),
        }
    };
    let (mut vmaddr, mut fileoff) = (None, None);
    for l in headers.lines() {
        if l.trim().starts_with("cmd LC_SEGMENT") {
            (vmaddr, fileoff) = (None, None);
        } else if let Some(v) = field(l, "vmaddr ") {
            vmaddr = Some(v);
        } else if let Some(v) = field(l, "fileoff ") {
            fileoff = Some(v);
        } else if let (Some(v), Some(0), Some(size)) = (vmaddr, fileoff, field(l, "filesize "))
            && size > 0
        {
            return v;
        }
    }
    0
}

/// Removes the "instructions" that llvm-objdump decoded from data in code. x86-64 Mach-O
/// keeps jump tables in `__text` after the function's code (LC_DATA_IN_CODE), and `-d` decodes
/// them as code: false indirect jumps and calls, stack accesses and returns.
pub(crate) fn drop_data(blocks: &mut [Block], base: u64, data: &[(u64, u64)]) {
    for b in blocks {
        b.insns.retain(|i| {
            !data
                .iter()
                .any(|&(off, len)| (base + off..base + off + len).contains(&i.addr))
        });
    }
}

/// Reads `llvm-objdump -d -r` output: the file format (empty if none) and one block per symbol.
pub(crate) fn parse_disasm(out: &str) -> (String, Vec<Block>) {
    let mut format = String::new();
    let mut member = String::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut open = false;
    for line in out.lines() {
        if let Some(i) = line
            .find(":\tfile format ")
            .or_else(|| line.find(": file format "))
        {
            let head = &line[..i];
            member = match (head.rfind('('), head.ends_with(')')) {
                (Some(p), true) => head[p + 1..head.len() - 1].to_string(),
                _ => String::new(),
            };
            format = line[i..]
                .split("file format")
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();
            open = false;
            continue;
        }
        let t = line.trim();
        if t.is_empty() {
            open = false;
            continue;
        }
        // "0000000000001234 <sym>:"
        if let Some((addr, sym)) = t.strip_suffix(">:").and_then(|r| r.split_once(" <"))
            && let Ok(addr) = u64::from_str_radix(addr, 16)
        {
            blocks.push(Block {
                member: member.clone(),
                raw: sym.to_string(),
                addr,
                insns: Vec::new(),
            });
            open = true;
            continue;
        }
        if !open {
            continue;
        }
        // "1234: \tmnemonic\toperands" or "0000000000001238:  R_X86_64_PLT32\tsym-0x4"
        let Some((addr, body)) = t.split_once(':') else {
            continue;
        };
        let Ok(addr) = u64::from_str_radix(addr.trim(), 16) else {
            continue;
        };
        let body = body.trim();
        let block = blocks.last_mut().unwrap();
        if is_reloc(body) {
            let target = body.split_whitespace().nth(1).unwrap_or("").to_string();
            if let Some(last) = block.insns.last_mut() {
                last.reloc = Some(target);
            }
        } else {
            block.insns.push(Insn {
                addr,
                text: body.to_string(),
                reloc: None,
            });
        }
    }
    (format, blocks)
}

/// Disassembles the handlers `raws`, then each symbol that a `tail` handler, or code it reaches,
/// jumps to and that is not a handler (a helper, for example an outlined function), until no
/// new name comes up. `analyze` checks that helper code as part of the handler. `dis`
/// disassembles a list of symbols.
fn disassemble_reach(
    raws: &[String],
    mut dis: impl FnMut(&[String]) -> Result<(String, Vec<Block>), String>,
) -> Result<(String, Vec<Block>), String> {
    let mut seen: BTreeSet<String> = raws.iter().cloned().collect();
    let mut names = raws.to_vec();
    let mut format = String::new();
    let mut all = Vec::new();
    while !names.is_empty() {
        let (f, blocks) = dis(&names)?;
        let (arch, _) = arch_of(&f)?;
        names.clear();
        for b in &blocks {
            if parse_handler(&b.raw).is_some_and(|h| h.2 == Class::Exit) {
                continue;
            }
            for i in &b.insns {
                if let Kind::Jump(Some(t)) | Kind::CondJump(Some(t)) = classify(arch, i, b)
                    && seen.insert(t.clone())
                {
                    names.push(t);
                }
            }
        }
        format = f;
        all.extend(blocks);
    }
    Ok((format, all))
}

fn arch_of(format: &str) -> Result<(Arch, &'static str), String> {
    let f = format.to_ascii_lowercase();
    if f.contains("arm64") || f.contains("aarch64") {
        Ok((
            Arch::A64,
            if f.contains("coff") { "a64-win" } else { "a64" },
        ))
    } else if f.contains("x86-64") || f.contains("x86_64") {
        Ok((
            Arch::X64,
            if f.contains("coff") {
                "x64-win64"
            } else {
                "x64-sysv"
            },
        ))
    } else {
        Err(format!("unsupported file format {format:?}"))
    }
}

#[derive(Debug, PartialEq)]
enum Kind {
    Ret,
    IndJump,
    IndCall,
    /// Direct jump; the target symbol if it leaves the block (see `exit_target`).
    Jump(Option<String>),
    CondJump(Option<String>),
    Call(String),
    Trap,
    Other,
}

/// Branch target symbol: the relocation if there is one, else the `<sym+0x..>` annotation.
fn target_sym(insn: &Insn) -> Option<String> {
    let s = if let Some(r) = &insn.reloc {
        r.clone()
    } else {
        let a = insn.text.find('<')?;
        let b = insn.text[a..].find('>')? + a;
        insn.text[a + 1..b].to_string()
    };
    // Drop an addend such as "+0x24" or "-0x4".
    let cut = ["+0x", "-0x"].iter().filter_map(|p| s.rfind(p)).max();
    Some(match cut {
        Some(i) => s[..i].to_string(),
        None => s,
    })
}

/// Target symbol of a direct branch that leaves block `b`; `None` if it stays in `b`.
/// Without a relocation, the printed address decides first: llvm-objdump can label a branch
/// inside `b` with another symbol at the same address (Mach-O `<ltmp0+0xc>`). An ELF relocation
/// against a function section (`.text.<sym>`, for a local `<sym>`) names `<sym>`.
fn exit_target(insn: &Insn, b: &Block) -> Option<String> {
    let t = target_sym(insn)?;
    if insn.reloc.is_none()
        && let Some(a) = insn.text.find('<').and_then(|i| {
            let w = insn.text[..i].split_whitespace().last()?;
            u64::from_str_radix(w.trim_end_matches(',').strip_prefix("0x")?, 16).ok()
        })
        && b.insns
            .last()
            .is_some_and(|l| (b.addr..=l.addr).contains(&a))
    {
        return None;
    }
    let t = t.strip_prefix(".text.").map(str::to_string).unwrap_or(t);
    (t != b.raw).then_some(t)
}

fn classify(arch: Arch, insn: &Insn, b: &Block) -> Kind {
    let mut words = insn.text.split_whitespace();
    let mut m = words.next().unwrap_or("");
    // x86 prefixes that llvm-objdump prints as separate words.
    while matches!(
        m,
        "rex64" | "notrack" | "bnd" | "lock" | "rep" | "repne" | "cs" | "ds" | "data16"
    ) {
        m = words.next().unwrap_or("");
    }
    let ops: String = words.collect::<Vec<_>>().join(" ");
    let out = || exit_target(insn, b);
    match arch {
        Arch::A64 => match m {
            "ret" | "retaa" | "retab" => Kind::Ret,
            "br" | "braa" | "brab" | "braaz" | "brabz" => Kind::IndJump,
            "blr" | "blraa" | "blrab" | "blraaz" | "blrabz" => Kind::IndCall,
            "b" => Kind::Jump(out()),
            "bl" => Kind::Call(target_sym(insn).unwrap_or_default()),
            "cbz" | "cbnz" | "tbz" | "tbnz" => Kind::CondJump(out()),
            _ if m.starts_with("b.") || m.starts_with("bc.") => Kind::CondJump(out()),
            "brk" | "udf" | "hlt" => Kind::Trap,
            _ => Kind::Other,
        },
        Arch::X64 => match m {
            "ret" | "retq" | "retl" => Kind::Ret,
            "jmp" | "jmpq" if ops.starts_with('*') => Kind::IndJump,
            "jmp" | "jmpq" => Kind::Jump(out()),
            "call" | "callq" if ops.starts_with('*') => Kind::IndCall,
            "call" | "callq" => Kind::Call(target_sym(insn).unwrap_or_default()),
            "ud2" | "ud1" | "ud0" | "int3" | "hlt" => Kind::Trap,
            _ if m.starts_with('j') => Kind::CondJump(out()),
            _ => Kind::Other,
        },
    }
}

/// True if the instruction saves registers or grows the stack: the handler builds a frame.
fn is_frame_setup(arch: Arch, text: &str) -> bool {
    let t = text.replace(char::is_whitespace, " ");
    match arch {
        Arch::A64 => {
            (t.starts_with("stp ") || t.starts_with("str "))
                && t.contains("[sp, #-")
                && t.ends_with("]!")
                || t.starts_with("sub sp, sp")
                || t.starts_with("pacibsp")
        }
        Arch::X64 => t.starts_with("push") || (t.starts_with("sub") && t.ends_with("%rsp")),
    }
}

#[derive(Default, Debug)]
struct Stats {
    /// The first instruction builds a frame, so the fast path pays for it too.
    frame_on_entry: bool,
    insns: usize,
    ind_jumps: usize,
    direct_tail: usize,
    returns: usize,
    ind_calls: usize,
    handler_calls: usize,
    runtime_calls: usize,
    /// Helper symbols that a `tail` handler jumps to; their code is counted above.
    helpers: Vec<String>,
    /// Jump targets of a `tail` handler that are neither a handler nor a block in `blocks`.
    unknown: Vec<String>,
}

/// Counts the exits of handler block `b`. In a `tail` handler, a direct jump to a handler in
/// `handlers` is a tail exit. A direct jump to any other symbol goes to its block in `blocks`,
/// and that code counts as part of the handler, so a helper cannot hide a call or a return.
/// A target with no block is an unknown exit.
fn analyze(
    arch: Arch,
    b: &Block,
    class: Class,
    blocks: &[Block],
    handlers: &BTreeSet<&str>,
) -> Stats {
    let mut s = Stats {
        insns: b.insns.len(),
        frame_on_entry: b
            .insns
            .first()
            .is_some_and(|i| is_frame_setup(arch, &i.text)),
        ..Stats::default()
    };
    let mut seen = BTreeSet::from([b.raw.as_str()]);
    let mut todo = vec![b];
    while let Some(b) = todo.pop() {
        for i in &b.insns {
            match classify(arch, i, b) {
                Kind::Ret => s.returns += 1,
                Kind::IndJump => s.ind_jumps += 1,
                Kind::IndCall => s.ind_calls += 1,
                Kind::Jump(Some(t)) | Kind::CondJump(Some(t)) => {
                    if class == Class::Exit || handlers.contains(t.as_str()) {
                        s.direct_tail += 1;
                    } else if let Some(h) =
                        blocks.iter().find(|h| h.raw == t && h.member == b.member)
                    {
                        if seen.insert(h.raw.as_str()) {
                            s.helpers.push(t);
                            todo.push(h);
                        }
                    } else {
                        s.unknown.push(t);
                    }
                }
                Kind::Call(t) => {
                    if parse_handler(&t).is_some() {
                        s.handler_calls += 1;
                    } else {
                        s.runtime_calls += 1;
                    }
                }
                _ => {}
            }
        }
    }
    s
}

/// Problems with one handler, empty if it passes.
fn verdict(class: Class, s: &Stats) -> Vec<String> {
    let mut p = Vec::new();
    if class == Class::Tail && s.returns > 0 {
        p.push(format!("{} return(s) in a tail handler", s.returns));
    }
    if s.ind_calls > 0 {
        p.push(format!(
            "{} indirect call(s): non-tail dispatch",
            s.ind_calls
        ));
    }
    if s.handler_calls > 0 {
        p.push(format!(
            "{} call(s) to a handler: non-tail dispatch",
            s.handler_calls
        ));
    }
    if !s.unknown.is_empty() {
        p.push(format!(
            "{} jump(s) to code that is not in the disassembly: {}",
            s.unknown.len(),
            s.unknown.join(", ")
        ));
    }
    if class == Class::Tail && s.ind_jumps + s.direct_tail == 0 {
        p.push("no tail jump".into());
    }
    p
}

/// Parameters of each `define` in an LLVM IR file whose name is a handler.
fn ir_params(ll: &Path) -> Result<BTreeMap<String, Vec<String>>, String> {
    let text = fs::read_to_string(ll).map_err(|e| format!("{}: {e}", ll.display()))?;
    let mut map = BTreeMap::new();
    // "@a = unnamed_addr alias i64 (ptr, ...), ptr @b": a merged function. a gets b's params.
    let mut aliases = Vec::new();
    for line in text
        .lines()
        .filter(|l| l.starts_with('@') && l.contains(" alias "))
    {
        let name = line[1..].split(' ').next().unwrap_or("").trim_matches('"');
        let target = line
            .rsplit('@')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        if parse_handler(name).is_some() {
            aliases.push((name.to_string(), target.to_string()));
        }
    }
    for line in text.lines().filter(|l| l.starts_with("define ")) {
        let Some(at) = line.find('@') else { continue };
        let rest = &line[at + 1..];
        let (name, after) = if let Some(q) = rest.strip_prefix('"') {
            let end = q.find('"').ok_or("unterminated IR name")?;
            (&q[..end], &q[end + 1..])
        } else {
            let end = rest.find('(').ok_or("IR define without '('")?;
            (&rest[..end], &rest[end..])
        };
        if parse_handler(name).is_none() {
            continue;
        }
        let after = after
            .strip_prefix('(')
            .ok_or("IR define: no parameter list")?;
        let mut depth = 0i32;
        let mut cur = String::new();
        let mut params = Vec::new();
        for c in after.chars() {
            match c {
                '(' | '[' | '{' | '<' => depth += 1,
                ')' if depth == 0 => break,
                ')' | ']' | '}' | '>' => depth -= 1,
                ',' if depth == 0 => {
                    params.push(cur.trim().to_string());
                    cur.clear();
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        if !cur.trim().is_empty() {
            params.push(cur.trim().to_string());
        }
        map.insert(name.to_string(), params);
    }
    for (name, target) in aliases {
        if let Some(p) = map.get(&target).cloned() {
            map.insert(name, p);
        }
    }
    Ok(map)
}

fn check_file(
    o: &Opts,
    tools: &Tools,
    file: &Path,
    ll: Option<&Path>,
    label: &str,
) -> Result<bool, String> {
    let Some(Loaded {
        arch,
        abi,
        format,
        handlers,
        raws,
        blocks,
    }) = load(tools, file)?
    else {
        println!(
            "== {label}: {}\n   FAIL: no handler symbols ({PREFIX}*) found",
            file.display()
        );
        return Ok(false);
    };
    println!("== {label} ({abi}, {format}): {}", file.display());

    let handler_raws: BTreeSet<&str> = raws.iter().map(String::as_str).collect();
    let mut ok = true;
    let mut exps: BTreeMap<String, Vec<&Handler>> = BTreeMap::new();
    for h in &handlers {
        exps.entry(h.exp.clone()).or_default().push(h);
    }
    for (exp, hs) in &exps {
        let mut non_tail = 0;
        let (mut tails, mut exits, mut tail_ok, mut runtime, mut exit_rets) = (0, 0, 0, 0, 0);
        let mut aliases = 0;
        let mut framed: Vec<String> = Vec::new();
        for h in hs {
            let Some(b) = block_of(&handlers, &blocks, h) else {
                println!("   FAIL {}: not found in the disassembly", h.name);
                ok = false;
                continue;
            };
            if b.raw != h.raw {
                aliases += 1;
            }
            let s = analyze(arch, b, h.class, &blocks, &handler_raws);
            let p = verdict(h.class, &s);
            non_tail += s.ind_calls
                + s.handler_calls
                + s.unknown.len()
                + if h.class == Class::Tail { s.returns } else { 0 };
            runtime += s.runtime_calls;
            if s.frame_on_entry {
                framed.push(h.name.rsplit("__").next().unwrap_or(&h.name).to_string());
            }
            match h.class {
                Class::Tail => {
                    tails += 1;
                    if p.is_empty() {
                        tail_ok += 1;
                    }
                }
                Class::Exit => {
                    exits += 1;
                    exit_rets += s.returns;
                }
            }
            if o.verbose || !p.is_empty() {
                println!(
                    "   {} {:<34} insns={:<3} frame={} br={} b_out={} ret={} blr={} bl_handler={} bl_rt={}{}{}",
                    if p.is_empty() { "ok  " } else { "FAIL" },
                    h.name,
                    s.insns,
                    if s.frame_on_entry { "entry" } else { "no   " },
                    s.ind_jumps,
                    s.direct_tail,
                    s.returns,
                    s.ind_calls,
                    s.handler_calls,
                    s.runtime_calls,
                    if s.helpers.is_empty() {
                        String::new()
                    } else {
                        format!(" via={}", s.helpers.join(","))
                    },
                    if p.is_empty() {
                        String::new()
                    } else {
                        format!("  <- {}", p.join("; "))
                    }
                );
            }
            ok &= p.is_empty();
        }
        println!(
            "   expansion {exp}: {} handlers ({tails} tail, {exits} exit{}); tail handlers that end in tail jumps: {tail_ok}/{tails}; non-tail exits: {non_tail}; returns in exit handlers: {exit_rets}; runtime calls: {runtime}",
            hs.len(),
            if aliases > 0 {
                format!(", {aliases} merged")
            } else {
                String::new()
            },
        );
        framed.sort();
        println!(
            "   expansion {exp}: handlers that build a frame on entry (fast path pays it): {} {:?}",
            framed.len(),
            framed
        );
        if let Some(n) = o.expect
            && hs.len() != n
        {
            println!(
                "   FAIL expansion {exp}: expected {n} handlers, found {}",
                hs.len()
            );
            ok = false;
        }
    }

    if let Some(ll) = ll {
        let params = ir_params(ll)?;
        let mut max = 0;
        let mut args_ok = true;
        let mut names: Vec<&str> = handlers.iter().map(|h| h.name.as_str()).collect();
        names.sort();
        names.dedup();
        for name in &names {
            let Some(ps) = params.get(*name) else {
                println!("   FAIL {name}: no LLVM IR definition in {}", ll.display());
                args_ok = false;
                continue;
            };
            max = max.max(ps.len());
            let mem: Vec<&String> = ps
                .iter()
                .filter(|p| {
                    ["sret", "byval", "inalloca", "preallocated"]
                        .iter()
                        .any(|k| p.contains(k))
                })
                .collect();
            if ps.len() > o.max_args || !mem.is_empty() || ps.iter().any(|p| p == "...") {
                println!("   FAIL {name}: {} parameters {ps:?}", ps.len());
                args_ok = false;
            }
        }
        println!(
            "   handler args (LLVM IR, {} handlers): max {max}, limit {}: {}",
            names.len(),
            o.max_args,
            if args_ok { "PASS" } else { "FAIL" }
        );
        ok &= args_ok;
    }
    println!("   RESULT {abi}: {}", if ok { "PASS" } else { "FAIL" });
    if o.match_loop {
        // Fast tail handlers: one per tail opcode, cold handlers not counted.
        let fast_tail = exps
            .values()
            .map(|hs| {
                hs.iter()
                    .filter(|h| h.class == Class::Tail && !h.name.ends_with("_cold"))
                    .count()
            })
            .max()
            .unwrap_or(0);
        ok &= check_loops(tools, file, arch, abi, fast_tail)?;
    }
    Ok(ok)
}

/// What [`load`] reads from one file.
struct Loaded {
    arch: Arch,
    abi: &'static str,
    /// The file format as llvm-objdump prints it.
    format: String,
    handlers: Vec<Handler>,
    /// The handler symbol names, sorted, each once.
    raws: Vec<String>,
    /// The handlers and the helper code they jump to.
    blocks: Vec<Block>,
}

/// Lists the handler symbols of `file` and disassembles them with the helpers they jump to.
/// `None` if the file has no handler symbol.
fn load(tools: &Tools, file: &Path) -> Result<Option<Loaded>, String> {
    let handlers = nm_handlers(&tools.nm, file)?;
    if handlers.is_empty() {
        return Ok(None);
    }
    let raws: Vec<String> = {
        let mut r: Vec<String> = handlers.iter().map(|h| h.raw.clone()).collect();
        r.sort();
        r.dedup();
        r
    };
    // An archive (staticlib) also holds every std and compiler_builtins member. Extract only
    // the members with handlers and disassemble those.
    let mut members: Vec<&str> = handlers.iter().map(|h| h.member.as_str()).collect();
    members.sort();
    members.dedup();
    let mut format = String::new();
    let mut blocks = Vec::new();
    for m in members {
        let (f, mut b) = in_member(tools, file, m, |obj| {
            disassemble_reach(&raws, |names| disassemble(&tools.objdump, obj, names))
        })?;
        b.iter_mut().for_each(|blk| blk.member = m.to_string());
        format = f;
        blocks.append(&mut b);
    }
    let (arch, abi) = arch_of(&format)?;
    Ok(Some(Loaded {
        arch,
        abi,
        format,
        handlers,
        raws,
        blocks,
    }))
}

/// Finds the block of handler `h` by name. Fallback for a symbol that LLVM merged into another
/// one (an alias at the same address): use that block, but only if exactly one block sits at
/// that address. In objects with one section per function every function starts at address 0,
/// so the fallback never applies there.
fn block_of<'a>(handlers: &[Handler], blocks: &'a [Block], h: &Handler) -> Option<&'a Block> {
    if let Some(mut i) = blocks
        .iter()
        .position(|b| b.raw == h.raw && b.member == h.member)
    {
        // Two labels at one address (an alias): objdump prints the first one with no
        // instructions, then the second one with the code.
        while blocks[i].insns.is_empty()
            && blocks
                .get(i + 1)
                .is_some_and(|n| n.addr == blocks[i].addr && n.member == blocks[i].member)
        {
            i += 1;
        }
        return Some(&blocks[i]);
    }
    let mut same: Vec<&Block> = handlers
        .iter()
        .filter(|g| g.member == h.member && g.addr == h.addr && g.raw != h.raw)
        .filter_map(|g| {
            blocks
                .iter()
                .find(|b| b.raw == g.raw && b.member == g.member)
        })
        .collect();
    same.dedup_by(|a, b| a.raw == b.raw);
    if same.len() == 1 { Some(same[0]) } else { None }
}

/// Runs `f` on `file`, or on archive member `m` of it (extracted to a temporary directory).
fn in_member<T>(
    tools: &Tools,
    file: &Path,
    m: &str,
    f: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    if m.is_empty() {
        return f(file);
    }
    let dir = std::env::temp_dir().join(format!("ep9-disasm-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let r = run_quiet(
        Command::new(&tools.ar)
            .arg("x")
            .arg(format!("--output={}", dir.display()))
            .arg(file)
            .arg(m),
    )
    .and_then(|_| f(&dir.join(m)));
    let _ = fs::remove_dir_all(&dir);
    r
}

/// Match-loop symbols (`ep9loop__*`, Mach-O adds `_`) with their archive member.
fn nm_loops(nm: &Path, file: &Path) -> Result<Vec<(String, String)>, String> {
    let out = run_quiet(Command::new(nm).arg("--defined-only").arg(file))?;
    let mut member = String::new();
    let mut v = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.ends_with(':') && !t.contains(' ') {
            member = t.trim_end_matches(':').to_string();
            continue;
        }
        let parts: Vec<&str> = t.split_whitespace().collect();
        if parts.len() == 3
            && matches!(parts[1], "T" | "t")
            && parts[2].trim_start_matches('_').starts_with(LOOP_PREFIX)
        {
            v.push((parts[2].to_string(), member.clone()));
        }
    }
    v.sort();
    v.dedup();
    Ok(v)
}

/// True if the instruction reads or writes stack memory (not push or pop). `fp`: the function
/// keeps a frame pointer (x86-64 `%rbp`). Without one, `%rbp` is a general register: the
/// match loop keeps the VM frame base there on Linux and Windows.
fn is_stack_access(arch: Arch, text: &str, fp: bool) -> bool {
    match arch {
        Arch::X64 => {
            !text.starts_with("lea") && (text.contains("(%rsp") || fp && text.contains("(%rbp"))
        }
        Arch::A64 => text.contains("[sp") || text.contains("[x29"),
    }
}

/// Shape of one match-loop function.
#[derive(Debug, Default)]
struct LoopStats {
    insns: usize,
    ind_jumps: usize,
    calls: usize,
    /// Stack accesses that are not next to a call, not in the entry block and not in an exit
    /// sequence: candidates for traffic on every dispatch.
    stack_far: Vec<String>,
}

fn analyze_loop(arch: Arch, b: &Block) -> LoopStats {
    let kinds: Vec<Kind> = b.insns.iter().map(|i| classify(arch, i, b)).collect();
    let first_jump = kinds
        .iter()
        .position(|k| *k == Kind::IndJump)
        .unwrap_or(kinds.len());
    let fp = b.insns[..first_jump]
        .iter()
        .any(|i| i.text.replace(char::is_whitespace, " ") == "movq %rsp, %rbp");
    let stack = |t: &str| is_stack_access(arch, t, fp);
    let mut ok = vec![false; b.insns.len()];
    // Entry block: everything before the first dispatch jump (prologue, register setup).
    for (i, k) in kinds.iter().enumerate() {
        if *k == Kind::IndJump {
            break;
        }
        ok[i] = true;
    }
    for (i, k) in kinds.iter().enumerate() {
        match k {
            Kind::Call(_) | Kind::IndCall => {
                let lo = i.saturating_sub(LOOP_CALL_WINDOW);
                let hi = (i + LOOP_CALL_WINDOW).min(b.insns.len() - 1);
                ok[lo..=hi].iter_mut().for_each(|x| *x = true);
            }
            // Exit sequence: the stack restores right before a return.
            Kind::Ret => {
                let mut j = i;
                while j > 0 {
                    let t = &b.insns[j - 1].text;
                    let restore = stack(t)
                        || t.starts_with("pop")
                        || t.contains("%rsp")
                        || t.contains("sp, sp");
                    if !restore {
                        break;
                    }
                    j -= 1;
                    ok[j] = true;
                }
            }
            _ => {}
        }
    }
    LoopStats {
        insns: b.insns.len(),
        ind_jumps: kinds.iter().filter(|k| **k == Kind::IndJump).count(),
        calls: kinds
            .iter()
            .filter(|k| matches!(k, Kind::Call(_) | Kind::IndCall))
            .count(),
        stack_far: b
            .insns
            .iter()
            .zip(&ok)
            .filter(|(i, ok)| !**ok && stack(&i.text))
            .map(|(i, _)| format!("{:x}: {}", i.addr, i.text.replace('\t', " ")))
            .collect(),
    }
}

/// Problems with one match loop on x86-64, empty if it passes. `fast_tail` is the number of
/// fast tail handlers (one per tail opcode).
fn loop_verdict(s: &LoopStats, fast_tail: usize) -> Vec<String> {
    let need = fast_tail.div_ceil(2).max(2);
    let mut p = Vec::new();
    if s.ind_jumps < need {
        p.push(format!(
            "{} indirect jump(s), need >= {need}: the dispatch is not copied into the arms",
            s.ind_jumps
        ));
    }
    if !s.stack_far.is_empty() {
        p.push(format!(
            "{} stack access(es) away from calls: {}",
            s.stack_far.len(),
            s.stack_far.join("; ")
        ));
    }
    p
}

/// Checks the match loops in `file`. The rule applies to the release loop
/// `ep9loop__<expansion>__run` on x86-64; other loop symbols (the dispatch-counting copy)
/// are printed only. Prints one `LOOP <abi>:` line.
fn check_loops(
    tools: &Tools,
    file: &Path,
    arch: Arch,
    abi: &str,
    fast_tail: usize,
) -> Result<bool, String> {
    let loops = nm_loops(&tools.nm, file)?;
    if loops.is_empty() {
        println!("   LOOP {abi}: FAIL (no {LOOP_PREFIX}* symbol)");
        return Ok(false);
    }
    let mut ok = loops.iter().any(|(raw, _)| raw.ends_with("__run"));
    if !ok {
        println!("   FAIL no release loop {LOOP_PREFIX}<expansion>__run");
    }
    for (raw, m) in &loops {
        let (_, blocks) = in_member(tools, file, m, |obj| {
            disassemble(&tools.objdump, obj, std::slice::from_ref(raw))
        })?;
        let Some(b) = blocks.iter().find(|b| &b.raw == raw && !b.insns.is_empty()) else {
            println!("   FAIL {raw}: not found in the disassembly");
            ok = false;
            continue;
        };
        let s = analyze_loop(arch, b);
        let release = raw.ends_with("__run");
        let p = if arch == Arch::X64 && release {
            loop_verdict(&s, fast_tail)
        } else {
            Vec::new()
        };
        println!(
            "   {} match loop {}: insns={} indirect_jumps={} (fast tail handlers: {fast_tail}) calls={} stack_far={}{}",
            match (p.is_empty(), release && arch == Arch::X64) {
                (false, _) => "FAIL",
                (true, true) => "ok  ",
                (true, false) => "info",
            },
            raw.trim_start_matches('_'),
            s.insns,
            s.ind_jumps,
            s.calls,
            s.stack_far.len(),
            if p.is_empty() {
                String::new()
            } else {
                format!("  <- {}", p.join("; "))
            }
        );
        ok &= p.is_empty();
    }
    if arch == Arch::X64 {
        println!("   LOOP {abi}: {}", if ok { "PASS" } else { "FAIL" });
    } else {
        println!("   LOOP {abi}: REPORT (no rule: the aarch64 release expansion is sibling calls)");
    }
    Ok(ok)
}

/// The comment head of a frame golden file (`--frames GOLDEN --bless` writes it).
const FRAMES_HEAD: &str = "\
# The hot ep9_vm handlers (name not ending in `_cold`; on x86-64 also disasm.rs HOT_COLD:
# `Wide_cold`, `ExtraWide_cold`, `CallProperty0_cold`..`CallProperty2_cold`) that build a frame
# or save a callee-saved register on entry, and the callee-saved registers each one saves: one
# `<expansion> <handler> <saves>` row. Checked by `cargo xtask disasm-check --frames <this file>`
# in the CI disasm job (rule 81: no new entry save on a hot x86 handler; docs/plan/10-quality.md,
# 2026-11-16 note). A new row, a lost row or a changed count fails. A change that is meant
# updates this file in the same PR: run that command with `--bless`, or copy the CI `frame` rows.
";

/// One row of the frame golden: a hot handler that builds a frame on entry.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FrameRow {
    exp: String,
    op: String,
    /// The registers that the entry prologue saves ([`entry_saves`]).
    saves: usize,
}

/// The comment of the x86-64 alignment column in a frame golden (after [`FRAMES_HEAD`]; only a
/// golden with `align64` lines has it, so the aarch64 goldens keep their text).
const ALIGN_HEAD: &str = "\
# x86-64 alignment column (H15 x64 function alignment; M10-PRE-ALIGN-FORM): one
# `align64 <expansion> <aligned>/<total>` line. Of the <total> hot handlers, <aligned> start on a
# 64-byte line: the alignment of the handler's section is 64 or more and its offset in that section
# is a multiple of 64. A handler that does not fails the check (the x86-64 rustflags of the root
# .cargo/config.toml are lost: RUSTFLAGS in the environment replaces them), and so does a line that
# differs from this file.
";

/// One `align64` line of the frame golden (x86-64 only; [`align_rows`]): of the `total` hot
/// handlers of expansion `exp`, `aligned` start on a 64-byte line.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AlignRow {
    exp: String,
    aligned: usize,
    total: usize,
}

/// A frame golden file: its target, its `align64` lines and its frame rows, each sorted.
#[derive(Debug, PartialEq, Eq)]
struct Golden {
    target: String,
    align: Vec<AlignRow>,
    rows: Vec<FrameRow>,
}

/// Registers that one prologue instruction saves, or `None` if the instruction is not part of
/// a prologue. x86-64: a `push` of a callee-saved register ([`callee_saved`]: rbx, rbp, r12-r15)
/// saves 1, a `push` of any other register (a stack alignment push, `push %rax`) saves 0
/// (M9-P1-WPRE-3, route MC-w42-4); `sub ..., %rsp` and setting `%rbp` from `%rsp` save 0 (the
/// handler ABI is sysv64 on every OS, `dsl::handler!`, so no xmm register is callee-saved, also
/// on Win64). aarch64: an `stp` or `str` of callee-saved registers (x19-x30, d8-d15) to
/// `[sp ...]` saves 1 per register; `sub sp, sp`, `pacibsp`/`paciasp` and setting x29 from sp
/// save 0.
fn prologue_saves(arch: Arch, text: &str) -> Option<usize> {
    let t = text.replace(char::is_whitespace, " ");
    let (m, ops) = t.split_once(' ').unwrap_or((t.as_str(), ""));
    let ops = ops.trim();
    match arch {
        Arch::X64 => {
            if m.starts_with("push") {
                Some(usize::from(
                    callee_saved(arch, ops).is_some_and(|(_, full)| full),
                ))
            } else if m.starts_with("sub") && ops.ends_with("%rsp")
                || m.starts_with("mov") && ops == "%rsp, %rbp"
                || m.starts_with("lea") && ops.ends_with("(%rsp), %rbp")
            {
                Some(0)
            } else {
                None
            }
        }
        Arch::A64 => {
            let callee_saved = |r: &str| match r.as_bytes().first() {
                Some(b'x') => r[1..].parse::<u32>().is_ok_and(|n| (19..=30).contains(&n)),
                Some(b'd') => r[1..].parse::<u32>().is_ok_and(|n| (8..=15).contains(&n)),
                _ => r == "fp" || r == "lr",
            };
            if m == "sub" && ops.starts_with("sp, sp,")
                || matches!(m, "pacibsp" | "paciasp")
                || m == "mov" && ops == "x29, sp"
                || m == "add" && ops.starts_with("x29, sp,")
            {
                Some(0)
            } else if matches!(m, "stp" | "str")
                && let Some((regs, mem)) = ops.split_once(", [")
                && (mem.starts_with("sp]") || mem.starts_with("sp,"))
            {
                let regs: Vec<&str> = regs.split(", ").collect();
                regs.iter().all(|r| callee_saved(r)).then_some(regs.len())
            } else {
                None
            }
        }
    }
}

/// The mnemonic and the operands of one instruction, whitespace made single spaces. x86 prefix
/// words (`lock`, `rep`, ...) are skipped, as in [`classify`].
fn mnemonic_ops(text: &str) -> (String, String) {
    let t = text.replace(char::is_whitespace, " ");
    let mut words = t.split(' ').filter(|w| !w.is_empty()).peekable();
    while words
        .next_if(|m| {
            matches!(
                *m,
                "rex64" | "notrack" | "bnd" | "lock" | "rep" | "repne" | "cs" | "ds" | "data16"
            )
        })
        .is_some()
    {}
    let m = words.next().unwrap_or("").to_string();
    (m, words.collect::<Vec<_>>().join(" "))
}

/// The callee-saved register of the handler ABI that register operand `r` is a part of, by its
/// full name, and true if `r` is the full register. x86-64 (sysv64 on every OS,
/// `dsl::handler!`): rbx, rbp, r12-r15 (`%ebx`, `%bl`, `%r12d` and the other parts too).
/// aarch64: x19-x30 (`w19`, `fp`, `lr` too) and d8-d15 (`s8`, `q8`, `v8.2d` too; only the d form
/// is the full register that AAPCS64 saves).
fn callee_saved(arch: Arch, r: &str) -> Option<(String, bool)> {
    match arch {
        Arch::X64 => {
            let r = r.strip_prefix('%')?;
            let full = match r {
                "rbx" | "ebx" | "bx" | "bl" | "bh" => "rbx",
                "rbp" | "ebp" | "bp" | "bpl" => "rbp",
                _ => {
                    let n = r.strip_prefix('r')?;
                    let d = n.find(|c: char| !c.is_ascii_digit()).unwrap_or(n.len());
                    if !matches!(&n[d..], "" | "d" | "w" | "b") {
                        return None;
                    }
                    match &n[..d] {
                        "12" => "r12",
                        "13" => "r13",
                        "14" => "r14",
                        "15" => "r15",
                        _ => return None,
                    }
                }
            };
            Some((full.to_string(), r == full))
        }
        Arch::A64 => {
            let r = r.split(['.', '[']).next().unwrap_or(r);
            match r {
                "fp" => return Some(("x29".to_string(), true)),
                "lr" => return Some(("x30".to_string(), true)),
                _ => {}
            }
            let (p, n) = r.split_at_checked(1)?;
            let n: u32 = n.parse().ok()?;
            match p {
                "x" | "w" if (19..=30).contains(&n) => Some((format!("x{n}"), p == "x")),
                "d" | "s" | "h" | "b" | "q" | "v" if (8..=15).contains(&n) => {
                    Some((format!("d{n}"), p == "d"))
                }
                _ => None,
            }
        }
    }
}

/// One instruction that writes registers to memory.
struct Store<'a> {
    /// The register operands that the instruction writes to memory.
    srcs: Vec<&'a str>,
    /// The base register of the address (`rsp`, `sp`, `x29`, ...).
    base: &'a str,
}

/// The store that instruction `m ops` ([`mnemonic_ops`]) does, or `None` if it writes no
/// register to memory. x86-64 (AT&T): an instruction with a register operand and a memory last
/// operand that it writes (not `cmp`, `test`, `bt`); `push` is not a store here. aarch64: the
/// `st*` stores (the status register of `stxr` and `stxp` is not a source), `swp*` and the
/// `ld<op>` atomics (the first register is the source) and `cas*`.
fn store_of<'a>(arch: Arch, m: &str, ops: &'a str) -> Option<Store<'a>> {
    match arch {
        Arch::X64 => {
            let parts: Vec<&str> = ops.split(", ").collect();
            let (last, srcs) = parts.split_last()?;
            let reads_only = m.starts_with("cmp") && !m.starts_with("cmpxchg")
                || m.starts_with("test")
                || matches!(m, "bt" | "btw" | "btl" | "btq")
                || ["ucomis", "comis", "vucomis", "vcomis", "ptest", "vptest"]
                    .iter()
                    .any(|p| m.starts_with(p));
            if srcs.is_empty() || reads_only || m.starts_with("push") {
                return None;
            }
            let (_, mem) = last.split_once('(')?;
            let base = mem.split([',', ')']).next()?.trim_start_matches('%');
            let srcs: Vec<&str> = srcs
                .iter()
                .copied()
                .filter(|s| s.starts_with('%'))
                .collect();
            (!srcs.is_empty()).then_some(Store { srcs, base })
        }
        Arch::A64 => {
            const LD_ATOMIC: [&str; 8] = [
                "ldadd", "ldclr", "ldeor", "ldset", "ldsmax", "ldsmin", "ldumax", "ldumin",
            ];
            let first_only = m.starts_with("swp") || LD_ATOMIC.iter().any(|p| m.starts_with(p));
            if !(m.starts_with("st") || m.starts_with("cas") || first_only) {
                return None;
            }
            let (regs, mem) = ops.split_once('[')?;
            let base = mem.split([',', ']']).next()?.trim();
            let mut srcs: Vec<&str> = regs
                .split(',')
                .map(|r| r.trim().trim_matches(['{', '}']).trim())
                .filter(|r| !r.is_empty())
                .collect();
            if (m.starts_with("stx") || m.starts_with("stlx")) && !srcs.is_empty() {
                srcs.remove(0);
            } else if first_only {
                srcs.truncate(1);
            }
            Some(Store { srcs, base })
        }
    }
}

/// The registers that instruction `m ops` ([`mnemonic_ops`]) writes, as far as the entry scan
/// needs them: x86-64 the last operand if it is a register (all register operands of `xchg`,
/// `xadd` and `cmpxchg`); aarch64 the first operand (all registers before the address of a
/// load or an atomic, the status register of `stxr`), and the base of a pre-index or
/// post-index address. Compares, tests, pushes and stores write none.
fn written_regs(arch: Arch, m: &str, ops: &str) -> Vec<String> {
    let mut w = Vec::new();
    match arch {
        Arch::X64 => {
            let parts: Vec<&str> = ops.split(", ").filter(|p| !p.is_empty()).collect();
            let none = m.starts_with("cmp") && !m.starts_with("cmpxchg")
                || m.starts_with("test")
                || m.starts_with("push")
                || matches!(m, "bt" | "btw" | "btl" | "btq");
            if none {
            } else if ["xchg", "xadd", "cmpxchg"].iter().any(|p| m.starts_with(p)) {
                w.extend(
                    parts
                        .iter()
                        .filter(|p| p.starts_with('%'))
                        .map(|p| p.to_string()),
                );
            } else if let Some(l) = parts.last().filter(|l| l.starts_with('%')) {
                w.push(l.to_string());
            }
        }
        Arch::A64 => {
            if let Some((regs, mem)) = ops.split_once('[') {
                let base = mem.split([',', ']']).next().unwrap_or("").trim();
                if mem.contains("]!") || mem.contains("], ") {
                    w.push(base.to_string());
                }
                let regs = regs
                    .split(',')
                    .map(|r| r.trim().trim_matches(['{', '}']).trim())
                    .filter(|r| !r.is_empty());
                if m.starts_with("ld") || m.starts_with("swp") || m.starts_with("cas") {
                    w.extend(regs.map(str::to_string));
                } else if m.starts_with("stx") || m.starts_with("stlx") {
                    w.extend(regs.take(1).map(str::to_string));
                }
            } else if !matches!(
                m,
                "cmp" | "cmn" | "tst" | "ccmp" | "ccmn" | "fcmp" | "fcmpe" | "fccmp" | "fccmpe"
            ) && let Some(r) = ops.split(", ").next().filter(|r| !r.is_empty())
            {
                w.push(r.to_string());
            }
        }
    }
    w
}

/// The address that direct branch `insn` jumps to, from its printed operand (`b 0x40 <h+0x40>`,
/// `jmp 0x40 <h+0x40>`); `None` if it has a relocation or no address.
fn branch_addr(insn: &Insn) -> Option<u64> {
    if insn.reloc.is_some() {
        return None;
    }
    let head = insn.text.split('<').next()?;
    let w = head.split_whitespace().last()?;
    u64::from_str_radix(w.trim_end_matches(',').strip_prefix("0x")?, 16).ok()
}

/// The registers that handler block `b` saves on entry, or `Err(<instruction>)` for a store of
/// a callee-saved register in a form that the rule does not know. The rule has two parts:
///
/// 1. The leading prologue: from the first instruction, each instruction that
///    [`prologue_saves`] accepts adds its count, up to the first instruction that it does not
///    accept (an argument spill, a load, a move). A `push` counts 1 only for a callee-saved
///    register; a stack alignment push (`push %rax`) counts 0 (M9-P1-WPRE-3).
/// 2. The rest of the entry code, up to the first conditional branch, call, return, trap,
///    indirect jump or jump out of the handler (a tail jump; [`classify`]). An unconditional
///    jump to an address in `b` is followed, once for each target, and the scan goes on there
///    (that code runs on every entry too). A save of a callee-saved register of the handler
///    ABI ([`callee_saved`]; x86-64: sysv64 on every OS, `dsl::handler!`) adds 1 for each
///    register that is not saved yet and that no earlier instruction of the scan wrote
///    ([`written_regs`]). A save is: x86-64 a `push`, or a `mov`/`movq` of the full register
///    to memory based on `%rsp`, or on `%rbp` after the scan set `%rbp` from `%rsp`; aarch64
///    an `stp`, `str` or `stur` (pre-index forms too) of x19-x30 or d8-d15 to memory based on
///    `sp`, or on x29 after the scan set x29 from `sp`. A store of a register that an earlier
///    instruction wrote is a spill (`mov x19, x1; stp x19, x20, [sp]` saves no new
///    register): LLVM saves a callee-saved register before its first write.
///
/// Any other store of a callee-saved register that is not saved or written yet (a part of the
/// register, another mnemonic, another base) is an error, not a silent 0: the check must learn
/// the form first. A save in a block that a conditional branch reaches (shrink-wrapping) is not
/// an entry save: that path does not run on every dispatch.
fn entry_saves(arch: Arch, b: &Block) -> Result<usize, String> {
    let mut seen = BTreeSet::new();
    let mut written = BTreeSet::new();
    let mut fp = false;
    let mut saves = 0;
    let mut leading = true;
    let mut jumped = BTreeSet::new();
    let mut at = 0;
    while let Some(i) = b.insns.get(at) {
        at += 1;
        let (m, ops) = mnemonic_ops(&i.text);
        let lead = if leading {
            prologue_saves(arch, &i.text)
        } else {
            None
        };
        if let Some(n) = lead {
            saves += n;
        } else {
            leading = false;
            match classify(arch, i, b) {
                Kind::Other => {}
                Kind::Jump(None) => {
                    match branch_addr(i).and_then(|a| b.insns.iter().position(|x| x.addr == a)) {
                        Some(t) if jumped.insert(t) => {
                            at = t;
                            continue;
                        }
                        _ => break,
                    }
                }
                _ => break,
            }
        }
        // The registers that this instruction stores, and if the form is a save.
        let (srcs, plain): (Vec<&str>, bool) = match arch {
            Arch::X64 if m.starts_with("push") => (vec![ops.as_str()], true),
            Arch::X64 => match store_of(arch, &m, &ops) {
                Some(s) => {
                    let base = s.base == "rsp" || s.base == "rbp" && fp;
                    (s.srcs, matches!(m.as_str(), "mov" | "movq") && base)
                }
                None => (Vec::new(), false),
            },
            Arch::A64 => match store_of(arch, &m, &ops) {
                Some(s) => {
                    let base = s.base == "sp" || matches!(s.base, "x29" | "fp") && fp;
                    (s.srcs, matches!(m.as_str(), "stp" | "str" | "stur") && base)
                }
                None => (Vec::new(), false),
            },
        };
        for r in srcs {
            let Some((reg, full)) = callee_saved(arch, r) else {
                continue;
            };
            if lead.is_some() {
                // A leading prologue instruction: its count is in `saves` already.
                seen.insert(reg);
            } else if seen.contains(&reg) || written.contains(&reg) {
                // A spill, or a register that is counted already.
            } else if plain && full {
                seen.insert(reg);
                saves += 1;
            } else {
                return Err(i.text.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
        // The frame pointer: set from the stack pointer, or a general register again.
        let sets_fp = match arch {
            Arch::X64 => {
                m.starts_with("mov") && ops == "%rsp, %rbp"
                    || m.starts_with("lea") && ops.ends_with("(%rsp), %rbp")
            }
            Arch::A64 => {
                m == "mov" && ops == "x29, sp" || m == "add" && ops.starts_with("x29, sp,")
            }
        };
        for r in written_regs(arch, &m, &ops) {
            if let Some((reg, _)) = callee_saved(arch, &r) {
                if reg == if arch == Arch::X64 { "rbp" } else { "x29" } {
                    fp = sets_fp;
                }
                written.insert(reg);
            }
        }
    }
    Ok(saves)
}

/// The cold handlers that the frame check scans as hot handlers on x86-64. `Wide_cold` and
/// `ExtraWide_cold` run the `ops::prefix_set` fast bodies there through `interp::tails::wide`
/// (M9-P1 X86-WIDE-FAST; `ops::PREFIXED_FAST` is false; MC-w43-2). `CallProperty0_cold`..
/// `CallProperty2_cold` run the host call of `slow::call_property` on x86-64 (M9
/// OXC-W2-HOSTCALL, MC-w44-4). The ics forms are a tail jump to the sib handler with no save
/// (`@forward`), so they make no row.
const HOT_COLD: [&str; 5] = [
    "Wide_cold",
    "ExtraWide_cold",
    "CallProperty0_cold",
    "CallProperty1_cold",
    "CallProperty2_cold",
];

/// True if handler `op` of `arch` is hot for the frame check: its name does not end in
/// `_cold`, or it is in [`HOT_COLD`] on x86-64.
fn is_hot(arch: Arch, op: &str) -> bool {
    !op.ends_with("_cold") || (arch == Arch::X64 && HOT_COLD.contains(&op))
}

/// The hot handlers ([`is_hot`]) that build a frame on entry or save a
/// callee-saved register on entry, with their entry saves, sorted. Every hot handler is scanned
/// ([`entry_saves`]; an unknown save form in any of them is an error). A row is made when the
/// first instruction builds a frame (the frame list of the default check, cold handlers left
/// out), or when the entry code saves a callee-saved register: a red-zone `mov` save as the
/// first instruction makes a row too (M9-P1-WPRE-3, route FRAME-GOLDEN-MOVFIRST).
fn frame_rows(arch: Arch, handlers: &[Handler], blocks: &[Block]) -> Result<Vec<FrameRow>, String> {
    let mut rows = BTreeMap::new();
    for h in handlers {
        let op = h.name.rsplit("__").next().unwrap_or(&h.name);
        if !is_hot(arch, op) {
            continue;
        }
        let b = block_of(handlers, blocks, h)
            .ok_or_else(|| format!("{}: not found in the disassembly", h.name))?;
        let saves =
            entry_saves(arch, b).map_err(|i| format!("unknown save form: {} {i}", h.name))?;
        if saves > 0
            || b.insns
                .first()
                .is_some_and(|i| is_frame_setup(arch, &i.text))
        {
            rows.insert((h.exp.clone(), op.to_string()), saves);
        }
    }
    Ok(rows
        .into_iter()
        .map(|((exp, op), saves)| FrameRow { exp, op, saves })
        .collect())
}

/// The golden file text for the `align64` lines `align` and the frame rows `rows` of `target`.
fn frames_text(target: &str, align: &[AlignRow], rows: &[FrameRow]) -> String {
    let mut s = FRAMES_HEAD.to_string();
    if !align.is_empty() {
        s.push_str(ALIGN_HEAD);
    }
    s.push_str(&format!("target {target}\n"));
    for a in align {
        s.push_str(&format!("align64 {} {}/{}\n", a.exp, a.aligned, a.total));
    }
    for r in rows {
        s.push_str(&format!("{} {} {}\n", r.exp, r.op, r.saves));
    }
    s
}

/// Reads a golden file: its target, its `align64` lines and its rows, sorted.
fn parse_frames(text: &str) -> Result<Golden, String> {
    let mut target: Option<String> = None;
    let mut rows = Vec::new();
    let mut align = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        match l.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["target", t] => {
                if target.replace(t.to_string()).is_some() {
                    return Err(format!("frame golden line {}: a second target line", n + 1));
                }
            }
            ["align64", exp, frac] => {
                let bad = || {
                    format!(
                        "frame golden line {}: not `align64 <expansion> <aligned>/<total>`: {l:?}",
                        n + 1
                    )
                };
                let (a, t) = frac.split_once('/').ok_or_else(bad)?;
                let (aligned, total) =
                    (a.parse().map_err(|_| bad())?, t.parse().map_err(|_| bad())?);
                if aligned > total {
                    return Err(bad());
                }
                align.push(AlignRow {
                    exp: exp.to_string(),
                    aligned,
                    total,
                });
            }
            [exp, op, saves] => rows.push(FrameRow {
                exp: exp.to_string(),
                op: op.to_string(),
                saves: saves
                    .parse()
                    .map_err(|e| format!("frame golden line {}: {e}: {l:?}", n + 1))?,
            }),
            _ => {
                return Err(format!(
                    "frame golden line {}: not `target <triple>`, `align64 <expansion> <aligned>/<total>` or `<expansion> <handler> <saves>`: {l:?}",
                    n + 1
                ));
            }
        }
    }
    let target = target.ok_or("frame golden: no `target` line")?;
    rows.sort();
    if let Some(w) = rows
        .windows(2)
        .find(|w| (&w[0].exp, &w[0].op) == (&w[1].exp, &w[1].op))
    {
        return Err(format!("frame golden: {} {} twice", w[0].exp, w[0].op));
    }
    align.sort();
    if let Some(w) = align.windows(2).find(|w| w[0].exp == w[1].exp) {
        return Err(format!("frame golden: align64 {} twice", w[0].exp));
    }
    Ok(Golden {
        target,
        align,
        rows,
    })
}

/// The differences between the `align64` lines `align` and the rows `got` of this build of
/// `target` and the golden `text`; empty if they are equal. Every difference fails: a new frame
/// or more saves breaks rule 81, and a lost frame or fewer saves leaves a stale golden that would
/// hide a later return. An `align64` line that is new, lost or changed fails too (an aarch64
/// build has none).
fn frames_verdict(
    target: &str,
    text: &str,
    align: &[AlignRow],
    got: &[FrameRow],
) -> Result<Vec<String>, String> {
    let Golden {
        target: want_target,
        align: want_align,
        rows: want,
    } = parse_frames(text)?;
    let mut p = Vec::new();
    if want_target != target {
        p.push(format!(
            "the golden is for {want_target}, this build is for {target}"
        ));
    }
    let frac = |a: &AlignRow| format!("{}/{}", a.aligned, a.total);
    let wa: BTreeMap<_, String> = want_align
        .iter()
        .map(|a| (a.exp.clone(), frac(a)))
        .collect();
    let ga: BTreeMap<_, String> = align.iter().map(|a| (a.exp.clone(), frac(a))).collect();
    for (exp, g) in &ga {
        match wa.get(exp) {
            None => p.push(format!("new align64 line: {exp} {g}")),
            Some(w) if w != g => p.push(format!("align64 changed: {exp} {w} -> {g}")),
            Some(_) => {}
        }
    }
    for (exp, w) in &wa {
        if !ga.contains_key(exp) {
            p.push(format!(
                "no align64 line now: {exp} (golden: {w}; this build has no alignment column)"
            ));
        }
    }
    let key = |r: &FrameRow| (r.exp.clone(), r.op.clone());
    let w: BTreeMap<_, usize> = want.iter().map(|r| (key(r), r.saves)).collect();
    let g: BTreeMap<_, usize> = got.iter().map(|r| (key(r), r.saves)).collect();
    for ((exp, op), &gs) in &g {
        match w.get(&(exp.clone(), op.clone())) {
            None => p.push(format!("new frame: {exp} {op} ({gs} saves on entry)")),
            Some(&ws) if gs > ws => p.push(format!("more saves: {exp} {op} {ws} -> {gs}")),
            Some(&ws) if gs < ws => p.push(format!(
                "fewer saves: {exp} {op} {ws} -> {gs} (a gain: update the golden)"
            )),
            Some(_) => {}
        }
    }
    for ((exp, op), &ws) in &w {
        if !g.contains_key(&(exp.clone(), op.clone())) {
            p.push(format!(
                "no frame now: {exp} {op} (golden: {ws} saves; a gain: update the golden)"
            ));
        }
    }
    Ok(p)
}

/// The frame check (`--frames`) of one built file: compares the frame rows (and on x86-64 the
/// `align64` lines, [`align_rows`]) with the golden file, or writes it (`--bless`). Prints every
/// row, so a CI log holds the new golden. A hot x86-64 handler that does not start on a 64-byte
/// line fails the check, and `--bless` then writes nothing.
fn check_frames(
    tools: &Tools,
    file: &Path,
    target: &str,
    golden: &Path,
    bless: bool,
) -> Result<bool, String> {
    let Some(l) = load(tools, file)? else {
        println!(
            "== {target}: {}\n   FAIL: no handler symbols ({PREFIX}*) found",
            file.display()
        );
        return Ok(false);
    };
    let rows = frame_rows(l.arch, &l.handlers, &l.blocks)?;
    println!("== {target} ({}, {}): {}", l.abi, l.format, file.display());
    let (align, misaligned) = if l.arch == Arch::X64 {
        let readobj = find_tool("llvm-readobj", &None)?;
        let mut places = BTreeMap::new();
        let mut members: Vec<&str> = l.handlers.iter().map(|h| h.member.as_str()).collect();
        members.sort();
        members.dedup();
        for m in members {
            let out = in_member(tools, file, m, |obj| {
                run_quiet(
                    Command::new(&readobj)
                        .args(["--sections", "--symbols"])
                        .arg(obj),
                )
            })?;
            for (name, ps) in parse_places(&out)? {
                places.insert((m.to_string(), name), ps);
            }
        }
        align_rows(&l.handlers, &places)?
    } else {
        (Vec::new(), Vec::new())
    };
    if l.arch == Arch::X64 {
        println!(
            "   alignment: hot handlers that start on a 64-byte line, per expansion (align64 <expansion> <aligned>/<total>):"
        );
    }
    for a in &align {
        println!("   align64 {} {}/{}", a.exp, a.aligned, a.total);
    }
    println!(
        "   frames: {} hot handler rows build a frame on entry (<expansion> <handler> <saves>):",
        rows.len()
    );
    for r in &rows {
        println!("   frame {} {} {}", r.exp, r.op, r.saves);
    }
    print_any_path(l.arch, &l.handlers, &l.blocks, &rows)?;
    for line in &misaligned {
        println!("   FAIL {line}");
    }
    if !misaligned.is_empty() {
        println!(
            "   A hot x86-64 handler does not start on a 64-byte line (H15 x64 function alignment). \
             The x86-64 [target] rustflags of the root .cargo/config.toml set it \
             (-C llvm-args=-align-all-functions=6); RUSTFLAGS or CARGO_ENCODED_RUSTFLAGS in the \
             environment replaces them. Build without them. --bless writes no golden then."
        );
    }
    if bless {
        if !misaligned.is_empty() {
            println!("   RESULT frames {}: FAIL (not written)", l.abi);
            return Ok(false);
        }
        fs::write(golden, frames_text(target, &align, &rows))
            .map_err(|e| format!("{}: {e}", golden.display()))?;
        println!("   wrote {}", golden.display());
        return Ok(true);
    }
    let text = fs::read_to_string(golden).map_err(|e| format!("{}: {e}", golden.display()))?;
    let p = frames_verdict(target, &text, &align, &rows)?;
    for line in &p {
        println!("   FAIL {line}");
    }
    if !p.is_empty() {
        println!(
            "   The hot handler frames differ from {}. A new frame or a new entry save on a hot \
             handler breaks plan rule 81 (docs/plan/10-quality.md). If the change is meant, run \
             this command again with --bless (or copy the rows above after `frame`, and the \
             `align64` lines) and commit the golden in the same PR.",
            golden.display()
        );
    }
    let ok = p.is_empty() && misaligned.is_empty();
    println!(
        "   RESULT frames {}: {}",
        l.abi,
        if ok { "PASS" } else { "FAIL" }
    );
    Ok(ok)
}

/// The callee-saved registers of the handler ABI ([`callee_saved`]) that handler block `b` stores
/// to the stack on any path, each once: x86-64 a `push`, or a `mov`/`movq` of the full register
/// to memory based on `%rsp` or `%rbp`; aarch64 an `stp`, `str` or `stur` of x19-x30 or d8-d15
/// to memory based on `sp` or x29. Report only (M9-P1-WPRE-3 (4), route MC-w42-4): a save that
/// LLVM shrink-wraps past the first branch is not an entry save, but a path that runs it still
/// pays for it. An upper bound: a later spill of a register that is saved already adds nothing,
/// and the code of a helper that a tail handler jumps to is not read.
fn any_path_saves(arch: Arch, b: &Block) -> usize {
    let mut seen = BTreeSet::new();
    for i in &b.insns {
        let (m, ops) = mnemonic_ops(&i.text);
        let srcs: Vec<&str> = match arch {
            Arch::X64 if m.starts_with("push") => vec![ops.as_str()],
            Arch::X64 => match store_of(arch, &m, &ops) {
                Some(s)
                    if matches!(m.as_str(), "mov" | "movq") && matches!(s.base, "rsp" | "rbp") =>
                {
                    s.srcs
                }
                _ => Vec::new(),
            },
            Arch::A64 => match store_of(arch, &m, &ops) {
                Some(s)
                    if matches!(m.as_str(), "stp" | "str" | "stur")
                        && matches!(s.base, "sp" | "x29" | "fp") =>
                {
                    s.srcs
                }
                _ => Vec::new(),
            },
        };
        for r in srcs {
            if let Some((reg, true)) = callee_saved(arch, r) {
                seen.insert(reg);
            }
        }
    }
    seen.len()
}

/// The report-only `any-path` lines of the frame check: each hot handler whose block stores a
/// callee-saved register on any path ([`any_path_saves`]), sorted; a handler with no frame row
/// is marked, so a save that LLVM moved past the entry is visible in the CI log. Not part of the
/// golden and of the verdict.
fn print_any_path(
    arch: Arch,
    handlers: &[Handler],
    blocks: &[Block],
    rows: &[FrameRow],
) -> Result<(), String> {
    let mut lines = BTreeMap::new();
    for h in handlers {
        let op = h.name.rsplit("__").next().unwrap_or(&h.name);
        if !is_hot(arch, op) {
            continue;
        }
        let b = block_of(handlers, blocks, h)
            .ok_or_else(|| format!("{}: not found in the disassembly", h.name))?;
        let n = any_path_saves(arch, b);
        if n > 0 {
            let row = rows.iter().any(|r| r.exp == h.exp && r.op == op);
            lines.insert((h.exp.clone(), op.to_string()), (n, row));
        }
    }
    println!(
        "   any-path (report only): {} hot handlers store a callee-saved register on some path \
         (<expansion> <handler> <registers>):",
        lines.len()
    );
    for ((exp, op), (n, row)) in &lines {
        println!(
            "   any-path {exp} {op} {n}{}",
            if *row {
                ""
            } else {
                " (no frame row: no save on entry)"
            }
        );
    }
    Ok(())
}

/// The x86-64 start alignment that the frame check holds (H15 x64 function alignment; the
/// `-C llvm-args=-align-all-functions=6` rustflags of the root `.cargo/config.toml`).
const START_ALIGN: u64 = 64;

/// Where a symbol is in its object: the alignment of its section and its offset in that section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    align: u64,
    offset: u64,
}

/// A number as `llvm-readobj` prints it: `0x` and hex digits, or decimal digits.
fn readobj_num(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Reads `llvm-readobj --sections --symbols` (the LLVM output style) of one ELF or COFF object:
/// each symbol in a section, with its places (a name can be in the symbol table more than once).
/// The section alignment is ELF `AddressAlignment` (`sh_addralign`) or the COFF
/// `IMAGE_SCN_ALIGN_<n>BYTES` flag (16 when the flag is not there, the PE default). The offset is
/// the symbol value minus the section address (0 in a relocatable object). The staticlib holds
/// relocatable objects with one section per function, so a handler symbol value is 0 with and
/// without the alignment flag: only the section alignment shows the flag. Other formats (Mach-O)
/// are an error: no x86-64 golden is for them.
fn parse_places(out: &str) -> Result<BTreeMap<String, Vec<Place>>, String> {
    let format = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("Format: "))
        .unwrap_or("");
    let coff = format.starts_with("COFF");
    if !coff && !format.starts_with("elf") {
        return Err(format!(
            "the alignment column reads ELF and COFF objects only, not {format:?}"
        ));
    }
    let mut sections: BTreeMap<u64, Place> = BTreeMap::new();
    let mut syms: Vec<(String, u64, u64)> = Vec::new();
    let mut depth = 0usize;
    let mut kind = "";
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    let mut flag_align: Option<u64> = None;
    for line in out.lines() {
        let t = line.trim();
        if let Some(k) = t.strip_suffix('{') {
            depth += 1;
            if depth == 1 {
                kind = k.trim();
                fields.clear();
                flag_align = None;
            }
            continue;
        }
        if t == "}" {
            if depth == 1 {
                let num = |k: &str| fields.get(k).and_then(|v| readobj_num(v));
                match kind {
                    "Section" => {
                        let (n, align, addr) = if coff {
                            (
                                num("Number"),
                                flag_align.or(Some(16)),
                                num("VirtualAddress"),
                            )
                        } else {
                            (num("Index"), num("AddressAlignment"), num("Address"))
                        };
                        match (n, align, addr) {
                            (Some(n), Some(align), Some(offset)) => {
                                sections.insert(n, Place { align, offset });
                            }
                            _ => {
                                return Err(format!(
                                    "llvm-readobj: a section with no number, alignment or address: {fields:?}"
                                ));
                            }
                        }
                    }
                    "Symbol" => {
                        // ELF: `Name: <name> (<string table offset>)`; COFF: `Name: <name>`.
                        let name = fields.get("Name").map(|v| match v.rsplit_once(" (") {
                            Some((n, _)) if !coff => n,
                            _ => v,
                        });
                        // `Section: <name> (<number>)`; an undefined or absolute symbol has a
                        // number that names no section (or does not parse) and is skipped.
                        let sec = fields
                            .get("Section")
                            .and_then(|v| v.rsplit_once(" ("))
                            .and_then(|(_, n)| readobj_num(n.trim_end_matches(')')));
                        if let (Some(name), Some(value), Some(sec)) = (name, num("Value"), sec) {
                            syms.push((name.to_string(), value, sec));
                        }
                    }
                    _ => {}
                }
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth != 1 {
            continue;
        }
        if let Some(r) = t.strip_prefix("IMAGE_SCN_ALIGN_") {
            flag_align = r.split_once("BYTES").and_then(|(n, _)| n.parse().ok());
        } else if let Some((k, v)) = t.split_once(": ") {
            fields.insert(k, v);
        }
    }
    let mut places: BTreeMap<String, Vec<Place>> = BTreeMap::new();
    for (name, value, sec) in syms {
        // Section 0 is "undefined" in ELF and COFF.
        let Some(s) = sections.get(&sec).filter(|_| sec != 0) else {
            continue;
        };
        let offset = value.checked_sub(s.offset).ok_or_else(|| {
            format!(
                "llvm-readobj: {name} at {value:#x} is before its section ({:#x})",
                s.offset
            )
        })?;
        places.entry(name).or_default().push(Place {
            align: s.align,
            offset,
        });
    }
    Ok(places)
}

/// The x86-64 alignment column of the frame check (H15 x64 function alignment; M10-P1
/// X86-ALIGN, M10-PRE-ALIGN-FORM): per expansion, the hot handlers ([`is_hot`]) that start on a
/// 64-byte line and all hot handlers, and one FAIL line per expansion with a handler that does
/// not. A handler starts on a 64-byte line when each of its places (`places`, keyed by archive
/// member and symbol name; [`parse_places`]) is in a section with an alignment of 64 or more, at
/// an offset that is a multiple of 64. A hot handler with no place is an error.
fn align_rows(
    handlers: &[Handler],
    places: &BTreeMap<(String, String), Vec<Place>>,
) -> Result<(Vec<AlignRow>, Vec<String>), String> {
    // (expansion, handler) -> its places, each handler once.
    let mut hot: BTreeMap<(String, String), &[Place]> = BTreeMap::new();
    for h in handlers {
        let op = h.name.rsplit("__").next().unwrap_or(&h.name);
        if !is_hot(Arch::X64, op) {
            continue;
        }
        let ps = places
            .get(&(h.member.clone(), h.raw.clone()))
            .filter(|ps| !ps.is_empty())
            .ok_or_else(|| {
                format!(
                    "{}: no section in llvm-readobj --sections --symbols",
                    h.name
                )
            })?;
        hot.insert((h.exp.clone(), h.name.clone()), ps);
    }
    let mut per: BTreeMap<String, (usize, usize, Vec<String>)> = BTreeMap::new();
    for ((exp, name), ps) in &hot {
        let e = per.entry(exp.clone()).or_default();
        e.1 += 1;
        match ps
            .iter()
            .find(|p| p.align < START_ALIGN || p.offset % START_ALIGN != 0)
        {
            None => e.0 += 1,
            Some(p) => e.2.push(format!(
                "{name} (section alignment {}, offset {:#x})",
                p.align, p.offset
            )),
        }
    }
    let mut rows = Vec::new();
    let mut fails = Vec::new();
    for (exp, (aligned, total, bad)) in per {
        if !bad.is_empty() {
            let more = if bad.len() > 3 {
                format!(", and {} more", bad.len() - 3)
            } else {
                String::new()
            };
            fails.push(format!(
                "x86-64 alignment: {exp} {aligned}/{total} hot handlers start on a 64-byte line; not: {}{more}",
                bad[..bad.len().min(3)].join(", ")
            ));
        }
        rows.push(AlignRow {
            exp,
            aligned,
            total,
        });
    }
    Ok((rows, fails))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Local CI checked ep9_vm and then the spike from one target dir; the cached spike build did
    // not rewrite `<target>.ll`, so the spike check read ep9_vm's IR ("no LLVM IR definition").
    #[test]
    fn ir_file_names_differ_per_crate() {
        let t = "aarch64-apple-darwin";
        let spike = parse_opts(&[]).unwrap();
        let vm = parse_opts(&[
            "--manifest-path".into(),
            "crates/ep9_vm/Cargo.toml".into(),
            "-p".into(),
            "ep9_vm".into(),
        ])
        .unwrap();
        assert_ne!(ir_tag(&spike, t), ir_tag(&vm, t));
        assert_eq!(ir_tag(&spike, t), format!("sp-dispatch-{t}"));
        assert_eq!(ir_tag(&vm, t), format!("ep9_vm-{t}"));
    }

    fn insn(text: &str, reloc: Option<&str>) -> Insn {
        Insn {
            addr: 0,
            text: text.into(),
            reloc: reloc.map(str::to_string),
        }
    }

    /// `classify` for one instruction in a block named `own` at address 0.
    fn kind(arch: Arch, i: Insn, own: &str) -> Kind {
        let b = Block {
            member: String::new(),
            raw: own.into(),
            addr: 0,
            insns: vec![i],
        };
        classify(arch, &b.insns[0], &b)
    }

    #[test]
    fn handler_names() {
        assert_eq!(
            parse_handler("_ep9h__sib__tail__add_smi"),
            Some(("ep9h__sib__tail__add_smi".into(), "sib".into(), Class::Tail))
        );
        assert_eq!(
            parse_handler("ep9h__become__exit__halt").map(|h| h.2),
            Some(Class::Exit)
        );
        assert_eq!(parse_handler("ep9h__sib__other__x"), None);
        assert_eq!(parse_handler("_RNvCs_foo"), None);
    }

    #[test]
    fn a64_kinds() {
        let own = "_ep9h__sib__tail__add";
        assert_eq!(kind(Arch::A64, insn("br\tx4", None), own), Kind::IndJump);
        assert_eq!(kind(Arch::A64, insn("blr\tx8", None), own), Kind::IndCall);
        assert_eq!(kind(Arch::A64, insn("ret", None), own), Kind::Ret);
        let local = insn("b.ne\t0x100021370 <_ep9h__sib__tail__add+0x24>", None);
        assert_eq!(kind(Arch::A64, local, own), Kind::CondJump(None));
        let out = insn("b\t0x10 <_ep9h__sib__tail__ret>", None);
        assert_eq!(
            kind(Arch::A64, out, own),
            Kind::Jump(Some("_ep9h__sib__tail__ret".into()))
        );
        let call = insn("bl\t0x0 <ltmp0>", Some("_RNvNtCs1_3ops8add_slow"));
        assert_eq!(
            kind(Arch::A64, call, own),
            Kind::Call("_RNvNtCs1_3ops8add_slow".into())
        );
    }

    // llvm-objdump -d -r output for objects that llvm-mc assembled from hand-written handlers.
    // F1: the handler jumps to a helper that does `blr` + `ret` (stack growth, then a return).
    const F1_A64_MACHO: &str = "f1.o:\tfile format mach-o arm64

0000000000000000 <_ep9h__fx__tail__add>:
       0:      \tldrb\tw9, [x1, #0x1]
       4:      \tldr\tx8, [x0, x9, lsl #3]
       8:      \tb\t0x8 <ltmp0+0x8>
\t\t0000000000000008:  ARM64_RELOC_BRANCH26\t_dispatch_helper

000000000000000c <_dispatch_helper>:
       c:      \tstp\tx29, x30, [sp, #-0x10]!
      10:      \tblr\tx8
      14:      \tldp\tx29, x30, [sp], #0x10
      18:      \tret
";
    // F1 as Win64 COFF: no relocation, objdump names the helper.
    const F1_X64_COFF: &str = "x1.o:\tfile format coff-x86-64

0000000000000000 <ep9h__fx__tail__add>:
       0:      \tmovzbl\t0x1(%rsi), %eax
       4:      \tmovq\t(%rdi,%rax,8), %rax
       8:      \tjmp\t0xa <dispatch_helper>

000000000000000a <dispatch_helper>:
       a:      \tpushq\t%rbp
       b:      \tcallq\t*%rax
       d:      \tpopq\t%rbp
       e:      \tretq
";
    // A conditional jump to the F1 helper, next to a good indirect jump.
    const X6_X64_ELF: &str = "x6.o:\tfile format elf64-x86-64

0000000000000000 <ep9h__fx__tail__add>:
       0:      \ttestq\t%rcx, %rcx
       3:      \tjne\t0x9 <ep9h__fx__tail__add+0x9>
\t\t0000000000000005:  R_X86_64_PLT32\tdispatch_helper-0x4
       9:      \tmovq\t(%rdi), %rax
       c:      \tjmpq\t*%rax

0000000000000000 <dispatch_helper>:
       0:      \tpushq\t%rbp
       1:      \tcallq\t*%rax
       3:      \tpopq\t%rbp
       4:      \tretq
";
    // The helper calls a handler (`bl`), then returns.
    const F5_A64_MACHO: &str = "f5.o:\tfile format mach-o arm64

0000000000000000 <_ep9h__fx__tail__add>:
       0:      \tb\t0x0 <ltmp0>
\t\t0000000000000000:  ARM64_RELOC_BRANCH26\t_call_helper

0000000000000004 <_call_helper>:
       4:      \tstp\tx29, x30, [sp, #-0x10]!
       8:      \tbl\t0x8 <_call_helper+0x4>
\t\t0000000000000008:  ARM64_RELOC_BRANCH26\t_ep9h__fx__tail__sub
       c:      \tldp\tx29, x30, [sp], #0x10
      10:      \tret

0000000000000014 <_ep9h__fx__tail__sub>:
      14:      \tldr\tx8, [x0]
      18:      \tbr\tx8
";
    // Good: a local `cbz` that objdump labels `ltmp0+0xc`, and a jump to a cold handler.
    const F3_A64_MACHO: &str = "f3.o:\tfile format mach-o arm64

0000000000000000 <_ep9h__fx__tail__add>:
       0:      \tcbz\tx3, 0xc <ltmp0+0xc>
       4:      \tldr\tx8, [x0]
       8:      \tbr\tx8
       c:      \tb\t0xc <ltmp0+0xc>
\t\t000000000000000c:  ARM64_RELOC_BRANCH26\t_ep9h__fx__tail__add_cold

0000000000000010 <_ep9h__fx__tail__add_cold>:
      10:      \tldr\tx8, [x0]
      14:      \tbr\tx8
";
    // Good: a jump to a local helper in its own ELF section (an outlined function) that ends
    // in an indirect jump. The relocation names the section.
    const X7_X64_ELF: &str = "x7.o:\tfile format elf64-x86-64

0000000000000000 <ep9h__fx__tail__add>:
       0:      \tmovzbl\t0x1(%rsi), %eax
       4:      \tjmp\t0x9 <ep9h__fx__tail__add+0x9>
\t\t0000000000000005:  R_X86_64_PLT32\t.text.outl_helper-0x4

0000000000000000 <outl_helper>:
       0:      \tmovq\t(%rdi,%rax,8), %rax
       4:      \tjmpq\t*%rax
";

    /// Runs the follow pass with a fake llvm-objdump that prints only the asked symbols (as
    /// `--disassemble-symbols` does), then checks `tail` handler `h`.
    fn tail_check(dump: &str, h: &str) -> (Vec<String>, Stats) {
        let (_, all) = parse_disasm(dump);
        let raws: Vec<String> = all
            .iter()
            .filter(|b| parse_handler(&b.raw).is_some())
            .map(|b| b.raw.clone())
            .collect();
        let (format, blocks) = disassemble_reach(&raws, |names| {
            let (f, mut b) = parse_disasm(dump);
            b.retain(|b| names.contains(&b.raw));
            Ok((f, b))
        })
        .unwrap();
        let (arch, _) = arch_of(&format).unwrap();
        let hs: BTreeSet<&str> = raws.iter().map(String::as_str).collect();
        let b = blocks.iter().find(|b| b.raw == h).unwrap();
        let s = analyze(arch, b, Class::Tail, &blocks, &hs);
        (verdict(Class::Tail, &s), s)
    }

    #[test]
    fn tail_exit_into_helper() {
        // Negative controls: a tail handler leaves through a direct jump to a helper that is
        // not a handler. The helper's code counts, so the gate must fail.
        for (dump, h, helper) in [
            (F1_A64_MACHO, "_ep9h__fx__tail__add", "_dispatch_helper"),
            (F1_X64_COFF, "ep9h__fx__tail__add", "dispatch_helper"),
            (X6_X64_ELF, "ep9h__fx__tail__add", "dispatch_helper"),
        ] {
            let (p, s) = tail_check(dump, h);
            assert!(!p.is_empty(), "{h}: {s:?}");
            assert_eq!(s.helpers, [helper], "{h}");
            assert_eq!((s.returns, s.ind_calls), (1, 1), "{h}");
        }
        let (p, s) = tail_check(F5_A64_MACHO, "_ep9h__fx__tail__add");
        assert!(!p.is_empty(), "{s:?}");
        assert_eq!((s.returns, s.handler_calls), (1, 1));
        // The helper is not in the file: an unknown exit fails too.
        let handler_only = &F1_A64_MACHO[..F1_A64_MACHO.find("\n000000000000000c").unwrap()];
        let (p, s) = tail_check(handler_only, "_ep9h__fx__tail__add");
        assert!(!p.is_empty(), "{s:?}");
        assert_eq!(s.unknown, ["_dispatch_helper"]);

        // Positive controls.
        let (p, s) = tail_check(F3_A64_MACHO, "_ep9h__fx__tail__add");
        assert!(p.is_empty(), "{p:?}");
        assert_eq!((s.ind_jumps, s.direct_tail), (1, 1), "the cbz is local");
        let (p, s) = tail_check(X7_X64_ELF, "ep9h__fx__tail__add");
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(s.helpers, ["outl_helper"]);
        assert_eq!(s.ind_jumps, 1);
    }

    #[test]
    fn frame_setup() {
        assert!(is_frame_setup(Arch::A64, "stp\tx22, x21, [sp, #-0x30]!"));
        assert!(!is_frame_setup(Arch::A64, "stp\tx20, x19, [sp, #0x10]"));
        assert!(!is_frame_setup(Arch::A64, "ldrb\tw8, [x1, #0x1]"));
        assert!(is_frame_setup(Arch::X64, "pushq\t%rsi"));
        assert!(is_frame_setup(Arch::X64, "subq\t$0x28, %rsp"));
        assert!(!is_frame_setup(Arch::X64, "movzbl\t0x1(%rdx), %eax"));
    }

    #[test]
    fn x64_kinds() {
        let own = "ep9h__sib__tail__add";
        assert_eq!(
            kind(Arch::X64, insn("jmpq\t*%rax", None), own),
            Kind::IndJump
        );
        assert_eq!(
            kind(Arch::X64, insn("rex64 jmpq\t*%rax", None), own),
            Kind::IndJump
        );
        assert_eq!(
            kind(Arch::X64, insn("jmpq\t*0x10(%rdi,%rax,8)", None), own),
            Kind::IndJump
        );
        assert_eq!(
            kind(Arch::X64, insn("callq\t*%rax", None), own),
            Kind::IndCall
        );
        assert_eq!(kind(Arch::X64, insn("retq", None), own), Kind::Ret);
        let rel = insn(
            "jmp\t0x1d <ep9h__sib__tail__add+0x1d>",
            Some("ep9h__sib__tail__ret-0x4"),
        );
        assert_eq!(
            kind(Arch::X64, rel, own),
            Kind::Jump(Some("ep9h__sib__tail__ret".into()))
        );
        let local = insn("jne\t0x40 <ep9h__sib__tail__add+0x40>", None);
        assert_eq!(kind(Arch::X64, local, own), Kind::CondJump(None));
    }

    #[test]
    fn ir_param_split() {
        let dir = std::env::temp_dir().join(format!("xtask-ir-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let ll = dir.join("t.ll");
        fs::write(
            &ll,
            "define i64 @ep9h__sib__tail__add(ptr noalias noundef align 8 dereferenceable(2120) %cx, ptr noundef %pc, ptr %fp, i64 noundef range(i64 0, 256) %acc) unnamed_addr #0 {\n\
             define void @\"ep9h__sib__exit__halt\"(ptr sret([16 x i8]) %0, ptr %cx) {\n\
             define void @other(i64 %a, i64 %b, i64 %c, i64 %d, i64 %e) {\n\
             @ep9h__x__exit__halt = unnamed_addr alias i64 (ptr, ptr, ptr, i64), ptr @ep9h__sib__tail__add\n",
        )
        .unwrap();
        let p = ir_params(&ll).unwrap();
        assert_eq!(p["ep9h__sib__tail__add"].len(), 4);
        assert!(p["ep9h__sib__exit__halt"][0].contains("sret"));
        assert!(!p.contains_key("other"));
        assert_eq!(p["ep9h__x__exit__halt"].len(), 4);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A block from `llvm-objdump -d` lines ("addr: text").
    fn block(raw: &str, lines: &[&str]) -> Block {
        Block {
            member: String::new(),
            raw: raw.into(),
            addr: 0,
            insns: lines
                .iter()
                .enumerate()
                .map(|(i, t)| Insn {
                    addr: i as u64,
                    text: t.to_string(),
                    reloc: None,
                })
                .collect(),
        }
    }

    // Shape of the x86-64 match loop (rustc 1.99.1): prologue, copied dispatch in each arm,
    // a spill around a cold call, pops before the return.
    const X64_LOOP: &[&str] = &[
        "pushq\t%rbp",
        "pushq\t%rax",
        "movzbl\t(%rsi), %ecx",
        "jmpq\t*%rcx",
        "addq\t$0x2, %rsi",
        "jmpq\t*%rcx",
        "movq\t%r12, (%rsp)",
        "movq\t%rax, %r12",
        "callq\t0x2f2 <ep9loop__match__run+0x2f2>",
        "movq\t(%rsp), %r12",
        "jmpq\t*%rcx",
        "popq\t%rbp",
        "retq",
    ];

    #[test]
    fn x64_match_loop_shape() {
        let b = block("ep9loop__match__run", X64_LOOP);
        let s = analyze_loop(Arch::X64, &b);
        assert_eq!((s.ind_jumps, s.calls, s.stack_far.len()), (3, 1, 0));
        assert!(loop_verdict(&s, 6).is_empty());
        // Fewer copied dispatch jumps than half the fast tail handlers: the switch-loop shape.
        assert_eq!(loop_verdict(&s, 8).len(), 1);
        // A reload far from any call: traffic on every dispatch.
        let mut lines = X64_LOOP.to_vec();
        lines.insert(4, "movq\t0x8(%rsp), %rdx");
        let s = analyze_loop(Arch::X64, &block("ep9loop__match__run", &lines));
        assert_eq!(s.stack_far.len(), 1, "{s:?}");
        assert!(!loop_verdict(&s, 6).is_empty());
        assert!(!is_stack_access(Arch::X64, "leaq\t0x8(%rsp), %rax", true));
        // %rbp is a stack pointer only with a frame pointer.
        assert!(is_stack_access(Arch::X64, "movq\t-0x8(%rbp), %rax", true));
        assert!(!is_stack_access(
            Arch::X64,
            "movq\t(%rbp,%rbx,8), %r14",
            false
        ));
    }

    // x86-64 Mach-O: a jump table after the handler's code (as in `TestTypeOf`, rustc 1.99.1).
    // `llvm-objdump -d` decodes its 8 bytes as an indirect call, junk and a return.
    const X8_X64_MACHO: &str = "x8.o:\tfile format mach-o 64-bit x86-64

0000000000000000 <_ep9h__fx__tail__TestTypeOf>:
       0:      \tmovzbl\t0x1(%rsi), %eax
       4:      \tleaq\t0x11(%rip), %rcx
       b:      \tmovslq\t(%rcx,%rax,4), %rax
       f:      \taddq\t%rcx, %rax
      12:      \tjmpq\t*%rax
      14:      \tmovzbl\t0x2(%rsi), %eax
      18:      \tjmpq\t*(%rdi,%rax,8)
      1c:      \tcallq\t*-0x1(%rax)
      1f:      \t<unknown>
      21:      \tretq
      22:      \tnop
      24:      \tnop
";
    const X8_DICE: &str = "x8.o:
Data in code table (1 entries)
offset     length kind
0x0000001c      8 JUMP_TABLE32
";
    // `llvm-objdump --macho --private-headers`: an object (one segment, not at file offset 0)
    // and a linked executable (__PAGEZERO, then __TEXT at file offset 0).
    const OBJECT_HEADERS: &str = "x8.o:
Mach header
      magic cputype cpusubtype  caps    filetype ncmds sizeofcmds      flags
MH_MAGIC_64  X86_64        ALL  0x00      OBJECT     5       2208 SUBSECTIONS_VIA_SYMBOLS
Load command 0
      cmd LC_SEGMENT_64
  cmdsize 2072
  segname
   vmaddr 0x0000000000000000
   vmsize 0x00000000003ae2c8
  fileoff 2240
 filesize 3859016
";
    const IMAGE_HEADERS: &str = "ep9:
Load command 0
      cmd LC_SEGMENT_64
  cmdsize 72
  segname __PAGEZERO
   vmaddr 0x0000000000000000
   vmsize 0x0000000100000000
  fileoff 0
 filesize 0
Load command 1
      cmd LC_SEGMENT_64
  cmdsize 712
  segname __TEXT
   vmaddr 0x0000000100000000
   vmsize 0x00000000001d1000
  fileoff 0
 filesize 1904640
Section
  sectname __text
      addr 0x0000000100001000
";

    #[test]
    fn macho_jump_table_is_data() {
        let h = "_ep9h__fx__tail__TestTypeOf";
        let hs = BTreeSet::from([h]);
        let (_, mut blocks) = parse_disasm(X8_X64_MACHO);
        // Negative control: decoded as code, the table fails the tail-exit rule.
        let s = analyze(Arch::X64, &blocks[0], Class::Tail, &blocks, &hs);
        assert_eq!((s.ind_jumps, s.ind_calls, s.returns), (2, 1, 1));
        assert_eq!(verdict(Class::Tail, &s).len(), 2, "{s:?}");

        let data = parse_data_in_code(X8_DICE);
        assert_eq!(data, [(0x1c, 8)]);
        drop_data(&mut blocks, macho_dice_base(OBJECT_HEADERS), &data);
        assert_eq!(blocks[0].insns.len(), 8);
        let s = analyze(Arch::X64, &blocks[0], Class::Tail, &blocks, &hs);
        assert_eq!((s.ind_jumps, s.ind_calls, s.returns), (2, 0, 0));
        assert!(verdict(Class::Tail, &s).is_empty(), "{s:?}");
    }

    // x86-64 Mach-O built by clang from assembly: a tail handler with code after its jump table
    // (`.data_region jt32`, 0x1c..0x24). `-d` decodes across the table end: `addb %al, %bl` at
    // 0x23 takes the byte 0xc3 of the real `retq` at 0x24.
    const X9_X64_MACHO: &str = "x9.o:\tfile format mach-o 64-bit x86-64

0000000000000000 <_ep9h__fx__tail__Foo>:
       0:      \tmovzbl\t0x1(%rsi), %eax
       4:      \tleaq\t0x11(%rip), %rcx        ## 0x1c <_ep9h__fx__tail__Foo+0x1c>
       b:      \tmovslq\t(%rcx,%rax,4), %rax
       f:      \taddq\t%rcx, %rax
      12:      \tjmpq\t*%rax
      14:      \tmovzbl\t0x2(%rsi), %eax
      18:      \tjmpq\t*(%rdi,%rax,8)
      1b:      \tnop
      1c:      \tclc
      1d:      \t<unknown>
      1f:      \tdecl\t(%rax)
      21:      \taddb\t%al, (%rax)
      23:      \taddb\t%al, %bl
";
    // The same file with `--start-address=0x24`.
    const X9_AFTER_TABLE: &str = "x9.o:\tfile format mach-o 64-bit x86-64

0000000000000000 <_ep9h__fx__tail__Foo>:
      24:      \tretq
";

    #[test]
    fn macho_code_after_jump_table() {
        let h = "_ep9h__fx__tail__Foo";
        let hs = BTreeSet::from([h]);
        // Negative control: dropping the table alone hides the `retq`, and the handler passes.
        let (_, mut blocks) = parse_disasm(X9_X64_MACHO);
        drop_data(&mut blocks, 0, &[(0x1c, 8)]);
        let s = analyze(Arch::X64, &blocks[0], Class::Tail, &blocks, &hs);
        assert_eq!((s.ind_jumps, s.returns), (2, 0));

        // Decoded again from the table end, the `retq` fails the tail-exit rule. Adjacent
        // ranges are one table; a later range stops the decode.
        for (data, want) in [
            (vec![(0x1c, 8)], vec![(0x24, None)]),
            (vec![(0x20, 4), (0x1c, 4)], vec![(0x24, None)]),
            (
                vec![(0x1c, 8), (0x2c, 4)],
                vec![(0x24, Some(0x2c)), (0x30, None)],
            ),
        ] {
            let (_, mut blocks) = parse_disasm(X9_X64_MACHO);
            let mut calls = Vec::new();
            redecode_after_data(&mut blocks, 0, &data, |sym, start, stop| {
                assert_eq!(sym, h);
                calls.push((start, stop));
                let (_, mut b) = parse_disasm(X9_AFTER_TABLE);
                b[0].insns
                    .retain(|i| i.addr >= start && stop.is_none_or(|t| i.addr < t));
                Ok(b)
            })
            .unwrap();
            assert_eq!(calls, want, "{data:?}");
            drop_data(&mut blocks, 0, &data);
            assert_eq!(blocks[0].insns.last().unwrap().addr, 0x24);
            let s = analyze(Arch::X64, &blocks[0], Class::Tail, &blocks, &hs);
            assert_eq!((s.ind_jumps, s.returns), (2, 1));
            assert_eq!(verdict(Class::Tail, &s), ["1 return(s) in a tail handler"]);
        }
    }

    #[test]
    fn macho_data_in_code_base() {
        assert_eq!(macho_dice_base(OBJECT_HEADERS), 0);
        assert_eq!(macho_dice_base(IMAGE_HEADERS), 0x1_0000_0000);
        assert_eq!(macho_dice_base(""), 0);
        assert!(
            parse_data_in_code("x.o:\nData in code table (0 entries)\noffset     length kind\n")
                .is_empty()
        );
    }

    #[test]
    fn json_filenames() {
        let l = r#"{"reason":"compiler-artifact","filenames":["/a/b/libx.a","C:\\t\\x.lib"],"fresh":true}"#;
        assert_eq!(
            json_string_array(l, "filenames"),
            vec!["/a/b/libx.a".to_string(), "C:\\t\\x.lib".to_string()]
        );
    }

    /// A frame-check fixture (tests/disasm_frames, plan v9 rule 24).
    fn frames_fixture(name: &str) -> String {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/disasm_frames")
                .join(name),
        )
        .unwrap()
    }

    const FRAMES_TARGET: &str = "x86_64-unknown-linux-gnu";

    /// The frame rows of a fixture dump (x86-64 SysV), with its handlers taken from the block
    /// names (nm gives them in the real check).
    fn fixture_rows(name: &str) -> Vec<FrameRow> {
        let (abi, handlers, rows) = dump_rows(name);
        assert_eq!(abi, "x64-sysv");
        assert_eq!(handlers, 4);
        rows
    }

    /// The ABI, the handler count and the frame rows of a fixture dump of any target.
    fn dump_rows(name: &str) -> (&'static str, usize, Vec<FrameRow>) {
        let (abi, handlers, rows) = dump_frames(name);
        (abi, handlers, rows.unwrap())
    }

    /// The ABI, the handler count and the `frame_rows` result of a fixture dump of any target.
    fn dump_frames(name: &str) -> (&'static str, usize, Result<Vec<FrameRow>, String>) {
        let (format, blocks) = parse_disasm(&frames_fixture(name));
        let (arch, abi) = arch_of(&format).unwrap();
        let handlers: Vec<Handler> = blocks
            .iter()
            .filter_map(|b| {
                let (name, exp, class) = parse_handler(&b.raw)?;
                Some(Handler {
                    raw: b.raw.clone(),
                    name,
                    exp,
                    class,
                    member: b.member.clone(),
                    addr: b.addr,
                })
            })
            .collect();
        let rows = frame_rows(arch, &handlers, &blocks);
        (abi, handlers.len(), rows)
    }

    #[test]
    fn frames_unchanged_list_passes() {
        let rows = fixture_rows("base.txt");
        // The cold handler builds a frame too, but only hot handlers are rows.
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.op.as_str(), r.saves)).collect();
        // BadOpcode's only push is `pushq %rax`, a stack alignment push: a row with 0 saves
        // (M9-P1-WPRE-3).
        assert_eq!(names, [("BadOpcode", 0), ("CallProperty0", 2)]);
        let golden = frames_fixture("golden.txt");
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &rows).unwrap(),
            Vec::<String>::new()
        );
        // --bless writes a file that passes and reads back the same rows.
        let text = frames_text(FRAMES_TARGET, &[], &rows);
        assert!(text.starts_with(FRAMES_HEAD));
        assert_eq!(
            parse_frames(&text).unwrap(),
            Golden {
                target: FRAMES_TARGET.to_string(),
                align: Vec::new(),
                rows: rows.clone()
            }
        );
        assert!(
            frames_verdict(FRAMES_TARGET, &text, &[], &rows)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn frames_new_frame_or_push_fails() {
        let rows = fixture_rows("head.txt");
        let golden = frames_fixture("golden.txt");
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &rows).unwrap(),
            [
                "more saves: sib CallProperty0 2 -> 3",
                "new frame: sib GetNamedProperty (1 saves on entry)",
            ]
        );
        // The golden of another target fails even with equal rows.
        let base = fixture_rows("base.txt");
        assert_eq!(
            frames_verdict("x86_64-pc-windows-msvc", &golden, &[], &base).unwrap(),
            [
                "the golden is for x86_64-unknown-linux-gnu, this build is for x86_64-pc-windows-msvc"
            ]
        );
    }

    /// Codex M9-P0 r1: a callee-saved register save after an argument spill changes the row, so
    /// the golden of the build before it fails the build after it.
    #[test]
    fn frames_save_after_spill_fails() {
        for (base, head, target, more) in [
            (
                "spill-a64-base.txt",
                "spill-a64-head.txt",
                "aarch64-unknown-linux-gnu",
                "more saves: sib StaSpill 0 -> 2",
            ),
            (
                "spill-x64-base.txt",
                "spill-x64-head.txt",
                FRAMES_TARGET,
                "more saves: sib StaSpill 0 -> 1",
            ),
        ] {
            let (_, _, before) = dump_rows(base);
            let golden = frames_text(target, &[], &before);
            assert!(
                frames_verdict(target, &golden, &[], &before)
                    .unwrap()
                    .is_empty()
            );
            let (_, _, after) = dump_rows(head);
            assert_eq!(
                frames_verdict(target, &golden, &[], &after).unwrap(),
                [more]
            );
        }
    }

    /// Codex M9-P0 r2: a save by `mov` to `%rsp` or to `%rbp` after `mov %rsp, %rbp` (x86-64), by
    /// `stp` through x29 after `add x29, sp` or by `stur` (aarch64), or in the code after an
    /// unconditional jump inside the handler, changes the row. So the golden of the build before
    /// it fails the build after it.
    #[test]
    fn frames_save_forms_fail() {
        for (base, head, target, rows, more) in [
            (
                "forms-x64-base.txt",
                "forms-x64-head.txt",
                FRAMES_TARGET,
                [("FpSave", 1), ("JmpSave", 0), ("MovSave", 0)],
                [
                    "more saves: sib FpSave 1 -> 2",
                    "more saves: sib JmpSave 0 -> 1",
                    "more saves: sib MovSave 0 -> 1",
                ],
            ),
            (
                "forms-a64-base.txt",
                "forms-a64-head.txt",
                "aarch64-unknown-linux-gnu",
                [("JmpSave", 2), ("StpFp", 2), ("Stur", 2)],
                [
                    "more saves: sib JmpSave 2 -> 3",
                    "more saves: sib StpFp 2 -> 4",
                    "more saves: sib Stur 2 -> 3",
                ],
            ),
        ] {
            let (_, _, before) = dump_rows(base);
            let got: Vec<(&str, usize)> = before.iter().map(|r| (r.op.as_str(), r.saves)).collect();
            assert_eq!(got, rows);
            let golden = frames_text(target, &[], &before);
            assert!(
                frames_verdict(target, &golden, &[], &before)
                    .unwrap()
                    .is_empty()
            );
            let (_, _, after) = dump_rows(head);
            assert_eq!(frames_verdict(target, &golden, &[], &after).unwrap(), more);
        }
    }

    /// M9-P0 r3: a store of a callee-saved register in a form that the rule does not know (here
    /// a part of the register) fails the check with the handler and the instruction, so the
    /// check never counts it as 0 in silence.
    #[test]
    fn frames_unknown_save_form_fails() {
        for (name, err) in [
            (
                "unknown-x64.txt",
                "unknown save form: ep9h__sib__tail__Part movl %ebx, 0x8(%rsp)",
            ),
            (
                "unknown-a64.txt",
                "unknown save form: ep9h__sib__tail__Part str q8, [sp]",
            ),
        ] {
            assert_eq!(dump_frames(name).2, Err(err.to_string()));
        }
    }

    #[test]
    fn frames_golden_change_passes() {
        let rows = fixture_rows("head.txt");
        let golden = frames_fixture("golden-head.txt");
        assert!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &rows)
                .unwrap()
                .is_empty()
        );
        // The reverse (a lost frame, fewer saves) fails too, so the golden never goes stale.
        let base = fixture_rows("base.txt");
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &base).unwrap(),
            [
                "fewer saves: sib CallProperty0 3 -> 2 (a gain: update the golden)",
                "no frame now: sib GetNamedProperty (golden: 1 saves; a gain: update the golden)",
            ]
        );
    }

    #[test]
    fn frames_golden_format_errors() {
        let t = "target x86_64-unknown-linux-gnu\n";
        assert!(
            parse_frames("sib Foo 1\n")
                .unwrap_err()
                .contains("no `target` line")
        );
        assert!(
            parse_frames(&format!("{t}{t}"))
                .unwrap_err()
                .contains("a second target line")
        );
        assert!(
            parse_frames(&format!("{t}sib Foo 1\nsib Foo 2\n"))
                .unwrap_err()
                .contains("sib Foo twice")
        );
        assert!(
            parse_frames(&format!("{t}sib Foo\n"))
                .unwrap_err()
                .contains("line 2")
        );
        assert!(parse_frames(&format!("{t}sib Foo x\n")).is_err());
        // CRLF and comments are fine.
        assert_eq!(
            parse_frames("# c\r\ntarget t\r\n\r\nsib Foo 1\r\n")
                .unwrap()
                .rows
                .len(),
            1
        );
    }

    #[test]
    fn frames_opts() {
        let a = |v: &[&str]| parse_opts(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(
            a(&["--bless"])
                .err()
                .unwrap()
                .contains("--bless needs --frames")
        );
        assert!(
            a(&["--frames", "g.txt", "--binary", "x.a"])
                .err()
                .unwrap()
                .contains("no --binary")
        );
        assert!(
            a(&["--frames", "g.txt", "--target", "a", "--target", "b"])
                .err()
                .unwrap()
                .contains("one target")
        );
        let o = a(&["--frames", "g.txt", "--target", "a", "--bless"]).unwrap();
        assert_eq!(o.frames.as_deref(), Some(Path::new("g.txt")));
        assert!(o.bless);
    }

    // Prologues from the m8-p5 builds of ep9_vm (llvm-objdump 1.99.1 text).
    #[test]
    fn prologue_saves_per_target() {
        let a64 = |lines: &[&str]| entry_saves(Arch::A64, &block("h", lines)).unwrap();
        // aarch64-apple-darwin CreateObjectLiteral: 12 callee-saved registers.
        assert_eq!(
            a64(&[
                "sub\tsp, sp, #0x70",
                "stp\tx28, x27, [sp, #0x10]",
                "stp\tx26, x25, [sp, #0x20]",
                "stp\tx24, x23, [sp, #0x30]",
                "stp\tx22, x21, [sp, #0x40]",
                "stp\tx20, x19, [sp, #0x50]",
                "stp\tx29, x30, [sp, #0x60]",
                "add\tx29, sp, #0x60",
                "mov\tx19, x1",
                "stp\tx19, x20, [sp]",
            ]),
            12
        );
        // aarch64-unknown-linux-gnu CallProperty: one register.
        assert_eq!(
            a64(&["str\tx19, [sp, #-0x10]!", "ldrb\tw16, [x1, #0x3]"]),
            1
        );
        assert_eq!(a64(&["pacibsp", "stp\td9, d8, [sp, #-0x20]!"]), 2);
        // An argument spill is not a save, but the saves after it count (Codex M9-P0 r1).
        assert_eq!(
            a64(&[
                "sub\tsp, sp, #0x20",
                "stp\tx0, x1, [sp]",
                "stp\tx20, x19, [sp, #0x10]"
            ]),
            2
        );
        // A store of a register that the prologue saved is a spill; the scan ends at a branch.
        assert_eq!(
            a64(&[
                "str\tx19, [sp, #-0x10]!",
                "mov\tx19, x0",
                "str\tx19, [sp, #0x8]",
                "cbz\tx0, 0x40",
                "stp\tx21, x22, [sp]",
            ]),
            1
        );
        // A store of a register that an earlier instruction wrote is a spill (M9-P0 r3).
        assert_eq!(
            a64(&["sub\tsp, sp, #0x20", "mov\tx21, x0", "str\tx21, [sp]"]),
            0
        );
        // A part of a callee-saved register that is not saved yet is an unknown save form (it
        // was a silent 0 before M9-P0 r3).
        let part = ["str\tx19, [sp, #-0x10]!", "str\tw20, [sp, #0x4]"];
        assert_eq!(
            entry_saves(Arch::A64, &block("h", &part)),
            Err("str w20, [sp, #0x4]".to_string())
        );
        // An unconditional jump back to a save is followed once (the cycle guard).
        assert_eq!(
            a64(&["sub\tsp, sp, #0x20", "str\tx19, [sp]", "b\t0x1 <h+0x1>"]),
            1
        );
        // x86-64 handlers are sysv64 on every OS (`dsl::handler!`), so a Win64 dump uses the
        // SysV callee-saved set too: rbx, rbp, r12-r15.
        let x64 = |lines: &[&str]| entry_saves(Arch::X64, &block("h", lines)).unwrap();
        // x86_64-pc-windows-msvc StaContextSlot: 2 pushes, then the shadow space.
        let sta = [
            "pushq\t%r14",
            "pushq\t%rbx",
            "subq\t$0x28, %rsp",
            "movsbq\t0x1(%rsi), %rax",
            "pushq\t%rax",
        ];
        assert_eq!(x64(&sta), 2);
        // Every xmm register is caller-saved under sysv64 (also on Win64), so the xmm6 store
        // is a spill: it ends the leading prologue, and the push of rbx after it counts.
        let xmm = [
            "subq\t$0x48, %rsp",
            "movaps\t%xmm6, 0x30(%rsp)",
            "pushq\t%rbx",
        ];
        assert_eq!(x64(&xmm), 1);
        // A push after an argument spill counts; a push of a caller-saved register does not,
        // and the scan ends at a call.
        let spill = [
            "subq\t$0x18, %rsp",
            "movq\t%rdi, 0x8(%rsp)",
            "pushq\t%rbx",
            "pushq\t%rax",
            "callq\t0x0",
            "pushq\t%r12",
        ];
        assert_eq!(x64(&spill), 1);
        // rsi and xmm6 are caller-saved under sysv64 (also on Win64): a push of rsi and an xmm6
        // store after the spill are spills, not saves.
        let w64 = [
            "subq\t$0x48, %rsp",
            "movq\t%rcx, 0x40(%rsp)",
            "pushq\t%rsi",
            "movaps\t%xmm6, 0x20(%rsp)",
        ];
        assert_eq!(x64(&w64), 0);
        // A leading xmm store is not a prologue instruction: it counts 0.
        assert_eq!(x64(&["movaps\t%xmm6, 0x30(%rsp)"]), 0);
        assert_eq!(x64(&["movaps\t%xmm5, 0x30(%rsp)"]), 0);
        assert_eq!(x64(&["movq\t%rsp, %rbp", "pushq\t%rbx"]), 1);
        // A store of a register that an earlier instruction wrote is a spill (M9-P0 r3).
        assert_eq!(
            x64(&[
                "subq\t$0x18, %rsp",
                "movq\t%rdi, %rbx",
                "movq\t%rbx, 0x8(%rsp)"
            ]),
            0
        );
        // A store through %rbp counts only after the scan set %rbp from %rsp.
        assert_eq!(
            entry_saves(
                Arch::X64,
                &block("h", &["subq\t$0x18, %rsp", "movq\t%r12, -0x8(%rbp)"])
            ),
            Err("movq %r12, -0x8(%rbp)".to_string())
        );
    }

    fn row(op: &str, saves: usize) -> FrameRow {
        FrameRow {
            exp: "sib".to_string(),
            op: op.to_string(),
            saves,
        }
    }

    /// M9-P1-WPRE-3 (route FRAME-GOLDEN-MOVFIRST, Codex M9-P0 r3): every hot handler is
    /// scanned, so a red-zone `mov` save as the first instruction makes a row, and the golden
    /// of the build before it fails the build after it (before, the row needed a first push or
    /// sub, and the new save passed the golden).
    #[test]
    fn frames_mov_first_save_makes_a_row() {
        let (_, _, before) = dump_rows("movfirst-x64-base.txt");
        assert!(before.is_empty());
        let golden = frames_text(FRAMES_TARGET, &[], &before);
        let (_, _, after) = dump_rows("movfirst-x64-head.txt");
        assert_eq!(after, [row("RedZone", 1)]);
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &after).unwrap(),
            ["new frame: sib RedZone (1 saves on entry)"]
        );
    }

    /// M9-P1-WPRE-3 (route MC-w42-4): a save counts only a callee-saved register, also in the
    /// leading prologue. A stack alignment push (`pushq %rax`, the X86-KEEPCALL form) adds 0,
    /// so a build that trades `subq $0x10, %rsp` for it keeps the golden (before, it read as
    /// "more saves").
    #[test]
    fn frames_alignment_push_is_not_a_save() {
        let (_, _, before) = dump_rows("alignpush-x64-base.txt");
        let (_, _, after) = dump_rows("alignpush-x64-head.txt");
        assert_eq!(before, [row("StaAlign", 1)]);
        assert_eq!(after, [row("StaAlign", 1)]);
        let golden = frames_text(FRAMES_TARGET, &[], &before);
        assert!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &after)
                .unwrap()
                .is_empty()
        );
        let x64 = |lines: &[&str]| entry_saves(Arch::X64, &block("h", lines));
        // The linux-x64 StaContextSlot prologue at m9-p0: 4 saves, then the alignment push.
        assert_eq!(
            x64(&[
                "pushq\t%r15",
                "pushq\t%r14",
                "pushq\t%r12",
                "pushq\t%rbx",
                "pushq\t%rax",
                "movq\t%rdi, %r9",
            ]),
            Ok(4)
        );
        assert_eq!(x64(&["pushq\t%rax", "callq\t0x0"]), Ok(0));
        assert_eq!(x64(&["pushq\t%rcx", "pushq\t%rbx"]), Ok(1));
    }

    /// M9-P1-WPRE-3 (4): the report-only any-path count sees a save that LLVM shrink-wraps past
    /// the first branch (no entry save, so no row), and counts each callee-saved register once.
    #[test]
    fn any_path_counts_shrink_wrapped_saves() {
        let (format, blocks) = parse_disasm(&frames_fixture("shrinkwrap-a64.txt"));
        let (arch, _) = arch_of(&format).unwrap();
        assert_eq!(entry_saves(arch, &blocks[0]), Ok(0));
        assert_eq!(any_path_saves(arch, &blocks[0]), 4);
        let (_, _, rows) = dump_rows("shrinkwrap-a64.txt");
        assert!(rows.is_empty());
        let x = block(
            "h",
            &[
                "pushq\t%rax",
                "testq\t%rdi, %rdi",
                "je\t0x10",
                "pushq\t%rbx",
                "movq\t%r12, 0x8(%rsp)",
                "movq\t%rbx, 0x10(%rsp)",
                "movq\t%r13, 0x8(%rdi)",
            ],
        );
        assert_eq!(any_path_saves(Arch::X64, &x), 2);
    }

    /// MC-w43-2, MC-w44-4: on x86-64 `Wide_cold`, `ExtraWide_cold` (the prefixed fast bodies,
    /// `interp::tails::wide`) and `CallProperty0_cold` (the host call of `slow::call_property`)
    /// are hot, so the frame check scans them: an entry save there makes a row, and a golden
    /// without that row fails. The ics `Wide_cold` (a forward tail jump, no save) makes no row;
    /// the other `_cold` handlers stay out, and on aarch64 the list does not apply.
    #[test]
    fn frames_x64_hot_cold_handlers_are_scanned() {
        let (abi, handlers, rows) = dump_rows("hotcold-x64.txt");
        assert_eq!((abi, handlers), ("x64-sysv", 5));
        let hot = ["CallProperty0_cold", "ExtraWide_cold", "Wide_cold"];
        assert_eq!(rows, hot.map(|op| row(op, 1)));
        let golden = frames_text(FRAMES_TARGET, &[], &[]);
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &golden, &[], &rows).unwrap(),
            hot.map(|op| format!("new frame: sib {op} (1 saves on entry)"))
        );
        assert!(is_hot(Arch::X64, "Wide_cold") && is_hot(Arch::X64, "Ldar"));
        assert!(is_hot(Arch::X64, "CallProperty1_cold") && is_hot(Arch::X64, "CallProperty2_cold"));
        assert!(!is_hot(Arch::X64, "Call_cold") && !is_hot(Arch::A64, "CallProperty0_cold"));
        assert!(!is_hot(Arch::X64, "Add_cold") && !is_hot(Arch::A64, "ExtraWide_cold"));
    }

    /// The `align64` lines and the FAIL lines of a `llvm-readobj` fixture (tests/disasm_frames,
    /// M10-P1 X86-ALIGN), with its handlers taken from the symbol names (nm gives them in the
    /// real check; one object, so the archive member is empty).
    fn fixture_align(name: &str) -> (Vec<AlignRow>, Vec<String>) {
        let ps = parse_places(&frames_fixture(name)).unwrap();
        let handlers: Vec<Handler> = ps
            .keys()
            .filter_map(|raw| {
                let (name, exp, class) = parse_handler(raw)?;
                Some(Handler {
                    raw: raw.clone(),
                    name,
                    exp,
                    class,
                    member: String::new(),
                    addr: 0,
                })
            })
            .collect();
        let places = ps
            .into_iter()
            .map(|(n, p)| ((String::new(), n), p))
            .collect();
        align_rows(&handlers, &places).unwrap()
    }

    fn arow(exp: &str, aligned: usize, total: usize) -> AlignRow {
        AlignRow {
            exp: exp.to_string(),
            aligned,
            total,
        }
    }

    /// The pass case: every hot handler is in a section with alignment 64 at an offset that is a
    /// multiple of 64 (Mov at 0x40 of the Ldar section); the cold handler (alignment 16) is not
    /// counted. The golden that --bless writes has the column and passes.
    #[test]
    fn align_column_passes() {
        let ps = parse_places(&frames_fixture("align-x64-elf.txt")).unwrap();
        assert_eq!(
            ps["ep9h__sib__tail__Mov"],
            [Place {
                align: 64,
                offset: 0x40
            }]
        );
        // Undefined and absolute symbols have no place.
        assert!(!ps.contains_key("ep9_host_call") && !ps.contains_key("frames.0"));
        let (align, fails) = fixture_align("align-x64-elf.txt");
        assert_eq!(align, [arow("sib", 4, 4)]);
        assert!(fails.is_empty());
        let rows = fixture_rows("base.txt");
        let text = frames_text(FRAMES_TARGET, &align, &rows);
        assert!(text.starts_with(&format!(
            "{FRAMES_HEAD}{ALIGN_HEAD}target {FRAMES_TARGET}\n"
        )));
        assert!(text.contains("\nalign64 sib 4/4\n"));
        assert_eq!(
            parse_frames(&text).unwrap(),
            Golden {
                target: FRAMES_TARGET.to_string(),
                align: align.clone(),
                rows: rows.clone()
            }
        );
        assert!(
            frames_verdict(FRAMES_TARGET, &text, &align, &rows)
                .unwrap()
                .is_empty()
        );
    }

    /// The misaligned case (the flag is lost): a section with alignment 16 and a handler at
    /// offset 0x20 of a 64-aligned section each fail, and the golden of the aligned build fails
    /// too.
    #[test]
    fn align_column_misaligned_fails() {
        let (align, fails) = fixture_align("align-x64-elf-misaligned.txt");
        assert_eq!(align, [arow("sib", 2, 4)]);
        assert_eq!(
            fails,
            [
                "x86-64 alignment: sib 2/4 hot handlers start on a 64-byte line; not: \
                 ep9h__sib__tail__Mov (section alignment 64, offset 0x20), \
                 ep9h__sib__tail__Star (section alignment 16, offset 0x0)"
            ]
        );
        let rows = fixture_rows("base.txt");
        let golden = frames_text(FRAMES_TARGET, &[arow("sib", 4, 4)], &rows);
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &golden, &align, &rows).unwrap(),
            ["align64 changed: sib 4/4 -> 2/4"]
        );
    }

    /// COFF: the alignment is the IMAGE_SCN_ALIGN_<n>BYTES flag (16 when there is none); the
    /// AuxSectionDef `Number` of a section symbol is not a section number.
    #[test]
    fn align_column_coff() {
        let (align, fails) = fixture_align("align-x64-coff.txt");
        assert_eq!(align, [arow("ics", 1, 3)]);
        assert_eq!(
            fails,
            [
                "x86-64 alignment: ics 1/3 hot handlers start on a 64-byte line; not: \
                 ep9h__ics__tail__Add (section alignment 16, offset 0x0), \
                 ep9h__ics__tail__Star (section alignment 16, offset 0x0)"
            ]
        );
        assert!(
            parse_places("File: x.o\nFormat: Mach-O 64-bit x86-64\n")
                .unwrap_err()
                .contains("ELF and COFF objects only")
        );
        // A hot handler with no section is an error, never a silent pass.
        let h = parse_handler("ep9h__ics__tail__Ldar").unwrap();
        let handler = Handler {
            raw: h.0.clone(),
            name: h.0,
            exp: h.1,
            class: h.2,
            member: String::new(),
            addr: 0,
        };
        assert!(
            align_rows(&[handler], &BTreeMap::new())
                .unwrap_err()
                .contains("no section")
        );
    }

    /// The golden form of the column: a new, lost or changed `align64` line fails, and a bad
    /// line is an error.
    #[test]
    fn align_golden_lines() {
        let t = "target x86_64-unknown-linux-gnu\n";
        let rows = fixture_rows("base.txt");
        // An x86-64 golden from before the column: the new lines fail.
        let old = frames_text(FRAMES_TARGET, &[], &rows);
        assert_eq!(
            frames_verdict(
                FRAMES_TARGET,
                &old,
                &[arow("ics", 2, 2), arow("sib", 4, 4)],
                &rows
            )
            .unwrap(),
            ["new align64 line: ics 2/2", "new align64 line: sib 4/4"]
        );
        // A build with no column (aarch64) against a golden with one fails.
        let with = frames_text(FRAMES_TARGET, &[arow("sib", 4, 4)], &rows);
        assert_eq!(
            frames_verdict(FRAMES_TARGET, &with, &[], &rows).unwrap(),
            ["no align64 line now: sib (golden: 4/4; this build has no alignment column)"]
        );
        for (bad, err) in [
            ("align64 sib 5/4\n", "line 2"),
            ("align64 sib 4\n", "line 2"),
            ("align64 sib x/4\n", "line 2"),
            ("align64 sib 4/4\nalign64 sib 4/4\n", "align64 sib twice"),
        ] {
            assert!(
                parse_frames(&format!("{t}{bad}"))
                    .unwrap_err()
                    .contains(err),
                "{bad:?}"
            );
        }
    }
}
