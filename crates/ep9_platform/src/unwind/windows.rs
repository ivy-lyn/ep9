//! Windows x64 backend of [`super`]: growable function tables and an `RtlVirtualUnwind` walk.

use std::ffi::c_void;
use std::io;
use std::mem::{align_of, size_of};
use std::ops::Range;

use windows_sys::Win32::System::Diagnostics::Debug::{
    CONTEXT, IMAGE_RUNTIME_FUNCTION_ENTRY, RtlAddGrowableFunctionTable, RtlCaptureContext,
    RtlDeleteGrowableFunctionTable, RtlLookupFunctionEntry, RtlVirtualUnwind, UNW_FLAG_NHANDLER,
};

use super::win64::RuntimeFunction;

const _: () = assert!(size_of::<RuntimeFunction>() == size_of::<IMAGE_RUNTIME_FUNCTION_ENTRY>());
const _: () = assert!(align_of::<RuntimeFunction>() == align_of::<IMAGE_RUNTIME_FUNCTION_ENTRY>());

/// A function table registered with `RtlAddGrowableFunctionTable`. Drop deletes it.
#[derive(Debug)]
pub struct FunctionTableRegistration {
    handle: *mut c_void,
    range: Range<usize>,
    /// The system reads the entries in place while the table is registered.
    entries: Box<[RuntimeFunction]>,
}

// SAFETY: the handle is a process-wide table id; RtlDeleteGrowableFunctionTable may run on any
// thread. The entries are never mutated after registration.
unsafe impl Send for FunctionTableRegistration {}
// SAFETY: no method mutates through `&self`.
unsafe impl Sync for FunctionTableRegistration {}

impl FunctionTableRegistration {
    /// Registers `entries` for the code in `range`. Entry offsets are relative to
    /// `range.start`. The call checks that the entries are sorted, do not overlap, lie in
    /// `range`, and that each `unwind_info` is 4-byte aligned inside `range`.
    ///
    /// # Safety
    /// - Each entry's UNWIND_INFO describes its function correctly; the unwinder trusts it.
    /// - The code and the UNWIND_INFO bytes stay mapped and unchanged while the registration
    ///   lives. Drop the registration before the code memory.
    /// - No other function table covers `range`.
    pub unsafe fn register(
        range: Range<usize>,
        entries: Box<[RuntimeFunction]>,
    ) -> io::Result<Self> {
        let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidInput, m);
        let len = range
            .end
            .checked_sub(range.start)
            .filter(|&l| l != 0 && l <= u32::MAX as usize)
            .ok_or_else(|| bad("range is empty, reversed or larger than 4 GiB"))?;
        if entries.is_empty() {
            return Err(bad("no entries"));
        }
        let mut prev_end = 0u32;
        for e in entries.iter() {
            if e.begin >= e.end || e.begin < prev_end || e.end as usize > len {
                return Err(bad("entries are unsorted, overlap or leave the range"));
            }
            if !e.unwind_info.is_multiple_of(4) || e.unwind_info as usize + 4 > len {
                return Err(bad("unwind_info is unaligned or outside the range"));
            }
            prev_end = e.end;
        }
        let count = u32::try_from(entries.len()).map_err(|_| bad("too many entries"))?;
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `entries` has `count` valid entries with the layout of
        // IMAGE_RUNTIME_FUNCTION_ENTRY (checked at compile time) and lives in the result; the
        // caller vouches for the UNWIND_INFO content.
        let status = unsafe {
            RtlAddGrowableFunctionTable(
                &mut handle,
                entries.as_ptr().cast::<IMAGE_RUNTIME_FUNCTION_ENTRY>(),
                count,
                count,
                range.start,
                range.end,
            )
        };
        if status != 0 {
            return Err(io::Error::other(format!(
                "RtlAddGrowableFunctionTable failed: NTSTATUS {status:#x}"
            )));
        }
        Ok(Self {
            handle,
            range,
            entries,
        })
    }

    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn entries(&self) -> &[RuntimeFunction] {
        &self.entries
    }
}

impl Drop for FunctionTableRegistration {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful RtlAddGrowableFunctionTable and is deleted
        // once; the entries are still alive here.
        unsafe { RtlDeleteGrowableFunctionTable(self.handle) };
    }
}

/// RtlCaptureContext stores XMM registers with aligned moves.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

pub(super) fn walk(mut f: impl FnMut(usize) -> bool) {
    // SAFETY: CONTEXT is plain data; all-zero is a valid value.
    let mut ctx: AlignedContext = unsafe { std::mem::zeroed() };
    // SAFETY: `ctx` is a 16-byte aligned CONTEXT that RtlCaptureContext fills for this thread.
    unsafe { RtlCaptureContext(&mut ctx.0) };
    // No FFI callback here, so a panic in `f` unwinds through plain Rust frames.
    loop {
        let ip = ctx.0.Rip;
        if ip == 0 || !f(ip as usize) {
            return;
        }
        let mut image_base = 0u64;
        // SAFETY: a lookup only reads the loaded images and registered function tables.
        let entry = unsafe { RtlLookupFunctionEntry(ip, &mut image_base, std::ptr::null_mut()) };
        if entry.is_null() {
            // A leaf function: the return address is at RSP.
            if ctx.0.Rsp == 0 {
                return;
            }
            // SAFETY: RSP of a frame on this thread's live stack points at its return
            // address (x64 leaf-function rule).
            ctx.0.Rip = unsafe { *(ctx.0.Rsp as *const u64) };
            ctx.0.Rsp += 8;
        } else {
            let mut handler_data: *mut c_void = std::ptr::null_mut();
            let mut establisher = 0u64;
            // SAFETY: `entry` and `image_base` come from the lookup for `ip`; `ctx` holds
            // the registers of that frame; the out pointers are valid locals.
            unsafe {
                RtlVirtualUnwind(
                    UNW_FLAG_NHANDLER,
                    image_base,
                    ip,
                    entry,
                    &mut ctx.0,
                    &mut handler_data,
                    &mut establisher,
                    std::ptr::null_mut(),
                )
            };
        }
    }
}
