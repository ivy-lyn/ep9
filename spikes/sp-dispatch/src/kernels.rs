//! Benchmark kernels. Each has a bytecode builder and a plain-Rust reference model.

use crate::Op::*;
use crate::asm::Asm;
use crate::ops::BUDGET;
use crate::program::Program;

/// A kernel: name, default size, bytecode builder and reference model.
pub struct Kernel {
    pub name: &'static str,
    pub about: &'static str,
    /// Default size (iterations, or n for fib): 0.2-0.4 s per run for sibling and cgoto on
    /// the research machine (Apple M5 Max).
    pub default_n: i64,
    pub build: fn(i64) -> Program,
    /// (result, interrupts, slow_paths)
    pub reference: fn(i64) -> (i64, u64, u64),
}

pub const KERNELS: &[Kernel] = &[
    Kernel {
        name: "loop",
        about: "tight loop: loads, int32 add/sub, bit ops, compare, back edge",
        default_n: 40_000_000,
        build: build_loop,
        reference: ref_loop,
    },
    Kernel {
        name: "branchy",
        about: "xorshift32 with two data-dependent branches per iteration",
        default_n: 15_000_000,
        build: build_branchy,
        reference: ref_branchy,
    },
    Kernel {
        name: "call",
        about: "recursive fib(n): one VM call per about 9.5 dispatches",
        default_n: 35,
        build: build_fib,
        reference: ref_fib,
    },
];

pub fn find(name: &str) -> Option<&'static Kernel> {
    KERNELS.iter().find(|k| k.name == name)
}

fn i32w(v: i64) -> i64 {
    v as i32 as i64
}

/// Interrupt count for `back_edges` JumpLoop executions.
fn interrupts(back_edges: i64) -> u64 {
    (back_edges / BUDGET) as u64
}

// ---- loop ----
// sum = 0; i = 0
// while (i < n) { sum += i & 7; sum ^= i >>> 3; sum &= 0xffff; sum -= i & 3; i += 1 }
// return sum

fn build_loop(n: i64) -> Program {
    let mut a = Asm::new();
    let main = a.func(5);
    let (sum, i, lim, m7, mffff) = (0, 1, 2, 3, 4);
    let kn = a.konst(n);
    let kffff = a.konst(0xffff);
    a.begin(main);
    for (r, v) in [(sum, 0), (i, 0), (m7, 7)] {
        a.op(LdaSmi, &[v]);
        a.op(Star, &[r]);
    }
    a.op(LdaConst, &[kn]);
    a.op(Star, &[lim]);
    a.op(LdaConst, &[kffff]);
    a.op(Star, &[mffff]);
    let (top, done) = (a.label(), a.label());
    a.bind(top);
    a.op(Ldar, &[lim]);
    a.op(TestLessThan, &[i]); // acc = i < n
    a.jump(JumpIfFalse, done);
    a.op(Ldar, &[i]);
    a.op(BitAnd, &[m7]);
    a.op(Add, &[sum]);
    a.op(Star, &[sum]); // sum += i & 7
    a.op(Ldar, &[i]);
    a.op(ShrSmi, &[3]);
    a.op(BitXor, &[sum]);
    a.op(Star, &[sum]); // sum ^= i >>> 3
    a.op(Ldar, &[mffff]);
    a.op(BitAnd, &[sum]);
    a.op(Star, &[sum]); // sum &= 0xffff
    a.op(LdaSmi, &[3]);
    a.op(BitAnd, &[i]);
    a.op(Sub, &[sum]);
    a.op(Star, &[sum]); // sum -= i & 3
    a.op(Ldar, &[i]);
    a.op(AddSmi, &[1]);
    a.op(Star, &[i]); // i += 1
    a.jump(JumpLoop, top);
    a.bind(done);
    a.op(Ldar, &[sum]);
    a.op(Return, &[]);
    a.end();
    a.finish()
}

fn ref_loop(n: i64) -> (i64, u64, u64) {
    let mut sum: i64 = 0;
    for i in 0..n {
        sum += i32w(i & 7);
        sum = i32w(sum ^ i32w(((i as u32) >> 3) as i32 as i64));
        sum = i32w(sum & 0xffff);
        sum -= i32w(i & 3);
    }
    (sum, interrupts(n), 0)
}

// ---- branchy ----
// x = seed; a = b = c = 0
// for i < n: x ^= x << 13; x ^= x >>> 17; x ^= x << 5     (xorshift32)
//   if (x & 1) a += 1 else b += 3
//   if ((x & 6) == 2) { c = c - (x & 0xff); c &= 0xfffff }
// return a + b + c

const SEED: i64 = 0x2545_f491;

fn build_branchy(n: i64) -> Program {
    let mut a = Asm::new();
    let main = a.func(10);
    let (x, ra, rb, rc, i, lim, t, mff, mfffff) = (0, 1, 2, 3, 4, 5, 6, 7, 8);
    let kseed = a.konst(SEED);
    let kn = a.konst(n);
    let kff = a.konst(0xff);
    let kfffff = a.konst(0xfffff);
    a.begin(main);
    for (r, k) in [(x, kseed), (lim, kn), (mff, kff), (mfffff, kfffff)] {
        a.op(LdaConst, &[k]);
        a.op(Star, &[r]);
    }
    for r in [ra, rb, rc, i] {
        a.op(LdaSmi, &[0]);
        a.op(Star, &[r]);
    }
    let (top, done, else1, end1, end2) = (a.label(), a.label(), a.label(), a.label(), a.label());
    a.bind(top);
    a.op(Ldar, &[lim]);
    a.op(TestLessThan, &[i]);
    a.jump(JumpIfFalse, done);
    for (shift, op) in [(13, ShlSmi), (17, ShrSmi), (5, ShlSmi)] {
        a.op(Ldar, &[x]);
        a.op(op, &[shift]);
        a.op(BitXor, &[x]);
        a.op(Star, &[x]);
    }
    a.op(LdaSmi, &[1]);
    a.op(BitAnd, &[x]);
    a.jump(JumpIfFalse, else1);
    a.op(Ldar, &[ra]);
    a.op(AddSmi, &[1]);
    a.op(Star, &[ra]);
    a.jump(Jump, end1);
    a.bind(else1);
    a.op(Ldar, &[rb]);
    a.op(AddSmi, &[3]);
    a.op(Star, &[rb]);
    a.bind(end1);
    a.op(LdaSmi, &[6]);
    a.op(BitAnd, &[x]);
    a.op(Star, &[t]);
    a.op(LdaSmi, &[2]);
    a.op(TestEqual, &[t]);
    a.jump(JumpIfFalse, end2);
    a.op(Ldar, &[mff]);
    a.op(BitAnd, &[x]);
    a.op(Sub, &[rc]);
    a.op(Star, &[rc]); // c = c - (x & 0xff)
    a.op(Ldar, &[mfffff]);
    a.op(BitAnd, &[rc]);
    a.op(Star, &[rc]); // c &= 0xfffff
    a.bind(end2);
    a.op(Ldar, &[i]);
    a.op(AddSmi, &[1]);
    a.op(Star, &[i]);
    a.jump(JumpLoop, top);
    a.bind(done);
    a.op(Ldar, &[rc]);
    a.op(Add, &[rb]);
    a.op(Add, &[ra]);
    a.op(Return, &[]);
    a.end();
    a.finish()
}

fn ref_branchy(n: i64) -> (i64, u64, u64) {
    let mut x = SEED as u32;
    let (mut a, mut b, mut c) = (0i64, 0i64, 0i64);
    let mut slow = 0u64;
    let mut add = |p: i64, q: i64| {
        let s = p + q;
        if i32w(s) != s {
            slow += 1;
        }
        s
    };
    for _ in 0..n {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        if (x & 1) != 0 {
            a = add(a, 1);
        } else {
            b = add(b, 3);
        }
        if (x & 6) == 2 {
            c = add(c, -((x & 0xff) as i64));
            c = i32w(c & 0xfffff);
        }
    }
    let bc = add(b, c);
    let r = add(bc, a);
    (r, interrupts(n), slow)
}

// ---- call ----
// fib(n) = n < 2 ? n : fib(n - 1) + fib(n - 2)

fn build_fib(n: i64) -> Program {
    let mut a = Asm::new();
    let main = a.func(2);
    let fib = a.func(3);
    let kn = a.konst(n);
    a.begin(main);
    a.op(LdaConst, &[kn]);
    a.op(Star, &[0]);
    a.op(Mov, &[0, 1]);
    a.op(Call, &[fib.0 as i64, 1, 1]);
    a.op(Return, &[]);
    a.end();

    let (rn, t, arg) = (0, 1, 2);
    a.begin(fib);
    let rec = a.label();
    a.op(LdaSmi, &[2]);
    a.op(TestLessThan, &[rn]); // acc = n < 2
    a.jump(JumpIfFalse, rec);
    a.op(Ldar, &[rn]);
    a.op(Return, &[]);
    a.bind(rec);
    a.op(Ldar, &[rn]);
    a.op(AddSmi, &[-1]);
    a.op(Star, &[arg]);
    a.op(Call, &[fib.0 as i64, arg, 1]);
    a.op(Star, &[t]); // t = fib(n - 1)
    a.op(Ldar, &[rn]);
    a.op(AddSmi, &[-2]);
    a.op(Star, &[arg]);
    a.op(Call, &[fib.0 as i64, arg, 1]); // acc = fib(n - 2)
    a.op(Add, &[t]);
    a.op(Return, &[]);
    a.end();
    a.finish()
}

fn ref_fib(n: i64) -> (i64, u64, u64) {
    fn fib(n: i64) -> i64 {
        let (mut a, mut b) = (0i64, 1i64);
        for _ in 0..n {
            (a, b) = (b, a + b);
        }
        a
    }
    (fib(n), 0, 0)
}

/// Test helper: endless recursion. Must end with the stack-overflow error, not a crash.
pub fn build_runaway() -> Program {
    let mut a = Asm::new();
    let main = a.func(4);
    a.begin(main);
    a.op(Call, &[main.0 as i64, 0, 4]);
    a.op(Return, &[]);
    a.end();
    a.finish()
}

/// Test helper: int32 overflow in Add, Sub and AddSmi. Result -2, 3 slow paths.
pub fn build_overflow() -> Program {
    let mut a = Asm::new();
    let main = a.func(3);
    let kmax = a.konst(i32::MAX as i64);
    let kmin = a.konst(i32::MIN as i64);
    a.begin(main);
    a.op(LdaConst, &[kmax]);
    a.op(Star, &[0]);
    a.op(LdaSmi, &[1]);
    a.op(Add, &[0]); // 2^31: slow
    a.op(Star, &[1]);
    a.op(LdaConst, &[kmin]);
    a.op(Star, &[2]);
    a.op(LdaSmi, &[1]);
    a.op(Sub, &[2]); // -2^31 - 1: slow
    a.op(AddSmi, &[-1]); // -2^31 - 2: slow
    a.op(Add, &[1]); // 2^31 + (-2^31 - 2) = -2: fast
    a.op(Return, &[]);
    a.end();
    a.finish()
}

/// Every opcode inside one loop (Halt is in the entry stub). The deep-loop guard runs it, so
/// a handler that lost its tail call overflows the small native stack, whichever it is.
pub fn build_all_ops(n: i64) -> Program {
    let mut a = Asm::new();
    let main = a.func(6);
    let leaf = a.func(1);
    let (i, lim, sum, t, arg, t2) = (0, 1, 2, 3, 4, 5);
    let kn = a.konst(n);
    let kmask = a.konst(0xffff);
    a.begin(main);
    a.op(LdaSmi, &[0]);
    a.op(Star, &[i]);
    a.op(LdaConst, &[kn]);
    a.op(Star, &[lim]);
    a.op(LdaSmi, &[0]);
    a.op(Star, &[sum]);
    let (top, done, odd, join) = (a.label(), a.label(), a.label(), a.label());
    a.bind(top);
    a.op(Ldar, &[lim]);
    a.op(TestLessThan, &[i]);
    a.jump(JumpIfFalse, done);
    a.op(Mov, &[i, t]);
    a.op(Ldar, &[t]);
    a.op(ShlSmi, &[2]);
    a.op(ShrSmi, &[1]);
    a.op(BitXor, &[sum]);
    a.op(Star, &[sum]); // sum ^= (i << 2) >>> 1
    a.op(LdaSmi, &[5]);
    a.op(Add, &[sum]);
    a.op(Star, &[sum]); // sum += 5
    a.op(LdaSmi, &[3]);
    a.op(Sub, &[sum]);
    a.op(Star, &[sum]); // sum -= 3
    a.op(LdaConst, &[kmask]);
    a.op(BitAnd, &[sum]);
    a.op(Star, &[sum]); // sum &= 0xffff
    a.op(LdaSmi, &[7]);
    a.op(BitAnd, &[t]);
    a.op(Star, &[t2]); // t2 = i & 7
    a.op(LdaSmi, &[3]);
    a.op(TestEqual, &[t2]);
    a.jump(JumpIfFalse, odd);
    a.op(Ldar, &[sum]);
    a.op(AddSmi, &[11]);
    a.op(Star, &[sum]);
    a.jump(Jump, join);
    a.bind(odd);
    a.op(Ldar, &[sum]);
    a.op(AddSmi, &[1]);
    a.op(Star, &[sum]);
    a.bind(join);
    a.op(Ldar, &[sum]);
    a.op(Star, &[arg]);
    a.op(Call, &[leaf.0 as i64, arg, 1]);
    a.op(Star, &[sum]); // sum = leaf(sum) = sum + 1
    a.op(Ldar, &[i]);
    a.op(AddSmi, &[1]);
    a.op(Star, &[i]);
    a.jump(JumpLoop, top);
    a.bind(done);
    a.op(Ldar, &[sum]);
    a.op(Return, &[]);
    a.end();
    a.begin(leaf);
    a.op(Ldar, &[0]);
    a.op(AddSmi, &[1]);
    a.op(Return, &[]);
    a.end();
    a.finish()
}
