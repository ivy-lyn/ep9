//! Class fields, static blocks, auto-accessors, private methods and the class constructor
//! prologue. M2 P0 routing stub: every element check returns today's `unsupported:` error, and
//! the constructor prologue emits nothing. P2-CLS-CG builds them (C7, C8):
//!
//! ```text
//! class definition: CreatePrivateName per #x (class context slot), a brand per class
//!                   SetClassFieldsInitializer(ctor, instance initializer)
//!                   static fields and static blocks: the static initializer, inline
//! base ctor entry:  RunClassFieldsInitializer(ctor, this)   (class_constructor_prologue)
//! after super():    RunClassFieldsInitializer(ctor, this)
//! field define:     DefineKeyedOwnProperty obj, key, flags (bit 0 set name, bit 1 private add)
//! ```
//!
//! Callers: `class.rs` (`class_constructor`, the element checks) and `func.rs` (`gen_function`).
#![allow(dead_code)] // m2-stub

use oxc_ast::ast::{
    AccessorProperty, MethodDefinition, PrivateIdentifier, PropertyDefinition, StaticBlock,
};

use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
    /// A field (`x = v`, `static x`, `#x`).
    pub(crate) fn check_class_field(&self, p: &'a PropertyDefinition<'a>) -> Gen {
        self.unsupported("class fields", p.span)
    }

    /// A `static { ... }` block.
    pub(crate) fn check_static_block(&self, s: &'a StaticBlock<'a>) -> Gen {
        self.unsupported("class static blocks", s.span)
    }

    /// An `accessor x` element.
    pub(crate) fn check_auto_accessor(&self, p: &'a AccessorProperty<'a>) -> Gen {
        self.unsupported("class auto-accessors", p.span)
    }

    /// A method, getter or setter with a private name (`#m() {}`); `p` is its key.
    pub(crate) fn check_private_method(
        &self,
        m: &'a MethodDefinition<'a>,
        p: &'a PrivateIdentifier<'a>,
    ) -> Gen {
        let _ = m;
        self.unsupported("private names", p.span)
    }

    /// The prologue of a class constructor, after the parameters: runs the instance field
    /// initializer in a base class. Nothing in v0 (no instance fields).
    pub(crate) fn class_constructor_prologue(&mut self) -> Gen {
        Ok(())
    }
}
