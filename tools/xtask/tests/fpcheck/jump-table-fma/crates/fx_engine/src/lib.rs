//! fp-check fixture: on x86-64 Mach-O, an FMA instruction right after a jump table, in the same
//! function (`.data_region jt32`, LC_DATA_IN_CODE). `llvm-objdump -d` decodes the 8 table bytes
//! as code, and its last "instruction" takes the first byte of the `vfmadd213sd`. fp-check must
//! decode from the table end again and name `_fx_table_fma`.

#[cfg(all(target_arch = "x86_64", target_vendor = "apple"))]
core::arch::global_asm!(
    ".p2align 4",
    ".globl _fx_table_fma",
    "_fx_table_fma:",
    "    leaq Lfx_table(%rip), %rcx",
    "    andl $1, %edi",
    "    movslq (%rcx,%rdi,4), %rax",
    "    addq %rcx, %rax",
    "    jmpq *%rax",
    "    .p2align 2",
    "    .data_region jt32",
    "Lfx_table:",
    "    .long Lfx_fused-Lfx_table",
    "    .long Lfx_plain-Lfx_table",
    "    .end_data_region",
    "Lfx_fused:",
    "    vfmadd213sd %xmm2, %xmm1, %xmm0",
    "    retq",
    "Lfx_plain:",
    "    vmulsd %xmm1, %xmm0, %xmm0",
    "    vaddsd %xmm2, %xmm0, %xmm0",
    "    retq",
    options(att_syntax)
);

#[cfg(all(target_arch = "x86_64", target_vendor = "apple"))]
unsafe extern "C" {
    /// `sel` odd: `a * b + c` with two roundings; even: one rounding (the FMA).
    fn fx_table_fma(sel: u64, a: f64, b: f64, c: f64) -> f64;
}

#[cfg(all(target_arch = "x86_64", target_vendor = "apple"))]
pub fn run(x: f64) -> f64 {
    // SAFETY: fx_table_fma reads only its arguments.
    unsafe { fx_table_fma(x as u64, x, 0.1, -1.0) }
}

#[cfg(not(all(target_arch = "x86_64", target_vendor = "apple")))]
pub fn run(x: f64) -> f64 {
    x * 0.1 - 1.0
}
