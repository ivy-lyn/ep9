//! Verifier rules (gate "verifier"): each test breaks one rule of a valid unit and checks the
//! exact error.

mod common;

use Opcode as O;
use ep9_bytecode::*;

/// A valid two-function script: f0 creates f1; f1 has 2 params, 3 registers, a few constants
/// and slots. Tests change one field of it.
fn base() -> CompiledUnit {
    let src = "function f(a, b) { return a; }";
    let mut u = UnitBuilder::new(src, common::script_options(true));
    let scope = u.add_scope(
        ScopeKind::Function,
        ScopeFlags::NEEDS_CONTEXT,
        None,
        vec![
            (UnitString::new("a"), VariableMode::Var),
            (UnitString::new("b"), VariableMode::Let),
        ],
    );
    let mut f = FunctionBuilder::new(FunctionMeta {
        kind: FunctionKind::Normal,
        flags: FunctionFlags::default(),
        name: Some(UnitString::new("f")),
        parent: Some(FunctionId(0)),
        outer_scope: Some(scope),
        source_start: 0,
        source_end: src.len() as u32,
        parameter_count: 2,
        length: 2,
    });
    f.new_register_list(3);
    let k_name = f.string_constant("x"); // k0
    let _k_num = f.number_constant(2.5); // k1
    let _k_scope = f.scope_constant(scope); // k2
    let _s_load = f.feedback_slot(FeedbackSlotKind::LoadNamed); // s0
    let _s_bin = f.feedback_slot(FeedbackSlotKind::BinaryOp); // s1
    let _ = k_name;
    f.set_statement_position(19);
    f.ldar(f.parameter(0)).return_();
    let fcode = f.finish().unwrap();
    let mut b = FunctionBuilder::new(FunctionMeta::entry(
        CodeKind::Script,
        false,
        src.len() as u32,
    ));
    let k = b.function_constant(FunctionId(1));
    let s = b.feedback_slot(FeedbackSlotKind::CreateClosure);
    b.create_closure(k, s, 0).return_();
    u.set_function(FunctionId(0), b.finish().unwrap());
    u.set_function(FunctionId(1), fcode);
    u.finish().unwrap().into_inner()
}

/// Replaces f1's bytecode (and drops its positions, which point into the old code).
fn with_code(code: &[u8]) -> CompiledUnit {
    let mut u = base();
    u.functions[1].bytecode = code.to_vec();
    u.functions[1].positions.clear();
    u
}

fn err(u: CompiledUnit) -> VerifyErrorKind {
    verify(u).expect_err("unit must fail verification").kind
}

const RET: u8 = O::Return as u8;

#[test]
fn base_is_valid() {
    verify(base()).unwrap();
    verify(with_code(&[O::LdaZero as u8, RET])).unwrap();
}

// ---- Unit level ----

#[test]
fn unit_rules() {
    let mut u = base();
    u.functions.clear();
    assert_eq!(err(u), VerifyErrorKind::NoFunctions);

    let mut u = base();
    u.header.code_kind = CodeKind::Module;
    assert_eq!(err(u), VerifyErrorKind::Strictness);

    let mut u = base();
    u.source = Some("changed".into());
    assert_eq!(err(u), VerifyErrorKind::Source);

    let mut u = base();
    u.scopes[0].outer = Some(ScopeId(0));
    assert_eq!(err(u), VerifyErrorKind::ScopeId(0));

    let mut u = base();
    u.scopes[0].slots[0].name = StringId(99);
    assert_eq!(err(u), VerifyErrorKind::StringId(99));
}

#[test]
fn entry_frame_rules() {
    // Function 0 must have the unit's code kind.
    let mut u = base();
    u.functions[0].kind = FunctionKind::Eval;
    assert_eq!(err(u), VerifyErrorKind::EntryKind);
    // No other function may be an entry kind.
    let mut u = base();
    u.functions[1].kind = FunctionKind::Script;
    assert_eq!(err(u), VerifyErrorKind::EntryKind);
    // The entry frame has no parameters...
    let mut u = base();
    u.functions[0].parameter_count = 1;
    assert!(matches!(err(u), VerifyErrorKind::EntryFrame(_)));
    // ... no arguments object, rest parameter or new.target.
    for op in [
        O::CreateMappedArguments,
        O::CreateUnmappedArguments,
        O::CreateRestParameter,
        O::LdaNewTarget,
    ] {
        let mut u = base();
        u.functions[0].bytecode = vec![op as u8, RET];
        u.functions[0].positions.clear();
        assert!(matches!(err(u), VerifyErrorKind::EntryFrame(_)), "{op}");
    }
    // Strictness of function 0 follows the header.
    let mut u = base();
    u.functions[0].flags = FunctionFlags::STRICT;
    assert_eq!(err(u), VerifyErrorKind::Strictness);
}

#[test]
fn tree_rules() {
    let mut u = base();
    u.functions[0].parent = Some(FunctionId(0));
    assert_eq!(err(u), VerifyErrorKind::Tree);
    let mut u = base();
    u.functions[1].parent = None;
    assert_eq!(err(u), VerifyErrorKind::Tree);
    // f2's parent is f0, then f3 claims f2's sibling f1 as parent: not a preorder.
    let mut u = base();
    let mut f2 = u.functions[1].clone();
    f2.parent = Some(FunctionId(0));
    let mut f3 = f2.clone();
    f3.parent = Some(FunctionId(1));
    u.functions.push(f2);
    u.functions.push(f3);
    assert_eq!(err(u), VerifyErrorKind::Tree);
    // A strict parent cannot have a sloppy child.
    let mut u = base();
    u.functions[1].flags = FunctionFlags::STRICT;
    let mut child = u.functions[1].clone();
    child.flags = FunctionFlags::default();
    child.parent = Some(FunctionId(1));
    u.functions.push(child);
    assert_eq!(err(u), VerifyErrorKind::Strictness);
}

#[test]
fn function_metadata_rules() {
    let mut u = base();
    u.functions[1].name = Some(StringId(77));
    assert_eq!(err(u), VerifyErrorKind::StringId(77));
    let mut u = base();
    u.functions[1].outer_scope = Some(ScopeId(5));
    assert_eq!(err(u), VerifyErrorKind::ScopeId(5));
    let mut u = base();
    u.functions[1].source_end = u.header.source_len + 1;
    assert_eq!(err(u), VerifyErrorKind::SourceRange);
    let mut u = base();
    u.functions[1].kind = FunctionKind::Getter;
    u.functions[1].flags = FunctionFlags::GENERATOR;
    assert_eq!(err(u), VerifyErrorKind::FunctionFlags);
    let mut u = base();
    u.functions[1].length = 3;
    assert_eq!(err(u), VerifyErrorKind::Limits);
    let mut u = base();
    u.functions[1].register_count = MAX_REGISTERS + 1;
    assert_eq!(err(u), VerifyErrorKind::Limits);
    assert_eq!(err(with_code(&[])), VerifyErrorKind::EmptyBytecode);
}

// ---- Decoding and prefixes ----

#[test]
fn decoding_and_prefix_rules() {
    assert_eq!(
        err(with_code(&[0xff, RET])),
        VerifyErrorKind::Decode(DecodeError::UndefinedOpcode(0xff))
    );
    assert_eq!(
        err(with_code(&[O::LdaSmi as u8])),
        VerifyErrorKind::Decode(DecodeError::Truncated(O::LdaSmi))
    );
    assert_eq!(
        err(with_code(&[
            O::Wide as u8,
            O::Wide as u8,
            O::LdaZero as u8,
            RET
        ])),
        VerifyErrorKind::Decode(DecodeError::BadPrefix)
    );
    assert_eq!(
        err(with_code(&[O::ExtraWide as u8, RET])),
        VerifyErrorKind::Decode(DecodeError::PrefixWithoutScalableOperand(O::Return))
    );
    // Wide LdaSmi 5: 5 fits one byte, so the prefix is not minimal.
    assert_eq!(
        err(with_code(&[O::Wide as u8, O::LdaSmi as u8, 5, 0, RET])),
        VerifyErrorKind::NonMinimalScale
    );
    // Wide LdaSmi 300 is fine; ExtraWide LdaSmi 300 is not minimal.
    verify(with_code(&[
        O::Wide as u8,
        O::LdaSmi as u8,
        0x2c,
        0x01,
        RET,
    ]))
    .unwrap();
    assert_eq!(
        err(with_code(&[
            O::ExtraWide as u8,
            O::LdaSmi as u8,
            0x2c,
            0x01,
            0,
            0,
            RET
        ])),
        VerifyErrorKind::NonMinimalScale
    );
    // Illegal and reserved opcodes.
    assert_eq!(
        err(with_code(&[O::Illegal as u8, RET])),
        VerifyErrorKind::ReservedOpcode(O::Illegal)
    );
    assert_eq!(
        err(with_code(&[O::TailCall as u8, 0, 0, 0, 0])),
        VerifyErrorKind::ReservedOpcode(O::TailCall)
    );
    assert_eq!(
        err(with_code(&[O::ResumeGenerator as u8, 0, 0, 0, RET])),
        VerifyErrorKind::ReservedOpcode(O::ResumeGenerator)
    );
}

// ---- Operands ----

#[test]
fn register_rules() {
    // f1 has r0..r2, a0..a1 and <this>.
    let reg = |v: i8| v as u8;
    verify(with_code(&[O::Ldar as u8, reg(2), RET])).unwrap();
    verify(with_code(&[O::Ldar as u8, reg(-1), RET])).unwrap(); // <this>
    verify(with_code(&[O::Ldar as u8, reg(-3), RET])).unwrap(); // a1
    assert_eq!(
        err(with_code(&[O::Ldar as u8, reg(3), RET])),
        VerifyErrorKind::Register(0)
    );
    assert_eq!(
        err(with_code(&[O::Ldar as u8, reg(-4), RET])),
        VerifyErrorKind::Register(0)
    );
    assert_eq!(
        err(with_code(&[O::Star as u8, reg(9), RET])),
        VerifyErrorKind::Register(0)
    );
    // CallUndefinedReceiver r0, r1-r2 (count 2), s? -> needs a Call slot; use CallRuntime.
    let call = |first: i8, count: u8| {
        with_code(&[
            O::CallRuntime as u8,
            RuntimeId::CreateIterResultObject as u8,
            0,
            first as u8,
            count,
            RET,
        ])
    };
    verify(call(1, 2)).unwrap();
    assert_eq!(err(call(2, 2)), VerifyErrorKind::Register(1)); // r2-r3: r3 is outside
    assert_eq!(err(call(-2, 2)), VerifyErrorKind::Register(1)); // lists hold locals only
    // An empty list must be r0.
    let empty = |first: u8| {
        with_code(&[
            O::CallRuntime as u8,
            RuntimeId::DeclareGlobals as u8,
            0,
            first,
            0,
            RET,
        ])
    };
    verify(empty(0)).unwrap();
    assert_eq!(err(empty(1)), VerifyErrorKind::Register(1));
    // Pairs and triples: consecutive locals inside the frame.
    let mut u = with_code(&[O::ForInStep as u8, 0, RET]);
    u.functions[1].register_count = 1;
    verify(u).unwrap();
    let mut u = base();
    u.functions[1].feedback_slots.push(FeedbackSlotKind::ForIn); // s2
    u.functions[1].bytecode = vec![O::ForInPrepare as u8, 1, 2, RET]; // r1-r3
    u.functions[1].positions.clear();
    assert_eq!(err(u), VerifyErrorKind::Register(0));
}

#[test]
fn constant_and_slot_rules() {
    // GetNamedProperty r0, k0 ("x"), s0 (LoadNamed) is valid.
    verify(with_code(&[O::GetNamedProperty as u8, 0, 0, 0, RET])).unwrap();
    // k1 is a Number: not a String operand.
    assert_eq!(
        err(with_code(&[O::GetNamedProperty as u8, 0, 1, 0, RET])),
        VerifyErrorKind::ConstantOperand(1)
    );
    // k9 is out of range.
    assert_eq!(
        err(with_code(&[O::GetNamedProperty as u8, 0, 9, 0, RET])),
        VerifyErrorKind::ConstantOperand(1)
    );
    // s1 is a BinaryOp slot, not LoadNamed; s5 is out of range.
    assert_eq!(
        err(with_code(&[O::GetNamedProperty as u8, 0, 0, 1, RET])),
        VerifyErrorKind::FeedbackSlot(2)
    );
    assert_eq!(
        err(with_code(&[O::GetNamedProperty as u8, 0, 0, 5, RET])),
        VerifyErrorKind::FeedbackSlot(2)
    );
    // LdaConstant accepts Number, String, BigInt but not a Scope (k2).
    verify(with_code(&[O::LdaConstant as u8, 1, RET])).unwrap();
    assert_eq!(
        err(with_code(&[O::LdaConstant as u8, 2, RET])),
        VerifyErrorKind::ConstantOperand(0)
    );
}

#[test]
fn flag_runtime_and_typeof_rules() {
    // CreateObjectLiteral flags mask is 0x03.
    let mut u = base();
    u.functions[1]
        .constants
        .push(Constant::ObjectBoilerplate(Box::new([]))); // k3
    u.functions[1]
        .feedback_slots
        .push(FeedbackSlotKind::Literal); // s2
    u.functions[1].bytecode = vec![O::CreateObjectLiteral as u8, 3, 2, 0x03, RET];
    u.functions[1].positions.clear();
    verify(u.clone()).unwrap();
    u.functions[1].bytecode[3] = 0x04;
    assert_eq!(err(u), VerifyErrorKind::FlagValue(2));
    // TestTypeOf literal must be defined.
    verify(with_code(&[O::TestTypeOf as u8, 8, RET])).unwrap();
    assert_eq!(
        err(with_code(&[O::TestTypeOf as u8, 9, RET])),
        VerifyErrorKind::FlagValue(0)
    );
    // Runtime id must be defined, and the arity must match.
    assert_eq!(
        err(with_code(&[O::CallRuntime as u8, 200, 0, 0, 0, RET])),
        VerifyErrorKind::FlagValue(0)
    );
    assert_eq!(
        err(with_code(&[
            O::CallRuntime as u8,
            RuntimeId::CreateIterResultObject as u8,
            0,
            0,
            1,
            RET
        ])),
        VerifyErrorKind::RuntimeArity
    );
}

#[test]
fn jump_rules() {
    // 0: Jump +3 -> 3 (LdaZero); 2: LdaZero is skipped... use real layouts:
    // 0: Jump +2 -> 2: Return
    verify(with_code(&[O::Jump as u8, 2, RET])).unwrap();
    // Offset 0 (a jump to itself) is not a forward jump.
    assert_eq!(
        err(with_code(&[O::Jump as u8, 0, RET])),
        VerifyErrorKind::Jump(0)
    );
    // Into the middle of an instruction: 0: Jump +3 -> 3 is the operand of LdaSmi at 2.
    assert_eq!(
        err(with_code(&[O::Jump as u8, 3, O::LdaSmi as u8, 7, RET])),
        VerifyErrorKind::Jump(0)
    );
    // Past the end.
    assert_eq!(
        err(with_code(&[O::Jump as u8, 9, RET])),
        VerifyErrorKind::Jump(0)
    );
    // JumpLoop goes backward (or to itself), never forward.
    let mut u = base();
    u.functions[1]
        .feedback_slots
        .push(FeedbackSlotKind::JumpLoop); // s2
    u.functions[1].bytecode = vec![O::LdaZero as u8, O::JumpLoop as u8, 1, 0, 2];
    u.functions[1].positions.clear();
    verify(u.clone()).unwrap();
    u.functions[1].bytecode[2] = 0; // JumpLoop to itself: allowed
    verify(u.clone()).unwrap();
    u.functions[1].bytecode[2] = 2; // to offset -1
    assert_eq!(err(u.clone()), VerifyErrorKind::Jump(0));
    u.functions[1].bytecode[2] = 1;
    u.functions[1].bytecode[3] = 0xff; // loop depth -1
    assert_eq!(err(u), VerifyErrorKind::FlagValue(1));
}

#[test]
fn jump_table_rules() {
    // 0: SwitchOnSmi k3, [0]; 3: LdaZero; 4: Return
    let mk = |entries: &[u32]| {
        let mut u = base();
        u.functions[1]
            .constants
            .push(Constant::JumpTable(entries.into())); // k3
        u.functions[1].bytecode = vec![O::SwitchOnSmi as u8, 3, 0, O::LdaZero as u8, RET];
        u.functions[1].positions.clear();
        u
    };
    verify(mk(&[3, 4])).unwrap();
    assert_eq!(err(mk(&[0])), VerifyErrorKind::JumpTable(0));
    assert_eq!(err(mk(&[3, 1])), VerifyErrorKind::JumpTable(1));
    assert_eq!(err(mk(&[5])), VerifyErrorKind::JumpTable(0));
    assert!(matches!(err(mk(&[])), VerifyErrorKind::Constant(3, _)));
}

#[test]
fn context_rules() {
    // CreateFunctionContext k2 (scope0 with 2 slots), [2].
    verify(with_code(&[O::CreateFunctionContext as u8, 2, 2, RET])).unwrap();
    assert_eq!(
        err(with_code(&[O::CreateFunctionContext as u8, 2, 3, RET])),
        VerifyErrorKind::Context(1)
    );
    // Context slot must be below the largest scope (2 slots).
    verify(with_code(&[O::LdaCurrentContextSlot as u8, 1, RET])).unwrap();
    assert_eq!(
        err(with_code(&[O::LdaCurrentContextSlot as u8, 2, RET])),
        VerifyErrorKind::Context(0)
    );
}

#[test]
fn terminator_rules() {
    assert_eq!(
        err(with_code(&[O::LdaZero as u8])),
        VerifyErrorKind::FallsOffEnd
    );
    // A conditional jump can fall through.
    assert_eq!(
        err(with_code(&[O::LdaTrue as u8, O::JumpIfTrue as u8, 2])),
        VerifyErrorKind::FallsOffEnd
    );
    assert_eq!(
        err(with_code(&[
            O::LdaTrue as u8,
            O::JumpIfTrue as u8,
            3,
            RET,
            O::LdaZero as u8
        ])),
        VerifyErrorKind::FallsOffEnd
    );
    for t in [O::Throw, O::ReThrow, O::Return] {
        verify(with_code(&[O::LdaZero as u8, t as u8])).unwrap();
    }
}

#[test]
fn constant_pool_rules() {
    // A Function constant must name a direct child.
    let mut u = base();
    u.functions[1]
        .constants
        .push(Constant::Function(FunctionId(0)));
    assert!(matches!(err(u), VerifyErrorKind::Constant(3, _)));
    // Boilerplate references must point to earlier entries.
    let mut u = base();
    u.functions[1]
        .constants
        .push(Constant::ArrayBoilerplate(Box::new([Some(ConstIndex(3))])));
    assert!(matches!(err(u), VerifyErrorKind::Constant(3, _)));
    // Object keys are Strings or Numbers; k2 is a Scope.
    let mut u = base();
    u.functions[1]
        .constants
        .push(Constant::ObjectBoilerplate(Box::new([(
            ConstIndex(2),
            None,
        )])));
    assert!(matches!(err(u), VerifyErrorKind::Constant(3, _)));
    // Template strings must exist.
    let mut u = base();
    u.functions[1]
        .constants
        .push(Constant::TemplateObject(Box::new([(StringId(50), None)])));
    assert!(matches!(err(u), VerifyErrorKind::Constant(3, _)));
    // A BigInt must be canonical.
    let mut u = base();
    u.functions[1].constants.push(Constant::BigInt(BigIntConst {
        negative: true,
        limbs: Box::new([]),
    }));
    assert!(matches!(err(u), VerifyErrorKind::Constant(3, _)));
}

#[test]
fn handler_rules() {
    // f1 code: 0 LdaZero, 1 LdaZero, 2 Return, 3 Return.
    let mk = |hs: &[Handler]| {
        let mut u = with_code(&[O::LdaZero as u8, O::LdaZero as u8, RET, RET]);
        u.functions[1].handlers = hs.to_vec();
        u
    };
    let h = |start, end, target| Handler {
        start,
        end,
        target,
        context: Register::local(0),
        kind: HandlerKind::Caught,
    };
    verify(mk(&[h(0, 2, 3)])).unwrap();
    verify(mk(&[h(0, 3, 3), h(1, 2, 3)])).unwrap(); // nested, outer first
    assert!(matches!(
        err(mk(&[h(1, 1, 3)])),
        VerifyErrorKind::Handler(0, "range")
    ));
    assert!(matches!(
        err(mk(&[h(0, 5, 3)])),
        VerifyErrorKind::Handler(0, "range")
    ));
    assert!(matches!(
        err(mk(&[h(0, 2, 4)])),
        VerifyErrorKind::Handler(0, "target")
    ));
    assert!(matches!(
        err(mk(&[h(0, 2, 1)])),
        VerifyErrorKind::Handler(0, "target inside its own range")
    ));
    assert!(matches!(
        err(mk(&[h(1, 2, 3), h(0, 3, 3)])),
        VerifyErrorKind::Handler(1, "order")
    ));
    assert!(matches!(
        err(mk(&[h(0, 2, 3), h(1, 3, 3)])),
        VerifyErrorKind::Handler(1, "partial overlap")
    ));
    let mut bad = h(0, 2, 3);
    bad.context = Register::parameter(0);
    assert!(matches!(
        err(mk(&[bad])),
        VerifyErrorKind::Handler(0, "context register")
    ));
    // A range end inside an instruction.
    let mut u = with_code(&[O::LdaSmi as u8, 1, RET, RET]);
    u.functions[1].handlers = vec![h(0, 1, 3)];
    assert!(matches!(err(u), VerifyErrorKind::Handler(0, "range")));
}

#[test]
fn position_rules() {
    let mk = |ps: &[(u32, u32)]| {
        let mut u = with_code(&[O::LdaSmi as u8, 1, RET]);
        u.functions[1].positions = ps
            .iter()
            .map(|&(b, s)| PositionEntry {
                bytecode_offset: b,
                source_offset: s,
                is_statement: true,
            })
            .collect();
        u
    };
    verify(mk(&[(0, 0), (2, 5)])).unwrap();
    assert!(matches!(
        err(mk(&[(2, 0), (0, 0)])),
        VerifyErrorKind::Position(1, _)
    ));
    assert!(matches!(
        err(mk(&[(0, 0), (0, 1)])),
        VerifyErrorKind::Position(1, _)
    ));
    assert!(matches!(
        err(mk(&[(1, 0)])),
        VerifyErrorKind::Position(0, _)
    ));
    assert!(matches!(
        err(mk(&[(0, 999)])),
        VerifyErrorKind::Position(0, _)
    ));
}

#[test]
fn errors_name_function_and_offset() {
    let e = verify(with_code(&[O::LdaZero as u8, O::Ldar as u8, 9, RET])).unwrap_err();
    assert_eq!(e.function, Some(1));
    assert_eq!(e.offset, Some(1));
    assert_eq!(e.to_string(), "f1: @1: Register(0)");
}
