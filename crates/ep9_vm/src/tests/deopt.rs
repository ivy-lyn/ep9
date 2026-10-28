//! Tests of the deopt core (`deopt.rs`) (owner M5-P1 SPIKEC-5; H29, V10-D29, V10-D30): the H29
//! byte form and its refusals with the list widths (w17 O4: fill each list, then the next
//! value and tag 15 are refused), the frame rebuild on a synthetic VM frame and machine
//! state, and the side table's counters. No `jit` and no `spike-native`: they run in the
//! jitless build (C-10). The native glue (`rt_deopt`) is tested in `tests::native` and by
//! Spike C.

use ep9_bytecode::Register;
use ep9_value::Value;

use crate::deopt::{
    CodeMeta, DUMP_REGS, ExitClass, ExitState, ExitValue, FrameShape, Loc, Record, RecordTag, Repr,
    Resume, SideTable, Table, Target, form, rebuild,
};
use crate::frame::{HEADER, int32_bits, slot};

fn value(target: Target, repr: Repr, loc: Loc) -> ExitValue {
    ExitValue { target, repr, loc }
}

fn exit(pc: u32, bytecode: u32, counter: u32, values: Vec<ExitValue>) -> Record {
    Record {
        tag: RecordTag::Exit,
        class: ExitClass::Speculation,
        pc,
        bytecode,
        counter,
        feedback: None,
        values,
    }
}

/// A table with every location tag, every repr, the accumulator, a parameter and a local
/// target, a feedback slot, an environment-class record and a poll and a call record.
fn sample() -> Table {
    let r = |i| Target::Reg(Register::local(i));
    Table {
        consts: vec![
            int32_bits(7),
            Value::UNDEFINED.to_bits(),
            Value::EMPTY.to_bits(),
        ],
        records: vec![
            Record {
                feedback: Some(4),
                ..exit(
                    40,
                    17,
                    0,
                    vec![
                        value(r(0), Repr::Int32, Loc::Reg(19)),
                        value(
                            Target::Reg(Register::parameter(1)),
                            Repr::Tagged,
                            Loc::Frame(-88),
                        ),
                        value(Target::Acc, Repr::Float64, Loc::Stack(104)),
                        value(r(3), Repr::Bool, Loc::Reg(4)),
                        value(r(2), Repr::Tagged, Loc::Const(2)),
                    ],
                )
            },
            Record {
                class: ExitClass::Environment,
                ..exit(44, 0, 2, vec![value(r(0), Repr::Tagged, Loc::Const(1))])
            },
            Record {
                tag: RecordTag::Poll,
                ..exit(48, 30, 1, vec![])
            },
            Record {
                tag: RecordTag::Call,
                ..exit(
                    4000,
                    31,
                    3,
                    vec![value(Target::Acc, Repr::Tagged, Loc::Reg(0))],
                )
            },
        ],
    }
}

#[test]
fn the_byte_form_round_trips() {
    let t = sample();
    let b = t.encode();
    assert_eq!(Table::decode(&b), Ok(t.clone()));
    assert_eq!(t.record_at(44).map(|r| r.counter), Some(2));
    assert!(t.record_at(45).is_none());
    // Every cut of the table is refused, and so is a byte after it.
    for n in 0..b.len() {
        assert!(
            Table::decode(&b[..n]).is_err(),
            "a cut at {n} of {}",
            b.len()
        );
    }
    let mut long = b.clone();
    long.push(0);
    let e = Table::decode(&long).unwrap_err();
    assert!(e.contains("1 bytes after the last record"), "{e}");
    // The empty table.
    assert_eq!(Table::decode(&[0, 0]), Ok(Table::default()));
}

/// The bytes of one record of a 1-record table with no const: a header byte, then pc 8,
/// bytecode 0, counter 0, no feedback, no inline frame, and the value bytes `values`.
fn one_record(header: u8, inline: u8, values: &[&[u8]]) -> Vec<u8> {
    let mut b = vec![1, 0, header, 8, 0, 0, 0, inline, values.len() as u8];
    for v in values {
        b.extend_from_slice(v);
    }
    b
}

/// The value bytes of `tag` with the accumulator as target and payload `payload`.
fn acc_value(tag: u8, payload: u8) -> Vec<u8> {
    vec![tag | form::ACC, payload]
}

/// The H29 lists, filled to their last legal value: the next value and tag 15 (the top of the
/// 4-bit field, never the next free value, H29 Freeze table) are refused, and so is every
/// reserved bit.
#[test]
fn the_list_widths_refuse_the_next_value_and_tag_15() {
    // Record tags: 0 exit, 1 poll, 2 call.
    for tag in 0..form::RECORD_TAGS {
        Table::decode(&one_record(tag, 0, &[])).unwrap_or_else(|e| panic!("record tag {tag}: {e}"));
    }
    for tag in [form::RECORD_TAGS, 15] {
        let e = Table::decode(&one_record(tag, 0, &[])).unwrap_err();
        assert!(e.contains(&format!("unknown record tag {tag}")), "{e}");
    }
    // The exit class bit is a value, not a reserved bit; bits 5-7 are reserved.
    Table::decode(&one_record(form::ENVIRONMENT, 0, &[])).expect("environment class");
    for bit in [0x20, 0x40, 0x80] {
        let e = Table::decode(&one_record(bit, 0, &[])).unwrap_err();
        assert!(e.contains("reserved bits 5-7"), "{e}");
    }
    // Inline frames: none is built.
    let e = Table::decode(&one_record(0, 1, &[])).unwrap_err();
    assert!(e.contains("1 inline frames"), "{e}");
    // Location tags: 0 reg, 1 stack, 2 const-index, 3 VM frame slot (payload 0 is legal for
    // each but const-index, which needs a const: the table below has none, so it is refused
    // for its index, not its tag).
    for loc in [form::LOC_REG, form::LOC_STACK, form::LOC_FRAME] {
        Table::decode(&one_record(0, 0, &[&acc_value(loc, 0)]))
            .unwrap_or_else(|e| panic!("location tag {loc}: {e}"));
    }
    let e = Table::decode(&one_record(0, 0, &[&acc_value(form::LOC_CONST, 0)])).unwrap_err();
    assert!(e.contains("const 0 of 0"), "{e}");
    assert_eq!(form::LOC_TAGS, form::LOC_FRAME + 1);
    for loc in [form::LOC_TAGS, 15] {
        let e = Table::decode(&one_record(0, 0, &[&acc_value(loc, 0)])).unwrap_err();
        assert!(e.contains(&format!("unknown location tag {loc}")), "{e}");
    }
    // Reprs: 0 Tagged, 1 Int32, 2 Bool, 3 Float64 in a 3-bit field; 4 and 7 are refused.
    for repr in 0..form::REPRS {
        let tag = repr << form::REPR_SHIFT;
        Table::decode(&one_record(0, 0, &[&acc_value(tag, 0)]))
            .unwrap_or_else(|e| panic!("repr {repr}: {e}"));
    }
    for repr in [form::REPRS, 7] {
        let tag = repr << form::REPR_SHIFT;
        let e = Table::decode(&one_record(0, 0, &[&acc_value(tag, 0)])).unwrap_err();
        assert!(e.contains(&format!("unknown repr {repr}")), "{e}");
    }
}

/// Each malformed table is refused with its reason.
#[test]
fn the_reader_refuses_what_the_form_does_not_allow() {
    let refused = |t: &Table, why: &str| {
        let e = Table::decode(&t.encode()).unwrap_err();
        assert!(e.contains(why), "{why}: {e}");
    };
    let t = sample();
    // A cell, and bits no Value has, in the const list.
    let mut c = t.clone();
    c.consts[0] = 0x1000;
    refused(&c, "is not an immediate Value");
    c.consts[0] = 0x0001_0000_0000_0003; // bits no Value has (not a cell either)
    assert!(!Value::is_valid_bits(c.consts[0]));
    refused(&c, "is not an immediate Value");
    // Records out of pc order, or two at one pc.
    let mut o = t.clone();
    o.records.swap(0, 1);
    refused(&o, "sorted by pc");
    let mut d = t.clone();
    d.records[1].pc = d.records[0].pc;
    refused(&d, "sorted by pc");
    // A counter index past the records.
    let mut k = t.clone();
    k.records[0].counter = 4;
    refused(&k, "counter 4 of 4 records");
    // A register past the dump, a stack or frame offset that is not a word, a const index
    // past the list.
    let mut r = t.clone();
    r.records[0].values[0].loc = Loc::Reg(DUMP_REGS as u8);
    refused(&r, "register x31 is not in the dump");
    r.records[0].values[0].loc = Loc::Stack(12);
    refused(&r, "byte offset 12 is not a word");
    r.records[0].values[0].loc = Loc::Frame(-84);
    refused(&r, "byte offset -84 is not a word");
    r.records[0].values[0].loc = Loc::Const(3);
    refused(&r, "const 3 of 3");
    // Two values for one target (a register, or the accumulator).
    let mut two = t.clone();
    let v = two.records[0].values[0];
    two.records[0].values.push(v);
    refused(&two, "a second value for");
    let mut acc = t.clone();
    let v = acc.records[3].values[0];
    acc.records[3].values.push(ExitValue {
        loc: Loc::Reg(1),
        ..v
    });
    refused(&acc, "a second value for Acc");
}

/// A synthetic VM frame: `params` parameter slots, the receiver, the 8 header words, then
/// `registers` register slots; every slot starts as its own index as an int32. Returns the
/// memory and the index of fp in it.
fn frame(params: usize, registers: usize) -> (Vec<u64>, usize) {
    let fp = params + 1 + HEADER;
    let mem = (0..fp + registers).map(|i| int32_bits(i as i32)).collect();
    (mem, fp)
}

fn word(mem: &[u64], fp: usize, s: isize) -> u64 {
    mem[(fp as isize + s) as usize]
}

/// The rebuild reads each location, boxes by repr (an int32 from the low 32 bits, a boolean,
/// a double as `Value::number` boxes it: an integer as an int32, -0 and NaN as doubles, NaN
/// made pure) and writes the VM registers and the accumulator; it reads every value before it
/// writes one (a parameter swap through two frame slots).
#[test]
fn the_rebuild_writes_the_frame_and_the_accumulator() {
    let (mut mem, fp) = frame(2, 6);
    let a = |k: u32| Target::Reg(Register::parameter(k));
    let r = |i: u32| Target::Reg(Register::local(i));
    let mut regs = [0u64; DUMP_REGS];
    regs[5] = 0xDEAD_BEEF_0000_0029; // int32 41 in the low half
    regs[19] = 1;
    regs[20] = (-0.0f64).to_bits();
    regs[21] = 0x7FF8_0000_DEAD_0001; // a NaN with a payload
    regs[22] = 3.0f64.to_bits();
    let native = [0u64, 0, 2.5f64.to_bits(), 0];
    let t = Table {
        consts: vec![Value::NULL.to_bits()],
        records: vec![exit(
            12,
            33,
            0,
            vec![
                value(r(0), Repr::Int32, Loc::Reg(5)),
                value(r(1), Repr::Float64, Loc::Stack(16)),
                value(r(2), Repr::Float64, Loc::Reg(20)),
                value(r(3), Repr::Float64, Loc::Reg(21)),
                value(r(4), Repr::Float64, Loc::Reg(22)),
                value(r(5), Repr::Tagged, Loc::Const(0)),
                // The swap: a0 <- a1's slot, a1 <- a0's slot.
                value(
                    a(0),
                    Repr::Tagged,
                    Loc::Frame((slot::PARAM0 as i32 - 1) * 8),
                ),
                value(a(1), Repr::Tagged, Loc::Frame(slot::PARAM0 as i32 * 8)),
                value(Target::Reg(Register::RECEIVER), Repr::Int32, Loc::Reg(5)),
                value(Target::Acc, Repr::Bool, Loc::Reg(19)),
            ],
        )],
    };
    let (a0, a1) = (
        word(&mem, fp, slot::PARAM0),
        word(&mem, fp, slot::PARAM0 - 1),
    );
    let header: Vec<u64> = (1..=HEADER as isize).map(|s| word(&mem, fp, -s)).collect();
    let shape = FrameShape {
        registers: 6,
        params: 2,
    };
    let state = ExitState {
        regs: &regs,
        native_fp: native.as_ptr().cast(),
    };
    let fpp = mem.as_mut_ptr().wrapping_add(fp);
    // SAFETY: `fpp` is the fp of the synthetic frame `mem` with the slots of `shape`; stack
    // offset 16 is a word of `native`.
    let res = unsafe { rebuild(&t, &t.records[0], &state, fpp, shape) };
    assert_eq!(
        res,
        Ok(Resume {
            bytecode: 33,
            acc: Value::TRUE.to_bits()
        })
    );
    let got: Vec<u64> = (0..6).map(|i| word(&mem, fp, i)).collect();
    assert_eq!(
        got,
        [
            int32_bits(41),
            Value::double(2.5).to_bits(),
            Value::double(-0.0).to_bits(),
            Value::double(f64::NAN).to_bits(),
            int32_bits(3),
            Value::NULL.to_bits(),
        ]
    );
    assert_eq!(word(&mem, fp, slot::PARAM0), a1);
    assert_eq!(word(&mem, fp, slot::PARAM0 - 1), a0);
    assert_eq!(word(&mem, fp, slot::RECEIVER), int32_bits(41));
    let after: Vec<u64> = (1..=HEADER as isize).map(|s| word(&mem, fp, -s)).collect();
    assert_eq!(after, header, "a header word changed");
}

/// The rebuild refuses, and writes nothing, for a target or frame slot the frame does not
/// have, a header word as a frame slot, Tagged bits no Value has, and a record that is not an
/// exit. A record with no accumulator value resumes with undefined.
#[test]
fn the_rebuild_refuses_and_writes_nothing() {
    let (mut mem, fp) = frame(1, 2);
    let before = mem.clone();
    let mut regs = [0u64; DUMP_REGS];
    regs[4] = int32_bits(9);
    regs[6] = 0x0001_0000_0000_0003; // no Value has these bits
    let shape = FrameShape {
        registers: 2,
        params: 1,
    };
    let state = ExitState {
        regs: &regs,
        native_fp: core::ptr::null(),
    };
    let ok = value(Target::Reg(Register::local(0)), Repr::Tagged, Loc::Reg(4));
    let cases: [(Vec<ExitValue>, &str); 5] = [
        (
            vec![
                ok,
                value(Target::Reg(Register::local(2)), Repr::Int32, Loc::Reg(4)),
            ],
            "no register r2",
        ),
        (
            vec![
                ok,
                value(
                    Target::Reg(Register::parameter(1)),
                    Repr::Int32,
                    Loc::Reg(4),
                ),
            ],
            "no register a1",
        ),
        (
            vec![
                ok,
                value(
                    Target::Acc,
                    Repr::Tagged,
                    Loc::Frame(slot::CALLEE as i32 * 8),
                ),
            ],
            "is not a value slot",
        ),
        (
            vec![
                ok,
                value(
                    Target::Acc,
                    Repr::Tagged,
                    Loc::Frame((slot::PARAM0 as i32 - 1) * 8),
                ),
            ],
            "is not a value slot",
        ),
        (
            vec![ok, value(Target::Acc, Repr::Tagged, Loc::Reg(6))],
            "not a Value",
        ),
    ];
    let fpp = mem.as_mut_ptr().wrapping_add(fp);
    for (values, why) in cases {
        let t = Table {
            consts: vec![],
            records: vec![exit(0, 5, 0, values)],
        };
        // SAFETY: `fpp` is the synthetic frame of `shape`; no stack location.
        let e = unsafe { rebuild(&t, &t.records[0], &state, fpp, shape) }.unwrap_err();
        assert!(e.contains(why), "{why}: {e}");
        assert_eq!(mem, before, "{why}: the frame changed");
    }
    let poll = Table {
        consts: vec![],
        records: vec![Record {
            tag: RecordTag::Poll,
            ..exit(0, 5, 0, vec![ok])
        }],
    };
    // SAFETY: as above.
    let e = unsafe { rebuild(&poll, &poll.records[0], &state, fpp, shape) }.unwrap_err();
    assert!(e.contains("not an exit"), "{e}");
    assert_eq!(mem, before);
    let t = Table {
        consts: vec![],
        records: vec![exit(0, 5, 0, vec![ok])],
    };
    // SAFETY: as above.
    let r = unsafe { rebuild(&t, &t.records[0], &state, fpp, shape) }.expect("rebuild");
    assert_eq!(r.acc, Value::UNDEFINED.to_bits());
    assert_eq!(word(&mem, fp, 0), int32_bits(9));
}

/// The side table: the install reads the table (and refuses one the reader refuses, a record
/// past the code's end or one that resumes between two instructions); each deopt finds its
/// record by pc and adds 1 to its counter; a pc with no record, another function's frame, a
/// pc past the code and code with no metadata are refused and count nothing.
#[test]
fn the_side_table_counts_each_exit() {
    let st = SideTable::new();
    assert!(st.is_empty());
    let t = Table {
        consts: vec![],
        records: vec![
            exit(8, 3, 0, vec![value(Target::Acc, Repr::Int32, Loc::Reg(4))]),
            exit(12, 7, 1, vec![value(Target::Acc, Repr::Int32, Loc::Reg(5))]),
        ],
    };
    let (start, len, fun) = (0x10_000, 64, 0x2000);
    let insn = |o: u32| o != 5;
    let read = |len: usize, bytes: &[u8]| CodeMeta::read(len, fun, bytes, insn);
    assert!(read(len, &t.encode()[1..]).is_err());
    let e = read(12, &t.encode()).err().expect("refused");
    assert!(e.contains("record at pc 12 of 12 code bytes"), "{e}");
    let mut mid = t.clone();
    mid.records[0].bytecode = 5;
    let e = read(len, &mid.encode()).err().expect("refused");
    assert!(
        e.contains("bytecode offset 5, not an instruction start"),
        "{e}"
    );
    assert!(st.is_empty());
    st.insert(start, read(len, &t.encode()).expect("read"));
    assert_eq!(st.counters(start), Some(vec![0, 0]));
    let (mut mem, fp) = frame(0, 1);
    let fpp = mem.as_mut_ptr().wrapping_add(fp);
    let mut regs = [0u64; DUMP_REGS];
    regs[4] = 1;
    regs[5] = 2;
    let state = ExitState {
        regs: &regs,
        native_fp: core::ptr::null(),
    };
    let shape = FrameShape {
        registers: 1,
        params: 0,
    };
    let deopt = |pc: u32, function: usize| {
        // SAFETY: `fpp` is the synthetic frame of `shape`; no record has a stack location.
        unsafe { st.deopt(start, function, pc, &state, fpp, shape) }
    };
    assert_eq!(
        deopt(12, fun).map(|r| (r.bytecode, r.acc)),
        Ok((7, int32_bits(2)))
    );
    assert_eq!(deopt(12, fun).map(|r| r.bytecode), Ok(7));
    assert_eq!(deopt(8, fun).map(|r| r.acc), Ok(int32_bits(1)));
    assert_eq!(st.counters(start), Some(vec![1, 2]));
    for (pc, f, why) in [
        (10, fun, "no exit record at pc 10"),
        (64, fun, "exit pc 64 of 64"),
        (8, fun + 8, "another function"),
    ] {
        let e = deopt(pc, f).unwrap_err();
        assert!(e.contains(why), "{why}: {e}");
    }
    // SAFETY: as above.
    let e = unsafe { st.deopt(start + 4, fun, 8, &state, fpp, shape) }.unwrap_err();
    assert!(e.contains("no deopt metadata"), "{e}");
    assert_eq!(st.counters(start), Some(vec![1, 2]));
    assert_eq!(st.counters(start + 4), None);
    // A new install at the same start replaces the old metadata and its counters.
    st.insert(start, read(len, &t.encode()).expect("read"));
    assert_eq!(st.counters(start), Some(vec![0, 0]));
    // An uninstall drops the entry: a deopt at that start then finds no metadata.
    assert!(!st.remove(start + 4));
    assert!(st.remove(start));
    assert!(st.is_empty());
    let e = deopt(12, fun).unwrap_err();
    assert!(e.contains("no deopt metadata"), "{e}");
}
