//! Shared test fixtures: hand-built sample units and a random well-formed unit generator.
#![allow(dead_code)]

use ep9_bytecode::*;

/// Deterministic PRNG (SplitMix64). Same seed, same sequence, on every target.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    /// A u32 of random magnitude: fits 1, 2 or 4 bytes.
    pub fn u32_any(&mut self) -> u32 {
        match self.below(3) {
            0 => self.below(256) as u32,
            1 => self.below(65536) as u32,
            _ => self.next() as u32,
        }
    }

    /// An i32 of random magnitude.
    pub fn i32_any(&mut self) -> i32 {
        match self.below(3) {
            0 => self.below(256) as i32 - 128,
            1 => self.below(65536) as i32 - 32768,
            _ => self.next() as i32,
        }
    }
}

pub fn script_options(embed: bool) -> UnitOptions {
    UnitOptions {
        code_kind: CodeKind::Script,
        strict: false,
        typescript: None,
        engine_build_id: [0; 16],
        embed_source: embed,
    }
}

/// A sample script unit with two functions. It covers globals, contexts, closures, calls,
/// a loop, a switch, try/catch, literals, a template object, BigInt, UTF-16 strings and
/// source positions. The bytecode is hand written (no BytecodeGen yet) but follows the
/// shape BytecodeGen will emit.
pub const SAMPLE_SCRIPT: &str = "var total = 0;\n\
function add(a, b) { return a + b; }\n\
for (let i = 0; i < 3; i++) { total = add(total, i); }\n\
switch (total) { case 0: break; case 1: total = 'caf\u{e9} \u{20ac}'; }\n\
try { print(tag`n=${total}`, 1n, [1, , 'x'], {a: 1.5, b: null}); } catch (e) { total = -1; }\n";

pub fn sample_script_unit() -> VerifiedUnit {
    let src = SAMPLE_SCRIPT;
    let pos = |needle: &str| src.find(needle).expect("needle in sample") as u32;
    let mut unit = UnitBuilder::new(src, script_options(true));
    let script_scope = unit.add_scope(
        ScopeKind::Script,
        ScopeFlags::default(),
        None,
        vec![
            (UnitString::new("total"), VariableMode::Var),
            (UnitString::new("add"), VariableMode::Function),
        ],
    );
    let catch_scope = unit.add_scope(
        ScopeKind::Catch,
        ScopeFlags::NEEDS_CONTEXT,
        Some(script_scope),
        vec![(UnitString::new("e"), VariableMode::Var)],
    );

    // f1: function add(a, b) { return a + b; }
    let add_start = pos("function add");
    let mut f = FunctionBuilder::new(FunctionMeta {
        kind: FunctionKind::Normal,
        flags: FunctionFlags::SIMPLE_PARAMETERS,
        name: Some(UnitString::new("add")),
        parent: Some(FunctionId(0)),
        outer_scope: Some(script_scope),
        source_start: add_start,
        source_end: pos("\nfor"),
        parameter_count: 2,
        length: 2,
    });
    let slot = f.feedback_slot(FeedbackSlotKind::BinaryOp);
    f.set_statement_position(pos("return a"));
    f.ldar(f.parameter(1));
    f.set_expression_position(pos("a + b"));
    f.add(f.parameter(0), slot);
    f.return_();
    let add_code = f.finish().expect("add builds");

    // f0: the script.
    let mut b = FunctionBuilder::new(FunctionMeta::entry(
        CodeKind::Script,
        false,
        src.len() as u32,
    ));
    let ctx = b.new_register();
    let i = b.new_register();
    let tmp = b.new_register();
    let k_total = b.string_constant("total");
    let k_add = b.string_constant("add");
    let k_add_fn = b.function_constant(FunctionId(1));
    let s_closure = b.feedback_slot(FeedbackSlotKind::CreateClosure);
    let s_store = b.feedback_slot(FeedbackSlotKind::StoreGlobal);
    let s_load = b.feedback_slot(FeedbackSlotKind::LoadGlobal);
    let s_cmp = b.feedback_slot(FeedbackSlotKind::CompareOp);
    let s_inc = b.feedback_slot(FeedbackSlotKind::BinaryOp);
    let s_call = b.feedback_slot(FeedbackSlotKind::Call);
    let s_loop = b.feedback_slot(FeedbackSlotKind::JumpLoop);

    // GlobalDeclarationInstantiation, then `add` and `var total = 0`.
    b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
    b.create_closure(k_add_fn, s_closure, 0);
    b.sta_global(k_add, s_store);
    b.set_statement_position(pos("var total"));
    b.lda_zero();
    b.sta_global(k_total, s_store);

    // for (let i = 0; i < 3; i++) { total = add(total, i); }
    b.set_statement_position(pos("for ("));
    b.lda_zero().star(i);
    let header = b.bound_label();
    let exit = b.new_label();
    b.lda_smi(3);
    b.set_expression_position(pos("i < 3"));
    b.test_less_than(i, s_cmp);
    b.jump_if_false(exit);
    b.set_statement_position(pos("total = add"));
    let args = b.new_register_list(2);
    b.lda_global(k_add, s_load).star(tmp);
    b.lda_global(k_total, s_load).star(args.get(0));
    b.mov(i, args.get(1));
    b.set_expression_position(pos("add(total"));
    b.call_undefined_receiver(tmp, args, 2, s_call);
    b.sta_global(k_total, s_store);
    b.ldar(i).inc(s_inc).star(i);
    b.jump_loop(header, 0, s_loop);
    b.bind(exit);

    // switch (total) { case 0: break; case 1: total = 'café €'; }
    b.set_statement_position(pos("switch"));
    b.lda_global(k_total, s_load);
    let (table, cases) = b.jump_table(2);
    let switch_end = b.new_label();
    b.switch_on_smi(table, 0);
    b.jump(switch_end);
    b.bind(cases[0]);
    b.jump(switch_end);
    b.bind(cases[1]);
    let k_cafe = b.string_constant("caf\u{e9} \u{20ac}");
    b.lda_constant(k_cafe);
    b.sta_global(k_total, s_store);
    b.bind(switch_end);

    // try { print(tag`n=${total}`, 1n, [1, , 'x'], {a: 1.5, b: null}); } catch (e) { total = -1; }
    let try_start = b.new_label();
    let try_end = b.new_label();
    let handler = b.new_label();
    let done = b.new_label();
    b.lda_current_context().star(ctx); // the handler restores this context
    b.add_handler(try_start, try_end, handler, ctx, HandlerKind::Caught);
    b.bind(try_start);
    b.set_statement_position(pos("print("));
    let k_print = b.string_constant("print");
    let k_tag = b.string_constant("tag");
    let call_args = b.new_register_list(4);
    let tag_args = b.new_register_list(2);
    let k_tpl = b.template_object(vec![
        (UnitString::new("n="), Some(UnitString::new("n="))),
        (UnitString::new(""), Some(UnitString::new(""))),
    ]);
    let s_tpl = b.feedback_slot(FeedbackSlotKind::Literal);
    b.lda_global(k_tag, s_load).star(tmp);
    b.get_template_object(k_tpl, s_tpl).star(tag_args.get(0));
    b.lda_global(k_total, s_load).star(tag_args.get(1));
    b.call_undefined_receiver(tmp, tag_args, 2, s_call);
    b.star(call_args.get(0));
    let k_big = b.bigint_constant(BigIntConst::new(false, &[1]));
    b.lda_constant(k_big).star(call_args.get(1));
    let k_one = b.number_constant(1.0);
    let k_hole = b.oddball_constant(Oddball::Hole);
    let k_x = b.string_constant("x");
    let k_arr = b.array_boilerplate(vec![Some(k_one), Some(k_hole), Some(k_x)]);
    let s_arr = b.feedback_slot(FeedbackSlotKind::Literal);
    b.create_array_literal(k_arr, s_arr, 1)
        .star(call_args.get(2));
    let k_a = b.string_constant("a");
    let k_15 = b.number_constant(1.5);
    let k_b = b.string_constant("b");
    let k_null = b.oddball_constant(Oddball::Null);
    let k_obj = b.object_boilerplate(vec![(k_a, Some(k_15)), (k_b, Some(k_null))]);
    let s_obj = b.feedback_slot(FeedbackSlotKind::Literal);
    b.create_object_literal(k_obj, s_obj, 1)
        .star(call_args.get(3));
    b.lda_global(k_print, s_load).star(tmp);
    b.call_undefined_receiver(tmp, call_args, 4, s_call);
    b.bind(try_end);
    b.jump(done);
    b.bind(handler);
    b.set_statement_position(pos("catch"));
    let k_catch = b.scope_constant(catch_scope);
    b.star(tmp);
    b.create_catch_context(tmp, k_catch);
    b.push_context(ctx);
    b.set_statement_position(pos("total = -1"));
    b.lda_smi(-1);
    b.sta_global(k_total, s_store);
    b.pop_context(ctx);
    b.bind(done);
    b.lda_undefined();
    b.return_();
    let script_code = b.finish().expect("script builds");

    unit.set_function(FunctionId(1), add_code);
    unit.set_function(FunctionId(0), script_code);
    unit.finish().expect("sample script verifies")
}

/// A TypeScript module unit with wide operands, a strict class-like function tree, a
/// generator-free async arrow, every TS flag set and (ABI 5) module records of every kind:
/// a default and a namespace import, a local re-export of the namespace, an indirect re-export
/// of the default import, a star export and an `export * as` with import attributes.
pub const SAMPLE_MODULE: &str = "import d, * as ns from \"./m.js\";\n\
export { d as e, ns };\n\
export * from \"./n.js\" with { type: \"json\" };\n\
export * as all from \"./n.js\" with { type: \"json\" };\n\
export const big = 100000;\n\
export async function f(x: number) { return () => x + big; }\n";

/// The module records of [`SAMPLE_MODULE`] (ECMA-262 §16.2.1.6 ParseModule): `export { d as e }`
/// re-exports an import, so it becomes an indirect entry; `export { ns }` re-exports a
/// namespace import, so it stays a local entry of the import's cell.
pub fn sample_module_records() -> ModuleRecords<UnitString> {
    let s = UnitString::new;
    ModuleRecords {
        requested_modules: vec![
            ModuleRequest {
                specifier: s("./m.js"),
                attributes: Vec::new(),
            },
            ModuleRequest {
                specifier: s("./n.js"),
                attributes: vec![(s("type"), s("json"))],
            },
        ],
        import_entries: vec![
            ImportEntry {
                module_request: 0,
                import_name: ImportName::Name(s("default")),
                local_name: s("d"),
                cell: -1,
            },
            ImportEntry {
                module_request: 0,
                import_name: ImportName::Namespace,
                local_name: s("ns"),
                cell: -2,
            },
        ],
        local_export_entries: vec![
            LocalExportEntry {
                export_name: s("ns"),
                local_name: s("ns"),
                cell: -2,
            },
            LocalExportEntry {
                export_name: s("big"),
                local_name: s("big"),
                cell: 1,
            },
            LocalExportEntry {
                export_name: s("f"),
                local_name: s("f"),
                cell: 2,
            },
        ],
        indirect_export_entries: vec![
            IndirectExportEntry {
                export_name: s("e"),
                module_request: 0,
                import_name: ExportImportName::Name(s("default")),
            },
            IndirectExportEntry {
                export_name: s("all"),
                module_request: 1,
                import_name: ExportImportName::All,
            },
        ],
        star_export_entries: vec![1],
        has_tla: false,
    }
}

pub fn sample_module_unit() -> VerifiedUnit {
    let src = SAMPLE_MODULE;
    let pos = |needle: &str| src.find(needle).expect("needle in sample") as u32;
    let mut unit = UnitBuilder::new(
        src,
        UnitOptions {
            code_kind: CodeKind::Module,
            strict: true,
            typescript: Some(TsFlags {
                use_define_for_class_fields: false,
                experimental_decorators: true,
                emit_decorator_metadata: true,
            }),
            engine_build_id: *b"ep9-test-build-1",
            embed_source: true,
        },
    );
    let module_scope = unit.add_scope(
        ScopeKind::Module,
        ScopeFlags::STRICT | ScopeFlags::NEEDS_CONTEXT,
        None,
        vec![
            (UnitString::new("big"), VariableMode::Const),
            (UnitString::new("f"), VariableMode::Function),
        ],
    );
    let f_scope = unit.add_scope(
        ScopeKind::Function,
        ScopeFlags::STRICT | ScopeFlags::NEEDS_CONTEXT,
        Some(module_scope),
        vec![(UnitString::new("x"), VariableMode::Var)],
    );
    unit.set_hints(vec![0xde, 0xad, 0xbe, 0xef]);
    unit.set_module_records(sample_module_records());

    // f2: () => x + big
    let mut a = FunctionBuilder::new(FunctionMeta {
        kind: FunctionKind::Arrow,
        flags: FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS,
        name: None,
        parent: Some(FunctionId(1)),
        outer_scope: Some(f_scope),
        source_start: pos("() =>"),
        source_end: pos("; }"),
        parameter_count: 0,
        length: 0,
    });
    let s = a.feedback_slot(FeedbackSlotKind::BinaryOp);
    let r = a.new_register();
    a.set_statement_position(pos("x + big"));
    a.lda_module_variable(1, 1).star(r);
    a.lda_current_context_slot(0);
    a.add(r, s).return_();
    let arrow = a.finish().expect("arrow builds");

    // f1: async function f(x) { ... } (simplified: no await)
    let mut f = FunctionBuilder::new(FunctionMeta {
        kind: FunctionKind::Normal,
        flags: FunctionFlags::STRICT | FunctionFlags::ASYNC | FunctionFlags::SIMPLE_PARAMETERS,
        name: Some(UnitString::new("f")),
        parent: Some(FunctionId(0)),
        outer_scope: Some(module_scope),
        source_start: pos("async function"),
        source_end: src.len() as u32 - 1,
        parameter_count: 1,
        length: 1,
    });
    let ctx = f.new_register();
    // 300 filler registers make the closure register need a Wide operand.
    let _filler = f.new_register_list(300);
    let wide_reg = f.new_register();
    let k_scope = f.scope_constant(f_scope);
    f.create_function_context(k_scope, 1);
    f.push_context(ctx);
    f.ldar(f.parameter(0));
    f.sta_current_context_slot(0);
    let k_arrow = f.function_constant(FunctionId(2));
    let s_cl = f.feedback_slot(FeedbackSlotKind::CreateClosure);
    f.set_statement_position(pos("return ()"));
    f.create_closure(k_arrow, s_cl, 0).star(wide_reg);
    f.ldar(wide_reg);
    f.return_();
    let fcode = f.finish().expect("f builds");

    // f0: module top level: big = 100000 (ExtraWide immediate); f is hoisted.
    let mut m = FunctionBuilder::new(FunctionMeta::entry(
        CodeKind::Module,
        true,
        src.len() as u32,
    ));
    let k_f = m.function_constant(FunctionId(1));
    let s_cl = m.feedback_slot(FeedbackSlotKind::CreateClosure);
    m.create_closure(k_f, s_cl, 0);
    m.sta_module_variable(2, 0);
    m.set_statement_position(pos("export const"));
    m.lda_smi(100_000);
    m.sta_module_variable(1, 0);
    m.lda_undefined().return_();
    let mcode = m.finish().expect("module builds");

    unit.set_function(FunctionId(2), arrow);
    unit.set_function(FunctionId(0), mcode);
    unit.set_function(FunctionId(1), fcode);
    unit.finish().expect("sample module verifies")
}

/// The smallest unit: an empty script that returns undefined. Its full hex dump is a golden
/// file, so it pins the byte layout.
pub fn tiny_unit() -> VerifiedUnit {
    let mut b = FunctionBuilder::new(FunctionMeta::entry(CodeKind::Script, false, 0));
    b.lda_undefined().return_();
    let mut u = UnitBuilder::new("", script_options(true));
    u.set_function(FunctionId(0), b.finish().unwrap());
    u.finish().unwrap()
}

// ---- Random well-formed units ----------------------------------------------------------

const SOURCE_CHARS: &[char] = &[
    'a',
    'b',
    ' ',
    '\n',
    '\u{e9}',
    '\u{20ac}',
    '\u{1F600}',
    ';',
    '(',
    ')',
];

struct FnPlan {
    parent: Option<u32>,
    kind: FunctionKind,
    flags: FunctionFlags,
    params: u32,
    regs: u32,
    start: u32,
    end: u32,
}

/// Input of `UnitBuilder::add_scope`.
pub type ScopeInput = (
    ScopeKind,
    ScopeFlags,
    Option<ScopeId>,
    Vec<(UnitString, VariableMode)>,
);

/// Generates a random, well-formed unit for `seed`, and returns its function codes in id
/// order together with the unit builder input, so callers can also shuffle the order.
pub struct RandomUnit {
    pub source: String,
    pub opts: UnitOptions,
    pub scopes: Vec<ScopeInput>,
    pub functions: Vec<FunctionCode>,
    pub hints: Option<Vec<u8>>,
    /// ABI 5: the module records of a module unit (`None` gives empty records).
    pub module_records: Option<ModuleRecords<UnitString>>,
}

impl RandomUnit {
    /// Builds the unit, setting functions in `order` (a permutation of ids).
    pub fn build(&self, order: &[usize]) -> Result<VerifiedUnit, BuildError> {
        let mut u = UnitBuilder::new(&self.source, self.opts);
        for (kind, flags, outer, slots) in &self.scopes {
            u.add_scope(*kind, *flags, *outer, slots.clone());
        }
        for &i in order {
            u.set_function(FunctionId(i as u32), self.functions[i].clone());
        }
        if let Some(h) = &self.hints {
            u.set_hints(h.clone());
        }
        if let Some(m) = &self.module_records {
            u.set_module_records(m.clone());
        }
        u.finish()
    }

    pub fn build_in_order(&self) -> Result<VerifiedUnit, BuildError> {
        let order: Vec<usize> = (0..self.functions.len()).collect();
        self.build(&order)
    }
}

fn random_string(rng: &mut Rng) -> UnitString {
    let n = rng.below(6) as usize;
    let mut units = Vec::with_capacity(n);
    for _ in 0..n {
        units.push(match rng.below(4) {
            0 => b'a' as u16 + rng.below(26) as u16,
            1 => rng.below(256) as u16,
            2 => 0xD800 + rng.below(0x800) as u16, // lone surrogates
            _ => rng.below(65536) as u16,
        });
    }
    UnitString::from_utf16(&units)
}

pub fn random_unit(seed: u64) -> RandomUnit {
    let mut rng = Rng::new(seed);
    // Source text.
    let len = rng.below(200) as usize;
    let source: String = (0..len).map(|_| *rng.pick(SOURCE_CHARS)).collect();
    let src_len = source.len() as u32;
    let code_kind = *rng.pick(&[CodeKind::Script, CodeKind::Module, CodeKind::Eval]);
    let strict = code_kind == CodeKind::Module || rng.chance(50);
    let typescript = rng.chance(30).then(|| TsFlags {
        use_define_for_class_fields: rng.chance(50),
        experimental_decorators: rng.chance(50),
        emit_decorator_metadata: rng.chance(50),
    });
    let mut build_id = [0u8; 16];
    for b in build_id.iter_mut() {
        *b = rng.next() as u8;
    }
    let opts = UnitOptions {
        code_kind,
        strict,
        typescript,
        engine_build_id: build_id,
        embed_source: rng.chance(70),
    };
    // Scopes: outer always earlier.
    let nscopes = 1 + rng.below(4) as usize;
    let mut scopes = Vec::new();
    for s in 0..nscopes {
        let kind = *rng.pick(&[
            ScopeKind::Script,
            ScopeKind::Function,
            ScopeKind::Block,
            ScopeKind::Catch,
            ScopeKind::Class,
        ]);
        let mut flags = ScopeFlags((rng.below(8)) as u8);
        // ABI 5: PARAMETER_ENV only on a sloppy function scope with a sloppy eval.
        if kind == ScopeKind::Function
            && flags.contains(ScopeFlags::CALLS_SLOPPY_EVAL)
            && !flags.contains(ScopeFlags::STRICT)
            && rng.chance(50)
        {
            flags = flags | ScopeFlags::PARAMETER_ENV;
        }
        let outer = (s > 0 && rng.chance(70)).then(|| ScopeId(rng.below(s as u64) as u32));
        let slots: Vec<(UnitString, VariableMode)> = (0..1 + rng.below(4))
            .map(|_| {
                (
                    random_string(&mut rng),
                    *rng.pick(&[
                        VariableMode::Var,
                        VariableMode::Let,
                        VariableMode::Const,
                        VariableMode::Function,
                        VariableMode::Class,
                        VariableMode::FunctionName,
                    ]),
                )
            })
            .collect();
        scopes.push((kind, flags, outer, slots));
    }
    let scope_sizes: Vec<u32> = scopes.iter().map(|s| s.3.len() as u32).collect();
    let max_ctx_slots = *scope_sizes.iter().max().unwrap();
    // Function tree in preorder: the parent of i is on the ancestor chain of i - 1.
    let nfuncs = 1 + rng.below(5) as usize;
    let mut plans: Vec<FnPlan> = Vec::new();
    let mut chain: Vec<u32> = Vec::new();
    for i in 0..nfuncs as u32 {
        let parent = if i == 0 {
            None
        } else {
            let keep = 1 + rng.below(chain.len() as u64) as usize;
            chain.truncate(keep);
            Some(*chain.last().unwrap())
        };
        chain.push(i);
        let (kind, mut flags, params) = if i == 0 {
            (
                code_kind.entry_function_kind(),
                if strict {
                    FunctionFlags::STRICT
                } else {
                    FunctionFlags::default()
                },
                0,
            )
        } else {
            let kind = *rng.pick(&[
                FunctionKind::Normal,
                FunctionKind::Arrow,
                FunctionKind::Method,
                FunctionKind::Getter,
                FunctionKind::Setter,
                FunctionKind::ClassConstructor,
                FunctionKind::DerivedConstructor,
                FunctionKind::ClassFieldsInitializer,
            ]);
            let mut flags = FunctionFlags::default();
            if rng.chance(50) {
                flags = flags.with(FunctionFlags::STRICT);
            }
            if matches!(kind, FunctionKind::Normal | FunctionKind::Method) && rng.chance(30) {
                flags = flags.with(FunctionFlags::GENERATOR);
            }
            if matches!(
                kind,
                FunctionKind::Normal | FunctionKind::Method | FunctionKind::Arrow
            ) && rng.chance(30)
            {
                flags = flags.with(FunctionFlags::ASYNC);
            }
            (kind, flags, rng.below(5) as u32)
        };
        if let Some(p) = parent
            && plans[p as usize].flags.contains(FunctionFlags::STRICT)
        {
            flags = flags.with(FunctionFlags::STRICT);
        }
        if rng.chance(50) {
            flags = flags.with(FunctionFlags::SIMPLE_PARAMETERS);
        }
        let a = rng.below(u64::from(src_len) + 1) as u32;
        let b = rng.below(u64::from(src_len) + 1) as u32;
        plans.push(FnPlan {
            parent,
            kind,
            flags,
            params,
            regs: if rng.chance(10) {
                300
            } else {
                1 + rng.below(12) as u32
            },
            start: a.min(b),
            end: a.max(b),
        });
    }
    let mut functions = Vec::new();
    for (i, plan) in plans.iter().enumerate() {
        let children: Vec<u32> = plans
            .iter()
            .enumerate()
            .filter(|(_, p)| p.parent == Some(i as u32))
            .map(|(j, _)| j as u32)
            .collect();
        functions.push(random_function(
            &mut rng,
            i as u32,
            plan,
            &children,
            &scopes,
            max_ctx_slots,
            src_len,
        ));
    }
    let hints = rng
        .chance(30)
        .then(|| (0..rng.below(16)).map(|_| rng.next() as u8).collect());
    let module_records =
        (code_kind == CodeKind::Module && rng.chance(80)).then(|| random_module_records(&mut rng));
    RandomUnit {
        source,
        opts,
        scopes,
        functions,
        hints,
        module_records,
    }
}

/// ABI 5: random well-formed module records (indices in range, keys sorted, cells not 0).
fn random_module_records(rng: &mut Rng) -> ModuleRecords<UnitString> {
    let cell = |rng: &mut Rng| {
        let c = 1 + rng.below(300) as i32;
        if rng.chance(50) { -c } else { c }
    };
    let requests: Vec<ModuleRequest<UnitString>> = (0..rng.below(4))
        .map(|_| {
            let mut attributes: Vec<(UnitString, UnitString)> = (0..rng.below(3))
                .map(|_| (random_string(rng), random_string(rng)))
                .collect();
            attributes.sort_by(|a, b| a.0.cmp_code_units(&b.0));
            attributes.dedup_by(|a, b| a.0 == b.0);
            ModuleRequest {
                specifier: random_string(rng),
                attributes,
            }
        })
        .collect();
    let n = requests.len() as u64;
    let mut m = ModuleRecords {
        requested_modules: requests,
        has_tla: rng.chance(30),
        ..ModuleRecords::default()
    };
    for _ in 0..rng.below(4) {
        m.local_export_entries.push(LocalExportEntry {
            export_name: random_string(rng),
            local_name: random_string(rng),
            cell: cell(rng),
        });
    }
    if n == 0 {
        return m;
    }
    for _ in 0..rng.below(4) {
        m.import_entries.push(ImportEntry {
            module_request: rng.below(n) as u32,
            import_name: if rng.chance(30) {
                ImportName::Namespace
            } else {
                ImportName::Name(random_string(rng))
            },
            local_name: random_string(rng),
            cell: cell(rng),
        });
    }
    for _ in 0..rng.below(3) {
        m.indirect_export_entries.push(IndirectExportEntry {
            export_name: random_string(rng),
            module_request: rng.below(n) as u32,
            import_name: if rng.chance(30) {
                ExportImportName::All
            } else {
                ExportImportName::Name(random_string(rng))
            },
        });
    }
    for _ in 0..rng.below(3) {
        m.star_export_entries.push(rng.below(n) as u32);
    }
    m
}

fn random_register(rng: &mut Rng, plan: &FnPlan) -> Register {
    match rng.below(10) {
        0 => Register::RECEIVER,
        1 | 2 if plan.params > 0 => Register::parameter(rng.below(u64::from(plan.params)) as u32),
        _ => Register::local(rng.below(u64::from(plan.regs)) as u32),
    }
}

fn random_list(rng: &mut Rng, plan: &FnPlan, len: u32) -> RegisterList {
    if len == 0 {
        return RegisterList::EMPTY;
    }
    let first = rng.below(u64::from(plan.regs - len + 1)) as u32;
    RegisterList::new(first, len)
}

fn random_value_constant(rng: &mut Rng, b: &mut FunctionBuilder) -> ConstIndex {
    match rng.below(5) {
        0 => b.number_constant(f64::from_bits(rng.next())),
        1 => b.number_constant(*rng.pick(&[0.0, -0.0, 1.5, f64::NAN, f64::INFINITY, 1e300])),
        2 => b.string_constant(random_string(rng)),
        3 => {
            let limbs: Vec<u64> = (0..rng.below(3)).map(|_| rng.next()).collect();
            b.bigint_constant(BigIntConst::new(rng.chance(50), &limbs))
        }
        _ => b.number_constant(rng.below(10) as f64),
    }
}

fn random_boilerplate_value(rng: &mut Rng, b: &mut FunctionBuilder) -> Option<ConstIndex> {
    match rng.below(4) {
        0 => None,
        1 => Some(b.oddball_constant(*rng.pick(&[
            Oddball::Undefined,
            Oddball::Null,
            Oddball::True,
            Oddball::False,
            Oddball::Hole,
        ]))),
        2 => {
            let inner = vec![Some(random_value_constant(rng, b))];
            Some(b.array_boilerplate(inner))
        }
        _ => Some(random_value_constant(rng, b)),
    }
}

#[allow(clippy::too_many_arguments)]
fn random_function(
    rng: &mut Rng,
    id: u32,
    plan: &FnPlan,
    children: &[u32],
    scopes: &[ScopeInput],
    max_ctx_slots: u32,
    src_len: u32,
) -> FunctionCode {
    let scope_sizes: Vec<u32> = scopes.iter().map(|s| s.3.len() as u32).collect();
    let scope_sizes = &scope_sizes[..];
    let nscopes = scope_sizes.len() as u32;
    let mut b = FunctionBuilder::new(FunctionMeta {
        kind: plan.kind,
        flags: plan.flags,
        name: rng.chance(50).then(|| random_string(rng)),
        parent: plan.parent.map(FunctionId),
        outer_scope: rng
            .chance(60)
            .then(|| ScopeId(rng.below(u64::from(nscopes)) as u32)),
        source_start: plan.start,
        source_end: plan.end,
        parameter_count: plan.params,
        length: rng.below(u64::from(plan.params) + 1) as u32,
    });
    let _regs = b.new_register_list(plan.regs);
    // ABI 5: sometimes a mapped-parameter list over a function scope with a context: the
    // function makes that context, then the mapped arguments object.
    if id != 0 && plan.params > 0 && rng.chance(25) {
        let fits: Vec<usize> = scopes
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.0 == ScopeKind::Function
                    && s.1.contains(ScopeFlags::NEEDS_CONTEXT)
                    && s.3.iter().any(|slot| slot.1 == VariableMode::Var)
            })
            .map(|(i, _)| i)
            .collect();
        if !fits.is_empty() {
            let si = *rng.pick(&fits);
            let mut free: Vec<u32> = (0..scopes[si].3.len() as u32)
                .filter(|&j| scopes[si].3[j as usize].1 == VariableMode::Var)
                .collect();
            let slots: Vec<Option<u32>> = (0..plan.params)
                .map(|_| {
                    (!free.is_empty() && rng.chance(70))
                        .then(|| free.swap_remove(rng.below(free.len() as u64) as usize))
                })
                .collect();
            let k = b.scope_constant(ScopeId(si as u32));
            b.create_function_context(k, scopes[si].3.len() as u32);
            b.create_mapped_arguments();
            b.set_mapped_parameters(ScopeId(si as u32), slots);
        }
    }
    // Sometimes push the constant pool past 255 entries, so Const operands need Wide.
    if rng.chance(10) {
        for i in 0..300 {
            b.number_constant(f64::from(i) + 0.5);
        }
    }
    let mut pending: Vec<Label> = Vec::new(); // forward labels to bind later
    let mut bound: Vec<Label> = Vec::new(); // labels bound so far (loop headers)
    let mut open_tries: Vec<(Label, Label, Label, u32)> = Vec::new();
    let candidates: Vec<Opcode> = OPCODES
        .iter()
        .map(|i| i.opcode)
        .filter(|op| {
            let f = op.info().flags;
            !f.contains(OpcodeFlags::RESERVED) && !f.contains(OpcodeFlags::PREFIX)
        })
        // H02 F10 (ABI 3): generator opcodes only in generator and async functions. The
        // generic path below gives SwitchOnGeneratorState a jump table with forward labels.
        .filter(|op| {
            !op.info().flags.contains(OpcodeFlags::GENERATOR)
                || plan.flags.contains(FunctionFlags::GENERATOR)
                || plan.flags.contains(FunctionFlags::ASYNC)
        })
        // CX-ABI1: CreateMappedArguments with formal parameters needs a mapped-parameter
        // list; only the list block above emits it there.
        .filter(|op| *op != Opcode::CreateMappedArguments || plan.params == 0)
        .filter(|op| {
            id != 0
                || !matches!(
                    op,
                    Opcode::CreateMappedArguments
                        | Opcode::CreateUnmappedArguments
                        | Opcode::CreateRestParameter
                        | Opcode::LdaNewTarget
                )
        })
        .collect();
    let max_insns = if rng.chance(10) { 400 } else { 40 };
    let count = 1 + rng.below(max_insns);
    for _ in 0..count {
        // Bind some pending forward labels here (they land on the next instruction).
        while !pending.is_empty() && rng.chance(30) {
            let l = pending.swap_remove(rng.below(pending.len() as u64) as usize);
            b.bind(l);
            bound.push(l);
        }
        if rng.chance(20) {
            let l = b.bound_label();
            bound.push(l);
        }
        // Try regions: open or close (stack order keeps them nested).
        if rng.chance(5) && plan.regs > 0 {
            let (s, e, t) = (b.new_label(), b.new_label(), b.new_label());
            b.bind(s);
            open_tries.push((s, e, t, b.instruction_count()));
        } else if rng.chance(5)
            && let Some(&(_, _, _, at)) = open_tries.last()
            && b.instruction_count() > at
        {
            let (s, e, t, _) = open_tries.pop().unwrap();
            b.bind(e);
            let ctx = Register::local(rng.below(u64::from(plan.regs)) as u32);
            let kind = *rng.pick(&[
                HandlerKind::Caught,
                HandlerKind::Uncaught,
                HandlerKind::Promise,
                HandlerKind::AsyncAwait,
            ]);
            b.add_handler(s, e, t, ctx, kind);
            pending.push(t);
        }
        if rng.chance(30) {
            let off = rng.below(u64::from(src_len) + 1) as u32;
            if rng.chance(50) {
                b.set_statement_position(off);
            } else {
                b.set_expression_position(off);
            }
        }
        let op = *rng.pick(&candidates);
        emit_random(
            rng,
            &mut b,
            op,
            plan,
            children,
            scope_sizes,
            max_ctx_slots,
            &mut pending,
            &bound,
        );
    }
    // Close open tries (each holds at least one instruction), bind every pending label, end.
    while let Some((s, e, t, at)) = open_tries.pop() {
        if b.instruction_count() == at {
            b.lda_zero();
        }
        b.bind(e);
        b.add_handler(s, e, t, Register::local(0), HandlerKind::Caught);
        pending.push(t);
    }
    for l in pending.drain(..) {
        b.bind(l);
    }
    match rng.below(3) {
        0 => {
            b.throw();
        }
        1 => {
            b.re_throw();
        }
        _ => {
            b.return_();
        }
    }
    b.finish()
        .unwrap_or_else(|e| panic!("random function {id} does not build: {e:?}"))
}

#[allow(clippy::too_many_arguments)]
fn emit_random(
    rng: &mut Rng,
    b: &mut FunctionBuilder,
    op: Opcode,
    plan: &FnPlan,
    children: &[u32],
    scope_sizes: &[u32],
    max_ctx_slots: u32,
    pending: &mut Vec<Label>,
    bound: &[Label],
) {
    let nscopes = scope_sizes.len() as u32;
    // Opcodes with cross-operand rules.
    match op {
        Opcode::CallRuntime | Opcode::CallRuntimeForPair => {
            // A function that returns a pair goes through CallRuntimeForPair only.
            let pair = op == Opcode::CallRuntimeForPair;
            let fit: Vec<_> = RUNTIME_FUNCTIONS
                .iter()
                .filter(|f| f.returns_pair == pair)
                .collect();
            let info = *rng.pick(&fit);
            let len = info
                .arity
                .unwrap_or_else(|| rng.below(u64::from(plan.regs.min(4)) + 1) as u32);
            if len > plan.regs || (pair && plan.regs < 2) {
                return;
            }
            let list = random_list(rng, plan, len);
            if pair {
                let out = random_list(rng, plan, 2);
                b.call_runtime_for_pair(info.id, list, len, out);
            } else {
                b.call_runtime(info.id, list, len);
            }
            return;
        }
        Opcode::CreateFunctionContext | Opcode::CreateEvalContext => {
            let s = rng.below(u64::from(nscopes)) as u32;
            let k = b.scope_constant(ScopeId(s));
            let slots = scope_sizes[s as usize];
            if op == Opcode::CreateFunctionContext {
                b.create_function_context(k, slots);
            } else {
                b.create_eval_context(k, slots);
            }
            return;
        }
        _ => {}
    }
    let info = op.info();
    // Skip opcodes whose operands this function cannot satisfy.
    for o in info.operands {
        let ok = match (o.kind, o.param) {
            (OperandKind::Const, OperandParam::Const(ConstOperand::Function)) => {
                !children.is_empty()
            }
            (OperandKind::JumpBack, _) => !bound.is_empty(),
            (OperandKind::RegPair | OperandKind::RegOutPair, _) => plan.regs >= 2,
            (OperandKind::RegOutTriple, _) => plan.regs >= 3,
            (OperandKind::CtxSlot, _) => max_ctx_slots > 0,
            _ => true,
        };
        if !ok {
            return;
        }
    }
    let mut ops: Vec<PendingOperand> = Vec::new();
    let mut k = 0;
    while k < info.operands.len() {
        let o = info.operands[k];
        match (o.kind, o.param) {
            (OperandKind::Reg | OperandKind::RegOut | OperandKind::RegInOut, _) => {
                ops.push(random_register(rng, plan).into_pending());
            }
            (OperandKind::RegList | OperandKind::RegOutList, _) => {
                let len = (rng.below(u64::from(plan.regs.min(4)) + 1) as u32).min(plan.regs);
                let list = random_list(rng, plan, len);
                ops.push(list.into_pending());
                ops.push(list.len().into_pending());
                k += 1; // the count goes with the list
            }
            (OperandKind::RegPair | OperandKind::RegOutPair, _) => {
                ops.push(random_list(rng, plan, 2).into_pending())
            }
            (OperandKind::RegOutTriple, _) => ops.push(random_list(rng, plan, 3).into_pending()),
            (OperandKind::Imm, _) => {
                let v = if op == Opcode::JumpLoop {
                    rng.below(4) as i32
                } else {
                    rng.i32_any()
                };
                ops.push(v.into_pending());
            }
            (OperandKind::UImm, _) => ops.push(rng.u32_any().into_pending()),
            (OperandKind::Const, OperandParam::Const(c)) => {
                let kc = match c {
                    ConstOperand::Value => random_value_constant(rng, b),
                    ConstOperand::String => b.string_constant(random_string(rng)),
                    ConstOperand::Function => b.function_constant(FunctionId(*rng.pick(children))),
                    ConstOperand::Scope => {
                        b.scope_constant(ScopeId(rng.below(u64::from(nscopes)) as u32))
                    }
                    ConstOperand::JumpTable => {
                        let (kc, labels) = b.jump_table(1 + rng.below(4) as u32);
                        pending.extend(labels);
                        kc
                    }
                    ConstOperand::ArrayBoilerplate => {
                        let e = (0..rng.below(4))
                            .map(|_| random_boilerplate_value(rng, b))
                            .collect();
                        b.array_boilerplate(e)
                    }
                    ConstOperand::ObjectBoilerplate => {
                        let p = (0..rng.below(4))
                            .map(|_| {
                                let key = if rng.chance(70) {
                                    b.string_constant(random_string(rng))
                                } else {
                                    b.number_constant(rng.below(100) as f64)
                                };
                                (key, random_boilerplate_value(rng, b))
                            })
                            .collect();
                        b.object_boilerplate(p)
                    }
                    ConstOperand::TemplateObject => {
                        let t = (0..1 + rng.below(3))
                            .map(|_| {
                                (
                                    random_string(rng),
                                    rng.chance(80).then(|| random_string(rng)),
                                )
                            })
                            .collect();
                        b.template_object(t)
                    }
                };
                ops.push(kc.into_pending());
            }
            (OperandKind::Slot, OperandParam::Slot(kind)) => {
                ops.push(b.feedback_slot(kind).into_pending());
            }
            (OperandKind::CtxSlot, _) => {
                ops.push((rng.below(u64::from(max_ctx_slots)) as u32).into_pending())
            }
            (OperandKind::CtxDepth, _) => {
                ops.push((rng.below(u64::from(nscopes) + 1) as u32).into_pending())
            }
            (OperandKind::JumpFwd, _) => {
                let l = b.new_label();
                pending.push(l);
                ops.push(l.into_pending());
            }
            (OperandKind::JumpBack, _) => ops.push(rng.pick(bound).into_pending()),
            (OperandKind::Flag8, OperandParam::FlagMask(m)) => {
                ops.push(((rng.next() as u16 & m) as u8).into_pending());
            }
            (OperandKind::Flag16, OperandParam::FlagMask(m)) => {
                ops.push((rng.next() as u16 & m).into_pending());
            }
            (OperandKind::TypeOfLiteral, _) => {
                ops.push(rng.pick(TypeOfLiteral::ALL).into_pending());
            }
            other => panic!("generator does not handle {other:?} in {op}"),
        }
        k += 1;
    }
    b.emit(op, &ops);
}
