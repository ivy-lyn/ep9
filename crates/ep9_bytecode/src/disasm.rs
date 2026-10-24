//! Disassembler with stable text output (golden files pin it).
//!
//! Instruction line format:
//!
//! ```text
//!   <offset, 5 wide> <S>|E>|  > <mnemonic[.Wide|.ExtraWide]> <operands>   ; <bytes in hex>
//!       12 S> LdaGlobal k0, s1                        ; 10 00 01
//! ```
//!
//! | Operand kind | Printed as |
//! |---|---|
//! | register | `r3`, `a0`, `<this>` |
//! | register list (+ count) | `r2-r4`, `r2`, or `()` when empty; the count is not printed |
//! | pair / triple | `r1-r2`, `r1-r3` |
//! | immediate, context slot/depth | `[5]` |
//! | constant | `k2`, and `; k2 = <value>` at the end of the line |
//! | feedback slot | `s1` |
//! | jump | `@20` (absolute target offset) |
//! | flags | `#0x01` |
//! | runtime id, typeof literal | its name |
//!
//! Text depends only on the unit: no addresses, no hash-map order, no platform float format
//! (Rust's own shortest round-trip `{:?}` for f64).

use core::fmt::Write;

use crate::decode::{Instruction, Instructions};
use crate::feedback::{RuntimeId, TypeOfLiteral};
use crate::operand::{FunctionId, OperandKind, OperandScale, Register};
use crate::positions::SourceLines;
use crate::sha256::to_hex;
use crate::unit::{
    ABI_VERSION, CompiledUnit, Constant, ExportImportName, Function, FunctionFlags, ImportName,
    ModuleRecords, PositionEntry, ScopeFlags,
};

/// Disassembles the whole unit.
pub fn disassemble(unit: &CompiledUnit) -> String {
    let mut s = String::new();
    let h = &unit.header;
    let _ = writeln!(s, "; ep9 bytecode, ABI {ABI_VERSION}");
    let ts = match h.typescript {
        None => "none".to_string(),
        Some(t) => format!(
            "[useDefineForClassFields={}, experimentalDecorators={}, emitDecoratorMetadata={}]",
            t.use_define_for_class_fields, t.experimental_decorators, t.emit_decorator_metadata
        ),
    };
    let _ = writeln!(
        s,
        "unit kind={} strict={} ts={} source_len={} source={} hints={}",
        h.code_kind.name(),
        h.strict,
        ts,
        h.source_len,
        if unit.source.is_some() {
            "embedded"
        } else {
            "hash-only"
        },
        unit.hints
            .as_ref()
            .map_or("none".to_string(), |b| format!("{} bytes", b.len())),
    );
    let _ = writeln!(s, "source_hash={}", to_hex(&h.source_hash));
    let _ = writeln!(s, "strings ({}):", unit.strings.len());
    for (i, st) in unit.strings.iter().enumerate() {
        let _ = writeln!(s, "  str{i} {}", st.quoted());
    }
    let _ = writeln!(s, "scopes ({}):", unit.scopes.len());
    for (i, sc) in unit.scopes.iter().enumerate() {
        let mut flags = Vec::new();
        for (bit, name) in [
            (ScopeFlags::NEEDS_CONTEXT, "needs-context"),
            (ScopeFlags::CALLS_SLOPPY_EVAL, "sloppy-eval"),
            (ScopeFlags::STRICT, "strict"),
            (ScopeFlags::PARAMETER_ENV, "parameter-env"),
        ] {
            if sc.flags.contains(bit) {
                flags.push(name);
            }
        }
        let slots: Vec<String> = sc
            .slots
            .iter()
            .map(|sl| format!("{}:{}", string_text(unit, sl.name.0), sl.mode.name()))
            .collect();
        let _ = writeln!(
            s,
            "  scope{i} {} outer={} flags=[{}] slots=[{}]",
            sc.kind.name(),
            sc.outer.map_or("-".to_string(), |o| o.to_string()),
            flags.join(","),
            slots.join(", ")
        );
    }
    if let Some(m) = &unit.module_records {
        module_records_text(&mut s, unit, m);
    }
    let lines = unit.source.as_deref().map(SourceLines::new);
    for i in 0..unit.functions.len() {
        s.push_str(&disassemble_function_with(
            unit,
            FunctionId(i as u32),
            lines.as_ref(),
        ));
    }
    s
}

/// The module records (ABI 5), one entry per line.
fn module_records_text(s: &mut String, unit: &CompiledUnit, m: &ModuleRecords) {
    let st = |id: crate::StringId| string_text(unit, id.0);
    let _ = writeln!(s, "module records: has-tla={}", m.has_tla);
    let _ = writeln!(s, "  requests ({}):", m.requested_modules.len());
    for (i, r) in m.requested_modules.iter().enumerate() {
        let attrs: Vec<String> = r
            .attributes
            .iter()
            .map(|&(k, v)| format!("{}: {}", st(k), st(v)))
            .collect();
        let _ = writeln!(
            s,
            "  {i:>5} {} attributes=[{}]",
            st(r.specifier),
            attrs.join(", ")
        );
    }
    let _ = writeln!(s, "  imports ({}):", m.import_entries.len());
    for (i, e) in m.import_entries.iter().enumerate() {
        let name = match e.import_name {
            ImportName::Namespace => "namespace".to_string(),
            ImportName::Name(n) => st(n),
        };
        let _ = writeln!(
            s,
            "  {i:>5} request={} import={name} local={} cell={}",
            e.module_request,
            st(e.local_name),
            e.cell
        );
    }
    let _ = writeln!(s, "  local exports ({}):", m.local_export_entries.len());
    for (i, e) in m.local_export_entries.iter().enumerate() {
        let _ = writeln!(
            s,
            "  {i:>5} export={} local={} cell={}",
            st(e.export_name),
            st(e.local_name),
            e.cell
        );
    }
    let _ = writeln!(
        s,
        "  indirect exports ({}):",
        m.indirect_export_entries.len()
    );
    for (i, e) in m.indirect_export_entries.iter().enumerate() {
        let name = match e.import_name {
            ExportImportName::All => "all".to_string(),
            ExportImportName::Name(n) => st(n),
        };
        let _ = writeln!(
            s,
            "  {i:>5} export={} request={} import={name}",
            st(e.export_name),
            e.module_request
        );
    }
    let _ = writeln!(s, "  star exports ({}):", m.star_export_entries.len());
    for (i, r) in m.star_export_entries.iter().enumerate() {
        let _ = writeln!(s, "  {i:>5} request={r}");
    }
}

/// Disassembles one function.
pub fn disassemble_function(unit: &CompiledUnit, id: FunctionId) -> String {
    let lines = unit.source.as_deref().map(SourceLines::new);
    disassemble_function_with(unit, id, lines.as_ref())
}

fn string_text(unit: &CompiledUnit, id: u32) -> String {
    unit.strings
        .get(id as usize)
        .map_or_else(|| format!("<bad str{id}>"), |s| s.quoted())
}

fn constant_text(unit: &CompiledUnit, c: &Constant) -> String {
    let opt = |v: Option<String>| v.unwrap_or_else(|| "_".to_string());
    match c {
        Constant::Number(n) => format!("Number {:?}", n.value()),
        Constant::String(id) => format!("String {id} {}", string_text(unit, id.0)),
        Constant::BigInt(b) => {
            let mut t = String::from(if b.negative {
                "BigInt -0x"
            } else {
                "BigInt 0x"
            });
            match b.limbs.split_last() {
                None => t.push('0'),
                Some((top, rest)) => {
                    let _ = write!(t, "{top:x}");
                    for l in rest.iter().rev() {
                        let _ = write!(t, "{l:016x}");
                    }
                }
            }
            t.push('n');
            t
        }
        Constant::Function(f) => format!("Function {f}"),
        Constant::Scope(sc) => format!("Scope {sc}"),
        Constant::JumpTable(t) => {
            let e: Vec<String> = t.iter().map(|o| format!("+{o}")).collect();
            format!("JumpTable [{}]", e.join(", "))
        }
        Constant::ArrayBoilerplate(e) => {
            let e: Vec<String> = e.iter().map(|v| opt(v.map(|k| k.to_string()))).collect();
            format!("ArrayBoilerplate [{}]", e.join(", "))
        }
        Constant::ObjectBoilerplate(p) => {
            let e: Vec<String> = p
                .iter()
                .map(|(k, v)| format!("{k}: {}", opt(v.map(|v| v.to_string()))))
                .collect();
            format!("ObjectBoilerplate {{{}}}", e.join(", "))
        }
        Constant::TemplateObject(p) => {
            let e: Vec<String> = p
                .iter()
                .map(|(raw, cooked)| {
                    format!(
                        "({}, {})",
                        string_text(unit, raw.0),
                        cooked.map_or("undefined".to_string(), |c| string_text(unit, c.0))
                    )
                })
                .collect();
            format!("TemplateObject [{}]", e.join(", "))
        }
        Constant::Oddball(o) => format!("Oddball {}", o.name()),
    }
}

/// The text of one instruction (mnemonic and operands, without offset and bytes).
pub fn instruction_text(unit: &CompiledUnit, func: &Function, insn: &Instruction<'_>) -> String {
    let mut t = String::from(insn.opcode.name());
    match insn.scale {
        OperandScale::Single => {}
        OperandScale::Wide => t.push_str(".Wide"),
        OperandScale::ExtraWide => t.push_str(".ExtraWide"),
    }
    let info = insn.opcode.info();
    let mut parts: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (k, op) in info.operands.iter().enumerate() {
        let v = insn.operand(k);
        let reg = |v: i64| Register::from_operand(v as i32).to_string();
        let range = |first: i64, n: i64| match n {
            0 => "()".to_string(),
            1 => reg(first),
            _ => format!("{}-{}", reg(first), reg(first + n - 1)),
        };
        let text = match op.kind {
            OperandKind::Reg | OperandKind::RegOut | OperandKind::RegInOut => reg(v),
            OperandKind::RegList | OperandKind::RegOutList => range(v, insn.operand(k + 1)),
            OperandKind::RegPair | OperandKind::RegOutPair => range(v, 2),
            OperandKind::RegOutTriple => range(v, 3),
            OperandKind::RegCount => continue,
            OperandKind::Imm | OperandKind::UImm | OperandKind::CtxSlot | OperandKind::CtxDepth => {
                format!("[{v}]")
            }
            OperandKind::Const => {
                if let Some(c) = func.constants.get(v as usize) {
                    notes.push(format!("k{v} = {}", constant_text(unit, c)));
                }
                format!("k{v}")
            }
            OperandKind::Slot => format!("s{v}"),
            OperandKind::JumpFwd | OperandKind::JumpBack => match insn.jump_target() {
                Some(target) => format!("@{target}"),
                None => format!("@?{v}"),
            },
            OperandKind::Flag8 | OperandKind::Flag16 => format!("#0x{v:02x}"),
            OperandKind::Runtime => RuntimeId::from_u16(v as u16)
                .map_or_else(|| format!("<runtime {v}>"), |id| id.info().name.to_string()),
            OperandKind::TypeOfLiteral => TypeOfLiteral::from_u8(v as u8)
                .map_or_else(|| format!("<typeof {v}>"), |l| l.name().to_string()),
        };
        parts.push(text);
    }
    if !parts.is_empty() {
        t.push(' ');
        t.push_str(&parts.join(", "));
    }
    for n in notes {
        t.push_str("  ; ");
        t.push_str(&n);
    }
    t
}

fn disassemble_function_with(
    unit: &CompiledUnit,
    id: FunctionId,
    lines: Option<&SourceLines<'_>>,
) -> String {
    let f = &unit.functions[id.index()];
    let mut s = String::new();
    let mut flags = Vec::new();
    for (bit, name) in [
        (FunctionFlags::STRICT, "strict"),
        (FunctionFlags::GENERATOR, "generator"),
        (FunctionFlags::ASYNC, "async"),
        (FunctionFlags::SIMPLE_PARAMETERS, "simple-params"),
    ] {
        if f.flags.contains(bit) {
            flags.push(name);
        }
    }
    let _ = writeln!(
        s,
        "function {id} {} name={} parent={} outer_scope={} flags=[{}] params={} length={} regs={} range={}..{}",
        f.kind.name(),
        f.name.map_or("-".to_string(), |n| string_text(unit, n.0)),
        f.parent.map_or("-".to_string(), |p| p.to_string()),
        f.outer_scope.map_or("-".to_string(), |o| o.to_string()),
        flags.join(","),
        f.parameter_count,
        f.length,
        f.register_count,
        f.source_start,
        f.source_end,
    );
    let _ = writeln!(s, "  code ({} bytes):", f.bytecode.len());
    let mut pos = f.positions.iter().peekable();
    for r in Instructions::new(&f.bytecode) {
        let insn = match r {
            Ok(i) => i,
            Err((at, e)) => {
                let _ = writeln!(s, "  {at:>5}    <decode error: {e}>");
                break;
            }
        };
        let mut marker = "  ";
        while let Some(p) = pos.peek() {
            if p.bytecode_offset < insn.offset {
                pos.next();
            } else {
                if p.bytecode_offset == insn.offset {
                    marker = if p.is_statement { "S>" } else { "E>" };
                }
                break;
            }
        }
        let text = instruction_text(unit, f, &insn);
        let hex: Vec<String> = insn.bytes.iter().map(|b| format!("{b:02x}")).collect();
        let _ = writeln!(
            s,
            "  {:>5} {marker} {text:<44} ; {}",
            insn.offset,
            hex.join(" ")
        );
    }
    let _ = writeln!(s, "  constants ({}):", f.constants.len());
    for (i, c) in f.constants.iter().enumerate() {
        let _ = writeln!(s, "  {i:>5} {}", constant_text(unit, c));
    }
    let _ = writeln!(s, "  feedback ({}):", f.feedback_slots.len());
    for (i, k) in f.feedback_slots.iter().enumerate() {
        let _ = writeln!(s, "  {i:>5} {}", k.name());
    }
    let _ = writeln!(s, "  handlers ({}):", f.handlers.len());
    for h in &f.handlers {
        let _ = writeln!(
            s,
            "        [{}, {}) -> @{} context={} {}",
            h.start,
            h.end,
            h.target,
            h.context,
            h.kind.name()
        );
    }
    let _ = writeln!(s, "  positions ({}):", f.positions.len());
    for p in &f.positions {
        let _ = writeln!(s, "  {}", position_text(p, lines));
    }
    if let Some(m) = &f.mapped_parameters {
        let slots: Vec<String> = m
            .slots
            .iter()
            .map(|v| v.map_or("-".to_string(), |v| v.to_string()))
            .collect();
        let _ = writeln!(
            s,
            "  mapped parameters ({}): scope={} slots=[{}]",
            m.slots.len(),
            m.scope,
            slots.join(", ")
        );
    }
    s
}

fn position_text(p: &PositionEntry, lines: Option<&SourceLines<'_>>) -> String {
    let kind = if p.is_statement { "S" } else { "E" };
    let mut t = format!("{:>5} {kind} {}", p.bytecode_offset, p.source_offset);
    if let Some(l) = lines {
        let lc = l.line_column(p.source_offset);
        let _ = write!(t, " ({}:{})", lc.line, lc.column);
    }
    t
}
