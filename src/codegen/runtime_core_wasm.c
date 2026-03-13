/**
 * runtime_core_wasm.c — wasm-min 最小ランタイム (runtime_core 相当)
 *
 * W-2a: Runtime 境界の分類
 * ==========================
 *
 * native_runtime.c (8000+ 行) の全ランタイム関数は以下の 4 カテゴリに分類される:
 *
 *   runtime_core:        I/O (stdout/stderr/debug), 整数演算, ブール演算,
 *                         Div/Mod mold, poly_eq/neq, retain/release (no-op)
 *   runtime_collections: BuchiPack, List, HashMap, Set, String 操作,
 *                         Lax, Result, Gorillax, Molten, Stub, Todo, Cage, JSON
 *   runtime_os:          ファイルI/O, プロセス, 環境変数, CLI 引数
 *   runtime_async:       Async[T], spawn, cancel, all, race, map
 *
 * wasm-min は runtime_core のみをリンクする。
 * native_runtime.c をそのまま wasm に持ち込まない。
 * wasm-min v1 の許可機能: static string print / int print / integer arithmetic のみ。
 *
 * 本ファイル (runtime_core_wasm.c) が wasm-min の全 runtime である。
 * wasm-ld --gc-sections + --strip-all により、未使用関数は .wasm に含まれない。
 * これにより hello_world で RC / collection / OS / async コードが一切混入しないことが
 * 構造的に保証される。
 *
 * WASI fd_write ベースの stdout のみ。ヒープアロケーション禁止。
 * RC / retain / release / collection runtime を持ち込まない。
 *
 * 全値は int64_t (boxed value) として統一。文字列は NUL 終端ポインタを
 * int64_t にキャストして保持する（Native backend と同一表現）。
 * static string literal は heap header を持たない borrowed data として扱う。
 * wasm data section に直接配置され、malloc/free/RC を必要としない。
 *
 * サポートする機能:
 *   - taida_io_stdout(val)    : 文字列ポインタ → stdout + 改行
 *   - taida_io_stderr(val)    : 文字列ポインタ → stderr + 改行
 *   - taida_debug_int(val)    : i64 の 10 進文字列化 + stdout
 *   - taida_debug_str(val)    : debug(Str) — 文字列ポインタ → stdout + 改行
 *   - taida_debug_bool(val)   : debug(Bool) — "true"/"false" + 改行
 *   - taida_int_add/sub/mul   : 整数演算
 *   - taida_int_neg            : 符号反転
 *   - taida_int_eq/neq/lt/gt/gte : 整数比較
 *   - taida_bool_and/or/not   : ブール演算
 *   - taida_div_mold/mod_mold : 除算/剰余（簡易版、ゼロ除算は 0 を返す）
 *   - taida_generic_unmold    : identity（Lax ラッパーなし）
 *   - taida_poly_eq/neq       : 多態比較（整数比較のみ）
 *   - taida_float_add/sub/mul   : Float 演算 (boxed float as int64_t via bitcast)
 *   - taida_float_neg           : Float 符号反転
 *   - taida_debug_float         : debug(Float) — f64 の文字列化 + stdout
 *   - taida_int_to_float        : Int→Float 変換 (bitcast to int64_t)
 *   - taida_float_to_int        : Float→Int 変換 (truncate toward zero)
 *   - taida_retain/taida_release : no-op (wasm-min ではヒープなし)
 *   - _start                  : WASI エントリポイント (_taida_main を呼び出す)
 */

#include <stdint.h>

/* ── WASI fd_write import ── */

typedef int32_t wasi_fd;

/* iovec: pointer(i32) + length(i32) */
typedef struct {
    int32_t buf;    /* pointer to data (wasm linear memory offset) */
    int32_t len;    /* length in bytes */
} __attribute__((packed)) wasi_ciovec;

/* WASI fd_write: (fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno */
__attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")))
extern int32_t __wasi_fd_write(wasi_fd fd, const wasi_ciovec *iovs,
                               int32_t iovs_len, int32_t *nwritten);

/* ── helper: write buffer to stdout ── */

static void write_stdout(const char *buf, int32_t len) {
    wasi_ciovec iov;
    iov.buf = (int32_t)(intptr_t)buf;
    iov.len = len;
    int32_t nwritten;
    __wasi_fd_write(1, &iov, 1, &nwritten);
}

/* ── libc stubs (no libc in freestanding wasm) ── */
/* clang may emit calls to memcpy/memset even for manual loops at -O2.
   Provide minimal implementations for the WASM freestanding environment. */

void *memcpy(void *dest, const void *src, unsigned long n) {
    char *d = (char *)dest;
    const char *s2 = (const char *)src;
    while (n--) *d++ = *s2++;
    return dest;
}

void *memset(void *dest, int c, unsigned long n) {
    char *d = (char *)dest;
    while (n--) *d++ = (char)c;
    return dest;
}

/* ── W-3: Bump allocator (WASM linear memory) ── */
/* Simple bump allocator that never frees. Suitable for wasm-min's
   short-lived programs where memory pressure is minimal.
   Uses __builtin_wasm_memory_size and __builtin_wasm_memory_grow
   to manage WASM linear memory pages (64KB each). */

/* WW-2: bump_ptr and wasm_alloc are non-static so runtime_wasi_io.c can
   share the same allocator. For wasm-min (which doesn't link runtime_wasi_io.c),
   this change has zero behavioral effect. */
unsigned int bump_ptr = 0;  /* 0 = uninitialized */

void *wasm_alloc(unsigned int size) {
    /* Align to 8 bytes */
    size = (size + 7) & ~7u;

    if (bump_ptr == 0) {
        /* Initialize: start after stack/data. Use __heap_base linker symbol. */
        extern unsigned int __heap_base;
        bump_ptr = (unsigned int)(unsigned long)&__heap_base;
        /* Align to 8 bytes */
        bump_ptr = (bump_ptr + 7) & ~7u;
    }

    unsigned int result = bump_ptr;
    bump_ptr += size;

    /* Check if we need to grow memory */
    unsigned int pages_needed = (bump_ptr + 65535) / 65536;
    unsigned int current_pages = __builtin_wasm_memory_size(0);
    if (pages_needed > current_pages) {
        int grew = __builtin_wasm_memory_grow(0, pages_needed - current_pages);
        if (grew == -1) {
            /* Out of memory — return NULL (will crash on use) */
            return (void *)0;
        }
    }

    return (void *)(unsigned long)result;
}

/* ── strlen (no libc) ── */

static int32_t wasm_strlen(const char *s) {
    int32_t n = 0;
    while (s[n]) n++;
    return n;
}

/* ── taida_io_stdout: stdout 出力（boxed 文字列ポインタ） ── */

int64_t taida_io_stdout(int64_t val_ptr) {
    const char *s = (const char *)(intptr_t)val_ptr;
    if (s) {
        int32_t len = wasm_strlen(s);
        write_stdout(s, len);
        write_stdout("\n", 1);
    }
    return 0;
}

/* ── taida_io_stderr: stderr 出力（wasm-min では stdout にフォールバック） ── */

int64_t taida_io_stderr(int64_t val_ptr) {
    const char *s = (const char *)(intptr_t)val_ptr;
    if (s) {
        int32_t len = wasm_strlen(s);
        /* stderr = fd 2 */
        wasi_ciovec iov;
        iov.buf = (int32_t)(intptr_t)s;
        iov.len = len;
        int32_t nwritten;
        __wasi_fd_write(2, &iov, 1, &nwritten);
        iov.buf = (int32_t)(intptr_t)"\n";
        iov.len = 1;
        __wasi_fd_write(2, &iov, 1, &nwritten);
    }
    return 0;
}

/* ── taida_debug_int: i64 の 10 進出力 ── */

int64_t taida_debug_int(int64_t val) {
    /* stack-local buffer で i64 → 10 進文字列化 */
    char buf[21]; /* -9223372036854775808 = 20 chars + NUL */
    int pos = 20;
    buf[pos] = '\0';

    int negative = 0;
    uint64_t uval;
    if (val < 0) {
        negative = 1;
        uval = (uint64_t)(-(val + 1)) + 1;
    } else {
        uval = (uint64_t)val;
    }

    if (uval == 0) {
        buf[--pos] = '0';
    } else {
        while (uval > 0) {
            buf[--pos] = '0' + (char)(uval % 10);
            uval /= 10;
        }
    }

    if (negative) {
        buf[--pos] = '-';
    }

    int32_t len = 20 - pos;
    write_stdout(buf + pos, len);
    write_stdout("\n", 1);
    return 0;
}

/* ── taida_debug_str: debug(Str) — 文字列の出力 ── */

int64_t taida_debug_str(int64_t val_ptr) {
    const char *s = (const char *)(intptr_t)val_ptr;
    if (s) {
        int32_t len = wasm_strlen(s);
        write_stdout(s, len);
        write_stdout("\n", 1);
    }
    return 0;
}

/* ── taida_debug_polymorphic: forward declaration (impl after _wasm_value_to_display_string) ── */
int64_t taida_debug_polymorphic(int64_t val);

/* ── taida_debug_bool: debug(Bool) ── */

int64_t taida_debug_bool(int64_t val) {
    if (val) {
        write_stdout("true\n", 5);
    } else {
        write_stdout("false\n", 6);
    }
    return 0;
}

/* ── 整数演算 ── */

int64_t taida_int_add(int64_t a, int64_t b) { return a + b; }
int64_t taida_int_sub(int64_t a, int64_t b) { return a - b; }
int64_t taida_int_mul(int64_t a, int64_t b) { return a * b; }
int64_t taida_int_neg(int64_t a) { return -a; }

/* ── 整数比較 ── */

int64_t taida_int_eq(int64_t a, int64_t b) { return a == b ? 1 : 0; }
int64_t taida_int_neq(int64_t a, int64_t b) { return a != b ? 1 : 0; }
int64_t taida_int_lt(int64_t a, int64_t b) { return a < b ? 1 : 0; }
int64_t taida_int_gt(int64_t a, int64_t b) { return a > b ? 1 : 0; }
int64_t taida_int_gte(int64_t a, int64_t b) { return a >= b ? 1 : 0; }

/* ── ブール演算 ── */

int64_t taida_bool_and(int64_t a, int64_t b) { return (a && b) ? 1 : 0; }
int64_t taida_bool_or(int64_t a, int64_t b) { return (a || b) ? 1 : 0; }
int64_t taida_bool_not(int64_t a) { return a ? 0 : 1; }

/* ── Forward declarations for Lax/Result/Gorillax (defined in W-5 section below) ── */
int64_t taida_lax_new(int64_t value, int64_t default_value);
int64_t taida_lax_empty(int64_t default_value);
int64_t taida_lax_unmold(int64_t lax_ptr);
static int _wasm_is_lax(int64_t val);
static int _wasm_is_result(int64_t val);
static int _wasm_is_gorillax(int64_t val);
int64_t taida_pack_get_idx(int64_t pack_ptr, int64_t index);
int64_t taida_pack_new(int64_t field_count);
int64_t taida_pack_get(int64_t pack_ptr, int64_t field_hash);
int64_t taida_pack_has_hash(int64_t pack_ptr, int64_t field_hash);
int64_t taida_throw(int64_t error_val);
int64_t taida_make_error(int64_t type_ptr, int64_t msg_ptr);
int64_t taida_can_throw_payload(int64_t val);
static int64_t _wasm_invoke_callback1(int64_t fn_ptr, int64_t arg0);
static int64_t _wasm_result_is_error_check(int64_t result);

/* ── Hash constants needed by taida_generic_unmold (full definitions in W-5) ── */
#define WASM_HASH___TYPE      0x84d2d84b631f799bLL  /* FNV-1a("__type") */
#define WASM_HASH___VALUE     0x0a7fc9f13472bbe0LL  /* FNV-1a("__value") */
#define WASM_HASH___DEFAULT   0xed4fba440f8602d4LL  /* FNV-1a("__default") */
#define WASM_HASH_TODO_SOL    0x824fa3195cf2e6c1LL  /* FNV-1a("sol") */
#define WASM_HASH_TODO_UNM    0x4cadac193e198b15LL  /* FNV-1a("unm") */

/* ── Div/Mod mold — W-5: now returns Lax (matching native backend) ── */

int64_t taida_div_mold(int64_t a, int64_t b) {
    if (b == 0) return taida_lax_empty(0);
    return taida_lax_new(a / b, 0);
}

int64_t taida_mod_mold(int64_t a, int64_t b) {
    if (b == 0) return taida_lax_empty(0);
    return taida_lax_new(a % b, 0);
}

/* ── generic_unmold — W-5g: Lax/Result/Gorillax-aware, predicate-evaluated ── */

int64_t taida_generic_unmold(int64_t val) {
    if (_wasm_is_lax(val)) {
        return taida_lax_unmold(val);
    }
    if (_wasm_is_result(val)) {
        /* Result unmold: evaluate predicate + check throw (matching native) */
        int64_t value = taida_pack_get_idx(val, 0);      /* __value */
        int64_t pred = taida_pack_get_idx(val, 1);        /* __predicate */
        int64_t throw_val = taida_pack_get_idx(val, 2);   /* throw */

        if (throw_val != 0) {
            if (pred != 0) {
                int64_t pred_result = _wasm_invoke_callback1(pred, value);
                if (!pred_result) {
                    /* Predicate failed — throw the error */
                    if (taida_can_throw_payload(throw_val)) return taida_throw(throw_val);
                    int64_t error = taida_make_error(
                        (int64_t)(intptr_t)"ResultError",
                        (int64_t)(intptr_t)"Result predicate failed");
                    return taida_throw(error);
                }
                /* Predicate passed even with throw set — return value */
                return value;
            }
            /* No predicate, throw is set — throw */
            if (taida_can_throw_payload(throw_val)) return taida_throw(throw_val);
            int64_t error = taida_make_error(
                (int64_t)(intptr_t)"ResultError",
                (int64_t)(intptr_t)"Result error");
            return taida_throw(error);
        }

        /* Evaluate predicate if present (no throw set) */
        if (pred != 0) {
            int64_t pred_result = _wasm_invoke_callback1(pred, value);
            if (pred_result) return value;  /* success */
            /* Predicate failed — throw default error */
            int64_t error = taida_make_error(
                (int64_t)(intptr_t)"ResultError",
                (int64_t)(intptr_t)"Result predicate failed");
            return taida_throw(error);
        }

        return value; /* no throw, no predicate — success */
    }
    if (_wasm_is_gorillax(val)) {
        /* Gorillax unmold: return __value if ok, throw otherwise */
        int64_t is_ok = taida_pack_get_idx(val, 0);
        if (is_ok) return taida_pack_get_idx(val, 1); /* __value */
        int64_t error_val = taida_pack_get_idx(val, 2); /* __error */
        if (taida_can_throw_payload(error_val)) return taida_throw(error_val);
        int64_t err = taida_make_error(
            (int64_t)(intptr_t)"GorillaxError",
            (int64_t)(intptr_t)"Gorillax error");
        return taida_throw(err);
    }
    /* BE-WASM-1: TODO unmold — return unm channel, fallback to sol/__default/__value.
       Matches native_runtime.c taida_generic_unmold TODO branch. */
    if (taida_pack_has_hash(val, WASM_HASH___TYPE)) {
        int64_t type_ptr = taida_pack_get(val, WASM_HASH___TYPE);
        /* Guard: ensure type_ptr looks like a valid pointer (> 4096) */
        if ((intptr_t)type_ptr <= 4096) return val;
        const char *type_str = (const char *)(intptr_t)type_ptr;
        if (type_str != 0 && type_str[0] == 'T' && type_str[1] == 'O' &&
            type_str[2] == 'D' && type_str[3] == 'O' && type_str[4] == '\0') {
            /* TODO pack: prefer unm > __default > sol > __value */
            if (taida_pack_has_hash(val, WASM_HASH_TODO_UNM))
                return taida_pack_get(val, WASM_HASH_TODO_UNM);
            if (taida_pack_has_hash(val, WASM_HASH___DEFAULT))
                return taida_pack_get(val, WASM_HASH___DEFAULT);
            if (taida_pack_has_hash(val, WASM_HASH_TODO_SOL))
                return taida_pack_get(val, WASM_HASH_TODO_SOL);
            if (taida_pack_has_hash(val, WASM_HASH___VALUE))
                return taida_pack_get(val, WASM_HASH___VALUE);
            return taida_pack_new(0);
        }
        /* Molten detection: cannot unmold Molten directly */
        if (type_str != 0 && type_str[0] == 'M' && type_str[1] == 'o' &&
            type_str[2] == 'l' && type_str[3] == 't' && type_str[4] == 'e' &&
            type_str[5] == 'n' && type_str[6] == '\0') {
            int64_t error = taida_make_error(
                (int64_t)(intptr_t)"TypeError",
                (int64_t)(intptr_t)"Cannot unmold Molten directly. Molten can only be used inside Cage.");
            return taida_throw(error);
        }
        /* Custom mold: pack with __type and __value fields */
        if (taida_pack_has_hash(val, WASM_HASH___VALUE))
            return taida_pack_get(val, WASM_HASH___VALUE);
    }
    return val;
}

/* ── 多態比較 (wasm-min: 整数比較のみ) ── */

int64_t taida_poly_eq(int64_t a, int64_t b) { return a == b ? 1 : 0; }
int64_t taida_poly_neq(int64_t a, int64_t b) { return a != b ? 1 : 0; }

/* ── W-3: String operations (dynamic allocation via bump allocator) ── */

int64_t taida_str_concat(int64_t a_ptr, int64_t b_ptr) {
    const char *a = (const char *)(intptr_t)a_ptr;
    const char *b = (const char *)(intptr_t)b_ptr;
    if (!a) a = "";
    if (!b) b = "";
    int32_t la = wasm_strlen(a);
    int32_t lb = wasm_strlen(b);
    char *buf = (char *)wasm_alloc(la + lb + 1);
    if (!buf) return 0;
    for (int32_t i = 0; i < la; i++) buf[i] = a[i];
    for (int32_t i = 0; i < lb; i++) buf[la + i] = b[i];
    buf[la + lb] = '\0';
    return (int64_t)(intptr_t)buf;
}

int64_t taida_str_length(int64_t s_ptr) {
    const char *s = (const char *)(intptr_t)s_ptr;
    if (!s) return 0;
    return (int64_t)wasm_strlen(s);
}

int64_t taida_str_eq(int64_t a_ptr, int64_t b_ptr) {
    const char *a = (const char *)(intptr_t)a_ptr;
    const char *b = (const char *)(intptr_t)b_ptr;
    if (a == b) return 1;
    if (!a || !b) return 0;
    while (*a && *b) {
        if (*a != *b) return 0;
        a++; b++;
    }
    return *a == *b ? 1 : 0;
}

int64_t taida_str_neq(int64_t a_ptr, int64_t b_ptr) {
    return taida_str_eq(a_ptr, b_ptr) ? 0 : 1;
}

/* ── W-3: Type conversions ── */

int64_t taida_int_to_str(int64_t a) {
    /* Same algorithm as taida_debug_int but returns a heap string */
    char tmp[21];
    int pos = 20;
    tmp[pos] = '\0';

    int negative = 0;
    uint64_t uval;
    if (a < 0) {
        negative = 1;
        uval = (uint64_t)(-(a + 1)) + 1;
    } else {
        uval = (uint64_t)a;
    }

    if (uval == 0) {
        tmp[--pos] = '0';
    } else {
        while (uval > 0) {
            tmp[--pos] = '0' + (char)(uval % 10);
            uval /= 10;
        }
    }
    if (negative) {
        tmp[--pos] = '-';
    }

    int32_t len = 20 - pos;
    char *buf = (char *)wasm_alloc(len + 1);
    if (!buf) return 0;
    for (int i = 0; i < len; i++) buf[i] = tmp[pos + i];
    buf[len] = '\0';
    return (int64_t)(intptr_t)buf;
}

int64_t taida_str_to_int(int64_t s_ptr) {
    const char *s = (const char *)(intptr_t)s_ptr;
    if (!s) return 0;
    int64_t result = 0;
    int negative = 0;
    int i = 0;
    if (s[i] == '-') { negative = 1; i++; }
    else if (s[i] == '+') { i++; }
    while (s[i] >= '0' && s[i] <= '9') {
        result = result * 10 + (s[i] - '0');
        i++;
    }
    return negative ? -result : result;
}

int64_t taida_str_from_bool(int64_t v) {
    /* Returns static string "true" or "false" — no alloc needed */
    return v ? (int64_t)(intptr_t)"true" : (int64_t)(intptr_t)"false";
}

/* ── W-3: Int methods ── */

int64_t taida_int_abs(int64_t a) { return a < 0 ? -a : a; }
int64_t taida_int_lte(int64_t a, int64_t b) { return a <= b ? 1 : 0; }

/* ── W-3: Float→Str (uses bump allocator) ── */

static int64_t taida_float_to_str_impl(double d);

/* ── W-3: f64 <-> i64 bitcast helpers ── */
/* Same representation as native backend: f64 bits stored in int64_t */

static double _l2d(int64_t v) {
    union { int64_t l; double d; } u;
    u.l = v;
    return u.d;
}

static int64_t _d2l(double v) {
    union { int64_t l; double d; } u;
    u.d = v;
    return u.l;
}

/* Smart conversion: if the bit pattern looks like a small integer, convert;
   otherwise treat as f64 bit pattern. Matches native runtime's _to_double(). */
static double _to_double(int64_t v) {
    if (v >= -1048576 && v <= 1048576) {
        return (double)v;
    }
    return _l2d(v);
}

/* ── W-3: Float 演算 (boxed float as int64_t) ── */

int64_t taida_float_add(int64_t a, int64_t b) { return _d2l(_to_double(a) + _to_double(b)); }
int64_t taida_float_sub(int64_t a, int64_t b) { return _d2l(_to_double(a) - _to_double(b)); }
int64_t taida_float_mul(int64_t a, int64_t b) { return _d2l(_to_double(a) * _to_double(b)); }
int64_t taida_float_neg(int64_t a) { return _d2l(-_to_double(a)); }

/* ── W-3f: %g-equivalent float formatter ── */
/* Implements printf("%g") behavior: 6 significant digits, scientific notation
   when exponent < -4 or >= 6, trailing zeros trimmed. */

static int fmt_g(double d, char *buf, int bufsize) {
    int len = 0;

    /* Handle negative */
    if (d < 0) { buf[len++] = '-'; d = -d; }

    /* NaN check: NaN != NaN */
    if (d != d) { buf[len++]='N'; buf[len++]='a'; buf[len++]='N'; return len; }
    /* Infinity */
    if (d > 1e308) { buf[len++]='i'; buf[len++]='n'; buf[len++]='f'; return len; }

    /* Zero */
    if (d == 0.0) { buf[len++] = '0'; return len; }

    /* Compute base-10 exponent */
    int exp10 = 0;
    double norm = d;
    if (norm >= 10.0) {
        while (norm >= 10.0) { norm /= 10.0; exp10++; }
    } else if (norm < 1.0) {
        while (norm < 1.0) { norm *= 10.0; exp10--; }
    }
    /* norm is in [1.0, 10.0) */

    /* %g precision=6: use scientific if exp < -4 or exp >= 6 */
    int use_sci = (exp10 < -4 || exp10 >= 6);

    if (use_sci) {
        /* Scientific notation: d.dddddde+/-dd */
        /* Round norm to 6 significant digits */
        double rounded = norm;
        {
            double factor = 1e5; /* 10^(6-1) */
            rounded = (double)((uint64_t)(rounded * factor + 0.5)) / factor;
            if (rounded >= 10.0) { rounded /= 10.0; exp10++; }
        }

        /* Extract digits: first digit . remaining */
        int first = (int)rounded;
        if (first > 9) first = 9;
        buf[len++] = '0' + (char)first;

        double frac = rounded - (double)first;
        /* Up to 5 more significant digits */
        int frac_start = len;
        if (frac > 0.0000005) {
            buf[len++] = '.';
            frac_start = len;
            for (int i = 0; i < 5; i++) {
                frac *= 10.0;
                int digit = (int)frac;
                if (digit > 9) digit = 9;
                frac -= (double)digit;
                buf[len++] = '0' + (char)digit;
            }
            /* Round */
            if (frac >= 0.5 && len > frac_start) {
                int carry = 1;
                for (int i = len - 1; i >= frac_start && carry; i--) {
                    int d2 = (buf[i] - '0') + carry;
                    if (d2 >= 10) { buf[i] = '0'; carry = 1; }
                    else { buf[i] = '0' + (char)d2; carry = 0; }
                }
            }
            /* Trim trailing zeros */
            while (len > frac_start && buf[len-1] == '0') len--;
            if (len == frac_start) len--; /* remove dot */
        }

        /* Exponent */
        buf[len++] = 'e';
        if (exp10 < 0) { buf[len++] = '-'; exp10 = -exp10; }
        else { buf[len++] = '+'; }
        if (exp10 >= 100) {
            buf[len++] = '0' + (char)(exp10 / 100);
            buf[len++] = '0' + (char)((exp10 / 10) % 10);
            buf[len++] = '0' + (char)(exp10 % 10);
        } else {
            buf[len++] = '0' + (char)(exp10 / 10);
            buf[len++] = '0' + (char)(exp10 % 10);
        }
    } else {
        /* Fixed notation */
        /* We have 6 significant digits total.
           Number of integer digits = exp10 + 1.
           Number of fractional significant digits = 6 - (exp10 + 1) = 5 - exp10 */

        /* Round d to 6 significant digits */
        {
            double factor = 1.0;
            for (int i = 0; i < 5 - exp10; i++) factor *= 10.0;
            d = (double)((uint64_t)(d * factor + 0.5)) / factor;
        }

        uint64_t ipart = (uint64_t)d;
        double frac = d - (double)ipart;

        /* Integer part */
        char itmp[21];
        int ipos = 20;
        itmp[ipos] = '\0';
        if (ipart == 0) { itmp[--ipos] = '0'; }
        else { while (ipart > 0) { itmp[--ipos] = '0' + (char)(ipart % 10); ipart /= 10; } }
        for (int i = ipos; i < 20; i++) buf[len++] = itmp[i];

        /* Fractional part */
        int frac_digits = 5 - exp10;
        if (frac_digits < 0) frac_digits = 0;
        if (frac_digits > 0 && frac > 0.0000005) {
            buf[len++] = '.';
            int frac_start = len;
            for (int i = 0; i < frac_digits; i++) {
                frac *= 10.0;
                int digit = (int)frac;
                if (digit > 9) digit = 9;
                frac -= (double)digit;
                buf[len++] = '0' + (char)digit;
            }
            /* Round */
            if (frac >= 0.5 && len > frac_start) {
                int carry = 1;
                for (int i = len - 1; i >= frac_start && carry; i--) {
                    int d2 = (buf[i] - '0') + carry;
                    if (d2 >= 10) { buf[i] = '0'; carry = 1; }
                    else { buf[i] = '0' + (char)d2; carry = 0; }
                }
            }
            /* Trim trailing zeros */
            while (len > frac_start && buf[len-1] == '0') len--;
            if (len == frac_start) len--; /* remove dot */
        }
    }

    return len;
}

/* ── W-3: taida_debug_float: debug(Float) — f64 の文字列化 + stdout ── */

int64_t taida_debug_float(int64_t val) {
    double d = _l2d(val);
    char buf[64];
    int len = fmt_g(d, buf, 64);
    write_stdout(buf, len);
    write_stdout("\n", 1);
    return 0;
}

/* ── W-3: Int→Float / Float→Int 変換 ── */

int64_t taida_int_to_float(int64_t a) {
    return _d2l((double)a);
}

int64_t taida_float_to_int(int64_t a) {
    return (int64_t)_to_double(a);
}

/* ── W-3: taida_float_to_str: Float→Str 変換 (uses bump allocator) ── */

int64_t taida_float_to_str(int64_t val) {
    double d = _l2d(val);
    char tmp[64];
    int len = fmt_g(d, tmp, 64);

    char *buf = (char *)wasm_alloc(len + 1);
    if (!buf) return 0;
    for (int i = 0; i < len; i++) buf[i] = tmp[i];
    buf[len] = '\0';
    return (int64_t)(intptr_t)buf;
}

/* W-4: Forward declarations for list functions (used by polymorphic helpers) */
int64_t taida_list_length(int64_t list_ptr);

/* W-4f2: Collection type markers for polymorphic dispatch */
#define WASM_SET_MARKER_VAL  0x53455400LL  /* "SET\0" */
/* W-4f2: Collection type markers and layout constants */
#define WASM_HM_HEADER 4
#define WASM_HM_MARKER_VAL 0x484D4150LL  /* "HMAP" — distinguishes HashMap from List/Set */

/* W-4f2: List/Set element offset — header is [cap, len, elem_tag, type_marker] = 4 slots */
#define WASM_LIST_ELEMS 4

/* ── W-3f/W-4: taida_polymorphic_length (wasm-min: string + list) ── */

/* Helper: check if a pointer looks like a list (bump-allocated).
   Lists have [capacity(int64), length(int64), ...] where capacity >= 16.
   Strings have printable ASCII/UTF-8 as first byte.
   We check the first int64_t: if it's a reasonable capacity (8-65536),
   it's likely a list. */
static int _looks_like_list(int64_t ptr) {
    if (ptr == 0) return 0;
    if (ptr < 0 || ptr > 0xFFFFFFFF) return 0;
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)ptr;
    /* Need at least 4 int64_t slots (header: cap, len, elem_tag, type_marker) */
    if (addr + 32 > mem_size) return 0;
    int64_t *data = (int64_t *)(intptr_t)ptr;
    int64_t cap = data[0];
    int64_t len = data[1];
    /* Valid list: capacity is a reasonable power-of-2-ish number,
       length is non-negative and <= capacity */
    if (cap >= 8 && cap <= 65536 && len >= 0 && len <= cap) return 1;
    return 0;
}

/* W-4f2: Check if a pointer is a Set (has WASM_SET_MARKER_VAL at slot[3]) */
static int _is_wasm_set(int64_t ptr) {
    if (!_looks_like_list(ptr)) return 0;
    int64_t *data = (int64_t *)(intptr_t)ptr;
    return data[3] == WASM_SET_MARKER_VAL;
}

int64_t taida_polymorphic_length(int64_t ptr) {
    if (!ptr) return 0;
    /* W-4: Check if it's a list first */
    if (_looks_like_list(ptr)) {
        return taida_list_length(ptr);
    }
    /* Otherwise treat as string */
    const char *s = (const char *)(intptr_t)ptr;
    return (int64_t)wasm_strlen(s);
}

/* ── W-3f/W-4f2: taida_polymorphic_to_string (full collection support) ── */

/* Helper: check if a value looks like a valid string pointer in WASM linear memory.
   In wasm32, valid heap/data addresses are positive and within linear memory.
   We check that the pointer is in a reasonable range and points to a NUL-terminated
   byte sequence with printable or whitespace characters. */
static int _looks_like_string(int64_t val) {
    /* Zero is not a string (it's the integer 0 or null) */
    if (val == 0) return 0;
    /* Negative values or values > 32-bit range are not pointers on wasm32 */
    if (val < 0 || val > 0xFFFFFFFF) return 0;
    /* Check if it's within current WASM memory */
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)val;
    if (addr >= mem_size) return 0;
    /* Check if it starts with a printable/whitespace ASCII byte (not \0) */
    const char *s = (const char *)(intptr_t)val;
    if (s[0] == '\0') return 0;
    /* Verify first few bytes are valid UTF-8/ASCII (not random garbage) */
    for (int i = 0; i < 8 && s[i]; i++) {
        unsigned char c = (unsigned char)s[i];
        /* Accept printable ASCII, whitespace, and high bytes (UTF-8 continuation) */
        if (c < 0x20 && c != '\t' && c != '\n' && c != '\r') return 0;
    }
    return 1;
}

/* W-4f2: Check if a value looks like a BuchiPack.
   Pack layout: [field_count, field0_hash, field0_tag, field0_value, ...]
   field_count is typically small (1-50), and field hashes are large int64_t values. */
static int _looks_like_pack(int64_t val) {
    if (val == 0) return 0;
    if (val < 0 || val > 0xFFFFFFFF) return 0;
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)val;
    /* Need at least 1 int64_t (field_count) */
    if (addr + 8 > mem_size) return 0;
    int64_t *data = (int64_t *)(intptr_t)val;
    int64_t fc = data[0];
    /* Valid pack: field_count is small and positive */
    if (fc < 1 || fc > 100) return 0;
    /* Verify there's enough memory for the full pack */
    int64_t total_bytes = (1 + fc * 3) * 8;
    if (addr + (unsigned int)total_bytes > mem_size) return 0;
    /* Check that at least the first field hash is a non-zero large value
       (field hashes from FNV-1a are typically large numbers) */
    int64_t first_hash = data[1];
    if (first_hash == 0) return 0;
    return 1;
}

/* Forward declarations for toString helpers */
static int64_t _wasm_value_to_display_string(int64_t val);
static int64_t _wasm_value_to_debug_string(int64_t val);
static int _is_wasm_hashmap(int64_t ptr);
static const char *_wasm_lookup_field_name(int64_t hash);
static int64_t _wasm_lookup_field_type(int64_t hash);
/* W-5f: Monadic type hash constants (FNV-1a hashes of field names).
   Centralized here for use by both display_string and the runtime constructors below. */
/* WFX-2: corrected FNV-1a hashes (previous values were wrong, causing
   field access mismatch with compiler-generated hashes from simple_hash()) */
#define WASM_HASH_HAS_VALUE   0x9e9c6dc733414d60LL  /* FNV-1a("hasValue") */
#ifndef WASM_HASH___VALUE
#define WASM_HASH___VALUE     0x0a7fc9f13472bbe0LL  /* FNV-1a("__value") */
#endif
#ifndef WASM_HASH___TYPE
#define WASM_HASH___TYPE      0x84d2d84b631f799bLL  /* FNV-1a("__type") */
#endif
#define WASM_HASH_IS_OK       0x6550c1c5b98b56bfLL  /* FNV-1a("isOk") */
#define WASM_HASH___ERROR     0x15c3e6e41a99a6cbLL  /* FNV-1a("__error") */
#ifndef WASM_HASH___DEFAULT
#define WASM_HASH___DEFAULT   0xed4fba440f8602d4LL  /* FNV-1a("__default") */
#endif
#define WASM_HASH_THROW       0x5a5fe3720c9584cfLL  /* FNV-1a("throw") */
#define WASM_HASH___PREDICATE 0x15592af3c2291540LL  /* FNV-1a("__predicate") */
/* BE-WASM-1: TODO field hashes (matching native_runtime.c) */
#define WASM_HASH_TODO_ID     0x08b72e07b55c3ac0LL  /* FNV-1a("id") */
#define WASM_HASH_TODO_TASK   0xd9603bef07a9524cLL  /* FNV-1a("task") */
#ifndef WASM_HASH_TODO_SOL
#define WASM_HASH_TODO_SOL    0x824fa3195cf2e6c1LL  /* FNV-1a("sol") */
#endif
#ifndef WASM_HASH_TODO_UNM
#define WASM_HASH_TODO_UNM    0x4cadac193e198b15LL  /* FNV-1a("unm") */
#endif

/* W-4f2: Dynamic string buffer for building collection toString output */
typedef struct {
    char *buf;
    int len;
    int cap;
} _wasm_strbuf;

static void _sb_init(_wasm_strbuf *sb) {
    sb->cap = 128;
    sb->buf = (char *)wasm_alloc(sb->cap);
    sb->len = 0;
    if (sb->buf) sb->buf[0] = '\0';
}

static void _sb_ensure(_wasm_strbuf *sb, int needed) {
    if (sb->len + needed + 1 > sb->cap) {
        int new_cap = sb->cap;
        while (sb->len + needed + 1 > new_cap) new_cap *= 2;
        char *new_buf = (char *)wasm_alloc(new_cap);
        if (!new_buf) return;
        for (int i = 0; i < sb->len; i++) new_buf[i] = sb->buf[i];
        new_buf[sb->len] = '\0';
        sb->buf = new_buf;
        sb->cap = new_cap;
    }
}

static void _sb_append(_wasm_strbuf *sb, const char *s) {
    int slen = wasm_strlen(s);
    _sb_ensure(sb, slen);
    for (int i = 0; i < slen; i++) sb->buf[sb->len + i] = s[i];
    sb->len += slen;
    sb->buf[sb->len] = '\0';
}

static int64_t _sb_finish(_wasm_strbuf *sb) {
    return (int64_t)(intptr_t)sb->buf;
}

/* W-4f2: HashMap toString: HashMap({"key": value, ...}) */
/* Tombstone: hash == 1, key == 0 (same as native TAIDA_HASHMAP_TOMBSTONE_HASH) */
#define WASM_HM_TOMBSTONE_HASH 1
#define WASM_HM_SLOT_EMPTY(h, k)     ((h) == 0 && (k) == 0)
#define WASM_HM_SLOT_TOMBSTONE(h, k) ((h) == WASM_HM_TOMBSTONE_HASH && (k) == 0)
#define WASM_HM_SLOT_OCCUPIED(h, k)  (!WASM_HM_SLOT_EMPTY(h, k) && !WASM_HM_SLOT_TOMBSTONE(h, k))

static int64_t _wasm_hashmap_to_string(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    _wasm_strbuf sb;
    _sb_init(&sb);
    _sb_append(&sb, "HashMap({");
    int64_t count = 0;
    for (int64_t i = 0; i < cap; i++) {
        int64_t sh = hm[WASM_HM_HEADER + i * 3];
        int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
        if (WASM_HM_SLOT_OCCUPIED(sh, sk)) {
            int64_t value = hm[WASM_HM_HEADER + i * 3 + 2];
            if (count > 0) _sb_append(&sb, ", ");
            int64_t key_str = _wasm_value_to_debug_string(sk);
            int64_t val_str = _wasm_value_to_debug_string(value);
            _sb_append(&sb, (const char *)(intptr_t)key_str);
            _sb_append(&sb, ": ");
            _sb_append(&sb, (const char *)(intptr_t)val_str);
            count++;
        }
    }
    _sb_append(&sb, "})");
    return _sb_finish(&sb);
}

/* W-4f2: Set toString: Set({elem1, elem2, ...}) */
static int64_t _wasm_set_to_string(int64_t set_ptr) {
    int64_t *list = (int64_t *)(intptr_t)set_ptr;
    int64_t len = list[1];
    _wasm_strbuf sb;
    _sb_init(&sb);
    _sb_append(&sb, "Set({");
    for (int64_t i = 0; i < len; i++) {
        if (i > 0) _sb_append(&sb, ", ");
        /* Native uses snprintf(int64_t) for set elements — integers only */
        int64_t elem = list[WASM_LIST_ELEMS + i];
        int64_t elem_str = _wasm_value_to_display_string(elem);
        _sb_append(&sb, (const char *)(intptr_t)elem_str);
    }
    _sb_append(&sb, "})");
    return _sb_finish(&sb);
}

/* W-4f2: List toString: @[elem1, elem2, ...] */
static int64_t _wasm_list_to_string(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    _wasm_strbuf sb;
    _sb_init(&sb);
    _sb_append(&sb, "@[");
    for (int64_t i = 0; i < len; i++) {
        if (i > 0) _sb_append(&sb, ", ");
        int64_t item = list[WASM_LIST_ELEMS + i];
        int64_t item_str = _wasm_value_to_debug_string(item);
        _sb_append(&sb, (const char *)(intptr_t)item_str);
    }
    _sb_append(&sb, "]");
    return _sb_finish(&sb);
}

/* W-4f2: Pack toString: @(field <= value, ...) */
static int64_t _wasm_pack_to_string(int64_t pack_ptr) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    int64_t fc = pack[0];
    _wasm_strbuf sb;
    _sb_init(&sb);
    _sb_append(&sb, "@(");
    int count = 0;
    for (int64_t i = 0; i < fc; i++) {
        int64_t field_hash = pack[1 + i * 3];
        int64_t field_val = pack[1 + i * 3 + 2];
        const char *fname = _wasm_lookup_field_name(field_hash);
        if (!fname) continue;
        /* Skip internal __ fields for display (same as native) */
        if (fname[0] == '_' && fname[1] == '_') continue;
        if (count > 0) _sb_append(&sb, ", ");
        _sb_append(&sb, fname);
        _sb_append(&sb, " <= ");
        /* Check if field is Bool via type registry */
        int64_t ftype = _wasm_lookup_field_type(field_hash);
        if (ftype == 4) {
            /* Bool type tag = 4 in native convention */
            _sb_append(&sb, field_val ? "true" : "false");
        } else {
            int64_t val_str = _wasm_value_to_debug_string(field_val);
            _sb_append(&sb, (const char *)(intptr_t)val_str);
        }
        count++;
    }
    _sb_append(&sb, ")");
    return _sb_finish(&sb);
}

/* W-5f: Detect Lax, Result, Gorillax, RelaxedGorillax by pack structure.
   These all have fc=4 with distinctive first-field hashes:
   - Lax:              hash0 = WASM_HASH_HAS_VALUE (0x9e9c6dc733414d60)
   - Gorillax/Relaxed: hash0 = WASM_HASH_IS_OK     (0x6550c1c5b98b56bf)
   - Result:           hash0 = WASM_HASH___VALUE    (0x0a7fc9f13472bbe0) */

/* W-5g: Bounds-check helper for WASM32. On wasm32, intptr_t is 32-bit,
   so int64_t values that are bitcast floats (e.g. _d2l(3.14)) would be
   truncated and dereference invalid memory. This helper prevents that. */
static int _wasm_is_valid_ptr(int64_t val, unsigned int min_bytes) {
    if (val <= 0 || val > 0xFFFFFFFF) return 0;
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)val;
    if (addr < 4096) return 0; /* skip low addresses (null, small ints) */
    if (addr + min_bytes > mem_size) return 0;
    return 1;
}

static int _wasm_is_result(int64_t val) {
    /* Need at least 13 int64_t slots (fc + 4*3 fields) = 104 bytes */
    if (!_wasm_is_valid_ptr(val, 104)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    /* Result: fc=4, hash0 = WASM_HASH___VALUE, hash2 = WASM_HASH_THROW */
    if (p[0] == 4 && p[1] == WASM_HASH___VALUE) {
        int64_t hash2 = p[1 + 2 * 3]; /* field 2 hash */
        if (hash2 == WASM_HASH_THROW) return 1;
    }
    return 0;
}

static int _wasm_is_gorillax(int64_t val) {
    if (!_wasm_is_valid_ptr(val, 104)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    /* Gorillax/RelaxedGorillax: fc=4, hash0 = WASM_HASH_IS_OK */
    if (p[0] == 4 && p[1] == WASM_HASH_IS_OK) return 1;
    return 0;
}

/* Detect Gorillax type: 0 = unknown, 1 = Gorillax, 2 = RelaxedGorillax */
static int _wasm_gorillax_type(int64_t val) {
    int64_t *p = (int64_t *)(intptr_t)val;
    /* __type field is at index 3: p[1 + 3*3 + 2] = p[12] */
    int64_t type_str = p[1 + 3 * 3 + 2]; /* field 3 value */
    if (type_str > 4096 && _looks_like_string(type_str)) {
        const char *s = (const char *)(intptr_t)type_str;
        if (s[0] == 'G') return 1; /* "Gorillax" */
        if (s[0] == 'R') return 2; /* "RelaxedGorillax" */
    }
    return 1; /* default to Gorillax */
}

/* W-5f: Lax.toString() — "Lax(value)" or "Lax(default: value)" */
static int64_t _wasm_lax_to_string(int64_t lax_ptr) {
    int64_t has_value = taida_pack_get_idx(lax_ptr, 0); /* hasValue */
    int64_t value = taida_pack_get_idx(lax_ptr, 1);     /* __value */
    int64_t def = taida_pack_get_idx(lax_ptr, 2);       /* __default */
    int64_t rendered = has_value
        ? _wasm_value_to_display_string(value)
        : _wasm_value_to_display_string(def);
    const char *rs = (const char *)(intptr_t)rendered;
    _wasm_strbuf sb;
    _sb_init(&sb);
    if (has_value) {
        _sb_append(&sb, "Lax(");
        _sb_append(&sb, rs);
        _sb_append(&sb, ")");
    } else {
        _sb_append(&sb, "Lax(default: ");
        _sb_append(&sb, rs);
        _sb_append(&sb, ")");
    }
    return _sb_finish(&sb);
}

/* W-5g: Result.toString() — predicate-aware, matching native */
static int64_t _wasm_result_to_string(int64_t result) {
    if (!_wasm_result_is_error_check(result)) {
        /* Success case */
        int64_t value = taida_pack_get_idx(result, 0); /* __value */
        int64_t value_str = _wasm_value_to_display_string(value);
        _wasm_strbuf sb;
        _sb_init(&sb);
        _sb_append(&sb, "Result(");
        _sb_append(&sb, (const char *)(intptr_t)value_str);
        _sb_append(&sb, ")");
        return _sb_finish(&sb);
    }
    /* Error case */
    int64_t throw_val = taida_pack_get_idx(result, 2); /* throw field */
    if (throw_val == 0) {
        return (int64_t)(intptr_t)"Result(throw <= error)";
    }
    int64_t err_str = _wasm_value_to_display_string(throw_val);
    _wasm_strbuf sb;
    _sb_init(&sb);
    _sb_append(&sb, "Result(throw <= ");
    _sb_append(&sb, (const char *)(intptr_t)err_str);
    _sb_append(&sb, ")");
    return _sb_finish(&sb);
}

/* W-5f: Gorillax.toString() — "Gorillax(value)" or "Gorillax(><)" */
static int64_t _wasm_gorillax_to_string(int64_t gx) {
    int64_t is_ok = taida_pack_get_idx(gx, 0);
    int gtype = _wasm_gorillax_type(gx);
    const char *prefix = (gtype == 2) ? "RelaxedGorillax" : "Gorillax";
    _wasm_strbuf sb;
    _sb_init(&sb);
    _sb_append(&sb, prefix);
    _sb_append(&sb, "(");
    if (is_ok) {
        int64_t value = taida_pack_get_idx(gx, 1);
        int64_t val_str = _wasm_value_to_display_string(value);
        _sb_append(&sb, (const char *)(intptr_t)val_str);
    } else {
        if (gtype == 2) {
            _sb_append(&sb, "escaped");
        } else {
            _sb_append(&sb, "><");
        }
    }
    _sb_append(&sb, ")");
    return _sb_finish(&sb);
}

/* W-4f2: Convert value to display string (like native's taida_value_to_display_string) */
static int64_t _wasm_value_to_display_string(int64_t val) {
    if (val == 0) return (int64_t)(intptr_t)"0";
    /* Check HashMap first (has distinctive marker) */
    if (_is_wasm_hashmap(val)) return _wasm_hashmap_to_string(val);
    /* Check Set (has WASM_SET_MARKER_VAL marker) */
    if (_is_wasm_set(val)) return _wasm_set_to_string(val);
    /* Check List (has list-like header but no set/hashmap marker) */
    if (_looks_like_list(val)) return _wasm_list_to_string(val);
    /* W-5f: Check monadic types before generic pack (Lax/Result/Gorillax) */
    if (_wasm_is_result(val)) return _wasm_result_to_string(val);
    if (_wasm_is_gorillax(val)) return _wasm_gorillax_to_string(val);
    if (_wasm_is_lax(val)) return _wasm_lax_to_string(val);
    /* Check Pack (field_count + hash pattern) */
    if (_looks_like_pack(val)) return _wasm_pack_to_string(val);
    /* Check if it's a string */
    if (_looks_like_string(val)) return val;
    /* Fallback: integer */
    return taida_int_to_str(val);
}

/* W-4f2: Convert value to debug string (strings are quoted, everything else like display) */
static int64_t _wasm_value_to_debug_string(int64_t val) {
    if (val == 0) return (int64_t)(intptr_t)"0";
    /* Check collection types first */
    if (_is_wasm_hashmap(val)) return _wasm_hashmap_to_string(val);
    if (_is_wasm_set(val)) return _wasm_set_to_string(val);
    if (_looks_like_list(val)) return _wasm_list_to_string(val);
    /* W-5f: Check monadic types before generic pack */
    if (_wasm_is_result(val)) return _wasm_result_to_string(val);
    if (_wasm_is_gorillax(val)) return _wasm_gorillax_to_string(val);
    if (_wasm_is_lax(val)) return _wasm_lax_to_string(val);
    if (_looks_like_pack(val)) return _wasm_pack_to_string(val);
    /* Check if it's a string — quote it for debug */
    if (_looks_like_string(val)) {
        const char *s = (const char *)(intptr_t)val;
        int slen = wasm_strlen(s);
        char *buf = (char *)wasm_alloc(slen + 3);
        if (!buf) return val;
        buf[0] = '"';
        for (int i = 0; i < slen; i++) buf[1 + i] = s[i];
        buf[slen + 1] = '"';
        buf[slen + 2] = '\0';
        return (int64_t)(intptr_t)buf;
    }
    /* Fallback: integer */
    return taida_int_to_str(val);
}

int64_t taida_polymorphic_to_string(int64_t obj) {
    return _wasm_value_to_display_string(obj);
}

/* ── W-6: taida_debug_polymorphic implementation ── */

int64_t taida_debug_polymorphic(int64_t val) {
    int64_t str = _wasm_value_to_display_string(val);
    const char *s = (const char *)(intptr_t)str;
    if (s) {
        int32_t len = wasm_strlen(s);
        write_stdout(s, len);
        write_stdout("\n", 1);
    }
    return 0;
}

/* ── W-3f: taida_int_mold_str (wasm-min: parse string to int, simplified) ── */
/* In native, this returns a Lax[Int]. In wasm-min, returns raw value (no Lax wrapper). */

int64_t taida_int_mold_str(int64_t v) {
    return taida_str_to_int(v);
}

/* ── W-4f2: Field name/type registry for Pack toString ── */
/* Simple linear array registry. Sufficient for wasm-min programs with few field names. */

#define WASM_FIELD_REGISTRY_MAX 256

static struct {
    int64_t hash;
    const char *name;
    int64_t type_tag;
} _wasm_field_registry[WASM_FIELD_REGISTRY_MAX];
static int _wasm_field_registry_count = 0;

int64_t taida_register_field_name(int64_t hash, int64_t name_ptr) {
    /* Check if already registered */
    for (int i = 0; i < _wasm_field_registry_count; i++) {
        if (_wasm_field_registry[i].hash == hash) return 0;
    }
    if (_wasm_field_registry_count < WASM_FIELD_REGISTRY_MAX) {
        _wasm_field_registry[_wasm_field_registry_count].hash = hash;
        _wasm_field_registry[_wasm_field_registry_count].name = (const char *)(intptr_t)name_ptr;
        _wasm_field_registry[_wasm_field_registry_count].type_tag = -1;
        _wasm_field_registry_count++;
    }
    return 0;
}

int64_t taida_register_field_type(int64_t hash, int64_t name_ptr, int64_t type_tag) {
    /* Update existing entry or add new one */
    for (int i = 0; i < _wasm_field_registry_count; i++) {
        if (_wasm_field_registry[i].hash == hash) {
            _wasm_field_registry[i].type_tag = type_tag;
            return 0;
        }
    }
    if (_wasm_field_registry_count < WASM_FIELD_REGISTRY_MAX) {
        _wasm_field_registry[_wasm_field_registry_count].hash = hash;
        _wasm_field_registry[_wasm_field_registry_count].name = (const char *)(intptr_t)name_ptr;
        _wasm_field_registry[_wasm_field_registry_count].type_tag = type_tag;
        _wasm_field_registry_count++;
    }
    return 0;
}

static const char *_wasm_lookup_field_name(int64_t hash) {
    for (int i = 0; i < _wasm_field_registry_count; i++) {
        if (_wasm_field_registry[i].hash == hash) return _wasm_field_registry[i].name;
    }
    return 0;
}

static int64_t _wasm_lookup_field_type(int64_t hash) {
    for (int i = 0; i < _wasm_field_registry_count; i++) {
        if (_wasm_field_registry[i].hash == hash) return _wasm_field_registry[i].type_tag;
    }
    return -1;
}

/* ── W-4: BuchiPack runtime (bump allocator, no RC/magic) ── */
/* Layout: [field_count, field0_hash, field0_tag, field0_value, field1_hash, ...]
   Same as native_runtime.c but without magic header and refcount.
   Each field occupies 3 int64_t slots: hash, tag, value.
   Total allocation: (1 + field_count * 3) * sizeof(int64_t) */

int64_t taida_pack_new(int64_t field_count) {
    int64_t slots = 1 + field_count * 3;
    int64_t *pack = (int64_t *)wasm_alloc((unsigned int)(slots * 8));
    if (!pack) return 0;
    pack[0] = field_count;
    /* Zero-initialize all field slots (hash=0, tag=0=INT, value=0) */
    for (int64_t i = 1; i < slots; i++) pack[i] = 0;
    return (int64_t)(intptr_t)pack;
}

int64_t taida_pack_set(int64_t pack_ptr, int64_t index, int64_t value) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    pack[1 + index * 3 + 2] = value;
    return pack_ptr;
}

int64_t taida_pack_set_tag(int64_t pack_ptr, int64_t index, int64_t tag) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    pack[1 + index * 3 + 1] = tag;
    return pack_ptr;
}

int64_t taida_pack_get_idx(int64_t pack_ptr, int64_t index) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    return pack[1 + index * 3 + 2];
}

int64_t taida_pack_set_hash(int64_t pack_ptr, int64_t index, int64_t hash) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    pack[1 + index * 3] = hash;
    return pack_ptr;
}

int64_t taida_pack_get(int64_t pack_ptr, int64_t field_hash) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    int64_t count = pack[0];
    for (int64_t i = 0; i < count; i++) {
        if (pack[1 + i * 3] == field_hash) {
            return pack[1 + i * 3 + 2];
        }
    }
    return 0; /* default value */
}

int64_t taida_pack_has_hash(int64_t pack_ptr, int64_t field_hash) {
    int64_t *pack = (int64_t *)(intptr_t)pack_ptr;
    int64_t count = pack[0];
    for (int64_t i = 0; i < count; i++) {
        if (pack[1 + i * 3] == field_hash) {
            return 1;
        }
    }
    return 0;
}

/* ── W-4: List runtime (bump allocator, no RC/magic) ── */
/* Layout: [capacity, length, elem_type_tag, elem0, elem1, ...]
   Same concept as native_runtime.c but without magic header and refcount.
   Note: bump allocator cannot realloc, so we use copy-on-grow. */

int64_t taida_list_new(void) {
    int64_t initial_cap = 16;
    int64_t slots = WASM_LIST_ELEMS + initial_cap;  /* header(4) + elements */
    int64_t *list = (int64_t *)wasm_alloc((unsigned int)(slots * 8));
    if (!list) return 0;
    list[0] = initial_cap;  /* capacity */
    list[1] = 0;            /* length */
    list[2] = -1;           /* elem_type_tag (UNKNOWN) */
    list[3] = 0;            /* type_marker (0 = plain list) */
    return (int64_t)(intptr_t)list;
}

void taida_list_set_elem_tag(int64_t list_ptr, int64_t tag) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    list[2] = tag;
}

int64_t taida_list_push(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t cap = list[0];
    int64_t len = list[1];
    if (len >= cap) {
        /* Grow: allocate new list with double capacity, copy over */
        int64_t new_cap = cap * 2;
        int64_t new_slots = WASM_LIST_ELEMS + new_cap;
        int64_t *new_list = (int64_t *)wasm_alloc((unsigned int)(new_slots * 8));
        if (!new_list) return list_ptr;
        /* Copy header + existing elements */
        for (int64_t i = 0; i < WASM_LIST_ELEMS + len; i++) new_list[i] = list[i];
        new_list[0] = new_cap;
        list = new_list;
        list_ptr = (int64_t)(intptr_t)new_list;
    }
    list[WASM_LIST_ELEMS + len] = item;
    list[1] = len + 1;
    return list_ptr;
}

int64_t taida_list_length(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    return list[1];
}

/* taida_list_get: in native returns Lax, in wasm-min returns raw value
   (no Lax wrapper). OOB returns 0. */
int64_t taida_list_get(int64_t list_ptr, int64_t index) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (index < 0 || index >= len) return 0;
    return list[WASM_LIST_ELEMS + index];
}

int64_t taida_list_is_empty(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    return list[1] == 0 ? 1 : 0;
}

/* ── W-4: HashMap runtime (bump allocator, no RC/magic) ── */
/* Layout: [capacity, length, value_type_tag, type_marker, entries...]
   Each entry: [key_hash, key_ptr, value] (3 slots)
   Header = 4 slots (including type_marker for polymorphic dispatch),
   then capacity * 3 entry slots.
   Open addressing with linear probing. */

/* WASM_HM_HEADER and WASM_HM_MARKER_VAL defined above (near WASM_LIST_ELEMS) */

/* String comparison helper */
static int _wasm_streq(const char *a, const char *b) {
    if (a == b) return 1;
    if (!a || !b) return 0;
    while (*a && *b) {
        if (*a != *b) return 0;
        a++; b++;
    }
    return *a == *b;
}

static int64_t _wasm_hashmap_new_with_cap(int64_t cap) {
    int64_t slots = WASM_HM_HEADER + cap * 3;
    int64_t *hm = (int64_t *)wasm_alloc((unsigned int)(slots * 8));
    if (!hm) return 0;
    /* Zero-initialize everything (empty slots have hash=0, key=0) */
    for (int64_t i = 0; i < slots; i++) hm[i] = 0;
    hm[0] = cap;
    hm[1] = 0;     /* length */
    hm[2] = -1;    /* value_type_tag = UNKNOWN */
    hm[3] = WASM_HM_MARKER_VAL;  /* type marker for polymorphic dispatch */
    return (int64_t)(intptr_t)hm;
}

int64_t taida_hashmap_new(void) {
    return _wasm_hashmap_new_with_cap(16);
}

void taida_hashmap_set_value_tag(int64_t hm_ptr, int64_t tag) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    hm[2] = tag;
}

/* FNV-1a hash for string keys */
int64_t taida_str_hash(int64_t str_ptr) {
    const unsigned char *s = (const unsigned char *)(intptr_t)str_ptr;
    if (!s) return 0;
    uint64_t hash = 0xcbf29ce484222325ULL;
    while (*s) {
        hash ^= *s++;
        hash *= 0x100000001b3ULL;
    }
    return (int64_t)hash;
}

int64_t taida_hashmap_set(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr, int64_t value) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t len = hm[1];

    /* Resize if load factor > 70% */
    if (len * 10 >= cap * 7) {
        int64_t new_cap = cap * 2;
        int64_t new_hm_ptr = _wasm_hashmap_new_with_cap(new_cap);
        int64_t *new_hm = (int64_t *)(intptr_t)new_hm_ptr;
        new_hm[2] = hm[2]; /* propagate value_type_tag */
        new_hm[3] = WASM_HM_MARKER_VAL;  /* propagate type marker */
        /* Re-hash all occupied entries */
        for (int64_t i = 0; i < cap; i++) {
            int64_t sh = hm[WASM_HM_HEADER + i * 3];
            int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
            if (sh != 0 || sk != 0) {
                /* Insert into new table */
                uint64_t uh = (uint64_t)sh;
                int64_t idx = (int64_t)(uh % (uint64_t)new_cap);
                for (int64_t j = 0; j < new_cap; j++) {
                    int64_t slot = (idx + j) % new_cap;
                    int64_t esh = new_hm[WASM_HM_HEADER + slot * 3];
                    int64_t esk = new_hm[WASM_HM_HEADER + slot * 3 + 1];
                    if (esh == 0 && esk == 0) {
                        new_hm[WASM_HM_HEADER + slot * 3] = sh;
                        new_hm[WASM_HM_HEADER + slot * 3 + 1] = sk;
                        new_hm[WASM_HM_HEADER + slot * 3 + 2] = hm[WASM_HM_HEADER + i * 3 + 2];
                        new_hm[1]++;
                        break;
                    }
                }
            }
        }
        hm = new_hm;
        hm_ptr = new_hm_ptr;
        cap = new_cap;
    }

    /* Insert or update */
    uint64_t uh = (uint64_t)key_hash;
    int64_t idx = (int64_t)(uh % (uint64_t)cap);
    for (int64_t i = 0; i < cap; i++) {
        int64_t slot = (idx + i) % cap;
        int64_t sh = hm[WASM_HM_HEADER + slot * 3];
        int64_t sk = hm[WASM_HM_HEADER + slot * 3 + 1];
        if (sh == 0 && sk == 0) {
            /* Empty slot — insert */
            hm[WASM_HM_HEADER + slot * 3] = key_hash;
            hm[WASM_HM_HEADER + slot * 3 + 1] = key_ptr;
            hm[WASM_HM_HEADER + slot * 3 + 2] = value;
            hm[1]++;
            return hm_ptr;
        }
        if (sh == key_hash && _wasm_streq((const char *)(intptr_t)sk, (const char *)(intptr_t)key_ptr)) {
            /* Existing key — update value */
            hm[WASM_HM_HEADER + slot * 3 + 2] = value;
            return hm_ptr;
        }
    }
    return hm_ptr;
}

int64_t taida_hashmap_get(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    uint64_t uh = (uint64_t)key_hash;
    int64_t idx = (int64_t)(uh % (uint64_t)cap);
    for (int64_t i = 0; i < cap; i++) {
        int64_t slot = (idx + i) % cap;
        int64_t sh = hm[WASM_HM_HEADER + slot * 3];
        int64_t sk = hm[WASM_HM_HEADER + slot * 3 + 1];
        if (sh == 0 && sk == 0) return 0; /* not found */
        if (sh == key_hash && _wasm_streq((const char *)(intptr_t)sk, (const char *)(intptr_t)key_ptr))
            return hm[WASM_HM_HEADER + slot * 3 + 2];
    }
    return 0;
}

int64_t taida_hashmap_has(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    uint64_t uh = (uint64_t)key_hash;
    int64_t idx = (int64_t)(uh % (uint64_t)cap);
    for (int64_t i = 0; i < cap; i++) {
        int64_t slot = (idx + i) % cap;
        int64_t sh = hm[WASM_HM_HEADER + slot * 3];
        int64_t sk = hm[WASM_HM_HEADER + slot * 3 + 1];
        if (sh == 0 && sk == 0) return 0;
        if (sh == key_hash && _wasm_streq((const char *)(intptr_t)sk, (const char *)(intptr_t)key_ptr))
            return 1;
    }
    return 0;
}

int64_t taida_hashmap_is_empty(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    return hm[1] == 0 ? 1 : 0;
}

/* Immutable set: clone the hashmap first to preserve immutable semantics.
   In wasm-min's bump allocator, taida_hashmap_set modifies in place,
   which would mutate the original hashmap. */
static int64_t _wasm_hashmap_clone(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t new_hm_ptr = _wasm_hashmap_new_with_cap(cap);
    int64_t *new_hm = (int64_t *)(intptr_t)new_hm_ptr;
    new_hm[2] = hm[2]; /* propagate value_type_tag */
    /* Copy all entries */
    for (int64_t i = 0; i < cap; i++) {
        int64_t sh = hm[WASM_HM_HEADER + i * 3];
        int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
        new_hm[WASM_HM_HEADER + i * 3] = sh;
        new_hm[WASM_HM_HEADER + i * 3 + 1] = sk;
        new_hm[WASM_HM_HEADER + i * 3 + 2] = hm[WASM_HM_HEADER + i * 3 + 2];
    }
    new_hm[1] = hm[1]; /* copy length */
    return new_hm_ptr;
}

int64_t taida_hashmap_set_immut(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr, int64_t value) {
    int64_t clone = _wasm_hashmap_clone(hm_ptr);
    return taida_hashmap_set(clone, key_hash, key_ptr, value);
}

/* taida_hashmap_get_lax: in native returns Lax, in wasm-min returns raw value */
int64_t taida_hashmap_get_lax(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr) {
    return taida_hashmap_get(hm_ptr, key_hash, key_ptr);
}

/* ── W-4f: HashMap type detection helper ── */
/* Check if a pointer is a HashMap by looking for the type marker at index 3. */
static int _is_wasm_hashmap(int64_t ptr) {
    if (ptr == 0) return 0;
    if (ptr < 0 || ptr > 0xFFFFFFFF) return 0;
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)ptr;
    /* Need at least 4 int64_t header slots */
    if (addr + 32 > mem_size) return 0;
    int64_t *data = (int64_t *)(intptr_t)ptr;
    return data[3] == WASM_HM_MARKER_VAL;
}

/* ── W-4f: taida_value_hash — polymorphic hash for collection keys ── */
/* For strings, compute FNV-1a hash. For scalars, use identity (adjusted). */
int64_t taida_value_hash(int64_t val) {
    /* Try to detect string pointers */
    if (_looks_like_string(val)) {
        int64_t h = taida_str_hash(val);
        /* Adjust to avoid 0/1 (reserved for empty/tombstone) */
        if (h == 0 || h == 1) h = h + 2;
        return h;
    }
    /* Scalar: use identity, adjusted to avoid 0/1 */
    int64_t h = val;
    if (h == 0 || h == 1) h = h + 2;
    return h;
}

/* ── W-4f: HashMap additional methods ── */

/* HashMap.keys() -> List of key pointers */
int64_t taida_hashmap_keys(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t list = taida_list_new();
    for (int64_t i = 0; i < cap; i++) {
        int64_t sh = hm[WASM_HM_HEADER + i * 3];
        int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
        if (sh != 0 || sk != 0) {
            list = taida_list_push(list, sk);
        }
    }
    return list;
}

/* HashMap.values() -> List of values */
int64_t taida_hashmap_values(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t list = taida_list_new();
    for (int64_t i = 0; i < cap; i++) {
        int64_t sh = hm[WASM_HM_HEADER + i * 3];
        int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
        if (sh != 0 || sk != 0) {
            list = taida_list_push(list, hm[WASM_HM_HEADER + i * 3 + 2]);
        }
    }
    return list;
}

/* HashMap.entries() -> List of BuchiPack @(key, value) */
int64_t taida_hashmap_entries(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t list = taida_list_new();
    /* FNV-1a hashes for "key" and "value" (same as native runtime) */
    #define WASM_HASH_KEY   0x3dc94a19365b10ecLL
    #define WASM_HASH_VAL   0x7ce4fd9430e80ceaLL
    for (int64_t i = 0; i < cap; i++) {
        int64_t sh = hm[WASM_HM_HEADER + i * 3];
        int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
        if (sh != 0 || sk != 0) {
            int64_t pair = taida_pack_new(2);
            taida_pack_set_hash(pair, 0, WASM_HASH_KEY);
            taida_pack_set(pair, 0, sk);
            taida_pack_set_hash(pair, 1, WASM_HASH_VAL);
            taida_pack_set(pair, 1, hm[WASM_HM_HEADER + i * 3 + 2]);
            list = taida_list_push(list, pair);
        }
    }
    return list;
}

/* HashMap.merge(other) -> new HashMap with other's entries overwriting */
int64_t taida_hashmap_merge(int64_t hm_ptr, int64_t other_ptr) {
    int64_t *other = (int64_t *)(intptr_t)other_ptr;
    int64_t cap = other[0];
    /* Start with a copy of hm (in wasm-min, just use the original since bump allocator) */
    /* Actually, we need a new copy. Iterate hm first, then apply other's entries. */
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t hm_cap = hm[0];
    int64_t result = taida_hashmap_new();
    int64_t *r = (int64_t *)(intptr_t)result;
    r[2] = hm[2]; /* propagate value_type_tag */
    /* Copy from hm */
    for (int64_t i = 0; i < hm_cap; i++) {
        int64_t sh = hm[WASM_HM_HEADER + i * 3];
        int64_t sk = hm[WASM_HM_HEADER + i * 3 + 1];
        if (sh != 0 || sk != 0) {
            result = taida_hashmap_set(result, sh, sk, hm[WASM_HM_HEADER + i * 3 + 2]);
        }
    }
    /* Overwrite/add from other */
    for (int64_t i = 0; i < cap; i++) {
        int64_t sh = other[WASM_HM_HEADER + i * 3];
        int64_t sk = other[WASM_HM_HEADER + i * 3 + 1];
        if (sh != 0 || sk != 0) {
            result = taida_hashmap_set(result, sh, sk, other[WASM_HM_HEADER + i * 3 + 2]);
        }
    }
    return result;
}

/* HashMap.remove(key_hash, key_ptr) -> new HashMap without the key (immutable) */
/* In wasm-min, mutate in place (bump allocator, no sharing). Uses tombstone. */
static int64_t taida_hashmap_remove(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    uint64_t uh = (uint64_t)key_hash;
    int64_t idx = (int64_t)(uh % (uint64_t)cap);
    for (int64_t i = 0; i < cap; i++) {
        int64_t slot = (idx + i) % cap;
        int64_t sh = hm[WASM_HM_HEADER + slot * 3];
        int64_t sk = hm[WASM_HM_HEADER + slot * 3 + 1];
        if (sh == 0 && sk == 0) return hm_ptr; /* not found */
        if (sh == key_hash && _wasm_streq((const char *)(intptr_t)sk, (const char *)(intptr_t)key_ptr)) {
            /* Found — tombstone it */
            hm[WASM_HM_HEADER + slot * 3] = 1; /* tombstone hash */
            hm[WASM_HM_HEADER + slot * 3 + 1] = 0;
            hm[WASM_HM_HEADER + slot * 3 + 2] = 0;
            hm[1]--;
            return hm_ptr;
        }
    }
    return hm_ptr;
}

/* ── W-4: Set runtime (simplified — backed by linear scan array) ── */
/* Layout: [capacity, length, elem_type_tag, WASM_SET_MARKER_VAL, elem0, elem1, ...]
   Same as List layout but with Set marker at slot[3]. Uses linear scan for has/add/remove.
   Sufficient for wasm-min's simple programs. */

int64_t taida_set_new(void) {
    int64_t set = taida_list_new();
    if (set) ((int64_t *)(intptr_t)set)[3] = WASM_SET_MARKER_VAL;
    return set;
}

void taida_set_set_elem_tag(int64_t set_ptr, int64_t tag) {
    taida_list_set_elem_tag(set_ptr, tag);
}

/* W-4f/F-3: Type-tag-aware equality for Set elements.
   For strings, compare by content (strcmp). For others, compare by raw value. */
static int _wasm_value_eq(int64_t a, int64_t b) {
    if (a == b) return 1;
    /* Check if both look like strings — if so, compare by content */
    if (_looks_like_string(a) && _looks_like_string(b)) {
        return _wasm_streq((const char *)(intptr_t)a, (const char *)(intptr_t)b);
    }
    return 0;
}

int64_t taida_set_has(int64_t set_ptr, int64_t item) {
    int64_t *set = (int64_t *)(intptr_t)set_ptr;
    int64_t len = set[1];
    for (int64_t i = 0; i < len; i++) {
        if (_wasm_value_eq(set[WASM_LIST_ELEMS + i], item)) return 1;
    }
    return 0;
}

int64_t taida_set_add(int64_t set_ptr, int64_t item) {
    if (taida_set_has(set_ptr, item)) return set_ptr;
    /* Create a new set (copy elements) to preserve immutable semantics.
       In wasm-min's bump allocator, taida_list_push modifies in place when
       there's room, which would mutate the original set. */
    int64_t *old = (int64_t *)(intptr_t)set_ptr;
    int64_t len = old[1];
    int64_t new_set = taida_set_new();
    for (int64_t i = 0; i < len; i++) {
        new_set = taida_list_push(new_set, old[WASM_LIST_ELEMS + i]);
    }
    new_set = taida_list_push(new_set, item);
    return new_set;
}

int64_t taida_set_from_list(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t set = taida_set_new();
    for (int64_t i = 0; i < len; i++) {
        set = taida_set_add(set, list[WASM_LIST_ELEMS + i]);
    }
    return set;
}

/* W-4f: Set.remove(item) -> new Set without the item */
int64_t taida_set_remove(int64_t set_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)set_ptr;
    int64_t len = list[1];
    int64_t result = taida_set_new();
    for (int64_t i = 0; i < len; i++) {
        if (!_wasm_value_eq(list[WASM_LIST_ELEMS + i], item)) {
            result = taida_list_push(result, list[WASM_LIST_ELEMS + i]);
        }
    }
    return result;
}

/* W-4f: Set.union(other) -> new Set with all elements from both */
int64_t taida_set_union(int64_t set_a, int64_t set_b) {
    int64_t *a = (int64_t *)(intptr_t)set_a;
    int64_t *b = (int64_t *)(intptr_t)set_b;
    int64_t a_len = a[1];
    int64_t b_len = b[1];
    int64_t result = taida_set_new();
    for (int64_t i = 0; i < a_len; i++) {
        result = taida_list_push(result, a[WASM_LIST_ELEMS + i]);
    }
    for (int64_t i = 0; i < b_len; i++) {
        if (!taida_set_has(result, b[WASM_LIST_ELEMS + i])) {
            result = taida_list_push(result, b[WASM_LIST_ELEMS + i]);
        }
    }
    return result;
}

/* W-4f: Set.intersect(other) -> new Set with common elements */
int64_t taida_set_intersect(int64_t set_a, int64_t set_b) {
    int64_t *a = (int64_t *)(intptr_t)set_a;
    int64_t a_len = a[1];
    int64_t result = taida_set_new();
    for (int64_t i = 0; i < a_len; i++) {
        if (taida_set_has(set_b, a[WASM_LIST_ELEMS + i])) {
            result = taida_list_push(result, a[WASM_LIST_ELEMS + i]);
        }
    }
    return result;
}

/* W-4f: Set.diff(other) -> new Set with elements in a but not in b */
int64_t taida_set_diff(int64_t set_a, int64_t set_b) {
    int64_t *a = (int64_t *)(intptr_t)set_a;
    int64_t a_len = a[1];
    int64_t result = taida_set_new();
    for (int64_t i = 0; i < a_len; i++) {
        if (!taida_set_has(set_b, a[WASM_LIST_ELEMS + i])) {
            result = taida_list_push(result, a[WASM_LIST_ELEMS + i]);
        }
    }
    return result;
}

/* W-4f: Set.toList() -> List copy */
int64_t taida_set_to_list(int64_t set_ptr) {
    int64_t *list = (int64_t *)(intptr_t)set_ptr;
    int64_t len = list[1];
    int64_t result = taida_list_new();
    for (int64_t i = 0; i < len; i++) {
        result = taida_list_push(result, list[WASM_LIST_ELEMS + i]);
    }
    return result;
}

/* ── W-4f: Polymorphic collection methods ── */
/* These work on both HashMap and Set (auto-detect via type marker). */

/* .get(key_or_index) — HashMap: hash-based lookup, List: index-based */
int64_t taida_collection_get(int64_t ptr, int64_t item) {
    if (_is_wasm_hashmap(ptr)) {
        int64_t key_hash = taida_value_hash(item);
        return taida_hashmap_get_lax(ptr, key_hash, item);
    }
    /* List/Set: index-based access */
    return taida_list_get(ptr, item);
}

/* .has(key_or_item) — HashMap: hash-based lookup, Set/List: linear scan */
int64_t taida_collection_has(int64_t ptr, int64_t item) {
    if (_is_wasm_hashmap(ptr)) {
        int64_t key_hash = taida_value_hash(item);
        return taida_hashmap_has(ptr, key_hash, item);
    }
    /* Set/List: linear scan */
    return taida_set_has(ptr, item);
}

/* .remove(key_or_item) — HashMap: clone + hash-based removal, Set: linear scan */
int64_t taida_collection_remove(int64_t ptr, int64_t item) {
    if (_is_wasm_hashmap(ptr)) {
        int64_t clone = _wasm_hashmap_clone(ptr);
        int64_t key_hash = taida_value_hash(item);
        return taida_hashmap_remove(clone, key_hash, item);
    }
    /* Set: taida_set_remove already creates a new set */
    return taida_set_remove(ptr, item);
}

/* .size() — both HashMap and Set store length at ptr[1] */
int64_t taida_collection_size(int64_t ptr) {
    int64_t *data = (int64_t *)(intptr_t)ptr;
    return data[1];
}

/* ── W-4f: taida_polymorphic_is_empty (wasm-min: List/Set/HashMap) ── */
/* For List/Set: length at index 1. For HashMap: length at index 1. */
int64_t taida_polymorphic_is_empty(int64_t ptr) {
    if (ptr == 0) return 1;
    /* All collection types store length at index 1 in wasm-min */
    if (_looks_like_list(ptr) || _is_wasm_hashmap(ptr)) {
        int64_t *data = (int64_t *)(intptr_t)ptr;
        return data[1] == 0 ? 1 : 0;
    }
    /* String: check if empty */
    if (_looks_like_string(ptr)) {
        const char *s = (const char *)(intptr_t)ptr;
        return s[0] == '\0' ? 1 : 0;
    }
    return 0;
}

/* ── W-5: Closure runtime ────────────────────────────────── */
/* Closure layout: [fn_ptr, env_ptr]
   No magic header or refcount in wasm-min (bump allocator, no free). */

#define WASM_CLOSURE_MARKER 0x434C4F53ULL /* "CLOS" */

int64_t taida_closure_new(int64_t fn_ptr, int64_t env_ptr, int64_t user_arity) {
    int64_t *closure = (int64_t *)wasm_alloc(4 * 8);
    if (!closure) return 0;
    closure[0] = (int64_t)WASM_CLOSURE_MARKER;
    closure[1] = fn_ptr;
    closure[2] = env_ptr;
    closure[3] = user_arity; /* W-5g: number of user args (excluding __env) */
    return (int64_t)(intptr_t)closure;
}

int64_t taida_closure_get_fn(int64_t closure_ptr) {
    int64_t *c = (int64_t *)(intptr_t)closure_ptr;
    return c[1];
}

int64_t taida_closure_get_env(int64_t closure_ptr) {
    int64_t *c = (int64_t *)(intptr_t)closure_ptr;
    return c[2];
}

int64_t taida_is_closure_value(int64_t val) {
    /* W-5g: bounds check before dereference (closure is 4 * int64_t = 32 bytes) */
    if (!_wasm_is_valid_ptr(val, 32)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    return (p[0] == (int64_t)WASM_CLOSURE_MARKER) ? 1 : 0;
}

/* ── W-5: Error ceiling (error-flag based, no setjmp/longjmp) ── */
/* WASM freestanding mode does not have setjmp.h. Instead we use an
   error-flag approach: taida_throw sets a global flag and stores the
   error value. taida_error_try_call wraps a function pointer call and
   checks the flag after return. Functions that may throw must check
   the error flag and propagate it via early return.

   For wasm-min's simpler use case, we implement the same API as native
   but with setjmp emulated via the error flag + wrapper function approach.
   taida_error_try_call calls the function; if taida_throw was invoked,
   the function returns normally (with a dummy value) and try_call detects
   the flag was set. */

static int64_t __wasm_error_val[64];
static int64_t __wasm_try_result[64];
static int __wasm_error_depth = 0;
static int __wasm_error_thrown = 0;

int64_t taida_error_ceiling_push(void) {
    if (__wasm_error_depth >= 64) {
        /* overflow: crash */
        const char *msg = "Error: maximum error handling depth exceeded\n";
        write_stdout(msg, wasm_strlen(msg));
        __builtin_trap();
    }
    int depth = __wasm_error_depth++;
    return (int64_t)depth;
}

void taida_error_ceiling_pop(void) {
    if (__wasm_error_depth > 0) __wasm_error_depth--;
    __wasm_error_thrown = 0;
}

int64_t taida_throw(int64_t error_val) {
    if (__wasm_error_depth > 0) {
        int depth = __wasm_error_depth - 1;
        __wasm_error_val[depth] = error_val;
        __wasm_error_thrown = 1;
        return 0; /* caller should check __wasm_error_thrown */
    }
    /* No error ceiling: gorilla crash */
    const char *msg = "Unhandled error (no error ceiling)\n";
    write_stdout(msg, wasm_strlen(msg));
    __builtin_trap();
    return 0;
}

/* taida_error_try_call: call fn_ptr(env_ptr) under error ceiling protection.
   Returns 0 if normal, 1 if error was thrown. */
int64_t taida_error_try_call(int64_t fn_ptr, int64_t env_ptr, int64_t depth) {
    typedef int64_t (*fn_t)(int64_t);
    fn_t func = (fn_t)(intptr_t)fn_ptr;
    __wasm_error_thrown = 0;
    int64_t result = func(env_ptr);
    if (__wasm_error_thrown) {
        __wasm_error_thrown = 0;
        return 1; /* error caught */
    }
    __wasm_try_result[(int)depth] = result;
    return 0; /* normal completion */
}

int64_t taida_error_try_get_result(int64_t depth) {
    return __wasm_try_result[(int)depth];
}

int64_t taida_error_setjmp(int64_t depth) {
    /* Legacy compat — not used in wasm-min's error flow */
    (void)depth;
    return 0;
}

int64_t taida_error_get_value(int64_t depth) {
    return __wasm_error_val[(int)depth];
}

/* ── W-5: Error object creation ── */
/* FNV-1a hashes for error BuchiPack fields (same as native_runtime.c) */
/* WFX-2: corrected FNV-1a hashes for error fields */
#define WASM_HASH_TYPE      0xa79439ef7bfa9c2dLL  /* FNV-1a("type") */
#define WASM_HASH_MESSAGE   0x546401b5d2a8d2a4LL  /* FNV-1a("message") */
#define WASM_HASH_FIELD     0x2c5d047ff4e6ffc7LL  /* FNV-1a("field") */
#define WASM_HASH_CODE      0x0bb51791194b4414LL  /* FNV-1a("code") */

static void _wasm_register_builtin_error_field_names(void) {
    static int registered = 0;
    if (registered) return;
    registered = 1;

    taida_register_field_name(WASM_HASH_TYPE, (int64_t)(intptr_t)"type");
    taida_register_field_name(WASM_HASH_MESSAGE, (int64_t)(intptr_t)"message");
    taida_register_field_name(WASM_HASH_FIELD, (int64_t)(intptr_t)"field");
    taida_register_field_name(WASM_HASH_CODE, (int64_t)(intptr_t)"code");
    taida_register_field_name(
        taida_str_hash((int64_t)(intptr_t)"kind"),
        (int64_t)(intptr_t)"kind"
    );
}

int64_t taida_make_error(int64_t type_ptr, int64_t msg_ptr) {
    _wasm_register_builtin_error_field_names();

    int64_t pack = taida_pack_new(2);
    taida_pack_set_hash(pack, 0, WASM_HASH_TYPE);
    taida_pack_set(pack, 0, type_ptr);
    taida_pack_set_hash(pack, 1, WASM_HASH_MESSAGE);
    taida_pack_set(pack, 1, msg_ptr);
    return pack;
}

/* ── W-5: Lax[T] runtime ────────────────────────────────── */
/* Lax is a BuchiPack @(hasValue: Bool, __value: T, __default: T, __type: Str)
   Layout: 4-field pack using same hash constants as native. */

/* WASM_HASH_HAS_VALUE, __VALUE, __DEFAULT, __TYPE defined early (near line 710) */

int64_t taida_lax_new(int64_t value, int64_t default_value) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_HAS_VALUE);
    taida_pack_set(pack, 0, 1);  /* hasValue = true */
    taida_pack_set_tag(pack, 0, 2); /* BOOL tag */
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, value);
    taida_pack_set_hash(pack, 2, WASM_HASH___DEFAULT);
    taida_pack_set(pack, 2, default_value);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"Lax");
    return pack;
}

int64_t taida_lax_empty(int64_t default_value) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_HAS_VALUE);
    taida_pack_set(pack, 0, 0);  /* hasValue = false */
    taida_pack_set_tag(pack, 0, 2); /* BOOL tag */
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, default_value);
    taida_pack_set_hash(pack, 2, WASM_HASH___DEFAULT);
    taida_pack_set(pack, 2, default_value);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"Lax");
    return pack;
}

int64_t taida_lax_has_value(int64_t lax_ptr) {
    return taida_pack_get_idx(lax_ptr, 0);  /* hasValue field */
}

int64_t taida_lax_get_or_default(int64_t lax_ptr, int64_t fallback) {
    if (taida_pack_get_idx(lax_ptr, 0)) {
        return taida_pack_get_idx(lax_ptr, 1);  /* __value */
    }
    return fallback;
}

int64_t taida_lax_unmold(int64_t lax_ptr) {
    if (taida_pack_get_idx(lax_ptr, 0)) {
        return taida_pack_get_idx(lax_ptr, 1);  /* __value */
    }
    return taida_pack_get_idx(lax_ptr, 2);  /* __default */
}

int64_t taida_lax_is_empty(int64_t lax_ptr) {
    return taida_pack_get_idx(lax_ptr, 0) ? 0 : 1;
}

/* ── W-5: generic_unmold — now Lax-aware ── */
/* Override the simplified version from W-1. When the value is a Lax pack
   (detected by field count == 4 and hasValue field), extract the value;
   otherwise return identity. */

/* Forward declare: check if a value is a Lax pack */
static int _wasm_is_lax(int64_t val) {
    if (!_wasm_is_valid_ptr(val, 104)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    /* Check if it looks like a pack with 4 fields and first hash = HASH_HAS_VALUE */
    if (p[0] == 4 && p[1] == WASM_HASH_HAS_VALUE) return 1;
    return 0;
}

/* ── W-5: Gorillax (Result container) ── */
/* Gorillax: @(isOk: Bool, __value: T, __error: Error, __type: "Gorillax")
   Using pack fields at fixed indices. */

/* WASM_HASH_IS_OK, __ERROR defined early (near line 710) */

int64_t taida_gorillax_new(int64_t value) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_IS_OK);
    taida_pack_set(pack, 0, 1); /* isOk = true */
    taida_pack_set_tag(pack, 0, 2); /* BOOL */
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, value);
    taida_pack_set_hash(pack, 2, WASM_HASH___ERROR);
    taida_pack_set(pack, 2, 0);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"Gorillax");
    return pack;
}

int64_t taida_gorillax_err(int64_t error) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_IS_OK);
    taida_pack_set(pack, 0, 0); /* isOk = false */
    taida_pack_set_tag(pack, 0, 2); /* BOOL */
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, 0);
    taida_pack_set_hash(pack, 2, WASM_HASH___ERROR);
    taida_pack_set(pack, 2, error);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"Gorillax");
    return pack;
}

int64_t taida_gorillax_is_ok(int64_t gx) {
    return taida_pack_get_idx(gx, 0);
}

int64_t taida_gorillax_get_value(int64_t gx) {
    return taida_pack_get_idx(gx, 1);
}

int64_t taida_gorillax_get_error(int64_t gx) {
    return taida_pack_get_idx(gx, 2);
}

int64_t taida_gorillax_relax(int64_t gx) {
    /* RelaxedGorillax: same layout, just change __type */
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_IS_OK);
    taida_pack_set(pack, 0, taida_pack_get_idx(gx, 0));
    taida_pack_set_tag(pack, 0, 2);
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, taida_pack_get_idx(gx, 1));
    taida_pack_set_hash(pack, 2, WASM_HASH___ERROR);
    taida_pack_set(pack, 2, taida_pack_get_idx(gx, 2));
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"RelaxedGorillax");
    return pack;
}

int64_t taida_relaxed_gorillax_new(int64_t value) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_IS_OK);
    taida_pack_set(pack, 0, 1);
    taida_pack_set_tag(pack, 0, 2);
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, value);
    taida_pack_set_hash(pack, 2, WASM_HASH___ERROR);
    taida_pack_set(pack, 2, 0);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"RelaxedGorillax");
    return pack;
}

int64_t taida_relaxed_gorillax_err(int64_t error) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH_IS_OK);
    taida_pack_set(pack, 0, 0);
    taida_pack_set_tag(pack, 0, 2);
    taida_pack_set_hash(pack, 1, WASM_HASH___VALUE);
    taida_pack_set(pack, 1, 0);
    taida_pack_set_hash(pack, 2, WASM_HASH___ERROR);
    taida_pack_set(pack, 2, error);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"RelaxedGorillax");
    return pack;
}

/* ── W-5: Result[T, P] ── */
/* Result: @(__value: T, __predicate: P, throw: Error, __type: "Result")
   field 0: __value, field 1: __predicate, field 2: throw, field 3: __type */

/* WASM_HASH___PREDICATE, WASM_HASH_THROW defined early (near line 710) */

int64_t taida_result_create(int64_t value, int64_t throw_val, int64_t predicate) {
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WASM_HASH___VALUE);
    taida_pack_set(pack, 0, value);
    taida_pack_set_hash(pack, 1, WASM_HASH___PREDICATE);
    taida_pack_set(pack, 1, predicate);
    taida_pack_set_hash(pack, 2, WASM_HASH_THROW);
    taida_pack_set(pack, 2, throw_val);
    taida_pack_set_hash(pack, 3, WASM_HASH___TYPE);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"Result");
    return pack;
}

/* W-5g: Helper — check if Result has error (matching native taida_result_is_error_check).
   1. If throw is set (not 0), it's an error — UNLESS predicate passes
   2. If predicate exists, evaluate P(value) — true = success, false = error
   3. No predicate + no throw = success (backward compatible) */
static int64_t _wasm_result_is_error_check(int64_t result) {
    int64_t throw_val = taida_pack_get_idx(result, 2); /* throw */
    int64_t pred = taida_pack_get_idx(result, 1);      /* __predicate */
    int64_t value = taida_pack_get_idx(result, 0);     /* __value */

    if (throw_val != 0) {
        if (pred != 0) {
            int64_t pred_result = _wasm_invoke_callback1(pred, value);
            if (!pred_result) return 1; /* predicate failed — error */
            return 0; /* predicate passed even though throw was set — success */
        }
        return 1; /* throw set, no predicate — error */
    }
    if (pred != 0) {
        int64_t pred_result = _wasm_invoke_callback1(pred, value);
        return pred_result ? 0 : 1;
    }
    return 0; /* no throw, no predicate — success */
}

int64_t taida_result_is_ok(int64_t result) {
    return _wasm_result_is_error_check(result) ? 0 : 1;
}

int64_t taida_result_is_error(int64_t result) {
    return _wasm_result_is_error_check(result);
}

int64_t taida_result_map_error(int64_t result, int64_t fn_ptr) {
    /* Simplified: not used in wasm-min common paths */
    (void)fn_ptr;
    return result;
}

/* ── W-5: Cage ── */

/* Callback invoker helpers for wasm-min
 * W-5g: In WASM, indirect call type signature must match exactly.
 * Zero-param lambdas (_ = expr) have user_arity=0, so the closure function
 * only takes (__env). We must dispatch based on arity to avoid type mismatch. */
static int64_t _wasm_invoke_callback1(int64_t fn_ptr, int64_t arg0) {
    if (taida_is_closure_value(fn_ptr)) {
        int64_t *closure = (int64_t *)(intptr_t)fn_ptr;
        int64_t user_arity = closure[3];
        if (user_arity == 0) {
            /* Zero-param lambda: call with env only, ignore arg0 */
            typedef int64_t (*closure_fn0_t)(int64_t);
            closure_fn0_t func = (closure_fn0_t)(intptr_t)closure[1];
            return func(closure[2]);
        }
        /* 1+ param lambda: call with env + arg0 */
        typedef int64_t (*closure_fn1_t)(int64_t, int64_t);
        closure_fn1_t func = (closure_fn1_t)(intptr_t)closure[1];
        return func(closure[2], arg0);
    }
    typedef int64_t (*fn_t)(int64_t);
    fn_t func = (fn_t)(intptr_t)fn_ptr;
    return func(arg0);
}

int64_t taida_cage_apply(int64_t cage_value, int64_t fn_ptr) {
    if (fn_ptr == 0) {
        int64_t error = taida_make_error(
            (int64_t)(intptr_t)"CageError",
            (int64_t)(intptr_t)"Cage second argument must be a function");
        return taida_gorillax_err(error);
    }

    int64_t depth = taida_error_ceiling_push();
    __wasm_error_thrown = 0;
    int64_t result = _wasm_invoke_callback1(fn_ptr, cage_value);
    if (__wasm_error_thrown) {
        int64_t error = taida_error_get_value(depth);
        taida_error_ceiling_pop();
        if (error == 0) {
            error = taida_make_error(
                (int64_t)(intptr_t)"CageError",
                (int64_t)(intptr_t)"Cage function failed");
        }
        return taida_gorillax_err(error);
    }
    taida_error_ceiling_pop();
    return taida_gorillax_new(result);
}

/* ── W-5: Molten/Stub/Todo stubs ── */

int64_t taida_molten_new(void) {
    int64_t pack = taida_pack_new(1);
    taida_pack_set_hash(pack, 0, WASM_HASH___TYPE);
    taida_pack_set(pack, 0, (int64_t)(intptr_t)"Molten");
    return pack;
}

int64_t taida_stub_new(int64_t message) {
    (void)message;
    return taida_molten_new();
}

int64_t taida_todo_new(int64_t id, int64_t task, int64_t sol, int64_t unm) {
    /* BE-WASM-1: proper TODO pack matching native_runtime.c layout.
       Fields: id(0), task(1), sol(2), unm(3), __value(4), __default(5), __type(6) */
    int64_t pack = taida_pack_new(7);
    taida_pack_set_hash(pack, 0, WASM_HASH_TODO_ID);
    taida_pack_set(pack, 0, id);
    taida_pack_set_hash(pack, 1, WASM_HASH_TODO_TASK);
    taida_pack_set(pack, 1, task);
    taida_pack_set_hash(pack, 2, WASM_HASH_TODO_SOL);
    taida_pack_set(pack, 2, sol);
    taida_pack_set_hash(pack, 3, WASM_HASH_TODO_UNM);
    taida_pack_set(pack, 3, unm);
    taida_pack_set_hash(pack, 4, WASM_HASH___VALUE);
    taida_pack_set(pack, 4, sol);
    taida_pack_set_hash(pack, 5, WASM_HASH___DEFAULT);
    taida_pack_set(pack, 5, unm);
    taida_pack_set_hash(pack, 6, WASM_HASH___TYPE);
    taida_pack_set(pack, 6, (int64_t)(intptr_t)"TODO");
    return pack;
}

/* BE-WASM-2: Gorilla literal — immediate crash (matching native exit(1)).
   In WASM, __builtin_trap() produces an unreachable instruction that
   terminates the module, which is the WASM equivalent of exit(). */
void taida_gorilla(void) {
    /* No output — matches native exit(1) behavior */
    __builtin_trap();
}

/* ── W-5: Type conversion molds (returning Lax) ── */
/* These wrap taida_lax_new with conversion logic, matching native_runtime.c */

int64_t taida_str_mold_int(int64_t v) {
    return taida_lax_new(taida_int_to_str(v), (int64_t)(intptr_t)"");
}

int64_t taida_str_mold_float(int64_t v) {
    return taida_lax_new(taida_float_to_str(v), (int64_t)(intptr_t)"");
}

int64_t taida_str_mold_bool(int64_t v) {
    return taida_lax_new(taida_str_from_bool(v), (int64_t)(intptr_t)"");
}

int64_t taida_str_mold_str(int64_t v) {
    return taida_lax_new(v, (int64_t)(intptr_t)"");
}

int64_t taida_int_mold_int(int64_t v) {
    return taida_lax_new(v, 0);
}

int64_t taida_int_mold_float(int64_t v) {
    return taida_lax_new(taida_float_to_int(v), 0);
}

int64_t taida_int_mold_bool(int64_t v) {
    return taida_lax_new(v != 0 ? 1 : 0, 0);
}

int64_t taida_float_mold_int(int64_t v) {
    return taida_lax_new(taida_int_to_float(v), _d2l(0.0));
}

int64_t taida_float_mold_float(int64_t v) {
    return taida_lax_new(v, _d2l(0.0));
}

int64_t taida_float_mold_str(int64_t v) {
    /* Parse string to float — manual parser (no strtod in wasm freestanding) */
    const char *s = (const char *)(intptr_t)v;
    if (!s || *s == '\0') return taida_lax_empty(_d2l(0.0));

    int i = 0;
    int negative = 0;
    if (s[i] == '-') { negative = 1; i++; }
    else if (s[i] == '+') { i++; }

    /* Must start with digit or '.' */
    if (!((s[i] >= '0' && s[i] <= '9') || s[i] == '.'))
        return taida_lax_empty(_d2l(0.0));

    double result = 0.0;
    int has_int_digits = 0;
    /* Integer part */
    while (s[i] >= '0' && s[i] <= '9') {
        result = result * 10.0 + (s[i] - '0');
        has_int_digits = 1;
        i++;
    }
    /* Fractional part */
    int has_frac_digits = 0;
    if (s[i] == '.') {
        i++;
        double frac = 0.1;
        while (s[i] >= '0' && s[i] <= '9') {
            result += (s[i] - '0') * frac;
            frac *= 0.1;
            has_frac_digits = 1;
            i++;
        }
    }
    /* "." alone (no digits before or after dot) is invalid */
    if (!has_int_digits && !has_frac_digits)
        return taida_lax_empty(_d2l(0.0));
    /* Exponent part (e/E) */
    if (s[i] == 'e' || s[i] == 'E') {
        i++;
        int exp_neg = 0;
        if (s[i] == '-') { exp_neg = 1; i++; }
        else if (s[i] == '+') { i++; }
        int exp = 0;
        int has_exp_digits = 0;
        while (s[i] >= '0' && s[i] <= '9') {
            exp = exp * 10 + (s[i] - '0');
            has_exp_digits = 1;
            i++;
        }
        /* "1e", "1e+", "1e-" (no exponent digits) is invalid */
        if (!has_exp_digits) return taida_lax_empty(_d2l(0.0));
        double multiplier = 1.0;
        for (int e = 0; e < exp; e++) multiplier *= 10.0;
        if (exp_neg) result /= multiplier;
        else result *= multiplier;
    }
    /* Must have consumed entire string */
    if (s[i] != '\0') return taida_lax_empty(_d2l(0.0));

    if (negative) result = -result;
    return taida_lax_new(_d2l(result), _d2l(0.0));
}

int64_t taida_float_mold_bool(int64_t v) {
    return taida_lax_new(_d2l(v ? 1.0 : 0.0), _d2l(0.0));
}

int64_t taida_bool_mold_int(int64_t v) {
    return taida_lax_new(v != 0 ? 1 : 0, 0);
}

int64_t taida_bool_mold_float(int64_t v) {
    double d = _to_double(v);
    return taida_lax_new(d != 0.0 ? 1 : 0, 0);
}

int64_t taida_bool_mold_str(int64_t v) {
    const char *s = (const char *)(intptr_t)v;
    if (!s) return taida_lax_empty(0);
    if (s[0] == 't' && s[1] == 'r' && s[2] == 'u' && s[3] == 'e' && s[4] == 0)
        return taida_lax_new(1, 0);
    if (s[0] == 'f' && s[1] == 'a' && s[2] == 'l' && s[3] == 's' && s[4] == 'e' && s[5] == 0)
        return taida_lax_new(0, 0);
    return taida_lax_empty(0); /* not "true" or "false" — empty Lax */
}

int64_t taida_bool_mold_bool(int64_t v) {
    return taida_lax_new(v, 0);
}

/* ── W-5: Float div/mod molds (returning Lax) ── */

int64_t taida_float_div_mold(int64_t a, int64_t b) {
    double da = _to_double(a), db = _to_double(b);
    if (db == 0.0) return taida_lax_new(_d2l(0.0), _d2l(0.0));
    return taida_lax_new(_d2l(da / db), _d2l(0.0));
}

int64_t taida_float_mod_mold(int64_t a, int64_t b) {
    double da = _to_double(a), db = _to_double(b);
    if (db == 0.0) return taida_lax_new(_d2l(0.0), _d2l(0.0));
    /* fmod without libc — use repeated subtraction (good enough for wasm-min) */
    double q = da / db;
    /* truncate toward zero */
    int64_t qi = (int64_t)q;
    double result = da - (double)qi * db;
    return taida_lax_new(_d2l(result), _d2l(0.0));
}

/* ── W-5: Float comparison ── */

int64_t taida_float_eq(int64_t a, int64_t b) { return _to_double(a) == _to_double(b) ? 1 : 0; }
int64_t taida_float_neq(int64_t a, int64_t b) { return _to_double(a) != _to_double(b) ? 1 : 0; }
int64_t taida_float_lt(int64_t a, int64_t b) { return _to_double(a) < _to_double(b) ? 1 : 0; }
int64_t taida_float_gt(int64_t a, int64_t b) { return _to_double(a) > _to_double(b) ? 1 : 0; }
int64_t taida_float_lte(int64_t a, int64_t b) { return _to_double(a) <= _to_double(b) ? 1 : 0; }
int64_t taida_float_gte(int64_t a, int64_t b) { return _to_double(a) >= _to_double(b) ? 1 : 0; }

/* ── W-5: String template helpers (aliases) ── */

int64_t taida_str_from_int(int64_t v) { return taida_int_to_str(v); }
int64_t taida_str_from_float(int64_t v) { return taida_float_to_str(v); }

/* ── W-5: Error helpers ── */

int64_t taida_can_throw_payload(int64_t val) {
    /* Check if val is a pack with a "type" field (looks like an error) */
    if (val < 4096) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    /* Simple heuristic: first field hash is type hash */
    if (p[0] >= 1 && p[0] <= 10 && p[1] == WASM_HASH_TYPE) return 1;
    return 0;
}

/* ── RC no-ops (wasm-min ではヒープなし) ── */

void taida_retain(int64_t val) { (void)val; }
void taida_release(int64_t val) { (void)val; }
void taida_str_retain(int64_t val) { (void)val; }

/* ── _taida_main: C emitter が生成する関数（extern） ── */

extern int64_t _taida_main(void);

/* ── _start: WASI エントリポイント ── */

void _start(void) {
    _taida_main();
}
