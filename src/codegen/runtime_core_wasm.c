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

/* ── RC no-ops (wasm-min ではヒープなし) ── */

void taida_retain(int64_t val) { (void)val; }
void taida_release(int64_t val) { (void)val; }

/* ── _taida_main: C emitter が生成する関数（extern） ── */

extern int64_t _taida_main(void);

/* ── _start: WASI エントリポイント ── */

void _start(void) {
    _taida_main();
}
