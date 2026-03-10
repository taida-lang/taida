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

static unsigned int bump_ptr = 0;  /* 0 = uninitialized */

static void *wasm_alloc(unsigned int size) {
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

/* ── Div/Mod mold (wasm-min: ヒープなし簡易実装) ── */
/* Native では Lax ラッパーを返すが、wasm-min では値を直接返す。     */
/* taida_generic_unmold は identity（Lax ラッパーがないため）。         */

int64_t taida_div_mold(int64_t a, int64_t b) {
    if (b == 0) return 0;
    return a / b;
}

int64_t taida_mod_mold(int64_t a, int64_t b) {
    if (b == 0) return 0;
    return a % b;
}

int64_t taida_generic_unmold(int64_t val) {
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

/* ── W-3: taida_debug_float: debug(Float) — f64 の文字列化 + stdout ── */

int64_t taida_debug_float(int64_t val) {
    double d = _l2d(val);
    /* Format as %g (same as native runtime's printf("%g\n", value)) */
    /* Stack-local buffer, no heap needed */
    char buf[64];
    int len = 0;

    /* Handle negative */
    if (d < 0) {
        buf[len++] = '-';
        d = -d;
    }

    /* Handle special cases */
    /* NaN check: NaN != NaN */
    if (d != d) {
        buf[len++] = 'N'; buf[len++] = 'a'; buf[len++] = 'N';
        write_stdout(buf, len);
        write_stdout("\n", 1);
        return 0;
    }
    /* Infinity: a very large value */
    if (d > 1e308) {
        buf[len++] = 'i'; buf[len++] = 'n'; buf[len++] = 'f';
        write_stdout(buf, len);
        write_stdout("\n", 1);
        return 0;
    }

    /* Integer part */
    uint64_t ipart = (uint64_t)d;
    double frac = d - (double)ipart;

    /* Convert integer part to string */
    char itmp[21];
    int ipos = 20;
    itmp[ipos] = '\0';
    if (ipart == 0) {
        itmp[--ipos] = '0';
    } else {
        while (ipart > 0) {
            itmp[--ipos] = '0' + (char)(ipart % 10);
            ipart /= 10;
        }
    }
    for (int i = ipos; i < 20; i++) buf[len++] = itmp[i];

    /* Fractional part: up to 6 significant digits, trim trailing zeros */
    /* Match %g behavior: no decimal point if fraction is zero */
    if (frac > 0.0000005) {
        buf[len++] = '.';
        int frac_start = len;
        int frac_digits = 0;
        /* %g uses at most 6 significant figures total, but for simplicity
           we print up to 6 fractional digits and trim trailing zeros */
        for (int i = 0; i < 6; i++) {
            frac *= 10.0;
            int digit = (int)frac;
            if (digit > 9) digit = 9;
            frac -= (double)digit;
            buf[len++] = '0' + (char)digit;
            frac_digits++;
        }
        /* Round last digit */
        if (frac >= 0.5 && len > frac_start) {
            int carry = 1;
            for (int i = len - 1; i >= frac_start && carry; i--) {
                int d2 = (buf[i] - '0') + carry;
                if (d2 >= 10) {
                    buf[i] = '0';
                    carry = 1;
                } else {
                    buf[i] = '0' + (char)d2;
                    carry = 0;
                }
            }
        }
        /* Trim trailing zeros */
        while (len > frac_start && buf[len - 1] == '0') len--;
        /* If all fractional digits were trimmed, remove the dot too */
        if (len == frac_start) len--;
        (void)frac_digits;
    }

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
    /* Reuse debug_float formatting logic but write to heap buffer */
    char tmp[64];
    int len = 0;

    if (d < 0) { tmp[len++] = '-'; d = -d; }
    if (d != d) { tmp[len++] = 'N'; tmp[len++] = 'a'; tmp[len++] = 'N'; }
    else if (d > 1e308) { tmp[len++] = 'i'; tmp[len++] = 'n'; tmp[len++] = 'f'; }
    else {
        uint64_t ipart = (uint64_t)d;
        double frac = d - (double)ipart;
        char itmp2[21]; int ipos = 20; itmp2[ipos] = '\0';
        if (ipart == 0) { itmp2[--ipos] = '0'; }
        else { while (ipart > 0) { itmp2[--ipos] = '0' + (char)(ipart % 10); ipart /= 10; } }
        for (int i = ipos; i < 20; i++) tmp[len++] = itmp2[i];
        if (frac > 0.0000005) {
            tmp[len++] = '.';
            int frac_start = len;
            for (int i = 0; i < 6; i++) {
                frac *= 10.0; int digit = (int)frac;
                if (digit > 9) digit = 9; frac -= (double)digit;
                tmp[len++] = '0' + (char)digit;
            }
            if (frac >= 0.5 && len > frac_start) {
                int carry = 1;
                for (int i = len - 1; i >= frac_start && carry; i--) {
                    int d2 = (tmp[i] - '0') + carry;
                    if (d2 >= 10) { tmp[i] = '0'; carry = 1; }
                    else { tmp[i] = '0' + (char)d2; carry = 0; }
                }
            }
            while (len > frac_start && tmp[len - 1] == '0') len--;
            if (len == frac_start) len--;
        }
    }

    char *buf = (char *)wasm_alloc(len + 1);
    if (!buf) return 0;
    for (int i = 0; i < len; i++) buf[i] = tmp[i];
    buf[len] = '\0';
    return (int64_t)(intptr_t)buf;
}

/* ── RC no-ops (wasm-min ではヒープなし) ── */

void taida_retain(int64_t val) { (void)val; }
void taida_release(int64_t val) { (void)val; }

/* ── _taida_main: C emitter が生成する関数（extern） ── */

extern int64_t _taida_main(void);

/* ── _start: WASI エントリポイント ── */

void _start(void) {
    _taida_main();
}
