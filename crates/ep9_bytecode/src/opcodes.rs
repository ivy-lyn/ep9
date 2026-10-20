//! The opcode list (H2), written once in [`for_each_opcode!`], and the tables made from it.
//!
//! The list follows V8 Ignition (`src/interpreter/bytecodes.h`, v8/v8 commit 0c3940db, read
//! 2026-10-20): register + accumulator machine, the same opcode names and operand order for
//! the opcodes ep9 shares with V8. ADR H02 lists what ep9 changes: feedback-slot operands on
//! arithmetic and compares (V8 now embeds that feedback), no `*Constant` jump variants (the
//! builder relaxes jump sizes instead), forward-only jumps except `JumpLoop`, a reserved
//! `TailCall` (D7), and reserved generator opcodes.

/// Calls `$m!` with the full opcode list. Tokens in `[...]` are passed through first.
///
/// This is the single source for the encoder, decoder, verifier, disassembler, the builder's
/// typed emitters and the `ep9_vm` interpreter DSL.
///
/// Entry syntax:
///
/// ```text
/// /// doc
/// Name = 0xNN, snake_name, acc: AccUse, flags: [Flag, ..], operands: (name: Kind[(param)], ..);
/// ```
///
/// - `Name` is the [`Opcode`](crate::Opcode) variant; `0xNN` its byte. Bytes are dense and in
///   list order (a const assert checks it).
/// - `acc` is one of `None`, `Read`, `Write`, `ReadWrite` ([`AccUse`](crate::AccUse)).
/// - `flags` names [`OpcodeFlags`](crate::OpcodeFlags) bits: `Prefix`, `Jump`, `Switch`,
///   `Terminator`, `Return`, `Throws`, `Call`, `Interrupt`, `Clobber`, `Reserved`.
/// - Each operand has a name and an [`OperandKind`](crate::OperandKind) variant. `Const(..)`
///   names a [`ConstOperand`](crate::ConstOperand), `Slot(..)` a
///   [`FeedbackSlotKind`](crate::FeedbackSlotKind), `Flag8(..)`/`Flag16(..)` the mask of
///   defined bits.
///
/// Example consumer (an external crate uses `ep9_bytecode::for_each_opcode!`):
///
/// ```
/// macro_rules! count_ops {
///     ([] $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
///        flags: [$($flag:ident),*],
///        operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*) => {
///         [$( stringify!($name) ),*].len()
///     };
/// }
/// let n = ep9_bytecode::for_each_opcode!(count_ops []);
/// assert_eq!(n, ep9_bytecode::OPCODE_COUNT);
/// ```
#[macro_export]
macro_rules! for_each_opcode {
    ($m:ident [$($arg:tt)*]) => {
        $m! {
            [$($arg)*]

            // ---- Special -------------------------------------------------------------------

            /// Invalid opcode. Byte 0, so zeroed memory never decodes as an instruction.
            Illegal = 0x00, illegal, acc: None, flags: [Terminator, Reserved], operands: ();
            /// Prefix: the next instruction's scalable operands are 2 bytes.
            Wide = 0x01, wide, acc: None, flags: [Prefix], operands: ();
            /// Prefix: the next instruction's scalable operands are 4 bytes.
            ExtraWide = 0x02, extra_wide, acc: None, flags: [Prefix], operands: ();

            // ---- Accumulator and register moves --------------------------------------------

            /// acc = 0
            LdaZero = 0x03, lda_zero, acc: Write, flags: [], operands: ();
            /// acc = value (an int32 immediate)
            LdaSmi = 0x04, lda_smi, acc: Write, flags: [], operands: (value: Imm);
            /// acc = undefined
            LdaUndefined = 0x05, lda_undefined, acc: Write, flags: [], operands: ();
            /// acc = null
            LdaNull = 0x06, lda_null, acc: Write, flags: [], operands: ();
            /// acc = the hole (TDZ marker)
            LdaTheHole = 0x07, lda_the_hole, acc: Write, flags: [], operands: ();
            /// acc = true
            LdaTrue = 0x08, lda_true, acc: Write, flags: [], operands: ();
            /// acc = false
            LdaFalse = 0x09, lda_false, acc: Write, flags: [], operands: ();
            /// acc = constant (Number, String or BigInt)
            LdaConstant = 0x0a, lda_constant, acc: Write, flags: [], operands: (constant: Const(Value));
            /// acc = src
            Ldar = 0x0b, ldar, acc: Write, flags: [], operands: (src: Reg);
            /// dst = acc
            Star = 0x0c, star, acc: Read, flags: [], operands: (dst: RegOut);
            /// dst = src
            Mov = 0x0d, mov, acc: None, flags: [], operands: (src: Reg, dst: RegOut);
            /// acc = the running function (the frame's callee)
            LdaCurrentFunction = 0x0e, lda_current_function, acc: Write, flags: [], operands: ();
            /// acc = new.target of the running frame (undefined for a `[[Call]]`)
            LdaNewTarget = 0x0f, lda_new_target, acc: Write, flags: [], operands: ();

            // ---- Globals ------------------------------------------------------------------

            /// acc = global `name`; ReferenceError if missing
            LdaGlobal = 0x10, lda_global, acc: Write, flags: [Throws],
                operands: (name: Const(String), slot: Slot(LoadGlobal));
            /// acc = global `name`, or undefined if missing (inside `typeof`)
            LdaGlobalInsideTypeof = 0x11, lda_global_inside_typeof, acc: Write, flags: [Throws],
                operands: (name: Const(String), slot: Slot(LoadGlobalInsideTypeof));
            /// global `name` = acc
            StaGlobal = 0x12, sta_global, acc: ReadWrite, flags: [Throws, Clobber],
                operands: (name: Const(String), slot: Slot(StoreGlobal));

            // ---- Contexts -----------------------------------------------------------------

            /// save = current context; current context = acc
            PushContext = 0x13, push_context, acc: Read, flags: [], operands: (save: RegOut);
            /// current context = context
            PopContext = 0x14, pop_context, acc: None, flags: [], operands: (context: Reg);
            /// acc = the current context (saved for exception handlers, H2 table `context`)
            LdaCurrentContext = 0x15, lda_current_context, acc: Write, flags: [], operands: ();
            /// acc = slot `slot_index` of the context `depth` hops out from `context`
            LdaContextSlot = 0x16, lda_context_slot, acc: Write, flags: [],
                operands: (context: Reg, slot_index: CtxSlot, depth: CtxDepth);
            /// Same as `LdaContextSlot` for a binding that never changes after init
            LdaImmutableContextSlot = 0x17, lda_immutable_context_slot, acc: Write, flags: [],
                operands: (context: Reg, slot_index: CtxSlot, depth: CtxDepth);
            /// acc = slot `slot_index` of the current context
            LdaCurrentContextSlot = 0x18, lda_current_context_slot, acc: Write, flags: [],
                operands: (slot_index: CtxSlot);
            /// Same as `LdaCurrentContextSlot` for a binding that never changes after init
            LdaImmutableCurrentContextSlot = 0x19, lda_immutable_current_context_slot, acc: Write,
                flags: [], operands: (slot_index: CtxSlot);
            /// slot `slot_index` of the context `depth` hops out from `context` = acc
            StaContextSlot = 0x1a, sta_context_slot, acc: Read, flags: [],
                operands: (context: Reg, slot_index: CtxSlot, depth: CtxDepth);
            /// slot `slot_index` of the current context = acc
            StaCurrentContextSlot = 0x1b, sta_current_context_slot, acc: Read, flags: [],
                operands: (slot_index: CtxSlot);
            /// acc = dynamic lookup of `name` (`with`, sloppy direct eval)
            LdaLookupSlot = 0x1c, lda_lookup_slot, acc: Write, flags: [Throws],
                operands: (name: Const(String));
            /// Same as `LdaLookupSlot` inside `typeof` (no ReferenceError)
            LdaLookupSlotInsideTypeof = 0x1d, lda_lookup_slot_inside_typeof, acc: Write,
                flags: [Throws], operands: (name: Const(String));
            /// dynamic store of acc to `name`; bit 0 of flags = strict
            StaLookupSlot = 0x1e, sta_lookup_slot, acc: ReadWrite, flags: [Throws],
                operands: (name: Const(String), flags: Flag8(0x01));
            /// acc = module variable `cell` (> 0 export, < 0 import) `depth` hops out
            LdaModuleVariable = 0x1f, lda_module_variable, acc: Write, flags: [],
                operands: (cell: Imm, depth: CtxDepth);
            /// module export `cell` `depth` hops out = acc
            StaModuleVariable = 0x20, sta_module_variable, acc: Read, flags: [],
                operands: (cell: Imm, depth: CtxDepth);

            // ---- Property access ----------------------------------------------------------

            /// acc = object.name
            GetNamedProperty = 0x21, get_named_property, acc: Write, flags: [Throws],
                operands: (object: Reg, name: Const(String), slot: Slot(LoadNamed));
            /// acc = super.name, with receiver; acc holds the home object on entry
            GetNamedPropertyFromSuper = 0x22, get_named_property_from_super, acc: ReadWrite,
                flags: [Throws],
                operands: (receiver: Reg, name: Const(String), slot: Slot(LoadNamed));
            /// `acc = object[acc]`
            GetKeyedProperty = 0x23, get_keyed_property, acc: ReadWrite, flags: [Throws],
                operands: (object: Reg, slot: Slot(LoadKeyed));
            /// object.name = acc (`[[Set]]`)
            SetNamedProperty = 0x24, set_named_property, acc: ReadWrite, flags: [Throws, Clobber],
                operands: (object: Reg, name: Const(String), slot: Slot(StoreNamed));
            /// define own property object.name = acc (`[[DefineOwnProperty]]`)
            DefineNamedOwnProperty = 0x25, define_named_own_property, acc: ReadWrite,
                flags: [Throws, Clobber],
                operands: (object: Reg, name: Const(String), slot: Slot(DefineNamedOwn));
            /// `object[key] = acc` (`[[Set]]`)
            SetKeyedProperty = 0x26, set_keyed_property, acc: ReadWrite, flags: [Throws, Clobber],
                operands: (object: Reg, key: Reg, slot: Slot(StoreKeyed));
            /// define own property `object[key] = acc`; bit 0 of flags = set function name
            DefineKeyedOwnProperty = 0x27, define_keyed_own_property, acc: ReadWrite,
                flags: [Throws, Clobber],
                operands: (object: Reg, key: Reg, flags: Flag8(0x01), slot: Slot(DefineKeyedOwn));
            /// `array[index] = acc` inside an array literal
            StaInArrayLiteral = 0x28, sta_in_array_literal, acc: ReadWrite, flags: [Throws, Clobber],
                operands: (array: Reg, index: Reg, slot: Slot(StoreInArrayLiteral));
            /// define `object[key] = acc` inside an object literal; bit 0 = set function name
            DefineKeyedOwnPropertyInLiteral = 0x29, define_keyed_own_property_in_literal,
                acc: Read, flags: [Throws],
                operands: (object: Reg, key: Reg, flags: Flag8(0x01), slot: Slot(DefineKeyedOwn));

            // ---- Binary operators: acc = lhs OP acc ---------------------------------------

            /// acc = lhs + acc
            Add = 0x2a, add, acc: ReadWrite, flags: [Throws], operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs - acc
            Sub = 0x2b, sub, acc: ReadWrite, flags: [Throws], operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs * acc
            Mul = 0x2c, mul, acc: ReadWrite, flags: [Throws], operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs / acc
            Div = 0x2d, div, acc: ReadWrite, flags: [Throws], operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs % acc
            Mod = 0x2e, r#mod, acc: ReadWrite, flags: [Throws], operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs ** acc
            Exp = 0x2f, exp, acc: ReadWrite, flags: [Throws], operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs | acc
            BitwiseOr = 0x30, bitwise_or, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs ^ acc
            BitwiseXor = 0x31, bitwise_xor, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs & acc
            BitwiseAnd = 0x32, bitwise_and, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs << acc
            ShiftLeft = 0x33, shift_left, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs >> acc
            ShiftRight = 0x34, shift_right, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(BinaryOp));
            /// acc = lhs >>> acc
            ShiftRightLogical = 0x35, shift_right_logical, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(BinaryOp));

            // ---- Binary operators with an int32 immediate: acc = acc OP value ------------------

            /// acc = acc + value
            AddSmi = 0x36, add_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc - value
            SubSmi = 0x37, sub_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc * value
            MulSmi = 0x38, mul_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc / value
            DivSmi = 0x39, div_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc % value
            ModSmi = 0x3a, mod_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc ** value
            ExpSmi = 0x3b, exp_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc | value
            BitwiseOrSmi = 0x3c, bitwise_or_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc ^ value
            BitwiseXorSmi = 0x3d, bitwise_xor_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc & value
            BitwiseAndSmi = 0x3e, bitwise_and_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc << value
            ShiftLeftSmi = 0x3f, shift_left_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc >> value
            ShiftRightSmi = 0x40, shift_right_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));
            /// acc = acc >>> value
            ShiftRightLogicalSmi = 0x41, shift_right_logical_smi, acc: ReadWrite, flags: [Throws],
                operands: (value: Imm, slot: Slot(BinaryOp));

            // ---- Unary operators ----------------------------------------------------------

            /// acc = acc + 1 (numeric)
            Inc = 0x42, inc, acc: ReadWrite, flags: [Throws], operands: (slot: Slot(BinaryOp));
            /// acc = acc - 1 (numeric)
            Dec = 0x43, dec, acc: ReadWrite, flags: [Throws], operands: (slot: Slot(BinaryOp));
            /// acc = -acc
            Negate = 0x44, negate, acc: ReadWrite, flags: [Throws], operands: (slot: Slot(BinaryOp));
            /// acc = ~acc
            BitwiseNot = 0x45, bitwise_not, acc: ReadWrite, flags: [Throws],
                operands: (slot: Slot(BinaryOp));
            /// acc = !ToBoolean(acc)
            ToBooleanLogicalNot = 0x46, to_boolean_logical_not, acc: ReadWrite, flags: [], operands: ();
            /// acc = !acc (acc is a boolean)
            LogicalNot = 0x47, logical_not, acc: ReadWrite, flags: [], operands: ();
            /// acc = typeof acc
            TypeOf = 0x48, type_of, acc: ReadWrite, flags: [], operands: (slot: Slot(TypeOf));
            /// `acc = delete object[acc]` (strict: TypeError on failure)
            DeletePropertyStrict = 0x49, delete_property_strict, acc: ReadWrite, flags: [Throws],
                operands: (object: Reg);
            /// `acc = delete object[acc]` (sloppy)
            DeletePropertySloppy = 0x4a, delete_property_sloppy, acc: ReadWrite, flags: [Throws],
                operands: (object: Reg);

            // ---- Comparisons: acc = lhs OP acc --------------------------------------------

            /// acc = lhs == acc
            TestEqual = 0x4b, test_equal, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(CompareOp));
            /// acc = lhs === acc
            TestEqualStrict = 0x4c, test_equal_strict, acc: ReadWrite, flags: [],
                operands: (lhs: Reg, slot: Slot(CompareOp));
            /// acc = lhs < acc
            TestLessThan = 0x4d, test_less_than, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(CompareOp));
            /// acc = lhs > acc
            TestGreaterThan = 0x4e, test_greater_than, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(CompareOp));
            /// acc = lhs <= acc
            TestLessThanOrEqual = 0x4f, test_less_than_or_equal, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(CompareOp));
            /// acc = lhs >= acc
            TestGreaterThanOrEqual = 0x50, test_greater_than_or_equal, acc: ReadWrite,
                flags: [Throws], operands: (lhs: Reg, slot: Slot(CompareOp));
            /// acc = lhs and acc are the same value (no conversion, no feedback)
            TestReferenceEqual = 0x51, test_reference_equal, acc: ReadWrite, flags: [],
                operands: (lhs: Reg);
            /// acc = lhs instanceof acc
            TestInstanceOf = 0x52, test_instance_of, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(InstanceOf));
            /// acc = lhs in acc
            TestIn = 0x53, test_in, acc: ReadWrite, flags: [Throws],
                operands: (lhs: Reg, slot: Slot(HasKeyed));
            /// acc = acc is undetectable (document.all) or null or undefined
            TestUndetectable = 0x54, test_undetectable, acc: ReadWrite, flags: [], operands: ();
            /// acc = acc === null
            TestNull = 0x55, test_null, acc: ReadWrite, flags: [], operands: ();
            /// acc = acc === undefined
            TestUndefined = 0x56, test_undefined, acc: ReadWrite, flags: [], operands: ();
            /// acc = typeof acc == literal
            TestTypeOf = 0x57, test_type_of, acc: ReadWrite, flags: [],
                operands: (literal: TypeOfLiteral);

            // ---- Conversions --------------------------------------------------------------

            /// acc = ToPropertyKey(acc)
            ToName = 0x58, to_name, acc: ReadWrite, flags: [Throws], operands: ();
            /// acc = ToNumber(acc)
            ToNumber = 0x59, to_number, acc: ReadWrite, flags: [Throws], operands: (slot: Slot(BinaryOp));
            /// acc = ToNumeric(acc)
            ToNumeric = 0x5a, to_numeric, acc: ReadWrite, flags: [Throws],
                operands: (slot: Slot(BinaryOp));
            /// dst = ToObject(acc)
            ToObject = 0x5b, to_object, acc: Read, flags: [Throws], operands: (dst: RegOut);
            /// acc = ToString(acc)
            ToString = 0x5c, to_string, acc: ReadWrite, flags: [Throws], operands: ();
            /// acc = ToBoolean(acc)
            ToBoolean = 0x5d, to_boolean, acc: ReadWrite, flags: [], operands: ();

            // ---- Calls --------------------------------------------------------------------

            /// `acc = callee.call(args[0], args[1..])` (receiver may be any value)
            CallAnyReceiver = 0x5e, call_any_receiver, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, args: RegList, argc: RegCount, slot: Slot(Call));
            /// `acc = callee.call(args[0], args[1..])` for a property call `o.f(..)`
            CallProperty = 0x5f, call_property, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, args: RegList, argc: RegCount, slot: Slot(Call));
            /// acc = callee.call(receiver)
            CallProperty0 = 0x60, call_property0, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, receiver: Reg, slot: Slot(Call));
            /// acc = callee.call(receiver, arg0)
            CallProperty1 = 0x61, call_property1, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, receiver: Reg, arg0: Reg, slot: Slot(Call));
            /// acc = callee.call(receiver, arg0, arg1)
            CallProperty2 = 0x62, call_property2, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, receiver: Reg, arg0: Reg, arg1: Reg, slot: Slot(Call));
            /// acc = callee(args..) with receiver undefined
            CallUndefinedReceiver = 0x63, call_undefined_receiver, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, args: RegList, argc: RegCount, slot: Slot(Call));
            /// acc = callee()
            CallUndefinedReceiver0 = 0x64, call_undefined_receiver0, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, slot: Slot(Call));
            /// acc = callee(arg0)
            CallUndefinedReceiver1 = 0x65, call_undefined_receiver1, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, arg0: Reg, slot: Slot(Call));
            /// acc = callee(arg0, arg1)
            CallUndefinedReceiver2 = 0x66, call_undefined_receiver2, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, arg0: Reg, arg1: Reg, slot: Slot(Call));
            /// `acc = callee.call(args[0], ...args[1..])`; the last argument is spread
            CallWithSpread = 0x67, call_with_spread, acc: Write, flags: [Throws, Call],
                operands: (callee: Reg, args: RegList, argc: RegCount, slot: Slot(Call));
            /// acc = runtime function `id`(args..)
            CallRuntime = 0x68, call_runtime, acc: Write, flags: [Throws, Call],
                operands: (id: Runtime, args: RegList, argc: RegCount);
            /// acc = new constructor(args..) with new.target = acc
            Construct = 0x69, construct, acc: ReadWrite, flags: [Throws, Call],
                operands: (constructor: Reg, args: RegList, argc: RegCount, slot: Slot(Call));
            /// Same as `Construct`; the last argument is spread
            ConstructWithSpread = 0x6a, construct_with_spread, acc: ReadWrite, flags: [Throws, Call],
                operands: (constructor: Reg, args: RegList, argc: RegCount, slot: Slot(Call));
            /// Reserved (D7: no proper tail calls). Layout as `CallProperty`
            TailCall = 0x6b, tail_call, acc: Write, flags: [Throws, Call, Terminator, Return, Reserved],
                operands: (callee: Reg, args: RegList, argc: RegCount, slot: Slot(Call));

            // ---- Literals -----------------------------------------------------------------

            /// acc = new RegExp(pattern, flags); flags bits: d g i m s u v y
            CreateRegExpLiteral = 0x6c, create_reg_exp_literal, acc: Write, flags: [Throws],
                operands: (pattern: Const(String), slot: Slot(Literal), flags: Flag16(0x00ff));
            /// acc = array from a boilerplate; bit 0 = shallow
            CreateArrayLiteral = 0x6d, create_array_literal, acc: Write, flags: [Throws],
                operands: (boilerplate: Const(ArrayBoilerplate), slot: Slot(Literal), flags: Flag8(0x01));
            /// `acc = []`
            CreateEmptyArrayLiteral = 0x6e, create_empty_array_literal, acc: Write, flags: [Throws],
                operands: (slot: Slot(Literal));
            /// `acc = [...acc]`
            CreateArrayFromIterable = 0x6f, create_array_from_iterable, acc: ReadWrite,
                flags: [Throws], operands: ();
            /// acc = object from a boilerplate; bit 0 = shallow, bit 1 = null prototype
            CreateObjectLiteral = 0x70, create_object_literal, acc: Write, flags: [Throws],
                operands: (boilerplate: Const(ObjectBoilerplate), slot: Slot(Literal), flags: Flag8(0x03));
            /// acc = {}
            CreateEmptyObjectLiteral = 0x71, create_empty_object_literal, acc: Write, flags: [Throws],
                operands: ();
            /// acc = { ...source }; bit 0 = null prototype
            CloneObject = 0x72, clone_object, acc: Write, flags: [Throws],
                operands: (source: Reg, flags: Flag8(0x01), slot: Slot(CloneObject));
            /// acc = the template object of a tagged template
            GetTemplateObject = 0x73, get_template_object, acc: Write, flags: [Throws],
                operands: (description: Const(TemplateObject), slot: Slot(Literal));

            // ---- Closures, contexts, arguments --------------------------------------------

            /// acc = new closure of `function` in the current context; bit 0 = pretenure
            CreateClosure = 0x74, create_closure, acc: Write, flags: [Throws],
                operands: (function: Const(Function), slot: Slot(CreateClosure), flags: Flag8(0x01));
            /// acc = new function context for `scope` with `slots` variable slots
            CreateFunctionContext = 0x75, create_function_context, acc: Write, flags: [Throws],
                operands: (scope: Const(Scope), slots: UImm);
            /// acc = new block context for `scope`
            CreateBlockContext = 0x76, create_block_context, acc: Write, flags: [Throws],
                operands: (scope: Const(Scope));
            /// acc = new catch context for `scope` that binds `exception`
            CreateCatchContext = 0x77, create_catch_context, acc: Write, flags: [Throws],
                operands: (exception: Reg, scope: Const(Scope));
            /// acc = new eval context for `scope` with `slots` variable slots
            CreateEvalContext = 0x78, create_eval_context, acc: Write, flags: [Throws],
                operands: (scope: Const(Scope), slots: UImm);
            /// acc = new `with` context over `object`
            CreateWithContext = 0x79, create_with_context, acc: Write, flags: [Throws],
                operands: (object: Reg, scope: Const(Scope));
            /// acc = mapped (sloppy) arguments object
            CreateMappedArguments = 0x7a, create_mapped_arguments, acc: Write, flags: [Throws],
                operands: ();
            /// acc = unmapped (strict) arguments object
            CreateUnmappedArguments = 0x7b, create_unmapped_arguments, acc: Write, flags: [Throws],
                operands: ();
            /// acc = rest parameter array
            CreateRestParameter = 0x7c, create_rest_parameter, acc: Write, flags: [Throws],
                operands: ();

            // ---- Control flow -------------------------------------------------------------

            /// Loop back edge: interrupt poll (H29 lazy-deopt point), then jump back.
            /// `depth` is the loop nesting depth (OSR). acc is undefined afterwards
            JumpLoop = 0x7d, jump_loop, acc: Write, flags: [Jump, Terminator, Interrupt, Clobber],
                operands: (target: JumpBack, depth: Imm, slot: Slot(JumpLoop));
            /// Unconditional forward jump
            Jump = 0x7e, jump, acc: None, flags: [Jump, Terminator], operands: (target: JumpFwd);
            /// Jump if acc is true
            JumpIfTrue = 0x7f, jump_if_true, acc: Read, flags: [Jump], operands: (target: JumpFwd);
            /// Jump if acc is false
            JumpIfFalse = 0x80, jump_if_false, acc: Read, flags: [Jump], operands: (target: JumpFwd);
            /// Jump if ToBoolean(acc) is true
            JumpIfToBooleanTrue = 0x81, jump_if_to_boolean_true, acc: Read, flags: [Jump],
                operands: (target: JumpFwd);
            /// Jump if ToBoolean(acc) is false
            JumpIfToBooleanFalse = 0x82, jump_if_to_boolean_false, acc: Read, flags: [Jump],
                operands: (target: JumpFwd);
            /// Jump if acc is null
            JumpIfNull = 0x83, jump_if_null, acc: Read, flags: [Jump], operands: (target: JumpFwd);
            /// Jump if acc is not null
            JumpIfNotNull = 0x84, jump_if_not_null, acc: Read, flags: [Jump], operands: (target: JumpFwd);
            /// Jump if acc is undefined
            JumpIfUndefined = 0x85, jump_if_undefined, acc: Read, flags: [Jump],
                operands: (target: JumpFwd);
            /// Jump if acc is not undefined
            JumpIfNotUndefined = 0x86, jump_if_not_undefined, acc: Read, flags: [Jump],
                operands: (target: JumpFwd);
            /// Jump if acc is undefined or null
            JumpIfUndefinedOrNull = 0x87, jump_if_undefined_or_null, acc: Read, flags: [Jump],
                operands: (target: JumpFwd);
            /// Jump if acc is an object (JSReceiver)
            JumpIfJSReceiver = 0x88, jump_if_js_receiver, acc: Read, flags: [Jump],
                operands: (target: JumpFwd);
            /// Jump if index == cache_length (for-in loop end)
            JumpIfForInDone = 0x89, jump_if_for_in_done, acc: None, flags: [Jump],
                operands: (target: JumpFwd, index: Reg, cache_length: Reg);
            /// If acc is an int32 in `[base, base + table length)`, jump to `table[acc - base]`;
            /// else fall through
            SwitchOnSmi = 0x8a, switch_on_smi, acc: Read, flags: [Switch],
                operands: (table: Const(JumpTable), base: Imm);

            // ---- Iteration ----------------------------------------------------------------

            /// acc = for-in enumeration of receiver
            ForInEnumerate = 0x8b, for_in_enumerate, acc: Write, flags: [Throws],
                operands: (receiver: Reg);
            /// `cache[0..3]` = for-in cache from the enumeration in acc
            ForInPrepare = 0x8c, for_in_prepare, acc: ReadWrite, flags: [Clobber],
                operands: (cache: RegOutTriple, slot: Slot(ForIn));
            /// acc = next for-in key, or undefined to skip a deleted key
            ForInNext = 0x8d, for_in_next, acc: Write, flags: [Throws],
                operands: (receiver: Reg, index: Reg, cache: RegPair, slot: Slot(ForIn));
            /// index = index + 1
            ForInStep = 0x8e, for_in_step, acc: None, flags: [], operands: (index: RegInOut);
            /// `acc = object[Symbol.iterator]()`; the call result must be an object
            GetIterator = 0x8f, get_iterator, acc: Write, flags: [Throws, Call],
                operands: (object: Reg, load_slot: Slot(LoadNamed), call_slot: Slot(Call));

            // ---- Exceptions and returns ---------------------------------------------------

            /// throw acc
            Throw = 0x90, throw, acc: Read, flags: [Throws, Terminator], operands: ();
            /// rethrow acc (no new message or stack)
            ReThrow = 0x91, re_throw, acc: Read, flags: [Throws, Terminator], operands: ();
            /// return acc to the caller
            Return = 0x92, return_, acc: Read, flags: [Terminator, Return], operands: ();
            /// swap acc with the pending message
            SetPendingMessage = 0x93, set_pending_message, acc: ReadWrite, flags: [], operands: ();
            /// ReferenceError naming `name` if acc is the hole (TDZ)
            ThrowReferenceErrorIfHole = 0x94, throw_reference_error_if_hole, acc: Read,
                flags: [Throws], operands: (name: Const(String));
            /// ReferenceError if acc (this) is the hole: super() not called
            ThrowSuperNotCalledIfHole = 0x95, throw_super_not_called_if_hole, acc: Read,
                flags: [Throws], operands: ();
            /// ReferenceError if acc (this) is not the hole: super() called twice
            ThrowSuperAlreadyCalledIfNotHole = 0x96, throw_super_already_called_if_not_hole,
                acc: Read, flags: [Throws], operands: ();
            /// TypeError if constructor is not a constructor
            ThrowIfNotSuperConstructor = 0x97, throw_if_not_super_constructor, acc: None,
                flags: [Throws], operands: (constructor: Reg);
            /// dst = `[[GetPrototypeOf]]` of the function in acc
            GetSuperConstructor = 0x98, get_super_constructor, acc: Read, flags: [],
                operands: (dst: RegOut);

            // ---- Generators (reserved until M1) -------------------------------------------

            /// Reserved. Resume: jump to `table[state of generator]`, else fall through
            SwitchOnGeneratorState = 0x99, switch_on_generator_state, acc: None,
                flags: [Switch, Reserved], operands: (generator: Reg, table: Const(JumpTable));
            /// Reserved. Save registers into generator, then return acc to the caller
            SuspendGenerator = 0x9a, suspend_generator, acc: Read,
                flags: [Terminator, Return, Reserved],
                operands: (generator: Reg, registers: RegList, count: RegCount, suspend_id: UImm);
            /// Reserved. Restore registers from generator; acc = the sent value
            ResumeGenerator = 0x9b, resume_generator, acc: Write, flags: [Reserved],
                operands: (generator: Reg, registers: RegOutList, count: RegCount);

            // ---- Debugger -----------------------------------------------------------------

            /// `debugger` statement: pause if a session is attached (H24). acc is undefined
            /// afterwards
            Debugger = 0x9c, debugger, acc: Write, flags: [Clobber], operands: ();
        }
    };
}

/// How an opcode uses the accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AccUse {
    /// Neither reads nor writes it.
    None = 0,
    /// Reads it and leaves it unchanged.
    Read = 1,
    /// Writes it without reading it.
    Write = 2,
    /// Reads it, then writes it.
    ReadWrite = 3,
}

impl AccUse {
    /// True if the opcode reads the accumulator.
    pub const fn reads(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    /// True if the opcode writes the accumulator.
    pub const fn writes(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }

    /// Name used in golden files.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Read => "Read",
            Self::Write => "Write",
            Self::ReadWrite => "ReadWrite",
        }
    }
}

/// Opcode flag bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct OpcodeFlags(pub u16);

impl OpcodeFlags {
    /// `Wide` or `ExtraWide`.
    pub const PREFIX: OpcodeFlags = OpcodeFlags(1 << 0);
    /// Has a jump operand ([`crate::OperandKind::JumpFwd`] or [`crate::OperandKind::JumpBack`]).
    pub const JUMP: OpcodeFlags = OpcodeFlags(1 << 1);
    /// Has a jump-table constant operand.
    pub const SWITCH: OpcodeFlags = OpcodeFlags(1 << 2);
    /// Never falls through to the next instruction.
    pub const TERMINATOR: OpcodeFlags = OpcodeFlags(1 << 3);
    /// Leaves the frame (return, suspend, tail call).
    pub const RETURN: OpcodeFlags = OpcodeFlags(1 << 4);
    /// May throw a JS exception.
    pub const THROWS: OpcodeFlags = OpcodeFlags(1 << 5);
    /// A call site: the instruction calls or constructs a JS function as its main effect and has
    /// call feedback. (Getters, setters and conversions in other opcodes can also run JS; `THROWS`
    /// covers them.)
    pub const CALL: OpcodeFlags = OpcodeFlags(1 << 6);
    /// Interrupt poll (loop back edge).
    pub const INTERRUPT: OpcodeFlags = OpcodeFlags(1 << 7);
    /// The accumulator holds no defined value afterwards.
    pub const CLOBBER: OpcodeFlags = OpcodeFlags(1 << 8);
    /// Not valid in ABI v1 bytecode; the verifier rejects it.
    pub const RESERVED: OpcodeFlags = OpcodeFlags(1 << 9);

    /// Every flag with its golden-file name, in bit order.
    pub const NAMES: [(OpcodeFlags, &'static str); 10] = [
        (Self::PREFIX, "Prefix"),
        (Self::JUMP, "Jump"),
        (Self::SWITCH, "Switch"),
        (Self::TERMINATOR, "Terminator"),
        (Self::RETURN, "Return"),
        (Self::THROWS, "Throws"),
        (Self::CALL, "Call"),
        (Self::INTERRUPT, "Interrupt"),
        (Self::CLOBBER, "Clobber"),
        (Self::RESERVED, "Reserved"),
    ];

    /// True if every bit of `other` is set.
    pub const fn contains(self, other: OpcodeFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Maps a flag name in the opcode list to its bit.
#[doc(hidden)]
#[macro_export]
macro_rules! __ep9_opcode_flag {
    (Prefix) => {
        $crate::OpcodeFlags::PREFIX
    };
    (Jump) => {
        $crate::OpcodeFlags::JUMP
    };
    (Switch) => {
        $crate::OpcodeFlags::SWITCH
    };
    (Terminator) => {
        $crate::OpcodeFlags::TERMINATOR
    };
    (Return) => {
        $crate::OpcodeFlags::RETURN
    };
    (Throws) => {
        $crate::OpcodeFlags::THROWS
    };
    (Call) => {
        $crate::OpcodeFlags::CALL
    };
    (Interrupt) => {
        $crate::OpcodeFlags::INTERRUPT
    };
    (Clobber) => {
        $crate::OpcodeFlags::CLOBBER
    };
    (Reserved) => {
        $crate::OpcodeFlags::RESERVED
    };
}

/// Maps an operand kind and its optional parameter to an [`crate::OperandParam`].
#[doc(hidden)]
#[macro_export]
macro_rules! __ep9_operand_param {
    (Const, $p:ident) => {
        $crate::OperandParam::Const($crate::ConstOperand::$p)
    };
    (Slot, $p:ident) => {
        $crate::OperandParam::Slot($crate::FeedbackSlotKind::$p)
    };
    (Flag8, $p:literal) => {
        $crate::OperandParam::FlagMask($p)
    };
    (Flag16, $p:literal) => {
        $crate::OperandParam::FlagMask($p)
    };
    ($k:ident) => {
        $crate::OperandParam::None
    };
}

/// Static facts about one opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpcodeInfo {
    /// The opcode.
    pub opcode: Opcode,
    /// CamelCase name (`LdaSmi`), as the disassembly prints it.
    pub name: &'static str,
    /// snake_case name (`lda_smi`): builder method and interpreter handler name.
    pub snake: &'static str,
    /// Accumulator use.
    pub acc: AccUse,
    /// Flags.
    pub flags: OpcodeFlags,
    /// Encoded operands, in order.
    pub operands: &'static [OperandInfo],
}

use crate::operand::{OperandInfo, OperandScale};

impl OpcodeInfo {
    /// True if any operand is scalable (a `Wide`/`ExtraWide` prefix may precede it).
    pub const fn has_scalable_operands(&self) -> bool {
        let mut i = 0;
        while i < self.operands.len() {
            if self.operands[i].kind.is_scalable() {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Size of the operands at `scale`, without the opcode byte and the prefix.
    pub const fn operands_size(&self, scale: OperandScale) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < self.operands.len() {
            n += self.operands[i].kind.size(scale);
            i += 1;
        }
        n
    }

    /// Instruction size at `scale`, prefix byte included.
    pub const fn size(&self, scale: OperandScale) -> usize {
        let prefix = match scale {
            OperandScale::Single => 0,
            _ => 1,
        };
        prefix + 1 + self.operands_size(scale)
    }

    /// Byte offset of operand `i` from the opcode byte (so the first operand is at 1).
    pub const fn operand_offset(&self, i: usize, scale: OperandScale) -> usize {
        let mut n = 1;
        let mut j = 0;
        while j < i {
            n += self.operands[j].kind.size(scale);
            j += 1;
        }
        n
    }
}

macro_rules! define_opcode_table {
    ( []
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        /// Opcodes of the bytecode ABI. The discriminant is the opcode byte.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum Opcode {
            $( $(#[$doc])* $name = $code, )*
        }

        /// Number of opcodes. Bytes `OPCODE_COUNT..=255` are undefined.
        pub const OPCODE_COUNT: usize = [$( Opcode::$name ),*].len();

        /// One entry per opcode, indexed by the opcode byte.
        pub static OPCODES: [OpcodeInfo; OPCODE_COUNT] = [ $(
            OpcodeInfo {
                opcode: Opcode::$name,
                name: stringify!($name),
                snake: $crate::opcodes::strip_raw(stringify!($snake)),
                acc: AccUse::$acc,
                flags: OpcodeFlags(0 $( | $crate::__ep9_opcode_flag!($flag).0 )*),
                operands: &[ $( OperandInfo {
                    name: stringify!($oname),
                    kind: $crate::OperandKind::$okind,
                    param: $crate::__ep9_operand_param!($okind $(, $oparam)?),
                } ),* ],
            },
        )* ];

        impl Opcode {
            /// The opcode with byte `b`, if it is defined.
            #[inline]
            pub const fn from_u8(b: u8) -> Option<Opcode> {
                match b {
                    $( $code => Some(Opcode::$name), )*
                    _ => None,
                }
            }
        }

        // Opcode bytes are dense and in list order, so OPCODES[op as usize] is op's entry.
        const _: () = {
            let codes: &[u8] = &[ $( $code ),* ];
            let mut i = 0;
            while i < codes.len() {
                assert!(codes[i] as usize == i, "opcode bytes must be 0, 1, 2, ... in list order");
                i += 1;
            }
        };
    };
}

for_each_opcode!(define_opcode_table []);

/// Removes the `r#` of a raw identifier name (`r#mod` -> `mod`).
#[doc(hidden)]
pub const fn strip_raw(s: &'static str) -> &'static str {
    let b = s.as_bytes();
    if b.len() > 2 && b[0] == b'r' && b[1] == b'#' {
        let (_, rest) = s.split_at(2);
        rest
    } else {
        s
    }
}

/// The largest number of operands of any opcode.
pub const MAX_OPERANDS: usize = {
    let mut max = 0;
    let mut i = 0;
    while i < OPCODE_COUNT {
        if OPCODES[i].operands.len() > max {
            max = OPCODES[i].operands.len();
        }
        i += 1;
    }
    max
};

/// The largest instruction size in bytes (prefix included).
pub const MAX_INSTRUCTION_SIZE: usize = {
    let mut max = 0;
    let mut i = 0;
    while i < OPCODE_COUNT {
        let n = OPCODES[i].size(OperandScale::ExtraWide);
        if n > max {
            max = n;
        }
        i += 1;
    }
    max
};

impl Opcode {
    /// Static facts about this opcode.
    #[inline]
    pub const fn info(self) -> &'static OpcodeInfo {
        &OPCODES[self as usize]
    }

    /// CamelCase name.
    pub const fn name(self) -> &'static str {
        self.info().name
    }

    /// The prefix byte for `scale`, or `None` for [`OperandScale::Single`].
    pub const fn prefix_for(scale: OperandScale) -> Option<Opcode> {
        match scale {
            OperandScale::Single => None,
            OperandScale::Wide => Some(Opcode::Wide),
            OperandScale::ExtraWide => Some(Opcode::ExtraWide),
        }
    }

    /// The scale a prefix opcode selects.
    pub const fn prefix_scale(self) -> Option<OperandScale> {
        match self {
            Opcode::Wide => Some(OperandScale::Wide),
            Opcode::ExtraWide => Some(OperandScale::ExtraWide),
            _ => None,
        }
    }
}

impl core::fmt::Display for Opcode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// The opcode table as stable text: one line per opcode. Golden file `opcodes.txt` pins it, and
/// its SHA-256 is recorded per ABI version.
pub fn opcode_table_text() -> String {
    use core::fmt::Write;
    let mut s = String::new();
    for info in OPCODES.iter() {
        let _ = write!(
            s,
            "0x{:02x} {} {} acc={}",
            info.opcode as u8,
            info.name,
            info.snake,
            info.acc.name()
        );
        s.push_str(" flags=[");
        let mut first = true;
        for (bit, name) in OpcodeFlags::NAMES {
            if info.flags.contains(bit) {
                if !first {
                    s.push(',');
                }
                s.push_str(name);
                first = false;
            }
        }
        s.push_str("] (");
        for (i, op) in info.operands.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            let _ = write!(s, "{}: {}", op.name, op.kind.name());
            match op.param {
                crate::OperandParam::None => {}
                crate::OperandParam::Const(c) => {
                    let _ = write!(s, "({})", c.name());
                }
                crate::OperandParam::Slot(k) => {
                    let _ = write!(s, "({})", k.name());
                }
                crate::OperandParam::FlagMask(m) => {
                    let _ = write!(s, "(0x{m:x})");
                }
            }
        }
        s.push_str(")\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operand::{OperandKind, OperandParam};

    #[test]
    fn table_is_dense_and_round_trips() {
        assert_eq!(OPCODE_COUNT, 0x9d);
        for (i, info) in OPCODES.iter().enumerate() {
            assert_eq!(info.opcode as usize, i);
            assert_eq!(Opcode::from_u8(i as u8), Some(info.opcode));
            assert!(!info.snake.starts_with("r#"), "{}", info.snake);
        }
        for b in OPCODE_COUNT..=255 {
            assert_eq!(Opcode::from_u8(b as u8), None);
        }
        assert_eq!(Opcode::Mod.info().snake, "mod");
        assert_eq!(Opcode::Return.info().snake, "return_");
        assert_eq!(MAX_OPERANDS, 5);
        // ExtraWide CallProperty2: prefix + opcode + 4 registers x 4 + slot x 4.
        assert_eq!(MAX_INSTRUCTION_SIZE, 22);
    }

    /// Consistency rules between flags and operands. The verifier relies on them.
    #[test]
    fn flags_agree_with_operands() {
        for info in OPCODES.iter() {
            let ops = info.operands;
            let has_jump = ops
                .iter()
                .any(|o| matches!(o.kind, OperandKind::JumpFwd | OperandKind::JumpBack));
            assert_eq!(
                has_jump,
                info.flags.contains(OpcodeFlags::JUMP),
                "{}",
                info.name
            );
            let has_table = ops
                .iter()
                .any(|o| o.param == OperandParam::Const(crate::ConstOperand::JumpTable));
            assert_eq!(
                has_table,
                info.flags.contains(OpcodeFlags::SWITCH),
                "{}",
                info.name
            );
            // Only JumpLoop jumps backward, and it is the only interrupt poll.
            let back = ops.iter().any(|o| o.kind == OperandKind::JumpBack);
            assert_eq!(back, info.opcode == Opcode::JumpLoop);
            assert_eq!(
                info.flags.contains(OpcodeFlags::INTERRUPT),
                info.opcode == Opcode::JumpLoop
            );
            // A jump operand, if any, is the first operand.
            for (i, o) in ops.iter().enumerate() {
                if matches!(o.kind, OperandKind::JumpFwd | OperandKind::JumpBack) {
                    assert_eq!(i, 0, "{}", info.name);
                }
            }
            // Lists are followed by their count; counts follow a list.
            for (i, o) in ops.iter().enumerate() {
                let is_list = matches!(o.kind, OperandKind::RegList | OperandKind::RegOutList);
                let next_is_count = ops.get(i + 1).map(|n| n.kind) == Some(OperandKind::RegCount);
                assert_eq!(is_list, next_is_count, "{} operand {i}", info.name);
            }
            // Parameters match kinds.
            for o in ops {
                let ok = match o.kind {
                    OperandKind::Const => matches!(o.param, OperandParam::Const(_)),
                    OperandKind::Slot => matches!(o.param, OperandParam::Slot(_)),
                    OperandKind::Flag8 => {
                        matches!(o.param, OperandParam::FlagMask(m) if m <= 0xff)
                    }
                    OperandKind::Flag16 => matches!(o.param, OperandParam::FlagMask(_)),
                    _ => o.param == OperandParam::None,
                };
                assert!(ok, "{} operand {}", info.name, o.name);
            }
            // Clobber only on opcodes that write the accumulator.
            if info.flags.contains(OpcodeFlags::CLOBBER) {
                assert!(info.acc.writes(), "{}", info.name);
            }
            // Return implies Terminator; Prefix opcodes have no operands.
            if info.flags.contains(OpcodeFlags::RETURN) {
                assert!(
                    info.flags.contains(OpcodeFlags::TERMINATOR),
                    "{}",
                    info.name
                );
            }
            if info.flags.contains(OpcodeFlags::PREFIX) {
                assert!(ops.is_empty());
            }
            assert!(ops.len() <= MAX_OPERANDS);
        }
    }

    #[test]
    fn sizes_and_offsets() {
        let info = Opcode::GetNamedProperty.info();
        assert_eq!(info.size(OperandScale::Single), 4);
        assert_eq!(info.size(OperandScale::Wide), 8);
        assert_eq!(info.size(OperandScale::ExtraWide), 14);
        assert_eq!(info.operand_offset(0, OperandScale::Wide), 1);
        assert_eq!(info.operand_offset(2, OperandScale::Wide), 5);
        // Fixed operands keep their size.
        let info = Opcode::CreateRegExpLiteral.info();
        assert_eq!(info.size(OperandScale::Single), 1 + 1 + 1 + 2);
        assert_eq!(info.size(OperandScale::ExtraWide), 1 + 1 + 4 + 4 + 2);
        // The interpreter DSL computes operand offsets at compile time.
        const OFF: usize = Opcode::GetNamedProperty
            .info()
            .operand_offset(2, OperandScale::ExtraWide);
        const LEN: usize = Opcode::CallProperty2.info().size(OperandScale::Wide);
        assert_eq!((OFF, LEN), (9, 1 + 1 + 5 * 2));
        assert!(!Opcode::TestTypeOf.info().has_scalable_operands());
        assert!(!Opcode::Return.info().has_scalable_operands());
        assert!(Opcode::LdaSmi.info().has_scalable_operands());
    }
}
