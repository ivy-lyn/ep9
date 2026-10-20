//! Runtime entries of the cold blocks (H15: every runtime call is in a cold block).
//!
//! Convention: `fn(cx, fp, pc, values..) -> u64`. All arguments are passed by value, so no
//! handler local escapes and the handler keeps its sibling call. The result is the new
//! accumulator, or 0 if an exception is pending. Entries that switch frames (calls, unwinding)
//! also set `cx.out_fp` and `cx.out_pc`, and never return 0 without doing so.
//!
//! Each entry first records the interpreter state (`Runtime::enter`): the top frame for the
//! GC root scan and the bytecode offset for stack traces.

use core::sync::atomic::Ordering;

use ep9_bytecode::{Constant, RuntimeId};
use ep9_value::Throw;

use crate::call::{self, CallSite};
use crate::conv::{self, BinOp, CompareOp, Res, describe};
use crate::error::{self, FrameRef, not_implemented, stack_overflow, throw_error};
use crate::frame::{HOLE, NULL, TRUE, UNDEFINED, slot};
use crate::interp::{BUDGET, CONSTRUCT_RETURN_STUB, Cx};
use crate::object::{self, attr, is_array, is_function, is_object, is_string, own_define, own_get};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, Site, frame_top, val};
use crate::unit::FunctionInfo;

#[inline(always)]
fn enter(cx: &Cx, fp: *mut u64, pc: *const u8) {
    cx.rt().enter(fp, pc);
}

#[inline(always)]
fn ret(r: Res<u64>) -> u64 {
    match r {
        Ok(v) => {
            debug_assert!(v != 0);
            v
        }
        Err(Throw) => 0,
    }
}

fn info<'a>(fp: *mut u64) -> &'a FunctionInfo {
    FrameRef(fp).info()
}

fn strict(fp: *mut u64) -> bool {
    info(fp).is_strict()
}

/// The current context of the frame `fp`.
fn context(fp: *mut u64) -> u64 {
    // SAFETY: header slot 6 of a live frame.
    unsafe { *fp.offset(slot::CONTEXT) }
}

/// Finds the handler for the pending exception (see `error::unwind`).
#[cold]
#[inline(never)]
pub(crate) fn unwind(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    error::unwind(cx, fp, pc)
}

/// A byte that is no opcode. This is a VM bug (the verifier accepts defined opcodes only), so
/// the run stops at once: no JS handler runs, the frames above the entry frame are dropped.
#[cold]
#[inline(never)]
pub(crate) fn bad_opcode(cx: &mut Cx, _fp: *mut u64, pc: *const u8) -> u64 {
    let rt = cx.rt();
    // SAFETY: `pc` is the byte that the dispatch just read.
    let b = unsafe { *pc };
    let s = object::new_string(rt, &format!("ep9 internal error: bad opcode 0x{b:02x}"));
    rt.throw_value(s)
}

/// The interrupt poll (budget exhausted): resets the budget and handles termination. Returns
/// 0 if the isolate must terminate (the pending exception cannot be caught).
#[cold]
#[inline(never)]
pub(crate) fn interrupt(cx: &mut Cx, _fp: *mut u64) -> u64 {
    cx.budget = BUDGET;
    let rt = cx.rt();
    if rt.terminating.get() || rt.terminate.swap(false, Ordering::AcqRel) {
        rt.terminating.set(true);
        return rt.throw_value(UNDEFINED);
    }
    TRUE
}

/// `interrupt` at a `JumpLoop`.
#[cold]
#[inline(never)]
pub(crate) fn interrupt_at(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    enter(cx, fp, pc);
    interrupt(cx, fp)
}

/// Throws the "not implemented" error of v0.
#[cold]
#[inline(never)]
pub(crate) fn not_impl(cx: &mut Cx, fp: *mut u64, pc: *const u8, what: &'static str) -> u64 {
    enter(cx, fp, pc);
    not_implemented(cx, what);
    0
}

/// Throws a new error of `kind` with a fixed message.
#[cold]
#[inline(never)]
pub(crate) fn throw_msg(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    kind: ErrorKind,
    msg: &'static str,
) -> u64 {
    enter(cx, fp, pc);
    throw_error(cx, kind, msg);
    0
}

/// The message templates that name a binding.
#[derive(Clone, Copy)]
pub(crate) enum NameMsg {
    /// ReferenceError: TDZ read or write.
    Tdz,
    /// ReferenceError: unknown global.
    NotDefined,
}

/// Throws a ReferenceError about the binding `name` (a string value).
#[cold]
#[inline(never)]
pub(crate) fn throw_name(cx: &mut Cx, fp: *mut u64, pc: *const u8, m: NameMsg, name: u64) -> u64 {
    enter(cx, fp, pc);
    throw_name_now(cx, m, name);
    0
}

fn throw_name_now(cx: &mut Cx, m: NameMsg, name: u64) -> Throw {
    let n = object::str_view(name).to_string_lossy();
    let msg = match m {
        NameMsg::Tdz => format!("Cannot access '{n}' before initialization"),
        NameMsg::NotDefined => format!("{n} is not defined"),
    };
    throw_error(cx, ErrorKind::ReferenceError, &msg)
}

/// `throw v` (`rethrow`: a `ReThrow`, which keeps the first throw site).
#[cold]
#[inline(never)]
pub(crate) fn throw(cx: &mut Cx, fp: *mut u64, pc: *const u8, v: u64, rethrow: bool) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if !rethrow {
        let fi = info(fp);
        let site = fi
            .loaded_unit()
            .zip(fi.offset_of(pc).and_then(|o| fi.source_offset(o)))
            .map(|(u, offset)| Site {
                unit: u.index,
                offset,
            });
        rt.shared.throw_site.set(site);
    }
    rt.throw_value(v)
}

// ---- operators -----------------------------------------------------------------------------

#[cold]
#[inline(never)]
pub(crate) fn binary(cx: &mut Cx, fp: *mut u64, pc: *const u8, op: BinOp, l: u64, r: u64) -> u64 {
    enter(cx, fp, pc);
    ret(conv::binary(cx, op, l, r))
}

/// Unary operators and conversions on the accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnOp {
    Inc,
    Dec,
    Negate,
    BitNot,
    ToNumber,
    ToNumeric,
    ToName,
    ToString,
    ToObject,
}

#[cold]
#[inline(never)]
pub(crate) fn unary(cx: &mut Cx, fp: *mut u64, pc: *const u8, op: UnOp, v: u64) -> u64 {
    enter(cx, fp, pc);
    let r = (|| -> Res<u64> {
        Ok(match op {
            UnOp::Inc => conv::num(conv::to_numeric(cx, v)? + 1.0),
            UnOp::Dec => conv::num(conv::to_numeric(cx, v)? - 1.0),
            UnOp::Negate => conv::num(-conv::to_numeric(cx, v)?),
            UnOp::BitNot => conv::num(f64::from(!conv::to_int32_of(cx, v)?)),
            UnOp::ToNumber | UnOp::ToNumeric => conv::num(conv::to_numeric(cx, v)?),
            UnOp::ToName => conv::to_property_key(cx, v)?,
            UnOp::ToString => conv::to_string(cx, v)?,
            UnOp::ToObject => {
                if is_object(v) {
                    v
                } else if v == UNDEFINED || v == NULL {
                    return Err(throw_error(
                        cx,
                        ErrorKind::TypeError,
                        "Cannot convert undefined or null to object",
                    ));
                } else {
                    return Err(not_implemented(cx, "wrapper objects for primitives"));
                }
            }
        })
    })();
    ret(r)
}

/// Comparison operators: `l OP r`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Eq,
    Lt,
    Gt,
    Le,
    Ge,
    InstanceOf,
    In,
}

#[cold]
#[inline(never)]
pub(crate) fn compare(cx: &mut Cx, fp: *mut u64, pc: *const u8, op: CmpOp, l: u64, r: u64) -> u64 {
    enter(cx, fp, pc);
    let b = match op {
        CmpOp::Eq => conv::loose_equals(cx, l, r),
        CmpOp::Lt => conv::compare(cx, CompareOp::Lt, l, r),
        CmpOp::Gt => conv::compare(cx, CompareOp::Gt, l, r),
        CmpOp::Le => conv::compare(cx, CompareOp::Le, l, r),
        CmpOp::Ge => conv::compare(cx, CompareOp::Ge, l, r),
        CmpOp::InstanceOf => property::instance_of(cx, l, r),
        CmpOp::In => property::has(cx, r, l),
    };
    ret(b.map(conv::boolean))
}

// ---- globals and properties ----------------------------------------------------------------

/// `LdaGlobal` / `LdaGlobalInsideTypeof`: the script lexical record first, then the global
/// object (§9.1.1.4.6 GetBindingValue of the global environment, v0).
#[cold]
#[inline(never)]
pub(crate) fn load_global(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    in_typeof: bool,
) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if let Some((v, _)) = own_get(rt.root(Root::Lexical), name) {
        if v == HOLE {
            throw_name_now(cx, NameMsg::Tdz, name);
            return 0;
        }
        return v;
    }
    let g = rt.global();
    let r = (|| -> Res<u64> {
        if !property::has(cx, g, name)? {
            if in_typeof {
                return Ok(UNDEFINED);
            }
            return Err(throw_name_now(cx, NameMsg::NotDefined, name));
        }
        property::get(cx, g, name)
    })();
    ret(r)
}

/// `StaGlobal`: SetMutableBinding of the global environment (v0). Returns the stored value.
#[cold]
#[inline(never)]
pub(crate) fn store_global(cx: &mut Cx, fp: *mut u64, pc: *const u8, name: u64, v: u64) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    let lexical = rt.root(Root::Lexical);
    if let Some((old, a)) = own_get(lexical, name) {
        if old == HOLE {
            throw_name_now(cx, NameMsg::Tdz, name);
            return 0;
        }
        if a & attr::WRITABLE == 0 {
            throw_error(cx, ErrorKind::TypeError, "Assignment to constant variable.");
            return 0;
        }
        object::own_replace(rt, lexical, name, v);
        return v;
    }
    let g = rt.global();
    let s = strict(fp);
    let r = (|| -> Res<u64> {
        if s && !property::has(cx, g, name)? {
            return Err(throw_name_now(cx, NameMsg::NotDefined, name));
        }
        property::set(cx, g, name, v, s)?;
        Ok(v)
    })();
    ret(r)
}

/// Property operations of the `*Property` opcodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PropOp {
    /// `o[k]`, `k` a property-key string.
    GetNamed,
    /// `o[k]`, any key.
    GetKeyed,
    /// `super[k]` with the home object `o` (v0: no receiver for accessors, none exist).
    GetSuper,
    /// `o[k] = v` (strict mode of the function).
    SetNamed,
    SetKeyed,
    /// CreateDataPropertyOrThrow.
    DefineNamed,
    DefineKeyed,
    /// `delete o[k]` in strict code.
    DeleteStrict,
    DeleteSloppy,
}

/// `super[key]` with home object `home` (§13.3.7.3 GetSuperBase, then [[Get]]).
fn super_get(cx: &mut Cx, home: u64, key: u64) -> Res<u64> {
    if !is_object(home) {
        return Err(not_implemented(
            cx,
            "super property access without a home object",
        ));
    }
    let p = object::proto_of(home);
    if p == NULL {
        let msg = format!(
            "Cannot read properties of null (reading '{}')",
            describe(key).trim_matches('"')
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    property::get_from(cx, p, key)
}

/// Runs `op`. Returns the accumulator: the loaded value, the stored value or the boolean.
#[cold]
#[inline(never)]
pub(crate) fn prop(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    op: PropOp,
    o: u64,
    k: u64,
    v: u64,
) -> u64 {
    enter(cx, fp, pc);
    let r = match op {
        PropOp::GetNamed => property::get(cx, o, k),
        PropOp::GetKeyed => property::get_keyed(cx, o, k),
        PropOp::GetSuper => super_get(cx, o, k),
        PropOp::SetNamed => property::set(cx, o, k, v, strict(fp)).map(|_| v),
        PropOp::SetKeyed => property::set_keyed(cx, o, k, v, strict(fp)).map(|_| v),
        PropOp::DefineNamed => define_checked(cx, o, k, v, false).map(|_| v),
        PropOp::DefineKeyed => define_checked(cx, o, k, v, true).map(|_| v),
        PropOp::DeleteStrict => property::delete(cx, o, k, true).map(conv::boolean),
        PropOp::DeleteSloppy => property::delete(cx, o, k, false).map(conv::boolean),
    };
    ret(r)
}

fn define_checked(cx: &mut Cx, o: u64, k: u64, v: u64, keyed: bool) -> Res<()> {
    if !is_object(o) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cannot define a property on a non-object",
        ));
    }
    if keyed {
        property::define_keyed(cx, o, k, v)
    } else {
        property::define(cx, o, k, v)
    }
}

// ---- calls ---------------------------------------------------------------------------------

/// The call or construct instruction at `pc` (all `Call*` and `Construct*` opcodes). `acc` is
/// `new.target` for `Construct`. A bytecode callee gets a new frame and the run continues in it
/// (no Rust recursion); a host callee runs here. Sets `cx.out_fp`/`cx.out_pc`.
#[cold]
#[inline(never)]
pub(crate) fn call(cx: &mut Cx, fp: *mut u64, pc: *const u8, acc: u64) -> u64 {
    enter(cx, fp, pc);
    if cx.budget <= 0 && interrupt(cx, fp) == 0 {
        return error::unwind(cx, fp, pc);
    }
    let site = call::decode_call(fp, pc);
    match do_call(cx, fp, &site, acc) {
        Ok(v) => v,
        Err(Throw) => error::unwind(cx, fp, pc),
    }
}

fn do_call(cx: &mut Cx, fp: *mut u64, s: &CallSite, acc: u64) -> Res<u64> {
    let rt = cx.rt();
    // The argument values stay rooted in the caller's registers while this copy exists.
    // SAFETY: the offsets come from register operands of verified code in the frame `fp`.
    let mut args: Vec<u64> = s.args.iter().map(|&o| unsafe { *fp.offset(o) }).collect();
    // A string spread gets a new array of code point strings; this keeps it (and so the
    // strings in `args`) alive until the call returns.
    let mut keep = KeepAlive(UNDEFINED);
    if s.spread {
        let last = args.pop().unwrap_or(UNDEFINED);
        let list = if is_array(last) {
            // The array stays rooted in the caller's last argument register.
            last
        } else if is_string(last) {
            keep.0 = object::new_array(rt, 0);
            push_code_points(rt, keep.0, 0, last);
            keep.0
        } else {
            return Err(not_iterable(cx, last));
        };
        for i in 0..object::array_len(list) {
            args.push(object::array_get(list, i).unwrap_or(UNDEFINED));
        }
    }
    let Ok(argc) = u32::try_from(args.len()) else {
        return stack_overflow(cx);
    };
    let callee = s.callee;
    if s.construct {
        if !object::is_constructor(callee) {
            let msg = format!("{} is not a constructor", describe(callee));
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        // `new.target` is the accumulator if it is a constructor (BytecodeGen sets it), else
        // the callee. v0 keeps it in the callee frame's feedback slot (push_frame).
        let new_target = if object::is_constructor(acc) {
            acc
        } else {
            callee
        };
        let fi = object::function_info(callee);
        if fi.host.is_some() {
            let r = call::call_host(
                cx,
                callee,
                fi,
                UNDEFINED,
                argc,
                |i| args[i as usize],
                new_target,
                fp,
                s.next_pc,
            )?;
            cx.out_fp = fp;
            cx.out_pc = s.next_pc;
            return Ok(r);
        }
        if !runs_bytecode(fi) {
            return Err(not_implemented(cx, "calling code of this dispatch kind"));
        }
        let this = if fi.is_derived() {
            HOLE
        } else {
            // GC point; `this` then lives in a Rust local until the stub frame holds it.
            call::ordinary_create_from_constructor(cx, new_target)?
        };
        // SAFETY: `fp` is the live top frame; `this` is valid Value bits.
        let Some(stub) = (unsafe { call::push_construct_stub(cx, fp, this, callee, s.next_pc) })
        else {
            return stack_overflow(cx);
        };
        // SAFETY: `stub` is the new top frame; the callee frame goes right above it.
        let nfp = unsafe {
            call::push_frame(
                frame_top(stub),
                cx.stack_limit,
                fi,
                callee,
                this,
                argc,
                |i| args[i as usize],
                stub,
                CONSTRUCT_RETURN_STUB.as_ptr(),
                object::function_context(callee),
                new_target,
            )
        };
        let Some(nfp) = nfp else {
            return stack_overflow(cx);
        };
        cx.budget -= 1;
        cx.out_fp = nfp;
        cx.out_pc = fi.code;
        return Ok(UNDEFINED);
    }
    if !is_function(callee) {
        let msg = format!("{} is not a function", describe(callee));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let fi = object::function_info(callee);
    if fi.host.is_some() {
        let r = call::call_host(
            cx,
            callee,
            fi,
            s.this,
            argc,
            |i| args[i as usize],
            UNDEFINED,
            fp,
            s.next_pc,
        )?;
        cx.out_fp = fp;
        cx.out_pc = s.next_pc;
        return Ok(r);
    }
    if fi.is_class_constructor() {
        let msg = format!(
            "Class constructor {} cannot be invoked without 'new'",
            fi.name
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    if !runs_bytecode(fi) {
        return Err(not_implemented(cx, "calling code of this dispatch kind"));
    }
    let this = call::bind_this(fi, s.this, rt.global());
    // SAFETY: `frame_top(fp)` is the first free slot above the live top frame.
    let nfp = unsafe {
        call::push_frame(
            frame_top(fp),
            cx.stack_limit,
            fi,
            callee,
            this,
            argc,
            |i| args[i as usize],
            fp,
            s.next_pc,
            object::function_context(callee),
            UNDEFINED,
        )
    };
    let Some(nfp) = nfp else {
        return stack_overflow(cx);
    };
    cx.budget -= 1;
    cx.out_fp = nfp;
    cx.out_pc = fi.code;
    Ok(UNDEFINED)
}

#[inline]
fn runs_bytecode(fi: &FunctionInfo) -> bool {
    fi.is_bytecode() && fi.slot.kind_byte() == crate::dispatch::DispatchKind::Interp as u8
}

// ---- CallRuntime ---------------------------------------------------------------------------

/// `CallRuntime(id, args)`. `first` is the slot offset of the first argument register.
#[cold]
#[inline(never)]
pub(crate) fn call_runtime(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    id: u16,
    first: isize,
    argc: u32,
) -> u64 {
    enter(cx, fp, pc);
    let a = |i: u32| -> u64 {
        if i < argc {
            // SAFETY: the verifier checked that the register list lies in the frame `fp`.
            unsafe { *fp.offset(first + i as isize) }
        } else {
            UNDEFINED
        }
    };
    use RuntimeId as Id;
    let r: Res<u64> = match RuntimeId::from_u16(id) {
        Some(Id::ThrowConstAssignError) => Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Assignment to constant variable.",
        )),
        Some(Id::ThrowIteratorResultNotAnObject) => {
            let msg = format!("Iterator result {} is not an object", describe(a(0)));
            Err(throw_error(cx, ErrorKind::TypeError, &msg))
        }
        Some(Id::ThrowSymbolIteratorInvalid) => Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Result of the Symbol.iterator method is not an object",
        )),
        Some(Id::CreateIterResultObject) => {
            let rt = cx.rt();
            let o = object::new_plain_object(rt);
            own_define(rt, o, rt.atom(Atom::ValueKey), a(0), attr::DEFAULT);
            let done = conv::boolean(conv::to_boolean(a(1)));
            own_define(rt, o, rt.atom(Atom::Done), done, attr::DEFAULT);
            Ok(o)
        }
        Some(Id::DefineClass) => define_class(cx, a(0), a(1)),
        Some(Id::DeclareGlobals) => {
            let fi = info(fp);
            match fi.loaded_unit() {
                Some(u) => {
                    crate::realm::declare_globals(cx, &u.unit, &|i| u.string(i)).map(|_| UNDEFINED)
                }
                None => Ok(UNDEFINED),
            }
        }
        Some(Id::InitializeGlobal) => initialize_global(cx, a(0), a(1)),
        // v0: there is no %eval%, so the callee is never %eval% and the call goes ahead.
        Some(Id::ResolvePossiblyDirectEval) => Ok(a(0)),
        Some(Id::CopyDataProperties) => {
            copy_data_properties(cx, a(0), a(1), UNDEFINED).map(|_| a(0))
        }
        Some(Id::CopyDataPropertiesExcluding) => (|| -> Res<u64> {
            let ex = key_array(cx, fp, first + 1, argc.saturating_sub(1))?;
            let o = object::new_plain_object(cx.rt());
            copy_data_properties(cx, o, a(0), ex)?;
            Ok(o)
        })(),
        Some(Id::SetPrototypeInLiteral) => {
            let (o, p) = (a(0), a(1));
            if is_object(o) && (is_object(p) || p == NULL) {
                // A cycle leaves the prototype as it is (as V8 does for `__proto__`).
                object::set_proto(cx.rt(), o, p);
            }
            Ok(UNDEFINED)
        }
        Some(Id::DefineClassMethod) => {
            let (home, k, f) = (a(0), a(1), a(2));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                if !is_object(home) {
                    return Err(not_implemented(cx, "class method on a non-object"));
                }
                if is_function(f) {
                    object::set_home_object(cx.rt(), f, home);
                }
                own_define(cx.rt(), home, key, f, attr::HIDDEN);
                Ok(UNDEFINED)
            })()
        }
        // MakeMethod(F, homeObject) (§10.2.7) for object literal methods that use `super`.
        Some(Id::SetHomeObject) => {
            let (f, home) = (a(0), a(1));
            if is_function(f) {
                object::set_home_object(cx.rt(), f, home);
            }
            Ok(UNDEFINED)
        }
        Some(Id::GetHomeObject) => {
            let f = a(0);
            Ok(if is_function(f) {
                object::home_object(f)
            } else {
                UNDEFINED
            })
        }
        // `super[key]` (§13.3.7.1 MakeSuperPropertyReference + GetValue). v0: no accessors,
        // so the receiver is not needed.
        Some(Id::LoadKeyedFromSuper) => {
            let (home, k) = (a(1), a(2));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                super_get(cx, home, key)
            })()
        }
        // `super[key] = value` (§10.1.9.2 OrdinarySetWithOwnDescriptor with receiver = this).
        // v0: no accessors, so the store goes to the receiver (strict: super is only in
        // methods, and a failed store throws in strict code).
        Some(Id::StoreToSuper) => {
            let (receiver, home, k, v) = (a(0), a(1), a(2), a(3));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                if !is_object(home) {
                    return Err(not_implemented(
                        cx,
                        "super property store without a home object",
                    ));
                }
                property::set(cx, receiver, key, v, strict(fp))?;
                Ok(v)
            })()
        }
        Some(other) => Err(not_implemented(cx, other.info().name)),
        None => Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: unknown runtime function",
        )),
    };
    ret(r)
}

/// `DefineClass(constructor, heritage)` (§15.7.14 ClassDefinitionEvaluation steps 5-8, 14-16).
/// Returns the prototype.
fn define_class(cx: &mut Cx, ctor: u64, heritage: u64) -> Res<u64> {
    let rt = cx.rt();
    if !is_function(ctor) {
        return Err(not_implemented(
            cx,
            "DefineClass without a constructor function",
        ));
    }
    let (proto_parent, ctor_parent) = if heritage == HOLE {
        (
            rt.root(Root::ObjectPrototype),
            rt.root(Root::FunctionPrototype),
        )
    } else if heritage == NULL {
        (NULL, rt.root(Root::FunctionPrototype))
    } else if !object::is_constructor(heritage) {
        let msg = format!(
            "Class extends value {} is not a constructor or null",
            describe(heritage)
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    } else {
        let p = property::get(cx, heritage, rt.atom(Atom::Prototype))?;
        if !is_object(p) && p != NULL {
            let msg = format!(
                "Class extends value does not have valid prototype property {}",
                describe(p)
            );
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        (p, heritage)
    };
    let proto = object::new_object(rt, proto_parent);
    own_define(rt, proto, rt.atom(Atom::Constructor), ctor, attr::HIDDEN);
    own_define(rt, ctor, rt.atom(Atom::Prototype), proto, 0);
    // The constructor is a method of the prototype: `super.x` inside it reads from the parent
    // prototype (§15.7.14 step 14, MakeMethod(F, proto)).
    object::set_home_object(rt, ctor, proto);
    if !object::set_proto(rt, ctor, ctor_parent) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cyclic __proto__ value",
        ));
    }
    Ok(proto)
}

/// `InitializeGlobal(name, value)` (v0): initializes a lexical binding, or sets the global
/// object property of a function declaration.
fn initialize_global(cx: &mut Cx, name: u64, v: u64) -> Res<u64> {
    let rt = cx.rt();
    if !is_string(name) {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: InitializeGlobal name is not a string",
        ));
    }
    let lexical = rt.root(Root::Lexical);
    if own_get(lexical, name).is_some() {
        object::own_replace(rt, lexical, name, v);
        return Ok(UNDEFINED);
    }
    let g = rt.global();
    if !object::own_replace(rt, g, name, v) {
        own_define(rt, g, name, v, attr::WRITABLE | attr::ENUMERABLE);
    }
    Ok(UNDEFINED)
}

/// CopyDataProperties(target, source, excluded) (§7.3.25, v0: own enumerable data properties,
/// array elements and string characters). `excluded` is an array of property keys or
/// undefined. No JS runs here, so `source` keeps its keys and values alive; fresh index keys
/// live in Rust locals (conservative scan, H10) until `define` stores them.
fn copy_data_properties(cx: &mut Cx, target: u64, source: u64, excluded: u64) -> Res<()> {
    let rt = cx.rt();
    if source == UNDEFINED || source == NULL || !is_object(target) {
        return Ok(());
    }
    let skip = |k: u64| {
        is_array(excluded)
            && (0..object::array_len(excluded))
                .any(|j| object::array_get(excluded, j).is_some_and(|e| object::key_eq(e, k)))
    };
    if is_string(source) {
        for i in 0..object::str_len(source) {
            let k = conv::number_string(cx, i as f64);
            if skip(k) {
                continue;
            }
            let u = [object::str_view(source).at(i)];
            let c = object::new_string_utf16(rt, &u);
            property::define(cx, target, k, c)?;
        }
        return Ok(());
    }
    if !is_object(source) {
        return Ok(());
    }
    if is_array(source) {
        for i in 0..object::array_len(source) {
            if let Some(v) = object::array_get(source, i) {
                let k = conv::number_string(cx, f64::from(i));
                if !skip(k) {
                    property::define(cx, target, k, v)?;
                }
            }
        }
    }
    let mut i = 0;
    while i < object::own_count(source) {
        let (k, v, a) = object::own_entry(source, i);
        if a & attr::ENUMERABLE != 0 && !skip(k) {
            property::define(cx, target, k, v)?;
        }
        i += 1;
    }
    Ok(())
}

/// The property keys of `count` registers from slot `first` of `fp`, as a new array.
fn key_array(cx: &mut Cx, fp: *mut u64, first: isize, count: u32) -> Res<u64> {
    let a = object::new_array(cx.rt(), count as usize);
    for i in 0..count {
        // SAFETY: the registers are part of a verified register list of the frame `fp`.
        let k = unsafe { *fp.offset(first + i as isize) };
        let key = conv::to_property_key(cx, k)?;
        object::array_set(cx.rt(), a, i, key);
    }
    Ok(a)
}

// ---- closures, contexts, literals ----------------------------------------------------------

/// `CreateClosure(function)`: a function cell for nested function `k` with the current context.
#[cold]
#[inline(never)]
pub(crate) fn create_closure(cx: &mut Cx, fp: *mut u64, pc: *const u8, k: u32) -> u64 {
    enter(cx, fp, pc);
    let fi = info(fp);
    let Constant::Function(id) = fi.constant_entry(k) else {
        return throw_msg_now(cx, "CreateClosure constant is not a function");
    };
    let u = fi.loaded_unit().expect("a bytecode function");
    let target: *const FunctionInfo = &u.infos[id.index()];
    object::new_function(cx.rt(), target, context(fp))
}

fn throw_msg_now(cx: &mut Cx, msg: &str) -> u64 {
    throw_error(cx, ErrorKind::Error, &format!("ep9 internal error: {msg}"));
    0
}

/// The kinds of `Create*Context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CtxKind {
    Function,
    Block,
    Catch,
    Eval,
}

/// `Create{Function,Block,Catch,Eval}Context`: a new context whose previous context is the
/// current one. `slots` is the slot count operand (0 for block and catch: the scope says).
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_context(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    kind: CtxKind,
    k: u32,
    slots: u32,
    exception: u64,
) -> u64 {
    enter(cx, fp, pc);
    let fi = info(fp);
    let Constant::Scope(id) = fi.constant_entry(k) else {
        return throw_msg_now(cx, "context constant is not a scope");
    };
    let u = fi.loaded_unit().expect("a bytecode function");
    let scope_len = u.unit.scopes[id.index()].slots.len();
    let len = match kind {
        CtxKind::Function | CtxKind::Eval => slots as usize,
        CtxKind::Block => scope_len,
        CtxKind::Catch => scope_len.max(1),
    };
    let c = object::new_context(cx.rt(), context(fp), id.0 as i32, len);
    if kind == CtxKind::Catch {
        object::context_set(cx.rt(), c, 0, exception);
    }
    c
}

/// A context slot access that the fast path rejected: a VM or BytecodeGen bug (bad depth or
/// slot), reported as an internal error.
#[cold]
#[inline(never)]
pub(crate) fn bad_context(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    enter(cx, fp, pc);
    throw_msg_now(cx, "context slot out of range")
}

/// Literal kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lit {
    Array,
    Object,
    Template,
    EmptyArray,
    EmptyObject,
}

/// `Create{Empty,}ArrayLiteral`, `Create{Empty,}ObjectLiteral`, `GetTemplateObject` (the
/// boilerplate or description is constant `k`).
#[cold]
#[inline(never)]
pub(crate) fn literal(cx: &mut Cx, fp: *mut u64, pc: *const u8, kind: Lit, k: u32) -> u64 {
    enter(cx, fp, pc);
    let r = match kind {
        Lit::Array => array_literal(cx, fp, k),
        Lit::Object => object_literal(cx, fp, k),
        Lit::Template => template_object(cx, fp, k),
        Lit::EmptyArray => Ok(object::new_array(cx.rt(), 0)),
        Lit::EmptyObject => Ok(object::new_plain_object(cx.rt())),
    };
    ret(r)
}

fn array_literal(cx: &mut Cx, fp: *mut u64, k: u32) -> Res<u64> {
    array_from_boilerplate(cx, info(fp), k)
}

fn object_literal(cx: &mut Cx, fp: *mut u64, k: u32) -> Res<u64> {
    object_from_boilerplate(cx, info(fp), k)
}

/// The value of boilerplate element constant `ci`: a primitive constant, or a new array or
/// object for a nested boilerplate (`[[1, 2]]`, `{a: {b: 1}}`; the pool refers to the inner
/// boilerplate by index). `None` = an elision (the hole).
fn boilerplate_value(cx: &mut Cx, fi: &FunctionInfo, ci: u32) -> Res<Option<u64>> {
    match fi.constant_entry(ci) {
        Constant::ArrayBoilerplate(_) => array_from_boilerplate(cx, fi, ci).map(Some),
        Constant::ObjectBoilerplate(_) => object_from_boilerplate(cx, fi, ci).map(Some),
        Constant::BigInt(_) => Err(not_implemented(cx, "BigInt values")),
        _ => {
            let v = fi.constant(ci);
            Ok((v != HOLE).then_some(v))
        }
    }
}

fn array_from_boilerplate(cx: &mut Cx, fi: &FunctionInfo, k: u32) -> Res<u64> {
    let rt = cx.rt();
    // Nested boilerplates recurse once per literal level (bounded by the front-end depth cap).
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let Constant::ArrayBoilerplate(elems) = fi.constant_entry(k) else {
        return Err(not_implemented(cx, "array literal constant"));
    };
    let n = elems.len();
    // `a` is a Rust local: the conservative stack scan keeps it alive across the nested
    // allocations below.
    let a = object::new_array(rt, n);
    for (i, e) in elems.iter().enumerate() {
        let Some(ci) = e else { continue };
        if let Some(v) = boilerplate_value(cx, fi, ci.0)? {
            object::array_set(rt, a, i as u32, v);
        }
    }
    object::array_set_length(rt, a, n as u32);
    Ok(a)
}

fn object_from_boilerplate(cx: &mut Cx, fi: &FunctionInfo, k: u32) -> Res<u64> {
    let rt = cx.rt();
    if !rt.native_stack_ok() {
        return stack_overflow(cx);
    }
    let Constant::ObjectBoilerplate(props) = fi.constant_entry(k) else {
        return Err(not_implemented(cx, "object literal constant"));
    };
    let o = object::new_plain_object(rt);
    for (kc, vc) in props.iter() {
        let key = conv::to_property_key(cx, fi.constant(kc.0))?;
        let v = match vc {
            Some(c) => boilerplate_value(cx, fi, c.0)?.unwrap_or(UNDEFINED),
            None => UNDEFINED,
        };
        property::define(cx, o, key, v)?;
    }
    Ok(o)
}

/// GetTemplateObject (§13.2.8.4): one frozen array per site (v0: not frozen), cached per
/// function and constant.
fn template_object(cx: &mut Cx, fp: *mut u64, k: u32) -> Res<u64> {
    let rt = cx.rt();
    let fi = info(fp);
    let key = (core::ptr::from_ref(fi).addr(), k);
    if let Some(h) = rt.templates.borrow().get(&key) {
        return Ok(rt.heap.global(*h).to_bits());
    }
    let Constant::TemplateObject(parts) = fi.constant_entry(k) else {
        return Err(not_implemented(cx, "template object constant"));
    };
    let u = fi.loaded_unit().expect("a bytecode function");
    let cooked = object::new_array(rt, parts.len());
    for (i, (_, c)) in parts.iter().enumerate() {
        let v = c.map_or(UNDEFINED, |s| u.string(s.index()));
        object::array_set(rt, cooked, i as u32, v);
    }
    object::array_set_length(rt, cooked, parts.len() as u32);
    let raw = object::new_array(rt, parts.len());
    for (i, (r, _)) in parts.iter().enumerate() {
        object::array_set(rt, raw, i as u32, u.string(r.index()));
    }
    object::array_set_length(rt, raw, parts.len() as u32);
    own_define(rt, cooked, rt.atom(Atom::Raw), raw, 0);
    let h = rt.heap.new_global(val(cooked));
    rt.templates.borrow_mut().insert(key, h);
    Ok(cooked)
}

/// `CloneObject(source)`: `{...source}`.
#[cold]
#[inline(never)]
pub(crate) fn clone_object(cx: &mut Cx, fp: *mut u64, pc: *const u8, source: u64) -> u64 {
    enter(cx, fp, pc);
    let o = object::new_plain_object(cx.rt());
    ret(copy_data_properties(cx, o, source, UNDEFINED).map(|_| o))
}

/// `CreateArrayFromIterable` (v0: arrays and strings, the only iterables without symbols).
#[cold]
#[inline(never)]
pub(crate) fn array_from_iterable(cx: &mut Cx, fp: *mut u64, pc: *const u8, v: u64) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if is_string(v) {
        let a = object::new_array(rt, 0);
        push_code_points(rt, a, 0, v);
        return a;
    }
    if !is_array(v) {
        return ret(Err(not_iterable(cx, v)));
    }
    let n = object::array_len(v);
    let a = object::new_array(rt, n as usize);
    for i in 0..n {
        object::array_set(rt, a, i, object::array_get(v, i).unwrap_or(UNDEFINED));
    }
    object::array_set_length(rt, a, n);
    a
}

/// The TypeError of GetIterator for a value without `@@iterator` (v0: not an array or string).
fn not_iterable(cx: &mut Cx, v: u64) -> Throw {
    let msg = format!("{} is not iterable", describe(v));
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// Appends the code points of the string `s` (one string each, as the string iterator yields
/// them, §22.1.5.2.1) to the array `out` from index `start`. `out` must be rooted by the
/// caller (a Rust local). Returns the new length.
fn push_code_points(rt: &crate::runtime::Runtime, out: u64, start: u32, s: u64) -> u32 {
    let units: Vec<u16> = object::str_view(s).to_utf16();
    let mut n = start;
    let mut i = 0;
    while i < units.len() {
        let pair = (0xD800..0xDC00).contains(&units[i])
            && i + 1 < units.len()
            && (0xDC00..0xE000).contains(&units[i + 1]);
        let len = if pair { 2 } else { 1 };
        let cp = object::new_string_utf16(rt, &units[i..i + len]);
        object::array_set(rt, out, n, cp);
        n += 1;
        i += len;
    }
    object::array_set_length(rt, out, n);
    n
}

/// Keeps a cell in a stack slot until the end of the scope, so the conservative stack scan
/// sees it (for a cell that only a Rust `Vec` would hold otherwise).
struct KeepAlive(u64);

impl Drop for KeepAlive {
    fn drop(&mut self) {
        core::hint::black_box(&self.0);
    }
}

/// `GetIterator(obj)` (§7.4.3) for v0: the built-in iterator of an array or a string.
#[cold]
#[inline(never)]
pub(crate) fn get_iterator(cx: &mut Cx, fp: *mut u64, pc: *const u8, o: u64) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if is_array(o) || is_string(o) {
        return object::new_iterator(rt, rt.root(Root::ArrayIteratorPrototype), o);
    }
    let msg = format!("{} is not iterable", describe(o));
    ret(Err(throw_error(cx, ErrorKind::TypeError, &msg)))
}

/// `CreateMappedArguments`, `CreateUnmappedArguments` (`rest = false`) and
/// `CreateRestParameter` (`rest = true`). v0: the arguments object is an Array, and mapped
/// arguments do not alias the parameters.
#[cold]
#[inline(never)]
pub(crate) fn arguments(cx: &mut Cx, fp: *mut u64, pc: *const u8, rest: bool) -> u64 {
    enter(cx, fp, pc);
    let f = FrameRef(fp);
    let argc = f.argc();
    let from = if rest { info(fp).parameter_count } else { 0 };
    let rt = cx.rt();
    let n = argc.saturating_sub(from);
    let a = object::new_array(rt, n as usize);
    for i in 0..n {
        object::array_set(rt, a, i, f.arg(from + i));
    }
    object::array_set_length(rt, a, n);
    a
}

// ---- for-in --------------------------------------------------------------------------------

/// `ForInEnumerate(receiver)`: the key array (v0: the cache type and the cache array are both
/// this array).
#[cold]
#[inline(never)]
pub(crate) fn for_in_enumerate(cx: &mut Cx, fp: *mut u64, pc: *const u8, receiver: u64) -> u64 {
    enter(cx, fp, pc);
    ret(property::for_in_keys(cx, receiver))
}

/// `ForInNext`: key `index` of the cache array, or undefined if the receiver lost it.
#[cold]
#[inline(never)]
pub(crate) fn for_in_next(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    receiver: u64,
    keys: u64,
    index: u64,
) -> u64 {
    enter(cx, fp, pc);
    let i = val(index).as_int32().unwrap_or(-1);
    if !is_array(keys) || i < 0 {
        return throw_msg_now(cx, "ForInNext state is broken");
    }
    let Some(k) = object::array_get(keys, i as u32) else {
        return UNDEFINED;
    };
    if !is_object(receiver) {
        return k;
    }
    ret(property::has(cx, receiver, k).map(|h| if h { k } else { UNDEFINED }))
}

// ---- misc ----------------------------------------------------------------------------------

/// `SwitchOnSmi(table, base)` with an int32 accumulator: the jump target, or the next
/// instruction if the value is outside the table.
#[cold]
#[inline(never)]
pub(crate) fn switch_target(
    fp: *mut u64,
    pc: *const u8,
    next: *const u8,
    k: u32,
    base: i32,
    v: i32,
) -> *const u8 {
    let fi = info(fp);
    let Constant::JumpTable(t) = fi.constant_entry(k) else {
        return next;
    };
    let i = i64::from(v) - i64::from(base);
    if i < 0 || i >= t.len() as i64 {
        return next;
    }
    pc.wrapping_add(t[i as usize] as usize)
}

/// `TypeOf`: the type string of `v`.
#[cold]
#[inline(never)]
pub(crate) fn type_of(cx: &Cx, v: u64) -> u64 {
    cx.rt().atom(conv::type_of(v))
}

/// ToBoolean for values that the fast path does not handle.
#[cold]
#[inline(never)]
pub(crate) fn to_boolean(v: u64) -> bool {
    conv::to_boolean(v)
}

/// `TestEqualStrict` for values that the fast path does not handle.
#[cold]
#[inline(never)]
pub(crate) fn strict_equals(l: u64, r: u64) -> bool {
    conv::strict_equals(l, r)
}

/// `ThrowIfNotSuperConstructor(constructor)` failure.
#[cold]
#[inline(never)]
pub(crate) fn not_super_constructor(cx: &mut Cx, fp: *mut u64, pc: *const u8, ctor: u64) -> u64 {
    enter(cx, fp, pc);
    let msg = format!(
        "Super constructor {} of anonymous class is not a constructor",
        describe(ctor)
    );
    throw_error(cx, ErrorKind::TypeError, &msg);
    0
}
