//! gc-lint case: `IteratorRecord` (crates/ep9_vm/src/abi/iter.rs, copied next to this file by the
//! test) holds `Local`s, so a `Vec` of records is a finding; a reference to one is not.

use crate::abi::IteratorRecord;

pub struct Pending<'s> {
    pub records: Vec<IteratorRecord<'s>>, // FLAG
}

pub fn first<'a, 's>(p: &'a Pending<'s>) -> Option<&'a IteratorRecord<'s>> {
    p.records.first()
}
