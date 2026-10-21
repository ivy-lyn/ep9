//! gc-lint case: `HashMap<_, PropertyDescriptor>` (a descriptor holds `Local`s).

use std::collections::HashMap;

use crate::abi::PropertyDescriptor;

pub fn by_index<'s>(d: PropertyDescriptor<'s>) -> usize {
    let mut m: HashMap<_, PropertyDescriptor<'s>> = HashMap::new(); // FLAG
    m.insert(1u32, d);
    m.len()
}
