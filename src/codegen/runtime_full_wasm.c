/// runtime_full_wasm.c -- wasm-full 拡張ランタイム
///
/// wasm-min (runtime_core_wasm.c) + wasm-wasi (runtime_wasi_io.c) の上に
/// 拡張 runtime 関数を追加する。対象:
///
/// - 文字列 molds (to_upper, to_lower, trim, split, replace, etc.)
/// - 数値 molds (float abs/ceil/floor/round/clamp, int clamp, etc.)
/// - 拡張 list 操作 (filter, fold, find, sort, unique, etc.)
/// - HashMap/Set 拡張 (length, clone, keys, values, entries, etc.)
/// - JSON runtime (parse, stringify, schema_cast, etc.)
/// - Gorillax/Lax/Result 拡張 (map, flat_map, to_string, etc.)
/// - bytes / bitwise / char / codepoint
/// - Pack/Error/Field/Callback 拡張
/// - Global get/set
///
/// このファイルは runtime_core_wasm.c で定義された関数・マクロを extern 参照する。
/// runtime_core_wasm.c を #include するのではなく、wasm-ld が symbol 解決する。

#include <stdint.h>

// ---------------------------------------------------------------------------
// Forward declarations from runtime_core_wasm.c
// (linked via wasm-ld, not #include)
// ---------------------------------------------------------------------------
extern int64_t wasm_alloc(int size);
extern void wasm_copy(void *dst, const void *src, int len);
extern int wasm_strlen(const char *s);
extern int wasm_strcmp(const char *a, const char *b);
extern int wasm_strncmp(const char *a, const char *b, int n);

// Lax/Error/Pack/List/HashMap/Set helpers from core
extern int64_t taida_lax_new(int64_t value, int64_t default_value);
extern int64_t taida_lax_empty(int64_t default_value);
extern int64_t taida_lax_unmold(int64_t lax_ptr);
extern int64_t taida_lax_has_value(int64_t lax_ptr);
extern int64_t taida_list_new(void);
extern int64_t taida_list_push(int64_t list_ptr, int64_t item);
extern int64_t taida_list_length(int64_t list_ptr);
extern int64_t taida_list_get(int64_t list_ptr, int64_t index);
extern int64_t taida_pack_new(int64_t field_count);
extern int64_t taida_pack_set(int64_t pack_ptr, int64_t index, int64_t value);
extern int64_t taida_pack_get_idx(int64_t pack_ptr, int64_t index);
extern int64_t taida_pack_set_hash(int64_t pack_ptr, int64_t index, int64_t hash);
extern int64_t taida_hashmap_new(void);
extern int64_t taida_hashmap_set(int64_t hm, int64_t kh, int64_t kp, int64_t v);
extern int64_t taida_hashmap_get(int64_t hm, int64_t kh, int64_t kp);
extern int64_t taida_hashmap_has(int64_t hm, int64_t kh, int64_t kp);
extern int64_t taida_str_hash(int64_t str_ptr);
extern int64_t taida_str_concat(int64_t a, int64_t b);
extern int64_t taida_str_length(int64_t s);
extern int64_t taida_str_eq(int64_t a, int64_t b);
extern int64_t taida_int_to_str(int64_t a);
extern int64_t taida_float_to_str(int64_t a);
extern int64_t taida_str_from_bool(int64_t v);
extern int64_t taida_make_error(int64_t type_ptr, int64_t msg_ptr);
extern int64_t taida_throw(int64_t error_val);
extern int64_t taida_gorillax_new(int64_t value);
extern int64_t taida_gorillax_err(int64_t error);
extern int64_t taida_gorillax_is_ok(int64_t gx);
extern int64_t taida_gorillax_get_value(int64_t gx);
extern int64_t taida_gorillax_get_error(int64_t gx);
extern int64_t taida_result_create(int64_t value, int64_t throw_val, int64_t predicate);
extern int64_t taida_result_is_ok(int64_t result);
extern int64_t taida_result_is_error(int64_t result);
extern int64_t taida_closure_get_fn(int64_t closure_ptr);
extern int64_t taida_closure_get_env(int64_t closure_ptr);
extern int64_t taida_is_closure_value(int64_t val);
extern int64_t taida_set_from_list(int64_t list_ptr);
extern int64_t taida_set_add(int64_t set_ptr, int64_t item);
extern int64_t taida_set_has(int64_t set_ptr, int64_t item);
extern int64_t taida_value_hash(int64_t val);
extern int64_t taida_register_field_name(int64_t hash, int64_t name_ptr);
extern int64_t taida_polymorphic_to_string(int64_t obj);

// Float bit-punning helpers (same as runtime_core_wasm.c)
static inline double _to_double(int64_t v) {
    union { int64_t i; double d; } u;
    u.i = v;
    return u.d;
}
static inline int64_t _d2l(double d) {
    union { int64_t i; double d2; } u;
    u.d2 = d;
    return u.i;
}

// ---------------------------------------------------------------------------
// WF-2b: String molds (placeholder -- implementation in subsequent phases)
// ---------------------------------------------------------------------------

// Implementation will be added in WF-2b through WF-3d phases.
// Each phase adds the corresponding function implementations.
// For now, this file compiles as an empty translation unit.
