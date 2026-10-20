// SP-Dispatch C baseline: the 20 opcodes of src/ops.rs as a computed-goto interpreter.
// It reads the same .ep9d files as the Rust variants (src/program.rs has the format) and
// prints the same result line. The Rust side verifies a program before it writes the file,
// so this file trusts the bytecode.
//
// Build: clang -O2 -o cgoto cgoto.c        Run: cgoto <program.ep9d>
// Windows (clang, target x86_64-pc-windows-msvc): the same command with -o cgoto.exe.

#ifdef _WIN32
#define _CRT_SECURE_NO_WARNINGS // fopen is standard C; the MSVC CRT marks it deprecated.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#endif
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

enum {
  HALT, LDA_SMI, LDA_CONST, LDAR, STAR, MOV, ADD, SUB, ADD_SMI, BIT_AND, BIT_XOR, SHL_SMI,
  SHR_SMI, TEST_LESS_THAN, TEST_EQUAL, JUMP, JUMP_IF_FALSE, JUMP_LOOP, CALL, RETURN, NUM_OPS
};

#define HEADER 3
#define BUDGET 10000
#define STACK_SLOTS (1 << 16)
#define ERR_ILLEGAL 1
#define ERR_STACK_OVERFLOW 2

typedef struct {
  const uint8_t *code;
  uint64_t nregs;
} Func;

typedef struct {
  const uint8_t *code;
  const int64_t *consts;
  const Func *funcs;
  const int64_t *stack_limit;
  const uint8_t *throw_pc;
  int64_t budget;
  uint64_t interrupts, slow_paths;
  uint32_t error;
} Cx;

__attribute__((noinline, cold)) static int64_t add_slow(Cx *cx, int64_t a, int64_t b) {
  cx->slow_paths++;
  return (int64_t)((uint64_t)a + (uint64_t)b);
}
__attribute__((noinline, cold)) static int64_t sub_slow(Cx *cx, int64_t a, int64_t b) {
  cx->slow_paths++;
  return (int64_t)((uint64_t)a - (uint64_t)b);
}
__attribute__((noinline, cold)) static void interrupt(Cx *cx) {
  cx->interrupts++;
  cx->budget = BUDGET;
}
__attribute__((noinline, cold)) static int64_t stack_overflow(Cx *cx) {
  cx->error = ERR_STACK_OVERFLOW;
  return INT64_MIN;
}

static inline int64_t add_i32(Cx *cx, int64_t a, int64_t b) {
  int64_t s = (int64_t)((uint64_t)a + (uint64_t)b);
  return (int64_t)(int32_t)s == s ? s : add_slow(cx, a, b);
}
static inline int64_t sub_i32(Cx *cx, int64_t a, int64_t b) {
  int64_t s = (int64_t)((uint64_t)a - (uint64_t)b);
  return (int64_t)(int32_t)s == s ? s : sub_slow(cx, a, b);
}
static inline intptr_t rd_off(const uint8_t *p) {
  int16_t v;
  memcpy(&v, p, 2);
  return v;
}

__attribute__((noinline)) static int64_t run(Cx *cx, const uint8_t *pc, int64_t *fp, int64_t acc) {
  // [0 ... 255] fills every slot first; the entries after it override that on purpose.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Winitializer-overrides"
  static void *const T[256] = {
    [0 ... 255] = &&L_ILLEGAL,
    [HALT] = &&L_HALT, [LDA_SMI] = &&L_LDA_SMI, [LDA_CONST] = &&L_LDA_CONST, [LDAR] = &&L_LDAR,
    [STAR] = &&L_STAR, [MOV] = &&L_MOV, [ADD] = &&L_ADD, [SUB] = &&L_SUB, [ADD_SMI] = &&L_ADD_SMI,
    [BIT_AND] = &&L_BIT_AND, [BIT_XOR] = &&L_BIT_XOR, [SHL_SMI] = &&L_SHL_SMI,
    [SHR_SMI] = &&L_SHR_SMI, [TEST_LESS_THAN] = &&L_TEST_LESS_THAN, [TEST_EQUAL] = &&L_TEST_EQUAL,
    [JUMP] = &&L_JUMP, [JUMP_IF_FALSE] = &&L_JUMP_IF_FALSE, [JUMP_LOOP] = &&L_JUMP_LOOP,
    [CALL] = &&L_CALL, [RETURN] = &&L_RETURN,
  };
#pragma clang diagnostic pop
#define DISPATCH() goto *T[*pc]
#define NEXT(n) do { pc += (n); DISPATCH(); } while (0)
  DISPATCH();
L_HALT:
  return acc;
L_LDA_SMI:
  acc = (int8_t)pc[1];
  NEXT(2);
L_LDA_CONST:
  acc = cx->consts[pc[1]];
  NEXT(2);
L_LDAR:
  acc = fp[pc[1]];
  NEXT(2);
L_STAR:
  fp[pc[1]] = acc;
  NEXT(2);
L_MOV:
  fp[pc[2]] = fp[pc[1]];
  NEXT(3);
L_ADD:
  acc = add_i32(cx, fp[pc[1]], acc);
  NEXT(2);
L_SUB:
  acc = sub_i32(cx, fp[pc[1]], acc);
  NEXT(2);
L_ADD_SMI:
  acc = add_i32(cx, acc, (int8_t)pc[1]);
  NEXT(2);
L_BIT_AND:
  acc = (int32_t)fp[pc[1]] & (int32_t)acc;
  NEXT(2);
L_BIT_XOR:
  acc = (int32_t)fp[pc[1]] ^ (int32_t)acc;
  NEXT(2);
L_SHL_SMI:
  acc = (int32_t)((uint32_t)acc << (pc[1] & 31));
  NEXT(2);
L_SHR_SMI:
  acc = (int32_t)((uint32_t)acc >> (pc[1] & 31));
  NEXT(2);
L_TEST_LESS_THAN:
  acc = fp[pc[1]] < acc;
  NEXT(2);
L_TEST_EQUAL:
  acc = fp[pc[1]] == acc;
  NEXT(2);
L_JUMP:
  pc += rd_off(pc + 1);
  DISPATCH();
L_JUMP_IF_FALSE:
  if (acc == 0) pc += rd_off(pc + 1); else pc += 3;
  DISPATCH();
L_JUMP_LOOP:
  if (--cx->budget == 0) interrupt(cx);
  pc += rd_off(pc + 1);
  DISPATCH();
L_CALL: {
  const Func *callee = &cx->funcs[pc[1]];
  uint64_t args = pc[2], argc = pc[3];
  uint64_t cur = (uint64_t)fp[-1];
  // Integer math for the limit check: a pointer past the stack end is undefined in C.
  uintptr_t nfp_u = (uintptr_t)fp + (cur + HEADER) * sizeof(int64_t);
  if (nfp_u + callee->nregs * sizeof(int64_t) > (uintptr_t)cx->stack_limit) {
    acc = stack_overflow(cx);
    pc = cx->throw_pc;
    DISPATCH();
  }
  int64_t *nfp = fp + cur + HEADER;
  nfp[-3] = (int64_t)(cur + HEADER);
  nfp[-2] = (int64_t)(pc + 4 - cx->code);
  nfp[-1] = (int64_t)callee->nregs;
  uint64_t i = 0;
  for (; i < argc; i++) nfp[i] = fp[args + i];
  for (; i < callee->nregs; i++) nfp[i] = 0;
  fp = nfp;
  pc = callee->code;
  DISPATCH();
}
L_RETURN: {
  int64_t back = fp[-3], ret = fp[-2];
  fp -= back;
  pc = cx->code + ret;
  DISPATCH();
}
L_ILLEGAL:
  cx->error = ERR_ILLEGAL;
  return INT64_MIN;
}

// Monotonic time in ns. Windows has no clock_gettime: it uses QueryPerformanceCounter, the
// same clock as Rust's Instant on Windows. Elsewhere CLOCK_MONOTONIC, as Instant uses.
#ifdef _WIN32
static uint64_t now_ns(void) {
  static LARGE_INTEGER freq; // ticks per second; fixed at boot
  LARGE_INTEGER t;
  if (freq.QuadPart == 0) QueryPerformanceFrequency(&freq);
  QueryPerformanceCounter(&t);
  uint64_t q = (uint64_t)t.QuadPart, f = (uint64_t)freq.QuadPart;
  // Seconds and remainder apart, so q * 10^9 cannot overflow.
  return q / f * 1000000000ull + q % f * 1000000000ull / f;
}
#else
static uint64_t now_ns(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
#endif

static uint32_t rd32(const uint8_t **p, const uint8_t *end) {
  if (end - *p < 4) { fprintf(stderr, "cgoto: truncated file\n"); exit(2); }
  uint32_t v;
  memcpy(&v, *p, 4);
  *p += 4;
  return v;
}

int main(int argc, char **argv) {
  if (argc != 2) { fprintf(stderr, "usage: cgoto <program.ep9d>\n"); return 2; }
  FILE *f = fopen(argv[1], "rb");
  if (!f) { perror(argv[1]); return 2; }
  static uint8_t buf[1 << 20];
  size_t n = fread(buf, 1, sizeof buf, f);
  fclose(f);
  const uint8_t *p = buf, *end = buf + n;
  if (n < 8 || memcmp(p, "EP9D", 4) != 0) { fprintf(stderr, "cgoto: bad magic\n"); return 2; }
  p += 4;
  if (rd32(&p, end) != 1) { fprintf(stderr, "cgoto: bad version\n"); return 2; }
  uint32_t code_len = rd32(&p, end);
  const uint8_t *code = p;
  p += code_len;
  uint32_t nconsts = rd32(&p, end);
  int64_t *consts = calloc(nconsts + 1, sizeof(int64_t));
  memcpy(consts, p, nconsts * sizeof(int64_t));
  p += nconsts * sizeof(int64_t);
  uint32_t nfuncs = rd32(&p, end);
  Func *funcs = calloc(nfuncs, sizeof(Func));
  for (uint32_t i = 0; i < nfuncs; i++) {
    uint32_t off = rd32(&p, end), len = rd32(&p, end), nregs = rd32(&p, end);
    (void)len;
    funcs[i].code = code + off;
    funcs[i].nregs = nregs;
  }
  uint32_t throw_off = rd32(&p, end);
  if (p != end) { fprintf(stderr, "cgoto: trailing bytes\n"); return 2; }

  int64_t *stack = calloc(STACK_SLOTS, sizeof(int64_t));
  int64_t *fp = stack + HEADER;
  fp[-1] = (int64_t)funcs[0].nregs;
  Cx cx = {
    .code = code, .consts = consts, .funcs = funcs, .stack_limit = stack + STACK_SLOTS,
    .throw_pc = code + throw_off, .budget = BUDGET,
  };
  uint64_t t0 = now_ns();
  int64_t r = run(&cx, funcs[0].code, fp, 0);
  uint64_t ns = now_ns() - t0;
  printf("result=%lld interrupts=%llu slow=%llu error=%u ns=%llu\n", (long long)r,
         (unsigned long long)cx.interrupts, (unsigned long long)cx.slow_paths, cx.error,
         (unsigned long long)ns);
  return 0;
}
