//! A stack walk with the system unwinder that also reads one callee-saved register (x19 on
//! aarch64, rbx on x86-64) in each frame. `ep9_platform::unwind::walk_native_stack` gives only
//! instruction pointers; this check needs the restored register too.

/// For each frame, innermost first: (instruction pointer, x19 or rbx in that frame).
pub fn frames_with_saved_reg(max: usize) -> Vec<(usize, u64)> {
    os::walk(max)
}

#[cfg(unix)]
mod os {
    use std::ffi::{c_int, c_void};

    use crate::code::SAVED_REG_DWARF;

    #[repr(C)]
    struct UnwindContext {
        _opaque: [u8; 0],
    }
    type TraceFn = extern "C" fn(*mut UnwindContext, *mut c_void) -> c_int;

    unsafe extern "C" {
        fn _Unwind_Backtrace(trace: TraceFn, arg: *mut c_void) -> c_int;
        fn _Unwind_GetIP(ctx: *mut UnwindContext) -> usize;
        fn _Unwind_GetGR(ctx: *mut UnwindContext, index: c_int) -> usize;
    }

    struct Out {
        frames: Vec<(usize, u64)>,
        max: usize,
    }

    extern "C" fn trace(ctx: *mut UnwindContext, arg: *mut c_void) -> c_int {
        // SAFETY: `arg` is the `Out` that `walk` passed; only this callback uses it.
        let out = unsafe { &mut *arg.cast::<Out>() };
        // SAFETY: `ctx` is the live context of this callback.
        let (ip, r) = unsafe { (_Unwind_GetIP(ctx), _Unwind_GetGR(ctx, SAVED_REG_DWARF)) };
        if ip == 0 {
            return 9;
        }
        out.frames.push((ip, r as u64));
        if out.frames.len() >= out.max { 9 } else { 0 }
    }

    pub fn walk(max: usize) -> Vec<(usize, u64)> {
        let mut out = Out {
            frames: Vec::new(),
            max,
        };
        // SAFETY: `trace` matches the callback type and never unwinds; `out` outlives the call.
        unsafe { _Unwind_Backtrace(trace, (&raw mut out).cast()) };
        out.frames
    }
}

#[cfg(windows)]
mod os {
    use std::ffi::c_void;

    use windows_sys::Win32::System::Diagnostics::Debug::{
        CONTEXT, RtlCaptureContext, RtlLookupFunctionEntry, RtlVirtualUnwind, UNW_FLAG_NHANDLER,
    };

    #[repr(C, align(16))]
    struct Aligned(CONTEXT);

    pub fn walk(max: usize) -> Vec<(usize, u64)> {
        // SAFETY: CONTEXT is plain data; all-zero is valid.
        let mut c: Aligned = unsafe { std::mem::zeroed() };
        // SAFETY: a 16-byte aligned CONTEXT for this thread.
        unsafe { RtlCaptureContext(&mut c.0) };
        let mut frames = Vec::new();
        while c.0.Rip != 0 && frames.len() < max {
            frames.push((c.0.Rip as usize, c.0.Rbx));
            let mut base = 0u64;
            // SAFETY: the lookup only reads loaded images and registered tables.
            let e = unsafe { RtlLookupFunctionEntry(c.0.Rip, &mut base, std::ptr::null_mut()) };
            if e.is_null() {
                // SAFETY: a leaf frame's RSP points at its return address on this stack.
                c.0.Rip = unsafe { *(c.0.Rsp as *const u64) };
                c.0.Rsp += 8;
            } else {
                let mut data: *mut c_void = std::ptr::null_mut();
                let mut frame = 0u64;
                // SAFETY: `e` and `base` belong to `Rip`; `c` holds that frame's registers.
                unsafe {
                    RtlVirtualUnwind(
                        UNW_FLAG_NHANDLER,
                        base,
                        c.0.Rip,
                        e,
                        &mut c.0,
                        &mut data,
                        &mut frame,
                        std::ptr::null_mut(),
                    )
                };
            }
        }
        frames
    }
}
