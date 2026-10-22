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
}

/// ABI 3 (H02 F10): the generator opcodes are valid only in generator and async functions.
#[test]
fn generator_opcode_rules() {
    // f1 is a plain function: SuspendGenerator r0, (), [0] is rejected.
    let suspend = [O::SuspendGenerator as u8, 0, 0, 0, 0];
    let resume = [O::ResumeGenerator as u8, 0, 0, 0, RET];
    assert_eq!(
        err(with_code(&suspend)),
        VerifyErrorKind::GeneratorOpcode(O::SuspendGenerator)
    );
    assert_eq!(
        err(with_code(&resume)),
        VerifyErrorKind::GeneratorOpcode(O::ResumeGenerator)
    );
    // The same code verifies in a generator and in an async function.
    for flag in [FunctionFlags::GENERATOR, FunctionFlags::ASYNC] {
        for code in [&suspend[..], &resume[..]] {
            let mut u = with_code(code);
            u.functions[1].flags = u.functions[1].flags.with(flag);
            verify(u).unwrap();
        }
    }
    // SwitchOnGeneratorState r0, k3 (a jump table to the Return at 3).
    let mut u = with_code(&[O::SwitchOnGeneratorState as u8, 0, 3, RET]);
    u.functions[1]
        .constants
        .push(Constant::JumpTable(Box::new([3]))); // k3
    assert_eq!(
        err(u.clone()),
        VerifyErrorKind::GeneratorOpcode(O::SwitchOnGeneratorState)
    );
    u.functions[1].flags = u.functions[1].flags.with(FunctionFlags::GENERATOR);
    verify(u).unwrap();
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
    // ABI 3: the RegOutPair of CallRuntimeForPair is two locals inside the frame. f1 has 3
    // registers, so r1-r2 is valid and r2-r3 (r(register_count - 1)) is not; nor is a
    // parameter.
    let out = |first: i8| {
        with_code(&[
            O::CallRuntimeForPair as u8,
            RuntimeId::LoadLookupSlotForCall as u8,
            0,
            0,
            2,
            first as u8,
            RET,
        ])
    };
    verify(out(1)).unwrap();
    assert_eq!(err(out(2)), VerifyErrorKind::Register(3));
    assert_eq!(err(out(-2)), VerifyErrorKind::Register(3));
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
    // ABI 3: DefineKeyedOwnProperty r0, r1, flags, s2 accepts bit 1 (private add); the
    // object literal form does not.
    let mut u = base();
    u.functions[1]
        .feedback_slots
        .push(FeedbackSlotKind::DefineKeyedOwn); // s2
    u.functions[1].positions.clear();
    let define = |u: &CompiledUnit, op: Opcode, flags: u8| {
        let mut u = u.clone();
        u.functions[1].bytecode = vec![op as u8, 0, 1, flags, 2, RET];
        u
    };
    for flags in [0x00, 0x01, 0x02, 0x03] {
        verify(define(&u, O::DefineKeyedOwnProperty, flags)).unwrap();
    }
    assert_eq!(
        err(define(&u, O::DefineKeyedOwnProperty, 0x04)),
        VerifyErrorKind::FlagValue(2)
    );
    verify(define(&u, O::DefineKeyedOwnPropertyInLiteral, 0x01)).unwrap();
    assert_eq!(
        err(define(&u, O::DefineKeyedOwnPropertyInLiteral, 0x02)),
        VerifyErrorKind::FlagValue(2)
    );
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
    // ABI 3: a function that returns a pair only through CallRuntimeForPair, and the others
    // only through CallRuntime. CallRuntimeForPair id, r0 (argc arguments), out r1-r2.
    // LoadLookupSlotForCall takes 2 arguments from ABI 4 (name, strict).
    let lookup = RuntimeId::LoadLookupSlotForCall as u8;
    let pair = |id: u8, argc: u8| with_code(&[O::CallRuntimeForPair as u8, id, 0, 0, argc, 1, RET]);
    verify(pair(lookup, 2)).unwrap();
    assert_eq!(err(pair(lookup, 1)), VerifyErrorKind::RuntimeArity);
    assert_eq!(
        err(pair(RuntimeId::GetHomeObject as u8, 1)),
        VerifyErrorKind::RuntimePair
    );
    assert_eq!(
        err(with_code(&[O::CallRuntime as u8, lookup, 0, 0, 2, RET])),
        VerifyErrorKind::RuntimePair
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
    verify(mk(&[h(0, 2, 2)])).unwrap(); // target at the range end
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
    for t in [0, 1] {
        assert_eq!(
            err(mk(&[h(1, 2, t)])),
            VerifyErrorKind::Handler(0, "target before the range end"),
            "target {t}"
        );
    }
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

/// A handler target before its range end is a back edge without an interrupt poll: `Throw`
/// at 4 went to 0 and back to `Throw` forever, and `TerminateHandle::terminate` never
/// stopped it. Such a unit must not load.
#[test]
fn handler_back_edge_does_not_load() {
    // f1: 0 LdaCurrentContext, 1 Star r0, 3 LdaZero, 4 Throw, 5 Return; handler [3, 5) -> t.
    let mk = |target| {
        let mut u = with_code(&[
            O::LdaCurrentContext as u8,
            O::Star as u8,
            0,
            O::LdaZero as u8,
            O::Throw as u8,
            RET,
        ]);
        u.functions[1].handlers = vec![Handler {
            start: 3,
            end: 5,
            target,
            context: Register::local(0),
            kind: HandlerKind::Caught,
        }];
        u
    };
    CompiledUnit::from_bytes(&mk(5).to_bytes()).unwrap();
    for target in [0, 1] {
        let e = CompiledUnit::from_bytes(&mk(target).to_bytes()).unwrap_err();
        assert_eq!(
            e,
            LoadError::Verify(VerifyError {
                function: Some(1),
                offset: None,
                kind: VerifyErrorKind::Handler(0, "target before the range end"),
            }),
            "target {target}"
        );
    }
}

/// `UnitBuilder::finish` output always loads: the verifier enforces the reader's rules for
/// strings and flag bits.
#[test]
fn builder_output_meets_the_reader_rules() {
    let build = |scope: ScopeFlags, func: FunctionFlags, s: UnitString| {
        let mut u = UnitBuilder::new("x", common::script_options(true));
        u.add_scope(ScopeKind::Block, scope, None, Vec::new());
        let mut meta = FunctionMeta::entry(CodeKind::Script, false, 1);
        meta.flags = func;
        let mut b = FunctionBuilder::new(meta);
        let k = b.string_constant(s);
        b.lda_constant(k).return_();
        u.set_function(FunctionId(0), b.finish().unwrap());
        u.finish()
    };
    // Every scope bit a block scope may have (PARAMETER_ENV is for a sloppy function scope
    // only, ABI 5; `abi5_parameter_env_rules` covers it).
    let (sf, ff) = (
        ScopeFlags(ScopeFlags::ALL & !ScopeFlags::PARAMETER_ENV.0),
        FunctionFlags::default(),
    );
    for s in [UnitString::new("a"), UnitString::new("\u{20ac}")] {
        let v = build(sf, ff, s).unwrap();
        assert_eq!(*CompiledUnit::from_bytes(&v.to_bytes()).unwrap(), *v);
    }
    let cases = [
        (
            (sf, ff, UnitString::Utf16(Box::new([0x61]))),
            VerifyErrorKind::NonCanonicalString(0),
        ),
        (
            (ScopeFlags(0x80), ff, UnitString::new("a")),
            VerifyErrorKind::ScopeFlags(0),
        ),
        (
            (sf, FunctionFlags(0x80), UnitString::new("a")),
            VerifyErrorKind::FunctionFlags,
        ),
    ];
    for ((scope, func, s), want) in cases {
        let Err(BuildError::Verify(e)) = build(scope, func, s) else {
            panic!("{want:?}: the builder accepted the unit");
        };
        assert_eq!(e.kind, want);
    }
    // The same units, written without the builder: the reader rejects their bytes too.
    let good = build(sf, ff, UnitString::new("a")).unwrap().into_inner();
    let edits: [fn(&mut CompiledUnit); 3] = [
        |u| u.strings[0] = UnitString::Utf16(Box::new([0x61])),
        |u| u.scopes[0].flags = ScopeFlags(0x80),
        |u| u.functions[0].flags = FunctionFlags(0x80),
    ];
    for edit in edits {
        let mut u = good.clone();
        edit(&mut u);
        assert!(verify_ref(&u).is_err());
        assert!(matches!(
            CompiledUnit::from_bytes(&u.to_bytes()),
            Err(LoadError::Malformed { .. })
        ));
    }
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

// ---- ABI 5 ----

/// L4-A3: `PARAMETER_ENV` (bit 3) loads on a sloppy function scope with a sloppy eval; bit 4
/// is undefined; the bit needs the eval bit, a function scope and sloppy code.
#[test]
fn abi5_parameter_env_rules() {
    let pe = ScopeFlags::NEEDS_CONTEXT | ScopeFlags::CALLS_SLOPPY_EVAL | ScopeFlags::PARAMETER_ENV;
    let mut u = base();
    u.scopes[0].flags = pe;
    let v = verify(u.clone()).unwrap();
    assert_eq!(*CompiledUnit::from_bytes(&v.to_bytes()).unwrap(), *v);
    assert!(disassemble(&v).contains("flags=[needs-context,sloppy-eval,parameter-env]"));
    // Bit 4 is not defined: the verifier and the reader reject it.
    let mut bad = u.clone();
    bad.scopes[0].flags = ScopeFlags(pe.0 | 0x10);
    assert_eq!(err(bad.clone()), VerifyErrorKind::ScopeFlags(0));
    assert!(matches!(
        CompiledUnit::from_bytes(&bad.to_bytes()),
        Err(LoadError::Malformed {
            section: Section::Scopes,
            ..
        })
    ));
    // Without the eval bit, in strict code, or on another scope kind.
    for (kind, flags) in [
        (
            ScopeKind::Function,
            ScopeFlags::NEEDS_CONTEXT | ScopeFlags::PARAMETER_ENV,
        ),
        (ScopeKind::Function, pe | ScopeFlags::STRICT),
        (ScopeKind::Block, pe),
        (ScopeKind::Eval, pe),
    ] {
        let mut u = base();
        u.scopes[0].kind = kind;
        u.scopes[0].flags = flags;
        assert_eq!(err(u), VerifyErrorKind::ScopeFlags(0), "{kind:?} {flags:?}");
    }
}

/// L4-A3: the arity of the ABI 5 ids (id 44 `SetFunctionName` takes 2 arguments, id 45
/// `ThrowTypeError` 1, id 47 `StoreEvalAnnexBFunction` 2); each is a single-result function.
#[test]
fn abi5_runtime_arity() {
    let call =
        |id: RuntimeId, argc: u8| with_code(&[O::CallRuntime as u8, id as u8, 0, 0, argc, RET]);
    verify(call(RuntimeId::SetFunctionName, 2)).unwrap();
    assert_eq!(
        err(call(RuntimeId::SetFunctionName, 1)),
        VerifyErrorKind::RuntimeArity
    );
    verify(call(RuntimeId::ThrowTypeError, 1)).unwrap();
    assert_eq!(
        err(call(RuntimeId::ThrowTypeError, 0)),
        VerifyErrorKind::RuntimeArity
    );
    verify(call(RuntimeId::GetImportMeta, 0)).unwrap();
    verify(call(RuntimeId::AsyncGeneratorResolve, 3)).unwrap();
    verify(call(RuntimeId::StoreEvalAnnexBFunction, 2)).unwrap();
    assert_eq!(
        err(call(RuntimeId::StoreEvalAnnexBFunction, 1)),
        VerifyErrorKind::RuntimeArity
    );
    // 48 ids, dense; 48 is not defined.
    assert_eq!(RUNTIME_FUNCTIONS.len(), 48);
    assert!(
        RUNTIME_FUNCTIONS
            .iter()
            .all(|f| !f.returns_pair || f.id == RuntimeId::LoadLookupSlotForCall)
    );
    assert_eq!(
        err(with_code(&[O::CallRuntime as u8, 48, 0, 0, 0, RET])),
        VerifyErrorKind::FlagValue(0)
    );
}

/// f1 of `base()` with a mapped arguments object: CreateFunctionContext k2 (scope0: `a` var,
/// `b` let), [2]; CreateMappedArguments; Return. Its list maps `a` (slot 0) and not `b`.
fn mapped() -> CompiledUnit {
    let mut u = with_code(&[
        O::CreateFunctionContext as u8,
        2,
        2,
        O::CreateMappedArguments as u8,
        RET,
    ]);
    u.functions[1].mapped_parameters = Some(MappedParameters {
        scope: ScopeId(0),
        slots: vec![Some(0), None],
    });
    u
}

/// L4-A11: the mapped-parameter list (P0-D16).
#[test]
fn abi5_mapped_parameter_rules() {
    let v = verify(mapped()).unwrap();
    assert_eq!(*CompiledUnit::from_bytes(&v.to_bytes()).unwrap(), *v);
    assert!(disassemble(&v).contains("mapped parameters (2): scope=scope0 slots=[0, -]"));
    let bad = |what: &str, edit: &dyn Fn(&mut CompiledUnit)| {
        let mut u = mapped();
        edit(&mut u);
        match err(u) {
            VerifyErrorKind::MappedParameters(w) => assert_eq!(w, what),
            other => panic!("{what}: {other:?}"),
        }
    };
    let list = |u: &mut CompiledUnit| u.functions[1].mapped_parameters.as_mut().unwrap().clone();
    let set =
        |u: &mut CompiledUnit, m: MappedParameters| u.functions[1].mapped_parameters = Some(m);
    // A slot out of range of the scope (scope0 has 2 slots).
    bad("slot out of range", &|u| {
        let mut m = list(u);
        m.slots[0] = Some(2);
        set(u, m);
    });
    // A slot that holds no var (`b` is a let).
    bad("slot is not a var binding", &|u| {
        let mut m = list(u);
        m.slots = vec![Some(0), Some(1)];
        set(u, m);
    });
    // Two parameters on one slot.
    bad("slot twice", &|u| {
        let mut m = list(u);
        m.slots = vec![Some(0), Some(0)];
        set(u, m);
    });
    // One entry per formal parameter (f1 has 2), never empty.
    bad("length differs from the parameter count", &|u| {
        let mut m = list(u);
        m.slots.pop();
        set(u, m);
    });
    bad("length differs from the parameter count", &|u| {
        let mut m = list(u);
        m.slots.clear();
        set(u, m);
    });
    // A non-empty list on a function with no CreateMappedArguments.
    bad("list in a function without CreateMappedArguments", &|u| {
        u.functions[1].bytecode = vec![O::CreateFunctionContext as u8, 2, 2, RET];
    });
    // The function must make a context of the list's scope.
    bad("the function makes no context of the scope", &|u| {
        u.functions[1].bytecode = vec![O::CreateMappedArguments as u8, RET];
    });
    // The scope must be a function scope with a context.
    bad("scope is not a function scope with a context", &|u| {
        u.scopes[0].kind = ScopeKind::Block;
    });
    bad("scope is not a function scope with a context", &|u| {
        u.scopes[0].flags = ScopeFlags::default();
    });
    bad("scope id", &|u| {
        let mut m = list(u);
        m.scope = ScopeId(7);
        set(u, m);
    });
    // The reader: a list without a scope, or a scope with an empty list, does not load.
    let mut u = mapped();
    u.functions[1]
        .mapped_parameters
        .as_mut()
        .unwrap()
        .slots
        .clear();
    assert!(matches!(
        CompiledUnit::from_bytes(&u.to_bytes()),
        Err(LoadError::Malformed {
            section: Section::Functions,
            what: "empty mapped-parameter list"
        })
    ));
}

fn module_unit() -> CompiledUnit {
    common::sample_module_unit().into_inner()
}

/// L4-A3: module records (section 5): present exactly in a module unit, indices in range.
#[test]
fn abi5_module_record_rules() {
    let u = module_unit();
    verify(u.clone()).unwrap();
    // A module unit without records.
    let mut m = u.clone();
    m.module_records = None;
    assert!(matches!(err(m), VerifyErrorKind::ModuleRecords(_)));
    // A script unit with a non-empty section 5: the verifier and the loader reject it.
    let mut s = base();
    s.module_records = Some(ModuleRecords::default());
    assert_eq!(
        err(s.clone()),
        VerifyErrorKind::ModuleRecords("module records in a script or eval unit")
    );
    assert!(matches!(
        CompiledUnit::from_bytes(&s.to_bytes()),
        Err(LoadError::Verify(VerifyError {
            kind: VerifyErrorKind::ModuleRecords(_),
            ..
        }))
    ));
    let bad = |what: &str, edit: &dyn Fn(&mut ModuleRecords)| {
        let mut m = module_unit();
        edit(m.module_records.as_mut().unwrap());
        match err(m) {
            VerifyErrorKind::ModuleRecords(w) => assert_eq!(w, what),
            other => panic!("{what}: {other:?}"),
        }
    };
    // Request indices out of range (2 requests).
    bad("request index", &|m| m.import_entries[0].module_request = 2);
    bad("request index", &|m| {
        m.indirect_export_entries[1].module_request = 9
    });
    bad("request index", &|m| m.star_export_entries[0] = 2);
    // String ids out of range.
    bad("string id", &|m| {
        m.requested_modules[0].specifier = StringId(999)
    });
    bad("string id", &|m| {
        m.local_export_entries[0].local_name = StringId(999)
    });
    bad("string id", &|m| {
        m.import_entries[0].import_name = ImportName::Name(StringId(999))
    });
    // Module cell 0 means "no cell".
    bad("module cell 0", &|m| m.import_entries[1].cell = 0);
    bad("module cell 0", &|m| m.local_export_entries[1].cell = 0);
    // Attribute keys sorted and unique.
    bad("attribute keys not sorted or not unique", &|m| {
        let a = m.requested_modules[1].attributes[0];
        m.requested_modules[1].attributes.push(a);
    });
    // Non-zero module flag bits do not load.
    let v = verify(module_unit()).unwrap();
    let bytes = v.to_bytes();
    let n = u32::from_le_bytes(
        bytes[header_offsets::SECTIONS + 40..][..4]
            .try_into()
            .unwrap(),
    ) as usize;
    let mut m = bytes.clone();
    m[n] = 0x02;
    let h = compute_content_hash(&m);
    m[header_offsets::CONTENT_HASH..header_offsets::CONTENT_HASH + 32].copy_from_slice(&h);
    assert_eq!(
        CompiledUnit::from_bytes(&m).unwrap_err(),
        LoadError::Malformed {
            section: Section::ModuleRecords,
            what: "module flags"
        }
    );
}
