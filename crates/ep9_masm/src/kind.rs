//! The relocation kinds of H34, in the order of the H34 Freeze table part "Relocation kinds
//! (append-only)": Local, Stub, Runtime, LinkSlot, IcCell, RootIndex, DeoptExit, HeapConst (JIT
//! only).
//!
//! | Code | Kind | Meaning (H34 Decision) |
//! |---|---|---|
//! | 0 | [`RelocKind::Local`] | label or branch inside the function or unit |
//! | 1 | [`RelocKind::Stub`] | shared IC stub or trampoline |
//! | 2 | [`RelocKind::Runtime`] | Rust slow path or builtin (H20) |
//! | 3 | [`RelocKind::LinkSlot`] | external reference resolved at load or first run |
//! | 4 | [`RelocKind::IcCell`] | IC or feedback cell |
//! | 5 | [`RelocKind::RootIndex`] | isolate root, intrinsic or atom by stable index (H13) |
//! | 6 | [`RelocKind::DeoptExit`] | exit to the shared deopt entry |
//! | 7 | [`RelocKind::HeapConst`] | raw heap pointer (JIT only; the unit finalizer rejects it) |
//!
//! The list is append-only: a code never changes and never goes away, and a new kind takes
//! the next code ([`KIND_COUNT`]). The `kinds!` list below is the one source: it makes the
//! enum, [`RelocKind::ALL`] and [`KIND_COUNT`], so the count cannot miss a kind. Each
//! finalizer matches the kinds with no wildcard arm, so a new kind does not build until both
//! finalizers handle or reject it (H34 Freeze table). `tests/golden/reloc-kinds.txt` holds
//! one line per code; `tests/kinds.rs` checks it and the H34 order.

macro_rules! kinds {
    ($($(#[$doc:meta])* $name:ident = $code:literal,)+) => {
        /// A relocation kind (H34). The value is the wire code of [`crate::Reloc::kind`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum RelocKind {
            $($(#[$doc])* $name = $code,)+
        }

        impl RelocKind {
            /// Every kind, in code order.
            pub const ALL: [RelocKind; KIND_COUNT] = [$(RelocKind::$name),+];

            /// The H34 name of the kind.
            pub const fn name(self) -> &'static str {
                match self {
                    $(RelocKind::$name => stringify!($name),)+
                }
            }
        }

        /// The number of kinds: the length of the `kinds!` list. A code at or above it is
        /// unknown, and both finalizers refuse it.
        pub const KIND_COUNT: usize = [$($code),+].len();
    };
}

kinds! {
    /// A label or branch inside the function or unit: pc-relative, resolved at emit.
    Local = 0,
    /// A shared IC stub or trampoline.
    Stub = 1,
    /// A Rust slow path or builtin (H20).
    Runtime = 2,
    /// An external reference resolved at load or first run.
    LinkSlot = 3,
    /// An IC or feedback cell.
    IcCell = 4,
    /// An isolate root, intrinsic or atom by stable index (H13).
    RootIndex = 5,
    /// An exit to the shared deopt entry (H29); the metadata is keyed by pc offset.
    DeoptExit = 6,
    /// A raw heap pointer. JIT only: the unit finalizer rejects it (H34, H30).
    HeapConst = 7,
}

// Item i of the list has code i: a code out of order, twice or with a gap does not build.
const _: () = {
    let mut i = 0;
    while i < KIND_COUNT {
        assert!(
            RelocKind::ALL[i] as usize == i,
            "kinds!: each code once, in order, from 0"
        );
        i += 1;
    }
};

impl RelocKind {
    /// The kind of a wire code, or `None` for an unknown code (at or above [`KIND_COUNT`]).
    pub const fn from_code(code: u8) -> Option<RelocKind> {
        if (code as usize) < KIND_COUNT {
            Some(RelocKind::ALL[code as usize])
        } else {
            None
        }
    }

    /// The wire code.
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// A label of an [`crate::Assembler`]: an index into [`crate::CodeDesc::labels`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label(pub u32);

/// What a relocation refers to: a kind and its operand, typed. [`crate::Reloc`] stores it as
/// the wire code and a `u32` index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// [`RelocKind::Local`]: a label of the same instruction stream.
    Local(Label),
    /// [`RelocKind::Stub`]: a stub id.
    Stub(u32),
    /// [`RelocKind::Runtime`]: a runtime function id (a RuntimeTable entry).
    Runtime(u32),
    /// [`RelocKind::LinkSlot`]: a link cell index.
    LinkSlot(u32),
    /// [`RelocKind::IcCell`]: an IC cell index.
    IcCell(u32),
    /// [`RelocKind::RootIndex`]: a stable root index (H13).
    RootIndex(u32),
    /// [`RelocKind::DeoptExit`]: an exit id.
    DeoptExit(u32),
    /// [`RelocKind::HeapConst`]: an index into the code's constant list.
    HeapConst(u32),
}

impl Target {
    /// The target of a kind and its operand. The match names every kind with no wildcard
    /// arm, so a new kind does not build until it has its `Target` value.
    pub const fn new(kind: RelocKind, index: u32) -> Target {
        match kind {
            RelocKind::Local => Target::Local(Label(index)),
            RelocKind::Stub => Target::Stub(index),
            RelocKind::Runtime => Target::Runtime(index),
            RelocKind::LinkSlot => Target::LinkSlot(index),
            RelocKind::IcCell => Target::IcCell(index),
            RelocKind::RootIndex => Target::RootIndex(index),
            RelocKind::DeoptExit => Target::DeoptExit(index),
            RelocKind::HeapConst => Target::HeapConst(index),
        }
    }

    /// The kind.
    pub const fn kind(self) -> RelocKind {
        match self {
            Target::Local(_) => RelocKind::Local,
            Target::Stub(_) => RelocKind::Stub,
            Target::Runtime(_) => RelocKind::Runtime,
            Target::LinkSlot(_) => RelocKind::LinkSlot,
            Target::IcCell(_) => RelocKind::IcCell,
            Target::RootIndex(_) => RelocKind::RootIndex,
            Target::DeoptExit(_) => RelocKind::DeoptExit,
            Target::HeapConst(_) => RelocKind::HeapConst,
        }
    }

    /// The operand.
    pub const fn index(self) -> u32 {
        match self {
            Target::Local(Label(i))
            | Target::Stub(i)
            | Target::Runtime(i)
            | Target::LinkSlot(i)
            | Target::IcCell(i)
            | Target::RootIndex(i)
            | Target::DeoptExit(i)
            | Target::HeapConst(i) => i,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_the_top_value_is_unknown() {
        for (i, k) in RelocKind::ALL.iter().enumerate() {
            assert_eq!(k.code() as usize, i);
            assert_eq!(RelocKind::from_code(k.code()), Some(*k));
            let t = Target::new(*k, 7);
            assert_eq!((t.kind(), t.index()), (*k, 7));
        }
        assert_eq!(RelocKind::from_code(KIND_COUNT as u8), None);
        assert_eq!(RelocKind::from_code(u8::MAX), None);
    }
}
