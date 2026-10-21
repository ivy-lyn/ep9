//! Property descriptors (§6.2.6): every field optional, as ToPropertyDescriptor gives them.
//! The builtin ABI's `PropertyDescriptor<'s>` (abi/mod.rs) converts to and from this form.
//!
//! A stored property becomes a complete descriptor through `accessor::stored_desc`: a data
//! descriptor ([`Desc::data`]) or an accessor one (`get` and `set` from its ACCESSOR_PAIR,
//! undefined for an absent half).

use crate::object::attr;

/// A property descriptor. `value`, `get` and `set` hold Value bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Desc {
    pub value: Option<u64>,
    pub get: Option<u64>,
    pub set: Option<u64>,
    pub writable: Option<bool>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl Desc {
    /// The complete data descriptor of a stored property: its value and attribute bits.
    pub fn data(value: u64, attrs: i32) -> Desc {
        Desc {
            value: Some(value),
            get: None,
            set: None,
            writable: Some(attrs & attr::WRITABLE != 0),
            enumerable: Some(attrs & attr::ENUMERABLE != 0),
            configurable: Some(attrs & attr::CONFIGURABLE != 0),
        }
    }

    /// IsAccessorDescriptor (§6.2.6.1).
    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// IsDataDescriptor (§6.2.6.2).
    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    /// The attribute bits of a new property from this descriptor: an absent field is false
    /// (§10.1.6.3 step 2).
    pub fn new_attrs(&self) -> i32 {
        self.merge_attrs(0)
    }

    /// The attribute bits after this descriptor is applied to a property with `current`
    /// bits: each present field replaces its bit (§10.1.6.3 step 6).
    pub fn merge_attrs(&self, current: i32) -> i32 {
        let bit = |field: Option<bool>, mask: i32| match field {
            Some(true) => mask,
            Some(false) => 0,
            None => current & mask,
        };
        bit(self.writable, attr::WRITABLE)
            | bit(self.enumerable, attr::ENUMERABLE)
            | bit(self.configurable, attr::CONFIGURABLE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_bits() {
        let d = Desc::data(0, attr::WRITABLE);
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(true), Some(false), Some(false))
        );
        assert!(!d.is_accessor());
        assert!(d.is_data());
        let generic = Desc::default();
        assert!(!generic.is_accessor() && !generic.is_data());
        assert_eq!(generic.new_attrs(), 0);
        assert_eq!(generic.merge_attrs(attr::DEFAULT), attr::DEFAULT);
        let e = Desc {
            enumerable: Some(false),
            configurable: Some(true),
            ..Desc::default()
        };
        assert_eq!(
            e.merge_attrs(attr::WRITABLE | attr::ENUMERABLE),
            attr::WRITABLE | attr::CONFIGURABLE
        );
        let acc = Desc {
            get: Some(0),
            ..Desc::default()
        };
        assert!(acc.is_accessor() && !acc.is_data());
        let w = Desc {
            writable: Some(false),
            ..Desc::default()
        };
        assert!(w.is_data() && !w.is_accessor());
    }
}
