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
  --binary FILE         check this built file instead of building; repeat for more
  --expect N            every expansion must have exactly N handlers
  --max-args N          handler argument limit (default 4)
  --objdump PATH, --nm PATH   tools (default: rustup llvm-tools, then PATH)
  -v, --verbose         one line per handler";

const ALL_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

const PREFIX: &str = "ep9h__";

struct Opts {
    manifest: PathBuf,
    package: Option<String>,
    targets: Vec<String>,
    features: Option<String>,
    toolchain: Option<String>,
    binaries: Vec<PathBuf>,
    expect: Option<usize>,
    max_args: usize,
    objdump: Option<PathBuf>,
    nm: Option<PathBuf>,
    verbose: bool,
}

fn repo_root() -> PathBuf {
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
        binaries: Vec::new(),
        expect: None,
        max_args: 4,
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
            "--binary" => o.binaries.push(PathBuf::from(val()?)),
            "--expect" => o.expect = Some(val()?.parse().map_err(|e| format!("--expect: {e}"))?),
            "--max-args" => o.max_args = val()?.parse().map_err(|e| format!("--max-args: {e}"))?,
            "--objdump" => o.objdump = Some(PathBuf::from(val()?)),
            "--nm" => o.nm = Some(PathBuf::from(val()?)),
            "-v" | "--verbose" => o.verbose = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    if o.targets.is_empty() && o.binaries.is_empty() {
        o.targets.push(host_triple()?);
    }
    Ok(o)
}

fn run_capture(cmd: &mut Command) -> Result<String, String> {
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

fn host_triple() -> Result<String, String> {
    let v = run_capture(Command::new("rustc").arg("-vV"))?;
    v.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_string)
        .ok_or_else(|| "rustc -vV: no host line".into())
}

/// Finds an LLVM tool: the override, then rustup's llvm-tools component, then PATH.
fn find_tool(name: &str, over: &Option<PathBuf>) -> Result<PathBuf, String> {
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

/// Builds the crate as a staticlib for `target` and returns (staticlib, LLVM IR file).
fn build(o: &Opts, target: &str) -> Result<(PathBuf, PathBuf), String> {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target"));
    let tdir = base.join("xtask-disasm");
    fs::create_dir_all(&tdir).map_err(|e| e.to_string())?;
    let tag = format!(
        "{target}{}{}",
        o.toolchain
            .as_deref()
            .map(|t| format!("-{t}"))
            .unwrap_or_default(),
        o.features
            .as_deref()
            .map(|f| format!("-{}", f.replace([',', ' '], "+")))
            .unwrap_or_default()
    );
    let ll = tdir.join(format!("{tag}.ll"));
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
        .arg(format!("--emit=llvm-ir={}", ll.display()));
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

#[derive(Debug)]
struct Insn {
    addr: u64,
    text: String,
    reloc: Option<String>,
}

#[derive(Debug)]
struct Block {
    member: String,
    raw: String,
    addr: u64,
    insns: Vec<Insn>,
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
    let (format, blocks) = parse_disasm(&out);
    if format.is_empty() {
        return Err(format!(
            "llvm-objdump printed no file format for {}",
            file.display()
        ));
    }
    Ok((format, blocks))
}

/// Reads `llvm-objdump -d -r` output: the file format (empty if none) and one block per symbol.
fn parse_disasm(out: &str) -> (String, Vec<Block>) {
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
    let handlers = nm_handlers(&tools.nm, file)?;
    if handlers.is_empty() {
        println!(
            "== {label}: {}\n   FAIL: no handler symbols ({PREFIX}*) found",
            file.display()
        );
        return Ok(false);
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
        let (f, mut b) = if m.is_empty() {
            disassemble_reach(&raws, |names| disassemble(&tools.objdump, file, names))?
        } else {
            let dir = std::env::temp_dir().join(format!("ep9-disasm-{}", std::process::id()));
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            run_quiet(
                Command::new(&tools.ar)
                    .arg("x")
                    .arg(format!("--output={}", dir.display()))
                    .arg(file)
                    .arg(m),
            )?;
            let r = disassemble_reach(&raws, |names| {
                disassemble(&tools.objdump, &dir.join(m), names)
            });
            let _ = fs::remove_dir_all(&dir);
            r?
        };
        b.iter_mut().for_each(|blk| blk.member = m.to_string());
        format = f;
        blocks.append(&mut b);
    }
    let (arch, abi) = arch_of(&format)?;
    println!("== {label} ({abi}, {format}): {}", file.display());

    // Find each handler's block by name. Fallback for a symbol that LLVM merged into another
    // one (an alias at the same address): use that block, but only if exactly one block sits
    // at that address. In objects with one section per function every function starts at
    // address 0, so the fallback never applies there.
    let block_of = |h: &Handler| -> Option<&Block> {
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
    };

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
            let Some(b) = block_of(h) else {
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
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn json_filenames() {
        let l = r#"{"reason":"compiler-artifact","filenames":["/a/b/libx.a","C:\\t\\x.lib"],"fresh":true}"#;
        assert_eq!(
            json_string_array(l, "filenames"),
            vec!["/a/b/libx.a".to_string(), "C:\\t\\x.lib".to_string()]
        );
    }
}
