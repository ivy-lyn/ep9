//! gc-lint case: a struct that holds a `Local` (through a `use .. as` name), inside a `Vec`.

use crate::abi::Local as Held;

pub struct Entry<'s> {
    pub key: u32,
    pub value: Option<Held<'s>>,
}

pub struct Table<'s> {
    pub entries: Vec<Entry<'s>>, // FLAG
}

pub fn first<'a, 's>(t: &'a Table<'s>) -> Option<&'a Entry<'s>> {
    t.entries.first()
}
