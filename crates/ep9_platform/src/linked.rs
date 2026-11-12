//! Static registration of a linked unit: the `.o` form of an AOT unit (H35 S1, H35-Q3; route
//! MC-w34-5). The blob form copies unit text into the process code range and makes it
//! executable with `mprotect` (`unit_text`); a hardened-runtime macOS binary without the JIT
//! entitlement, iOS and Windows with ACG cannot do that. The `.o` form needs no page
//! protection change at run time: the static linker puts the unit text into the binary's own
//! executable pages, its unwind entries into the binary's own unwind tables, and the whole
//! container into read-only data.
//!
//! ```text
//!  ep9_unit writer (aot) ──> unit.o: text section ............ the unit's text section bytes
//!                                    unit section ............ the whole container (ep9_unit::check)
//!                                    unwind section .......... .eh_frame / .pdata of the text
//!                                    record symbol <name> .... LinkedUnit { magic, version,
//!                                                              unit, unit_len, text, text_len }
//!  embedder: extern "C" { static <name>: LinkedUnit; }
//!            unsafe { linked::register(&<name>) } ──> the process list of linked texts
//!            ep9_unit::check_linked(rec.unit(), rec.text(), host) == Load
//!            the unsafe Cx unit-code entry with text = rec.text()
//!  loader:   linked_text(text) is Some ──> the text runs where the linker put it: no map, no
//!            copy, no protection change, no unwind registration call
//! ```
//!
//! The record is the only part of the object with relocations (two 64-bit addresses in a
//! data section); the text section has none (H35 S5 "zero text relocations"; the `ep9_unit`
//! writer tests).

use std::io;
use std::sync::Mutex;

/// The record of one linked unit, as the `ep9_unit` writer emits it in the object (48 bytes,
/// little-endian, 8-aligned): magic, version, 4 reserved bytes (0), the address and length of
/// the whole container, the address and length of the linked text.
#[repr(C)]
#[derive(Debug)]
pub struct LinkedUnit {
    magic: [u8; 8],
    version: u32,
    reserved: u32,
    unit: *const u8,
    unit_len: u64,
    text: *const u8,
    text_len: u64,
}

// SAFETY: the record is immutable static data; its two addresses point at immutable static
// bytes of the binary.
unsafe impl Sync for LinkedUnit {}

const _: () = assert!(size_of::<LinkedUnit>() == LinkedUnit::SIZE);

impl LinkedUnit {
    /// The record magic.
    pub const MAGIC: [u8; 8] = *b"EP9ULNK\0";
    /// The record version.
    pub const VERSION: u32 = 1;
    /// The record size in bytes.
    pub const SIZE: usize = 48;

    /// A record over `unit` and `text` (what the writer's object holds after the static
    /// link; for tests that place the bytes themselves).
    pub fn new(unit: &'static [u8], text: &'static [u8]) -> LinkedUnit {
        LinkedUnit {
            magic: Self::MAGIC,
            version: Self::VERSION,
            reserved: 0,
            unit: unit.as_ptr(),
            unit_len: unit.len() as u64,
            text: text.as_ptr(),
            text_len: text.len() as u64,
        }
    }

    /// The whole unit container (its bytes for `ep9_unit::check`).
    pub fn unit(&self) -> &'static [u8] {
        // SAFETY: `register` checked the record; the object's static data holds the bytes for
        // the life of the process.
        unsafe { core::slice::from_raw_parts(self.unit, self.unit_len as usize) }
    }

    /// The linked text: the unit's text section, in the binary's executable pages.
    pub fn text(&self) -> &'static [u8] {
        // SAFETY: as `unit`.
        unsafe { core::slice::from_raw_parts(self.text, self.text_len as usize) }
    }

    /// The record checks of [`register`]: magic, version, reserved bytes, non-null addresses
    /// and lengths that fit the address space.
    fn check(&self) -> io::Result<()> {
        let bad = |m: &'static str| Err(io::Error::new(io::ErrorKind::InvalidData, m));
        if self.magic != Self::MAGIC {
            return bad("linked unit: bad record magic");
        }
        if self.version != Self::VERSION || self.reserved != 0 {
            return bad("linked unit: unknown record version");
        }
        let fits = |p: *const u8, n: u64| {
            !p.is_null() && n != 0 && (p as usize).checked_add(n as usize).is_some()
        };
        if !fits(self.unit, self.unit_len) || !fits(self.text, self.text_len) {
            return bad("linked unit: an empty or null range");
        }
        Ok(())
    }
}

/// The registered linked texts: (address, length).
static LINKED: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Registers the linked unit of `rec` (static registration, H35 S1): from now on
/// [`linked_text`] knows its text, and the unit loader runs it in place. Registering one
/// record twice is a no-op. Errors (`InvalidData`): a record that is not version 1 of the
/// form above.
///
/// # Safety
/// `rec` is the record of a unit object that the `ep9_unit` writer made and the static linker
/// linked into this binary (or bytes with the same properties): its text is in the binary's
/// executable pages and is never written, and the binary's own unwind tables cover it.
pub unsafe fn register(rec: &'static LinkedUnit) -> io::Result<()> {
    rec.check()?;
    let t = (rec.text as usize, rec.text_len as usize);
    let mut l = LINKED.lock().unwrap_or_else(|e| e.into_inner());
    if !l.contains(&t) {
        l.push(t);
    }
    Ok(())
}

/// `text` itself when it is exactly the text of a registered linked unit (the same address and
/// length), else `None`.
pub fn linked_text(text: &[u8]) -> Option<&'static [u8]> {
    let t = (text.as_ptr() as usize, text.len());
    let l = LINKED.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: a registered text lives for the process (`register`'s contract).
    l.contains(&t)
        .then(|| unsafe { core::slice::from_raw_parts(text.as_ptr(), text.len()) })
}
