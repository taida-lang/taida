#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <math.h>
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <sys/wait.h>
#include <sys/mman.h>
#include <stdint.h>
#include <inttypes.h>
#include <limits.h>
#include <pthread.h>

// FL-9: Safe realloc wrapper — aborts on NULL with a diagnostic message.
// Usage: TAIDA_REALLOC(ptr, new_size, context_label)
// ptr is reassigned in-place; on failure, prints OOM message and exits.
#define TAIDA_REALLOC(ptr, size, label) do { \
    void *_tmp = realloc((ptr), (size)); \
    if (!_tmp) { fprintf(stderr, "taida: out of memory (%s)\n", (label)); exit(1); } \
    (ptr) = _tmp; \
} while(0)

// Safe malloc wrapper — aborts on NULL with a diagnostic message.
// Usage: void *p = TAIDA_MALLOC(size, context_label);
// Returns the allocated pointer; on failure, prints OOM message and exits.
static inline void *taida_safe_malloc(size_t size, const char *label) {
    // R-02: malloc(0) is implementation-defined (may return NULL or a unique
    // pointer). Normalize to 1 so that the NULL check below reliably detects
    // real OOM rather than a valid zero-size allocation.
    if (size == 0) size = 1;
    void *p = malloc(size);
    if (!p) { fprintf(stderr, "taida: out of memory (%s)\n", label); exit(1); }
    return p;
}
#define TAIDA_MALLOC(size, label) taida_safe_malloc((size), (label))

// M-00: Safe arithmetic helpers for allocation size calculations.
// These detect overflow BEFORE passing the result to malloc, preventing
// heap corruption from silently-wrapped size_t values.

// Multiply two size_t values; abort on overflow.
static inline size_t taida_safe_mul(size_t a, size_t b, const char *label) {
    if (a != 0 && b > SIZE_MAX / a) {
        fprintf(stderr, "taida: integer overflow in size calculation (%s): %zu * %zu\n", label, a, b);
        exit(1);
    }
    return a * b;
}

// Add two size_t values; abort on overflow.
static inline size_t taida_safe_add(size_t a, size_t b, const char *label) {
    if (a > SIZE_MAX - b) {
        fprintf(stderr, "taida: integer overflow in size calculation (%s): %zu + %zu\n", label, a, b);
        exit(1);
    }
    return a + b;
}

// ---------------------------------------------------------------------------
// W-0d/W-0f: ABI 正規化 — 値型・ポインタ型・関数ポインタ型の typedef
//
// Native (LP64): taida_val = int64_t (8B), taida_ptr = intptr_t (8B)
// WASM32 (ILP32): taida_val = int64_t (8B), taida_ptr = intptr_t (4B)
//
// W-0 では Native のみ。LP64 環境で taida_val = int64_t = intptr_t = 64-bit なので
// 動作は一切変わらない。WASM32 対応時に taida_ptr が 32-bit になる。
//
// W-0f: Forward declarations と主要関数定義（pack, closure, retain/release, list）に
// taida_ptr/taida_fn_ptr を適用済み。残りの関数定義は LP64 では taida_val と同一型
// のため動作に影響なし。WASM32 runtime は別ファイル（runtime_core_wasm.c）になる
// ため、native_runtime.c の完全移行は必須ではない。
// ---------------------------------------------------------------------------
typedef int64_t   taida_val;     // 整数値 (Int, Bool, Hash, tag, count, index, dummy)
typedef intptr_t  taida_ptr;     // ヒープポインタ (Str, Pack, List, HashMap, Set, ...)
typedef intptr_t  taida_fn_ptr;  // 関数ポインタ

// Taida Magic Numbers for pointer tagging and type safety
#define TAIDA_MAGIC_MASK  0xFFFFFFFFFFFFFF00LL
#define TAIDA_RC_MASK     0x00000000000000FFLL
#define TAIDA_LIST_MAGIC  0x544149444C535400LL // "TAIDLST\0"
#define TAIDA_PACK_MAGIC  0x5441494450414B00LL // "TAIDPAK\0"
#define TAIDA_STR_MAGIC   0x5441494453545200LL // "TAIDSTR\0"
#define TAIDA_HMAP_MAGIC  0x54414944484D4100LL // "TAIDHMA\0"
#define TAIDA_SET_MAGIC   0x5441494453455400LL // "TAIDSET\0"
#define TAIDA_ASYNC_MAGIC 0x5441494441535900LL // "TAIDASY\0"
#define TAIDA_BYTES_MAGIC 0x5441494442595400LL // "TAIDBYT\0"
#define TAIDA_CLOSURE_MAGIC 0x54414944434C4F00LL // "TAIDCLO\0"

// Type tags for BuchiPack field values (A-4a)
#define TAIDA_TAG_INT     0
#define TAIDA_TAG_FLOAT   1
#define TAIDA_TAG_BOOL    2
#define TAIDA_TAG_STR     3
#define TAIDA_TAG_PACK    4
#define TAIDA_TAG_LIST    5
#define TAIDA_TAG_CLOSURE 6
#define TAIDA_TAG_HMAP    7
#define TAIDA_TAG_SET     8
#define TAIDA_TAG_UNKNOWN -1

// ============================================================================
// NO-4: Native Ownership Rules (runtime helper 監査ルール)
// See docs/NATIVE_OWNERSHIP_AUDIT.md for the full specification.
//
// RULE 1 — No raw store: aggregate objects (Pack, List, HashMap, Set, Async)
//   MUST use retain + tag helper paths when storing heap children.
//   Direct pointer writes without retain/tag are PROHIBITED.
//   Helpers: taida_retain_and_tag_field (Pack), taida_list_elem_retain (List),
//            taida_str_retain (String), taida_hashmap_set (HashMap),
//            taida_set_add (Set).
//
// RULE 2 — Display/toString temporaries: every heap string created by
//   taida_value_to_display_string / taida_value_to_debug_string / *_to_string
//   MUST be released by the caller via taida_str_release after use.
//   Pattern: taida_val tmp = taida_value_to_debug_string(val); use(tmp); taida_str_release(tmp);
//
// RULE 3 — New runtime helper checklist (see NATIVE_OWNERSHIP_AUDIT.md):
//   (a) Does it allocate heap objects? -> Set magic header + initial rc=1.
//   (b) Does it store heap children? -> Use retain + tag helper, never raw store.
//   (c) Does it return heap objects? -> Document ownership transfer (move vs borrow).
//   (d) Does it create temporary heap values? -> Release after use.
//   (e) Does it copy from existing containers? -> Retain each copied child.
//   (f) Add regression test in tests/native_compile.rs.
// ============================================================================

// HashMap layout macros (needed by taida_release before HashMap section)
#define HM_HEADER 4  // number of header slots before entries
#define TAIDA_HASHMAP_TOMBSTONE_HASH ((taida_val)1)
#define HM_SLOT_EMPTY(h, k)     ((h) == 0 && (k) == 0)
#define HM_SLOT_TOMBSTONE(h, k) ((h) == TAIDA_HASHMAP_TOMBSTONE_HASH && (k) == 0)
#define HM_SLOT_OCCUPIED(h, k)  (!HM_SLOT_EMPTY(h, k) && !HM_SLOT_TOMBSTONE(h, k))

#define TAIDA_IS_LIST(ptr)  (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_LIST_MAGIC)
#define TAIDA_IS_PACK(ptr)  (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_PACK_MAGIC)
#define TAIDA_IS_STR(ptr)   (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_STR_MAGIC)
#define TAIDA_IS_HMAP(ptr)  (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_HMAP_MAGIC)
#define TAIDA_IS_SET(ptr)   (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_SET_MAGIC)
#define TAIDA_IS_ASYNC(ptr) (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_ASYNC_MAGIC)
#define TAIDA_IS_BYTES(ptr) (taida_ptr_is_readable(ptr, 8) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_BYTES_MAGIC)
#define TAIDA_IS_CLOSURE(ptr) (taida_ptr_is_readable(ptr, sizeof(taida_val) * 3) && (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) == TAIDA_CLOSURE_MAGIC)

// NB-31: Callable check — closure OR readable non-heap-tagged pointer (function pointer).
// Catches: integers (not readable), data objects (readable + heap magic), null/zero.
// Cannot catch all adversarial cases (e.g., aligned readable int in code segment range)
// but prevents the common httpServe(port, 42) / httpServe(port, 50000) crash paths.
#define TAIDA_IS_CALLABLE(val) _taida_is_callable_impl(val)

#define TAIDA_GET_RC(ptr)      (((taida_val*)ptr)[0] & TAIDA_RC_MASK)
#define TAIDA_SET_RC(ptr, rc)  (((taida_val*)ptr)[0] = (((taida_val*)ptr)[0] & TAIDA_MAGIC_MASK) | ((rc) & TAIDA_RC_MASK))
// NB2-7: Thread-safe RC increment/decrement using CAS loop.
// Prevents data races when multiple worker threads retain/release objects concurrently.
#define TAIDA_INC_RC(ptr) do { \
    volatile taida_val *_hdr = (volatile taida_val*)(ptr); \
    taida_val _old, _new; \
    do { \
        _old = __atomic_load_n(_hdr, __ATOMIC_RELAXED); \
        taida_val _rc = _old & TAIDA_RC_MASK; \
        if (_rc >= 255) break; \
        _new = (_old & TAIDA_MAGIC_MASK) | ((_rc + 1) & TAIDA_RC_MASK); \
    } while (!__atomic_compare_exchange_n(_hdr, &_old, _new, 0, __ATOMIC_ACQ_REL, __ATOMIC_RELAXED)); \
} while (0)
#define TAIDA_DEC_RC(ptr) do { \
    volatile taida_val *_hdr = (volatile taida_val*)(ptr); \
    taida_val _old, _new; \
    do { \
        _old = __atomic_load_n(_hdr, __ATOMIC_RELAXED); \
        taida_val _rc = _old & TAIDA_RC_MASK; \
        if (_rc == 0) break; \
        _new = (_old & TAIDA_MAGIC_MASK) | ((_rc - 1) & TAIDA_RC_MASK); \
    } while (!__atomic_compare_exchange_n(_hdr, &_old, _new, 0, __ATOMIC_ACQ_REL, __ATOMIC_RELAXED)); \
} while (0)

extern taida_val _taida_main(void);
static int taida_cli_argc = 0;
static char **taida_cli_argv = NULL;

// Forward declarations
// W-0f F-3: taida_ptr = heap pointer, taida_fn_ptr = function pointer, taida_val = value
taida_val taida_retain(taida_ptr ptr);
taida_ptr taida_async_ok(taida_val value);
taida_ptr taida_async_ok_tagged(taida_val value, taida_val value_tag);
taida_ptr taida_async_err(taida_ptr error);
void taida_async_set_value_tag(taida_ptr async_ptr, taida_val tag);
taida_ptr taida_pack_new(taida_val field_count);
taida_ptr taida_pack_set(taida_ptr pack_ptr, taida_val field_idx, taida_val value);
taida_ptr taida_pack_set_hash(taida_ptr pack_ptr, taida_val index, taida_val hash);
taida_ptr taida_pack_set_tag(taida_ptr pack_ptr, taida_val index, taida_val tag);
taida_val taida_pack_get_idx(taida_ptr pack_ptr, taida_val index);
taida_val taida_throw(taida_ptr error_val);
taida_ptr taida_lax_new(taida_val value, taida_val default_value);
taida_ptr taida_lax_empty(taida_val default_value);
taida_val taida_lax_unmold(taida_ptr lax_ptr);
taida_val taida_async_unmold(taida_ptr async_ptr);
taida_ptr taida_async_all(taida_ptr list_ptr);
taida_ptr taida_async_race(taida_ptr list_ptr);
taida_ptr taida_async_map(taida_ptr async_ptr, taida_fn_ptr fn_ptr);
taida_val taida_async_get_or_default(taida_ptr async_ptr, taida_val def);
taida_ptr taida_async_spawn(taida_fn_ptr fn_ptr, taida_val arg);
taida_ptr taida_async_cancel(taida_ptr async_ptr);
static void taida_async_join(taida_ptr async_ptr);
static taida_ptr taida_async_to_string(taida_ptr async_ptr);
taida_ptr taida_str_from_int(taida_val v);
taida_ptr taida_str_from_float(double v);
taida_ptr taida_str_from_bool(taida_val v);
taida_val taida_is_closure_value(taida_val ptr);
taida_ptr taida_json_schema_cast(taida_ptr raw_ptr, taida_ptr schema_ptr);
taida_ptr taida_list_new(void);
taida_ptr taida_list_push(taida_ptr list_ptr, taida_val item);
taida_ptr taida_result_create(taida_val value, taida_ptr throw_val, taida_fn_ptr predicate);
taida_ptr taida_result_map(taida_ptr result, taida_fn_ptr fn_ptr);
taida_ptr taida_result_flat_map(taida_ptr result, taida_fn_ptr fn_ptr);
taida_ptr taida_result_map_error(taida_ptr result, taida_fn_ptr fn_ptr);
taida_val taida_result_get_or_throw(taida_ptr result);
taida_ptr taida_result_to_string(taida_ptr result);
taida_val taida_list_length(taida_ptr list_ptr);
taida_ptr taida_lax_map(taida_ptr lax_ptr, taida_fn_ptr fn_ptr);
taida_ptr taida_lax_flat_map(taida_ptr lax_ptr, taida_fn_ptr fn_ptr);
taida_ptr taida_gorillax_new(taida_val value);
taida_ptr taida_molten_new(void);
taida_ptr taida_stub_new(taida_ptr message);
taida_ptr taida_todo_new(taida_ptr id, taida_ptr task, taida_ptr sol, taida_ptr unm);
taida_ptr taida_cage_apply(taida_val cage_value, taida_fn_ptr fn_ptr);
taida_ptr taida_gorillax_err(taida_ptr error);
taida_val taida_gorillax_unmold(taida_ptr ptr);
taida_ptr taida_gorillax_relax(taida_ptr ptr);
taida_ptr taida_gorillax_to_string(taida_ptr ptr);
taida_val taida_relaxed_gorillax_unmold(taida_ptr ptr);
taida_ptr taida_relaxed_gorillax_to_string(taida_ptr ptr);
taida_ptr taida_list_map(taida_ptr list_ptr, taida_fn_ptr fn_ptr);
taida_val taida_list_is_empty(taida_ptr list_ptr);
taida_ptr taida_hashmap_new(void);
void taida_hashmap_set_value_tag(taida_ptr hm_ptr, taida_val tag);
taida_ptr taida_hashmap_set(taida_ptr hm_ptr, taida_val key_hash, taida_ptr key_ptr, taida_val value);
taida_val taida_hashmap_get(taida_ptr hm_ptr, taida_val key_hash, taida_ptr key_ptr);
taida_val taida_hashmap_has(taida_ptr hm_ptr, taida_val key_hash, taida_ptr key_ptr);
taida_ptr taida_hashmap_remove(taida_ptr hm_ptr, taida_val key_hash, taida_ptr key_ptr);
taida_ptr taida_hashmap_remove_immut(taida_ptr hm_ptr, taida_val key_hash, taida_ptr key_ptr);
taida_val taida_hashmap_is_empty(taida_ptr hm_ptr);
taida_ptr taida_hashmap_get_lax(taida_ptr hm_ptr, taida_val key_hash, taida_ptr key_ptr);
taida_ptr taida_hashmap_to_string(taida_ptr hm_ptr);
taida_val taida_str_hash(taida_ptr str_ptr);
taida_ptr taida_str_concat(const char* a, const char* b);
// Heap string helpers (hidden header: [magic+rc, len, bytes...\0])
static char* taida_str_alloc(size_t len);
static char* taida_str_new_copy(const char* src);
void  taida_str_retain(taida_ptr ptr);
static void  taida_str_release(taida_ptr ptr);
// List element retain/release helpers (based on elem_type_tag)
static void taida_list_elem_retain(taida_val elem, taida_val elem_tag);
static void taida_list_elem_release(taida_val elem, taida_val elem_tag);
taida_ptr taida_list_get(taida_ptr list_ptr, taida_val index);
taida_val taida_set_has(taida_ptr set_ptr, taida_val item);
taida_ptr taida_set_remove(taida_ptr set_ptr, taida_val item);
taida_ptr taida_set_add(taida_ptr set_ptr, taida_val item);
void taida_set_set_elem_tag(taida_ptr set_ptr, taida_val tag);
// Str state check methods
taida_val taida_str_contains(const char* s, const char* sub);
taida_val taida_str_starts_with(const char* s, const char* prefix);
taida_val taida_str_ends_with(const char* s, const char* suffix);
taida_val taida_str_last_index_of(const char* s, const char* sub);
taida_ptr taida_str_get(const char* s, taida_val idx);
taida_ptr taida_str_chars(const char* s);
// List state check methods
taida_val taida_list_last_index_of(taida_ptr list_ptr, taida_val item);
taida_val taida_list_any(taida_ptr list_ptr, taida_fn_ptr fn_ptr);
taida_val taida_list_all(taida_ptr list_ptr, taida_fn_ptr fn_ptr);
taida_val taida_list_none(taida_ptr list_ptr, taida_fn_ptr fn_ptr);
// Prelude functions
taida_ptr taida_io_stdin(taida_ptr prompt_ptr);
taida_ptr taida_sha256(taida_val value);
taida_val taida_time_now_ms(void);
taida_val taida_time_sleep(taida_val ms);
taida_ptr taida_json_encode(taida_val val);
taida_ptr taida_json_pretty(taida_val val);
taida_val taida_register_field_name(taida_val hash, taida_ptr name_ptr);
taida_val taida_register_field_type(taida_val hash, taida_ptr name_ptr, taida_val type_tag);
static const char* taida_lookup_field_name(taida_val hash);
static int taida_lookup_field_type(taida_val hash);
static taida_val taida_is_hashmap(taida_val ptr);
// NO-1: HashMap ownership helpers (forward declarations)
static void taida_hashmap_val_retain(taida_val val, taida_val val_tag);
static void taida_hashmap_val_release(taida_val val, taida_val val_tag);
static void taida_hashmap_key_retain(taida_val key);
static void taida_hashmap_key_release(taida_val key);
static taida_val taida_is_set(taida_val ptr);
static int taida_is_list(taida_val ptr);
static int taida_is_bytes(taida_val ptr);
static int taida_is_buchi_pack(taida_val ptr);
static taida_ptr taida_lax_to_string(taida_ptr lax_ptr);
static taida_ptr taida_set_to_string(taida_ptr set_ptr);
static int taida_has_magic_header(taida_val tag);
static taida_val taida_make_error(const char *error_type, const char *error_msg);
static const char *taida_os_error_kind(int err_code, const char *err_msg);
static taida_val taida_make_io_error(int err_code, const char *err_msg);
static int taida_ptr_is_readable(taida_val ptr, size_t bytes);
static int taida_read_cstr_len_safe(const char *s, size_t max_len, size_t *out_len);
static taida_val taida_value_to_display_string(taida_val val);
static taida_val taida_value_to_debug_string(taida_val val);
static taida_val taida_throw_to_display_string(taida_val throw_val);
taida_val taida_typeof(taida_val val, taida_val tag);
taida_val taida_polymorphic_contains(taida_val obj, taida_val needle);
taida_val taida_polymorphic_index_of(taida_val obj, taida_val needle);
taida_val taida_polymorphic_last_index_of(taida_val obj, taida_val needle);
// C12-6c: Regex polymorphic dispatchers + constructor.
taida_val taida_regex_new(const char *pattern_s, const char *flags_s);
taida_val taida_str_split_poly(const char *s, taida_val sep);
taida_val taida_str_replace_first_poly(const char *s, taida_val target, const char *rep);
taida_val taida_str_replace_poly(const char *s, taida_val target, const char *rep);
taida_val taida_str_match_regex(const char *s, taida_val regex_pack);
taida_val taida_str_search_regex(const char *s, taida_val regex_pack);
static taida_val taida_bytes_new_filled(taida_val len, unsigned char fill);
static taida_val taida_bytes_from_raw(const unsigned char *data, taida_val len);
static taida_val taida_bytes_clone(taida_val bytes_ptr);
static taida_val taida_bytes_len(taida_val bytes_ptr);
static taida_val taida_bytes_default_value(void);
static taida_val taida_bytes_get_lax(taida_val bytes_ptr, taida_val index);
static int taida_bytes_cursor_unpack(taida_val cursor_ptr, taida_val *bytes_out, taida_val *offset_out);
static taida_val taida_bytes_cursor_step(taida_val value, taida_val cursor);
static double _l2d(taida_val v);
static taida_val _d2l(double v);
// OS package forward declarations
taida_val taida_os_read(taida_val path_ptr);
taida_val taida_os_read_bytes(taida_val path_ptr);
taida_val taida_os_list_dir(taida_val path_ptr);
taida_val taida_os_stat(taida_val path_ptr);
taida_val taida_os_exists(taida_val path_ptr);
taida_val taida_os_env_var(taida_val name_ptr);
taida_val taida_os_write_file(taida_val path_ptr, taida_val content_ptr);
taida_val taida_os_write_bytes(taida_val path_ptr, taida_val content_ptr);
taida_val taida_os_append_file(taida_val path_ptr, taida_val content_ptr);
taida_val taida_os_remove(taida_val path_ptr);
taida_val taida_os_create_dir(taida_val path_ptr);
taida_val taida_os_rename(taida_val from_ptr, taida_val to_ptr);
taida_val taida_os_run(taida_val program_ptr, taida_val args_list_ptr);
taida_val taida_os_exec_shell(taida_val command_ptr);
taida_val taida_os_all_env(void);
taida_val taida_os_argv(void);
// OS package Phase 2: async APIs
taida_val taida_os_read_async(taida_val path_ptr);
taida_val taida_os_http_get(taida_val url_ptr);
taida_val taida_os_http_post(taida_val url_ptr, taida_val body_ptr);
taida_val taida_os_http_request(taida_val method_ptr, taida_val url_ptr, taida_val headers_ptr, taida_val body_ptr);
taida_val taida_os_dns_resolve(taida_val host_ptr, taida_val timeout_ms);
taida_val taida_os_tcp_connect(taida_val host_ptr, taida_val port, taida_val timeout_ms);
taida_val taida_os_tcp_listen(taida_val port, taida_val timeout_ms);
taida_val taida_os_tcp_accept(taida_val listener_fd, taida_val timeout_ms);
taida_val taida_os_socket_send(taida_val socket_fd, taida_val data_ptr, taida_val timeout_ms);
taida_val taida_os_socket_send_all(taida_val socket_fd, taida_val data_ptr, taida_val timeout_ms);
taida_val taida_os_socket_recv(taida_val socket_fd, taida_val timeout_ms);
taida_val taida_os_socket_send_bytes(taida_val socket_fd, taida_val data_ptr, taida_val timeout_ms);
taida_val taida_os_socket_recv_bytes(taida_val socket_fd, taida_val timeout_ms);
taida_val taida_os_socket_recv_exact(taida_val socket_fd, taida_val size, taida_val timeout_ms);
taida_val taida_os_udp_bind(taida_val host_ptr, taida_val port, taida_val timeout_ms);
taida_val taida_os_udp_send_to(taida_val socket_fd, taida_val host_ptr, taida_val port, taida_val data_ptr, taida_val timeout_ms);
taida_val taida_os_udp_recv_from(taida_val socket_fd, taida_val timeout_ms);
taida_val taida_os_socket_close(taida_val socket_fd);
taida_val taida_os_listener_close(taida_val listener_fd);
// Pool package runtime
taida_val taida_pool_create(taida_val config_ptr);
taida_val taida_pool_acquire(taida_val pool_or_pack, taida_val timeout_ms);
taida_val taida_pool_release(taida_val pool_or_pack, taida_val token, taida_val resource);
taida_val taida_pool_close(taida_val pool_or_pack);
taida_val taida_pool_health(taida_val pool_or_pack);
taida_val taida_bit_and(taida_val a, taida_val b);
taida_val taida_bit_or(taida_val a, taida_val b);
taida_val taida_bit_xor(taida_val a, taida_val b);
taida_val taida_bit_not(taida_val x);
taida_val taida_shift_l(taida_val x, taida_val n);
taida_val taida_shift_r(taida_val x, taida_val n);
taida_val taida_shift_ru(taida_val x, taida_val n);
taida_val taida_to_radix(taida_val value, taida_val base);
taida_val taida_int_mold_auto(taida_val v);
taida_val taida_int_mold_str_base(taida_val v, taida_val base);
taida_val taida_uint8_mold(taida_val v);
taida_val taida_uint8_mold_float(double v);
taida_val taida_u16be_mold(taida_val value);
taida_val taida_u16le_mold(taida_val value);
taida_val taida_u32be_mold(taida_val value);
taida_val taida_u32le_mold(taida_val value);
taida_val taida_u16be_decode_mold(taida_val value);
taida_val taida_u16le_decode_mold(taida_val value);
taida_val taida_u32be_decode_mold(taida_val value);
taida_val taida_u32le_decode_mold(taida_val value);
taida_val taida_bytes_mold(taida_val value, taida_val fill);
taida_val taida_bytes_set(taida_val bytes_ptr, taida_val idx, taida_val value);
taida_val taida_bytes_to_list(taida_val bytes_ptr);
taida_val taida_bytes_cursor_new(taida_val bytes_ptr, taida_val offset);
taida_val taida_bytes_cursor_remaining(taida_val cursor_ptr);
taida_val taida_bytes_cursor_take(taida_val cursor_ptr, taida_val size);
taida_val taida_bytes_cursor_u8(taida_val cursor_ptr);
taida_val taida_char_mold_int(taida_val value);
taida_val taida_char_mold_str(taida_val value);
taida_val taida_codepoint_mold_str(taida_val value);
taida_val taida_utf8_encode_mold(taida_val value);
taida_val taida_utf8_decode_mold(taida_val value);
taida_val taida_slice_mold(taida_val value, taida_val start, taida_val end);
// NB-14: Call-site arg tag propagation (Bool/Int disambiguation)
void taida_push_call_tags(void);
void taida_pop_call_tags(void);
taida_val taida_set_call_arg_tag(taida_val index, taida_val tag);
taida_val taida_get_call_arg_tag(taida_val index);
// C12B-022: Runtime primitive-type check for TypeIs on param-tag idents
taida_val taida_primitive_tag_match(taida_val tag, taida_val expected);

// Taida runtime functions

// NB-14/NB-21: Stack-based call-site argument type tag propagation.
// Bool and Int are indistinguishable at the value level (both are unboxed i64).
// When a Bool value passes through a function parameter into a BuchiPack field,
// the field tag becomes UNKNOWN because the compiler cannot infer the parameter type.
// This mechanism propagates the caller's compile-time type tag to the callee:
//   Caller: taida_push_call_tags() + taida_set_call_arg_tag(i, tag) before CallUser
//   Callee: taida_get_call_arg_tag(i) at function entry
//   Caller: taida_pop_call_tags() after CallUser returns
// The stack ensures nested calls do not overwrite the outer call's tags.
#define TAG_STACK_DEPTH 64
#define TAG_FRAME_SIZE 256

// NB2-7: Thread-local tag stack prevents concurrent worker threads from corrupting
// each other's call-site type tag frames during handler invocation.
static __thread int64_t __taida_tag_stack[TAG_STACK_DEPTH][TAG_FRAME_SIZE];
static __thread int __taida_tag_stack_top = 0;

void taida_push_call_tags(void) {
    if (__taida_tag_stack_top < TAG_STACK_DEPTH) {
        memset(__taida_tag_stack[__taida_tag_stack_top], 0xFF, sizeof(__taida_tag_stack[0]));
        __taida_tag_stack_top++;
    }
}

void taida_pop_call_tags(void) {
    if (__taida_tag_stack_top > 0) {
        __taida_tag_stack_top--;
    }
}

taida_val taida_set_call_arg_tag(taida_val index, taida_val tag) {
    if (__taida_tag_stack_top > 0 && index >= 0 && index < TAG_FRAME_SIZE) {
        __taida_tag_stack[__taida_tag_stack_top - 1][index] = tag;
    }
    return 0;
}

taida_val taida_get_call_arg_tag(taida_val index) {
    if (__taida_tag_stack_top > 0 && index >= 0 && index < TAG_FRAME_SIZE) {
        return __taida_tag_stack[__taida_tag_stack_top - 1][index];
    }
    return TAIDA_TAG_UNKNOWN;
}

/* C12B-022: Runtime primitive-type check for `TypeIs[v, :T]()` when `v`
 * is a parameter whose type tag is threaded through `param_tag_vars`.
 *
 * `tag` is the runtime tag value (TAIDA_TAG_INT / _FLOAT / _BOOL / _STR / ...).
 * `expected` is the lowerer-emitted sentinel describing which primitive to
 * match:
 *    0  → Int  (tag == TAIDA_TAG_INT)
 *    1  → Float (tag == TAIDA_TAG_FLOAT)
 *    2  → Bool  (tag == TAIDA_TAG_BOOL)
 *    3  → Str   (tag == TAIDA_TAG_STR)
 *   -10 → Num   (tag == TAIDA_TAG_INT || tag == TAIDA_TAG_FLOAT)
 *
 * Returns 0 (false) for TAIDA_TAG_UNKNOWN (-1) so unknown-tag callers do
 * not accidentally match. This preserves the pre-C12B-022 output on
 * compile-time-literal paths (which never reach this helper because the
 * lowerer only emits it when `get_param_tag_var` returns Some).
 */
taida_val taida_primitive_tag_match(taida_val tag, taida_val expected) {
    if (tag == TAIDA_TAG_UNKNOWN) return 0;
    if (expected == -10) {
        return (tag == TAIDA_TAG_INT || tag == TAIDA_TAG_FLOAT) ? 1 : 0;
    }
    return (tag == expected) ? 1 : 0;
}

// NB-14: Return type tag propagation.
// Allows type info to survive through generic function returns (e.g. `id x = x`).
// Callee sets before return, caller reads after CallUser.
// NB2-7: Thread-local return tag prevents worker thread A's return value
// from being overwritten by worker thread B's concurrent handler call.
static __thread int64_t __taida_return_tag = TAIDA_TAG_UNKNOWN;

taida_val taida_set_return_tag(taida_val tag) {
    __taida_return_tag = tag;
    return 0;
}

taida_val taida_get_return_tag(void) {
    taida_val tag = __taida_return_tag;
    __taida_return_tag = TAIDA_TAG_UNKNOWN;
    return tag;
}

void taida_gorilla(void) { exit(1); }

taida_val taida_debug_int(taida_val value) {
    printf("%" PRId64 "\n", value);
    return 0;
}

taida_val taida_debug_float(double value) {
    printf("%g\n", value);
    return 0;
}

taida_val taida_debug_bool(taida_val value) {
    if (value) printf("true\n"); else printf("false\n");
    return 0;
}

taida_val taida_debug_str(const char* ptr) {
    if (ptr) printf("%s\n", ptr); else printf("\n");
    return 0;
}

// Polymorphic debug: convert any value to string and print
// Uses taida_value_to_display_string (forward-declared above)
taida_val taida_debug_polymorphic(taida_val val) {
    taida_val str = taida_value_to_display_string(val);
    const char *s = (const char *)(intptr_t)str;
    if (s) printf("%s\n", s); else printf("\n");
    return 0;
}

// Arithmetic runtime
taida_val taida_int_add(taida_val a, taida_val b) { return a + b; }
taida_val taida_int_sub(taida_val a, taida_val b) { return a - b; }
taida_val taida_int_mul(taida_val a, taida_val b) { return a * b; }

// Bitwise molds (Int64 semantics)
taida_val taida_bit_and(taida_val a, taida_val b) { return (taida_val)(((uint64_t)a) & ((uint64_t)b)); }
taida_val taida_bit_or(taida_val a, taida_val b) { return (taida_val)(((uint64_t)a) | ((uint64_t)b)); }
taida_val taida_bit_xor(taida_val a, taida_val b) { return (taida_val)(((uint64_t)a) ^ ((uint64_t)b)); }
taida_val taida_bit_not(taida_val x) { return (taida_val)(~((uint64_t)x)); }

taida_val taida_shift_l(taida_val x, taida_val n) {
    if (n < 0 || n > 63) return taida_lax_empty(0);
    uint64_t shifted = ((uint64_t)x) << (unsigned int)n;
    return taida_lax_new((taida_val)shifted, 0);
}

taida_val taida_shift_r(taida_val x, taida_val n) {
    if (n < 0 || n > 63) return taida_lax_empty(0);
    int64_t shifted = ((int64_t)x) >> (unsigned int)n;
    return taida_lax_new((taida_val)shifted, 0);
}

taida_val taida_shift_ru(taida_val x, taida_val n) {
    if (n < 0 || n > 63) return taida_lax_empty(0);
    uint64_t shifted = ((uint64_t)x) >> (unsigned int)n;
    return taida_lax_new((taida_val)shifted, 0);
}

static taida_val taida_digit_to_char(taida_val digit) {
    return (digit < 10) ? ('0' + digit) : ('a' + (digit - 10));
}

taida_val taida_to_radix(taida_val value, taida_val base) {
    if (base < 2 || base > 36) return taida_lax_empty((taida_val)"");
    if (value == 0) {
        char *out = taida_str_alloc(1);
        out[0] = '0';
        return taida_lax_new((taida_val)out, (taida_val)"");
    }

    uint64_t mag = value < 0
        ? (uint64_t)(-(value + 1)) + 1
        : (uint64_t)value;
    char tmp[70];
    size_t pos = 0;
    while (mag > 0) {
        uint64_t rem = mag % (uint64_t)base;
        tmp[pos++] = (char)taida_digit_to_char((taida_val)rem);
        mag /= (uint64_t)base;
    }
    if (value < 0) tmp[pos++] = '-';

    char *out = taida_str_alloc(pos);
    for (size_t i = 0; i < pos; i++) {
        out[i] = tmp[pos - 1 - i];
    }
    return taida_lax_new((taida_val)out, (taida_val)"");
}
// FNV-1a hashes for error field names
#define HASH_TYPE    0xa79439ef7bfa9c2dULL
#define HASH_MESSAGE 0x546401b5d2a8d2a4ULL

static void taida_register_builtin_error_field_names(void) {
    static int registered = 0;
    if (registered) return;
    registered = 1;

    taida_register_field_name((taida_val)HASH_TYPE, (taida_val)"type");
    taida_register_field_name((taida_val)HASH_MESSAGE, (taida_val)"message");
    taida_register_field_name(taida_str_hash((taida_val)"field"), (taida_val)"field");
    taida_register_field_name(taida_str_hash((taida_val)"code"), (taida_val)"code");
    taida_register_field_name(taida_str_hash((taida_val)"kind"), (taida_val)"kind");
}

static taida_val taida_make_error(const char *error_type, const char *error_msg) {
    taida_register_builtin_error_field_names();

    taida_val pack = taida_pack_new(3);
    // Set hash for "type" field (index 0)
    taida_pack_set_hash(pack, 0, (taida_val)HASH_TYPE);
    char *type_str = taida_str_new_copy(error_type);
    taida_pack_set(pack, 0, (taida_val)type_str);
    taida_pack_set_tag(pack, 0, TAIDA_TAG_STR);
    // Set hash for "message" field (index 1)
    taida_pack_set_hash(pack, 1, (taida_val)HASH_MESSAGE);
    char *msg_str = taida_str_new_copy(error_msg);
    taida_pack_set(pack, 1, (taida_val)msg_str);
    taida_pack_set_tag(pack, 1, TAIDA_TAG_STR);
    // RCB-101 fix: Set __type field (index 2) so error type matching works.
    // Without this, taida_error_type_matches falls through to catch-all
    // because it looks for __type, not type.
    // Use literal FNV-1a("__type") hash since HASH___TYPE is defined later.
    taida_pack_set_hash(pack, 2, (taida_val)0x84d2d84b631f799bULL);
    char *type_str2 = taida_str_new_copy(error_type);
    taida_pack_set(pack, 2, (taida_val)type_str2);
    taida_pack_set_tag(pack, 2, TAIDA_TAG_STR);
    return pack;
}

static int taida_os_msg_contains(const char *msg, const char *needle) {
    return msg && needle && strstr(msg, needle) != NULL;
}

static const char *taida_os_error_kind(int err_code, const char *err_msg) {
    int code = err_code;
    if (code < 0) code = -code;

    switch (code) {
#ifdef EAGAIN
        case EAGAIN:
#endif
#if defined(EWOULDBLOCK) && (!defined(EAGAIN) || EWOULDBLOCK != EAGAIN)
        case EWOULDBLOCK:
#endif
#ifdef ETIMEDOUT
        case ETIMEDOUT:
#endif
            return "timeout";
#ifdef ECONNREFUSED
        case ECONNREFUSED:
#endif
            return "refused";
#ifdef ECONNRESET
        case ECONNRESET:
#endif
            return "reset";
#ifdef ECONNABORTED
        case ECONNABORTED:
#endif
#ifdef EPIPE
        case EPIPE:
#endif
#ifdef ENOTCONN
        case ENOTCONN:
#endif
            return "peer_closed";
#ifdef ENOENT
        case ENOENT:
#endif
            return "not_found";
#ifdef EINVAL
        case EINVAL:
#endif
            return "invalid";
        default:
            break;
    }

    if (
        taida_os_msg_contains(err_msg, "timed out") ||
        taida_os_msg_contains(err_msg, "Timed out") ||
        taida_os_msg_contains(err_msg, "timeout") ||
        taida_os_msg_contains(err_msg, "Timeout")
    ) {
        return "timeout";
    }
    if (
        taida_os_msg_contains(err_msg, "connection refused") ||
        taida_os_msg_contains(err_msg, "Connection refused")
    ) {
        return "refused";
    }
    if (
        taida_os_msg_contains(err_msg, "connection reset") ||
        taida_os_msg_contains(err_msg, "Connection reset")
    ) {
        return "reset";
    }
    if (
        taida_os_msg_contains(err_msg, "peer closed") ||
        taida_os_msg_contains(err_msg, "broken pipe") ||
        taida_os_msg_contains(err_msg, "socket hang up") ||
        taida_os_msg_contains(err_msg, "unexpected eof")
    ) {
        return "peer_closed";
    }
    if (
        taida_os_msg_contains(err_msg, "getaddrinfo") ||
        taida_os_msg_contains(err_msg, "Name or service not known") ||
        taida_os_msg_contains(err_msg, "dns") ||
        taida_os_msg_contains(err_msg, "DNS") ||
        taida_os_msg_contains(err_msg, "resolution failed")
    ) {
        return "dns";
    }
    if (taida_os_msg_contains(err_msg, "invalid") || taida_os_msg_contains(err_msg, "Invalid")) {
        return "invalid";
    }
    return "other";
}

static taida_val taida_make_io_error(int err_code, const char *err_msg) {
    taida_register_builtin_error_field_names();

    const char *message = err_msg ? err_msg : "unknown io error";
    const char *kind = taida_os_error_kind(err_code, message);

    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_TYPE);
    char *type_str = taida_str_new_copy("IoError");
    taida_pack_set(pack, 0, (taida_val)type_str);
    taida_pack_set_tag(pack, 0, TAIDA_TAG_STR);

    taida_pack_set_hash(pack, 1, (taida_val)HASH_MESSAGE);
    char *msg_str = taida_str_new_copy(message);
    taida_pack_set(pack, 1, (taida_val)msg_str);
    taida_pack_set_tag(pack, 1, TAIDA_TAG_STR);

    taida_val code_hash = taida_str_hash((taida_val)"code");
    taida_pack_set_hash(pack, 2, code_hash);
    taida_pack_set(pack, 2, (taida_val)err_code);
    // code is Int, tag defaults to 0 (TAIDA_TAG_INT)

    taida_val kind_hash = taida_str_hash((taida_val)"kind");
    taida_pack_set_hash(pack, 3, kind_hash);
    char *kind_str = taida_str_new_copy(kind);
    taida_pack_set(pack, 3, (taida_val)kind_str);
    taida_pack_set_tag(pack, 3, TAIDA_TAG_STR);
    return pack;
}

// ── Lax[T] runtime ────────────────────────────────────────
// Lax is a BuchiPack with 4 fields: @(hasValue, __value, __default, __type)
// Layout: [refcount, field_count=4, hash0, val0, hash1, val1, hash2, val2, hash3, val3]
// Field 0: hasValue (0 or 1)
// Field 1: __value
// Field 2: __default
// Field 3: __type (pointer to "Lax" string)

static const char __lax_type_str[] = "Lax";
static const char __gorillax_type_str[] = "Gorillax";
static const char __relaxed_gorillax_type_str[] = "RelaxedGorillax";
static const char __molten_type_str[] = "Molten";
static const char __todo_type_str[] = "TODO";
static const char __bytes_cursor_type_str[] = "BytesCursor";

// FNV-1a hashes for Lax field names (computed with FNV-1a algorithm)
#define HASH_HAS_VALUE 0x9e9c6dc733414d60ULL
#define HASH___VALUE   0x0a7fc9f13472bbe0ULL
#define HASH___DEFAULT 0xed4fba440f8602d4ULL
#define HASH___TYPE    0x84d2d84b631f799bULL
#define HASH_TODO_ID   0x08b72e07b55c3ac0ULL
#define HASH_TODO_TASK 0xd9603bef07a9524cULL
#define HASH_TODO_SOL  0x824fa3195cf2e6c1ULL
#define HASH_TODO_UNM  0x4cadac193e198b15ULL
#define HASH_CURSOR_BYTES  0x2f2ec0474f1c4fe4ULL
#define HASH_CURSOR_OFFSET 0x0268b0f8129435caULL
#define HASH_CURSOR_LENGTH 0xea11573f1af59eb5ULL
#define HASH_STEP_VALUE    0x7ce4fd9430e80ceaULL
#define HASH_STEP_CURSOR   0xf927453fbe6252efULL

// retain-on-store helper: detect heap type via magic header and retain + set tag
// NO-4 RULE 1: All runtime-built packs MUST use this helper (or taida_pack_set_tag
// + explicit retain) when storing heap children. Raw taida_pack_set alone is
// INSUFFICIENT — it skips type_tag and retain, causing leak on release.
static void taida_retain_and_tag_field(taida_val pack, taida_val field_idx, taida_val value) {
    if (value > 4096 && taida_ptr_is_readable(value, sizeof(taida_val))) {
        taida_val vtag = ((taida_val*)value)[0] & TAIDA_MAGIC_MASK;
        if (vtag == TAIDA_PACK_MAGIC) {
            taida_retain(value);
            taida_pack_set_tag(pack, field_idx, TAIDA_TAG_PACK);
        } else if (vtag == TAIDA_LIST_MAGIC) {
            taida_retain(value);
            taida_pack_set_tag(pack, field_idx, TAIDA_TAG_LIST);
        } else if (vtag == TAIDA_CLOSURE_MAGIC) {
            taida_retain(value);
            taida_pack_set_tag(pack, field_idx, TAIDA_TAG_CLOSURE);
        } else if (vtag == TAIDA_HMAP_MAGIC) {
            taida_retain(value);
            taida_pack_set_tag(pack, field_idx, TAIDA_TAG_HMAP);
        } else if (vtag == TAIDA_SET_MAGIC) {
            taida_retain(value);
            taida_pack_set_tag(pack, field_idx, TAIDA_TAG_SET);
        } else if (vtag == TAIDA_ASYNC_MAGIC) {
            taida_retain(value);
            taida_pack_set_tag(pack, field_idx, TAIDA_TAG_PACK);  // Async uses PACK tag for retain/release
        }
    }
    // Str の hidden-header は value の 16 バイト前にあるため、上記の value[0] チェックでは検出できない。
    // 別途 ptr-16 を調べて TAIDA_STR_MAGIC を確認する。
    if (value > 4096) {
        taida_val *hdr = ((taida_val*)value) - 2;
        if (taida_ptr_is_readable((taida_val)hdr, sizeof(taida_val))) {
            taida_val htag = hdr[0] & TAIDA_MAGIC_MASK;
            if (htag == TAIDA_STR_MAGIC) {
                taida_str_retain(value);
                taida_pack_set_tag(pack, field_idx, TAIDA_TAG_STR);
            }
        }
    }
}

// TF-15: Register Lax internal field names so display_string can render them
static void taida_register_lax_field_names(void) {
    static int registered = 0;
    if (registered) return;
    registered = 1;
    taida_register_field_name((taida_val)HASH_HAS_VALUE, (taida_val)"hasValue");
    taida_register_field_name((taida_val)HASH___VALUE, (taida_val)"__value");
    taida_register_field_name((taida_val)HASH___DEFAULT, (taida_val)"__default");
    taida_register_field_name((taida_val)HASH___TYPE, (taida_val)"__type");
    // Register hasValue as Bool type for correct display (true/false instead of 0/1)
    taida_register_field_type((taida_val)HASH_HAS_VALUE, (taida_val)"hasValue", 4);
}

taida_val taida_lax_new(taida_val value, taida_val default_value) {
    taida_register_lax_field_names();
    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_HAS_VALUE);
    taida_pack_set(pack, 0, 1);  // hasValue = true
    taida_pack_set_tag(pack, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(pack, 1, (taida_val)HASH___VALUE);
    taida_pack_set(pack, 1, value);
    // retain-on-store: value が Pack/List/Closure の場合 retain + tag 設定
    taida_retain_and_tag_field(pack, 1, value);
    taida_pack_set_hash(pack, 2, (taida_val)HASH___DEFAULT);
    taida_pack_set(pack, 2, default_value);
    // retain-on-store: default_value が Pack/List/Closure の場合 retain + tag 設定
    taida_retain_and_tag_field(pack, 2, default_value);
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__lax_type_str);
    // __lax_type_str is static, not heap-allocated - leave as INT(0) to skip free
    return pack;
}

taida_val taida_lax_empty(taida_val default_value) {
    taida_register_lax_field_names();
    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_HAS_VALUE);
    taida_pack_set(pack, 0, 0);  // hasValue = false
    taida_pack_set_tag(pack, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(pack, 1, (taida_val)HASH___VALUE);
    taida_pack_set(pack, 1, default_value);
    // retain-on-store: default_value stored in __value slot
    taida_retain_and_tag_field(pack, 1, default_value);
    taida_pack_set_hash(pack, 2, (taida_val)HASH___DEFAULT);
    taida_pack_set(pack, 2, default_value);
    // retain-on-store: same default_value stored in __default slot (needs separate retain)
    taida_retain_and_tag_field(pack, 2, default_value);
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__lax_type_str);
    // __lax_type_str is static - leave tag as INT(0)
    return pack;
}

taida_val taida_lax_has_value(taida_val lax_ptr) {
    return taida_pack_get_idx(lax_ptr, 0);  // hasValue field
}

taida_val taida_lax_get_or_default(taida_val lax_ptr, taida_val fallback) {
    if (taida_pack_get_idx(lax_ptr, 0)) {
        return taida_pack_get_idx(lax_ptr, 1);  // __value
    }
    return fallback;
}

taida_val taida_lax_unmold(taida_val lax_ptr) {
    if (taida_pack_get_idx(lax_ptr, 0)) {
        return taida_pack_get_idx(lax_ptr, 1);  // __value
    }
    return taida_pack_get_idx(lax_ptr, 2);  // __default
}

taida_val taida_lax_is_empty(taida_val lax_ptr) {
    return taida_pack_get_idx(lax_ptr, 0) ? 0 : 1;
}

taida_val taida_molten_new(void) {
    taida_val pack = taida_pack_new(1);
    taida_pack_set_hash(pack, 0, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 0, (taida_val)__molten_type_str);
    // Static string - leave tag as INT(0) to skip free
    return pack;
}

taida_val taida_stub_new(taida_val message) {
    if (message == 0 || message < 4096) {
        taida_val err = taida_make_error("TypeError", "Stub message must be a string literal/expression");
        return taida_throw(err);
    }
    size_t len = 0;
    if (!taida_read_cstr_len_safe((const char*)message, 1024, &len)) {
        taida_val err = taida_make_error("TypeError", "Stub message must be a string literal/expression");
        return taida_throw(err);
    }
    return taida_molten_new();
}

taida_val taida_todo_new(taida_val id, taida_val task, taida_val sol, taida_val unm) {
    taida_val pack = taida_pack_new(7);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_TODO_ID);
    taida_pack_set(pack, 0, id);
    taida_pack_set_hash(pack, 1, (taida_val)HASH_TODO_TASK);
    taida_pack_set(pack, 1, task);
    taida_pack_set_hash(pack, 2, (taida_val)HASH_TODO_SOL);
    taida_pack_set(pack, 2, sol);
    taida_pack_set_hash(pack, 3, (taida_val)HASH_TODO_UNM);
    taida_pack_set(pack, 3, unm);
    taida_pack_set_hash(pack, 4, (taida_val)HASH___VALUE);
    taida_pack_set(pack, 4, sol);
    taida_pack_set_hash(pack, 5, (taida_val)HASH___DEFAULT);
    taida_pack_set(pack, 5, unm);
    taida_pack_set_hash(pack, 6, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 6, (taida_val)__todo_type_str);
    return pack;
}

static int taida_is_molten(taida_val ptr) {
    if (!TAIDA_IS_PACK(ptr)) return 0;
    if (!taida_ptr_is_readable(ptr, sizeof(taida_val) * 5)) return 0;
    taida_val *obj = (taida_val*)ptr;
    if (obj[1] != 1) return 0;
    if (obj[2] != (taida_val)HASH___TYPE) return 0;  // hash at stride-3 offset 0
    taida_val type_ptr = obj[4];  // value at stride-3 offset 2
    if (type_ptr == (taida_val)__molten_type_str) return 1;
    if (!taida_ptr_is_readable(type_ptr, 1)) return 0;
    const char *type_str = (const char*)type_ptr;
    size_t len = 0;
    if (!taida_read_cstr_len_safe(type_str, 32, &len)) return 0;
    return len == 6 && memcmp(type_str, "Molten", 6) == 0;
}

// ── Gorillax / RelaxedGorillax ──────────────────────────────
// Gorillax is like Lax but unmold failure = program termination (gorilla).
// RelaxedGorillax is like Gorillax but unmold failure = throw.
// Same BuchiPack layout as Lax: @(hasValue, __value, __error, __type)
// Field 2 is __error (not __default like Lax).

taida_val taida_gorillax_new(taida_val value) {
    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_HAS_VALUE);
    taida_pack_set(pack, 0, 1);  // hasValue = true
    taida_pack_set_tag(pack, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(pack, 1, (taida_val)HASH___VALUE);
    taida_pack_set(pack, 1, value);
    // retain-on-store: value が Pack/List/Closure の場合 retain + tag 設定
    taida_retain_and_tag_field(pack, 1, value);
    taida_pack_set_hash(pack, 2, (taida_val)HASH___DEFAULT);  // reuse hash slot (field 2)
    taida_pack_set(pack, 2, 0);  // __error = Unit (no error)
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__gorillax_type_str);
    return pack;
}

taida_val taida_gorillax_err(taida_val error) {
    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_HAS_VALUE);
    taida_pack_set(pack, 0, 0);  // hasValue = false
    taida_pack_set_tag(pack, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(pack, 1, (taida_val)HASH___VALUE);
    taida_pack_set(pack, 1, 0);  // __value = Unit
    taida_pack_set_hash(pack, 2, (taida_val)HASH___DEFAULT);  // reuse hash slot (field 2)
    taida_pack_set(pack, 2, error);  // __error may be a Pack
    taida_pack_set_tag(pack, 2, TAIDA_TAG_PACK);
    if (error != 0) taida_retain(error);  // retain-on-store: error pack child
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__gorillax_type_str);
    return pack;
}

taida_val taida_gorillax_unmold(taida_val ptr) {
    if (taida_pack_get_idx(ptr, 0)) {
        return taida_pack_get_idx(ptr, 1);  // hasValue=true → __value
    }
    // hasValue=false → GORILLA (program terminates)
    fprintf(stderr, "><\n");
    exit(1);
    return 0;  // unreachable
}

// Gorillax.relax() → RelaxedGorillax (same data, different __type)
taida_val taida_gorillax_relax(taida_val ptr) {
    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_HAS_VALUE);
    taida_pack_set(pack, 0, taida_pack_get_idx(ptr, 0));  // hasValue
    taida_pack_set_tag(pack, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(pack, 1, (taida_val)HASH___VALUE);
    taida_pack_set(pack, 1, taida_pack_get_idx(ptr, 1));  // __value
    // QF-50: retain-on-store for __value (may be Pack/List/Closure/HMap/Set/Str)
    taida_retain_and_tag_field(pack, 1, taida_pack_get_idx(ptr, 1));
    taida_pack_set_hash(pack, 2, (taida_val)HASH___DEFAULT);
    taida_pack_set(pack, 2, taida_pack_get_idx(ptr, 2));  // __error
    // QF-50: retain-on-store for __error (typically a Pack)
    taida_retain_and_tag_field(pack, 2, taida_pack_get_idx(ptr, 2));
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__relaxed_gorillax_type_str);
    return pack;
}

taida_val taida_relaxed_gorillax_unmold(taida_val ptr) {
    if (taida_pack_get_idx(ptr, 0)) {
        return taida_pack_get_idx(ptr, 1);  // hasValue=true → __value
    }
    // hasValue=false → throw RelaxedGorillaEscaped
    taida_val error = taida_make_error("RelaxedGorillaEscaped", "Relaxed gorilla escaped");
    return taida_throw(error);
}

taida_val taida_gorillax_to_string(taida_val ptr) {
    char tmp[128];
    if (taida_pack_get_idx(ptr, 0)) {
        taida_val value = taida_pack_get_idx(ptr, 1);
        snprintf(tmp, sizeof(tmp), "Gorillax(%" PRId64 ")", value);
    } else {
        memcpy(tmp, "Gorillax(><)", 13); /* 12 chars + '\0' */
    }
    return (taida_val)taida_str_new_copy(tmp);
}

taida_val taida_relaxed_gorillax_to_string(taida_val ptr) {
    char tmp[128];
    if (taida_pack_get_idx(ptr, 0)) {
        taida_val value = taida_pack_get_idx(ptr, 1);
        snprintf(tmp, sizeof(tmp), "RelaxedGorillax(%" PRId64 ")", value);
    } else {
        memcpy(tmp, "RelaxedGorillax(escaped)", 24); /* 23 chars + '\0' */
    }
    return (taida_val)taida_str_new_copy(tmp);
}

// Helper: check __type field of fc=4 BuchiPack for Gorillax/RelaxedGorillax
// Returns: 0 = Lax, 1 = Gorillax, 2 = RelaxedGorillax
static int taida_detect_gorillax_type(taida_val ptr) {
    if (!taida_ptr_is_readable(ptr, sizeof(taida_val) * 10)) return 0;
    taida_val type_ptr = taida_pack_get_idx(ptr, 3);  // __type field
    if (type_ptr == 0 || type_ptr < 4096) return 0;
    if (type_ptr == (taida_val)__gorillax_type_str) return 1;
    if (type_ptr == (taida_val)__relaxed_gorillax_type_str) return 2;
    const char *type_str = (const char*)type_ptr;
    size_t len = 0;
    if (!taida_read_cstr_len_safe(type_str, 64, &len)) return 0;
    if (len == 8 && memcmp(type_str, "Gorillax", 8) == 0) return 1;
    if (len == 15 && memcmp(type_str, "RelaxedGorillax", 15) == 0) return 2;
    return 0;  // Lax
}

// taida_int_div and taida_int_mod removed — use Div/Mod molds
// Div[x, y]() and Mod[x, y]() return Lax BuchiPack
taida_val taida_div_mold(taida_val a, taida_val b) {
    if (b == 0) return taida_lax_empty(0);
    return taida_lax_new(a / b, 0);
}
taida_val taida_mod_mold(taida_val a, taida_val b) {
    if (b == 0) return taida_lax_empty(0);
    return taida_lax_new(a % b, 0);
}

// ── Type conversion molds (Str/Int/Float/Bool) ──────────────
// Each returns a Lax BuchiPack. Str default="", Int default=0, Float default=0.0, Bool default=false(0).

// Str[x]() — always succeeds
taida_val taida_str_mold_int(taida_val v) {
    return taida_lax_new(taida_str_from_int(v), (taida_val)"");
}
taida_val taida_str_mold_float(double v) {
    return taida_lax_new(taida_str_from_float(v), (taida_val)"");
}
taida_val taida_str_mold_bool(taida_val v) {
    return taida_lax_new(taida_str_from_bool(v), (taida_val)"");
}
taida_val taida_str_mold_str(taida_val v) {
    return taida_lax_new(v, (taida_val)"");
}

// Int[x]() — Str parse can fail
taida_val taida_int_mold_int(taida_val v) {
    return taida_lax_new(v, 0);
}
taida_val taida_int_mold_float(double v) {
    return taida_lax_new((taida_val)v, 0);
}
taida_val taida_int_mold_str(taida_val v) {
    const char *s = (const char *)v;
    if (!s || *s == '\0') return taida_lax_empty(0);
    // Reject leading whitespace to match Interpreter parity (Rust parse::<i64>)
    if (s[0] == ' ' || s[0] == '\t' || s[0] == '\n' || s[0] == '\r') return taida_lax_empty(0);
    char *end;
    taida_val result = strtol(s, &end, 10);
    if (*end != '\0') return taida_lax_empty(0);  // parse failed
    return taida_lax_new(result, 0);
}
taida_val taida_int_mold_bool(taida_val v) {
    return taida_lax_new(v ? 1 : 0, 0);
}

taida_val taida_int_mold_auto(taida_val v) {
    if (v == 0) return taida_int_mold_int(0);
    if (v < 0 || v < 4096) return taida_int_mold_int(v);

    if (taida_ptr_is_readable(v, sizeof(taida_val))) {
        taida_val tag = ((taida_val*)v)[0];
        if (taida_has_magic_header(tag)) {
            return taida_lax_empty(0);
        }
    }

    size_t len = 0;
    if (taida_read_cstr_len_safe((const char*)v, 4096, &len)) {
        return taida_int_mold_str(v);
    }

    return taida_int_mold_int(v);
}

// Float[x]() — Str parse can fail, result stored as bitcast taida_val
taida_val taida_float_mold_int(taida_val v) {
    double d = (double)v;
    return taida_lax_new(_d2l(d), _d2l(0.0));
}
taida_val taida_float_mold_float(double v) {
    return taida_lax_new(_d2l(v), _d2l(0.0));
}
taida_val taida_float_mold_str(taida_val v) {
    const char *s = (const char *)v;
    if (!s || *s == '\0') return taida_lax_empty(_d2l(0.0));
    char *end;
    double result = strtod(s, &end);
    if (*end != '\0') return taida_lax_empty(_d2l(0.0));  // parse failed
    return taida_lax_new(_d2l(result), _d2l(0.0));
}
taida_val taida_float_mold_bool(taida_val v) {
    return taida_lax_new(_d2l(v ? 1.0 : 0.0), _d2l(0.0));
}

// Bool[x]() — Str accepts only "true"/"false"
taida_val taida_bool_mold_int(taida_val v) {
    return taida_lax_new(v != 0 ? 1 : 0, 0);
}
taida_val taida_bool_mold_float(double v) {
    return taida_lax_new(v != 0.0 ? 1 : 0, 0);
}
taida_val taida_bool_mold_str(taida_val v) {
    const char *s = (const char *)v;
    if (!s) return taida_lax_empty(0);
    if (strcmp(s, "true") == 0) return taida_lax_new(1, 0);
    if (strcmp(s, "false") == 0) return taida_lax_new(0, 0);
    return taida_lax_empty(0);  // not "true" or "false"
}
taida_val taida_bool_mold_bool(taida_val v) {
    return taida_lax_new(v, 0);
}

static int taida_char_to_digit(int c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'z') return c - 'a' + 10;
    if (c >= 'A' && c <= 'Z') return c - 'A' + 10;
    return -1;
}

taida_val taida_int_mold_str_base(taida_val v, taida_val base) {
    if (base < 2 || base > 36) return taida_lax_empty(0);
    const char *s = (const char*)v;
    size_t len = 0;
    if (!taida_read_cstr_len_safe(s, 4096, &len) || len == 0) return taida_lax_empty(0);

    int negative = 0;
    size_t i = 0;
    if (s[0] == '-') {
        negative = 1;
        i = 1;
        if (len == 1) return taida_lax_empty(0);
    } else if (s[0] == '+') {
        i = 1;
        if (len == 1) return taida_lax_empty(0);
    }

    uint64_t acc = 0;
    uint64_t limit = negative ? ((uint64_t)INT64_MAX + 1ULL) : (uint64_t)INT64_MAX;
    for (; i < len; i++) {
        int d = taida_char_to_digit((unsigned char)s[i]);
        if (d < 0 || d >= base) return taida_lax_empty(0);
        if (acc > (limit - (uint64_t)d) / (uint64_t)base) return taida_lax_empty(0);
        acc = acc * (uint64_t)base + (uint64_t)d;
    }

    int64_t out = 0;
    if (negative) {
        if (acc == ((uint64_t)INT64_MAX + 1ULL)) out = INT64_MIN;
        else out = -(int64_t)acc;
    } else {
        out = (int64_t)acc;
    }
    return taida_lax_new((taida_val)out, 0);
}

taida_val taida_uint8_mold(taida_val v) {
    taida_val parsed = v;
    const char *s = (const char*)v;
    size_t len = 0;
    if (taida_read_cstr_len_safe(s, 256, &len) && len > 0) {
        char *end = NULL;
        taida_val tmp = strtol(s, &end, 10);
        if (end && *end == '\0') parsed = tmp;
    }
    if (parsed < 0 || parsed > 255) return taida_lax_empty(0);
    return taida_lax_new(parsed, 0);
}

taida_val taida_uint8_mold_float(double v) {
    if (!isfinite(v)) return taida_lax_empty(0);
    if (v < 0.0 || v > 255.0) return taida_lax_empty(0);
    if (floor(v) != v) return taida_lax_empty(0);
    return taida_lax_new((taida_val)v, 0);
}

taida_val taida_u16be_mold(taida_val value) {
    if (value < 0 || value > 65535) return taida_lax_empty(taida_bytes_default_value());
    unsigned char raw[2];
    uint16_t n = (uint16_t)value;
    raw[0] = (unsigned char)((n >> 8) & 0xFF);
    raw[1] = (unsigned char)(n & 0xFF);
    taida_val out = taida_bytes_from_raw(raw, 2);
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_u16le_mold(taida_val value) {
    if (value < 0 || value > 65535) return taida_lax_empty(taida_bytes_default_value());
    unsigned char raw[2];
    uint16_t n = (uint16_t)value;
    raw[0] = (unsigned char)(n & 0xFF);
    raw[1] = (unsigned char)((n >> 8) & 0xFF);
    taida_val out = taida_bytes_from_raw(raw, 2);
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_u32be_mold(taida_val value) {
    if (value < 0 || (uint64_t)value > 0xFFFFFFFFULL) {
        return taida_lax_empty(taida_bytes_default_value());
    }
    unsigned char raw[4];
    uint32_t n = (uint32_t)(uint64_t)value;
    raw[0] = (unsigned char)((n >> 24) & 0xFF);
    raw[1] = (unsigned char)((n >> 16) & 0xFF);
    raw[2] = (unsigned char)((n >> 8) & 0xFF);
    raw[3] = (unsigned char)(n & 0xFF);
    taida_val out = taida_bytes_from_raw(raw, 4);
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_u32le_mold(taida_val value) {
    if (value < 0 || (uint64_t)value > 0xFFFFFFFFULL) {
        return taida_lax_empty(taida_bytes_default_value());
    }
    unsigned char raw[4];
    uint32_t n = (uint32_t)(uint64_t)value;
    raw[0] = (unsigned char)(n & 0xFF);
    raw[1] = (unsigned char)((n >> 8) & 0xFF);
    raw[2] = (unsigned char)((n >> 16) & 0xFF);
    raw[3] = (unsigned char)((n >> 24) & 0xFF);
    taida_val out = taida_bytes_from_raw(raw, 4);
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_u16be_decode_mold(taida_val value) {
    if (!TAIDA_IS_BYTES(value)) return taida_lax_empty(0);
    taida_val *bytes = (taida_val*)value;
    if (bytes[1] != 2) return taida_lax_empty(0);
    uint16_t out = (uint16_t)(((uint16_t)bytes[2] << 8) | (uint16_t)bytes[3]);
    return taida_lax_new((taida_val)out, 0);
}

taida_val taida_u16le_decode_mold(taida_val value) {
    if (!TAIDA_IS_BYTES(value)) return taida_lax_empty(0);
    taida_val *bytes = (taida_val*)value;
    if (bytes[1] != 2) return taida_lax_empty(0);
    uint16_t out = (uint16_t)(((uint16_t)bytes[3] << 8) | (uint16_t)bytes[2]);
    return taida_lax_new((taida_val)out, 0);
}

taida_val taida_u32be_decode_mold(taida_val value) {
    if (!TAIDA_IS_BYTES(value)) return taida_lax_empty(0);
    taida_val *bytes = (taida_val*)value;
    if (bytes[1] != 4) return taida_lax_empty(0);
    uint32_t out =
        ((uint32_t)bytes[2] << 24) |
        ((uint32_t)bytes[3] << 16) |
        ((uint32_t)bytes[4] << 8) |
        (uint32_t)bytes[5];
    return taida_lax_new((taida_val)(uint64_t)out, 0);
}

taida_val taida_u32le_decode_mold(taida_val value) {
    if (!TAIDA_IS_BYTES(value)) return taida_lax_empty(0);
    taida_val *bytes = (taida_val*)value;
    if (bytes[1] != 4) return taida_lax_empty(0);
    uint32_t out =
        ((uint32_t)bytes[5] << 24) |
        ((uint32_t)bytes[4] << 16) |
        ((uint32_t)bytes[3] << 8) |
        (uint32_t)bytes[2];
    return taida_lax_new((taida_val)(uint64_t)out, 0);
}

static int taida_utf8_decode_one(
    const unsigned char *buf,
    size_t len,
    size_t *consumed,
    uint32_t *out_cp
) {
    if (len == 0) return 0;
    unsigned char b0 = buf[0];
    if (b0 < 0x80) {
        *consumed = 1;
        *out_cp = (uint32_t)b0;
        return 1;
    }

    if (b0 >= 0xC2 && b0 <= 0xDF) {
        if (len < 2) return 0;
        unsigned char b1 = buf[1];
        if ((b1 & 0xC0) != 0x80) return 0;
        *consumed = 2;
        *out_cp = ((uint32_t)(b0 & 0x1F) << 6) | (uint32_t)(b1 & 0x3F);
        return 1;
    }

    if (b0 >= 0xE0 && b0 <= 0xEF) {
        if (len < 3) return 0;
        unsigned char b1 = buf[1];
        unsigned char b2 = buf[2];
        if ((b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80) return 0;
        if (b0 == 0xE0 && b1 < 0xA0) return 0; // overlong
        if (b0 == 0xED && b1 >= 0xA0) return 0; // surrogate
        uint32_t cp = ((uint32_t)(b0 & 0x0F) << 12)
            | ((uint32_t)(b1 & 0x3F) << 6)
            | (uint32_t)(b2 & 0x3F);
        if (cp >= 0xD800 && cp <= 0xDFFF) return 0;
        *consumed = 3;
        *out_cp = cp;
        return 1;
    }

    if (b0 >= 0xF0 && b0 <= 0xF4) {
        if (len < 4) return 0;
        unsigned char b1 = buf[1];
        unsigned char b2 = buf[2];
        unsigned char b3 = buf[3];
        if ((b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80 || (b3 & 0xC0) != 0x80) return 0;
        if (b0 == 0xF0 && b1 < 0x90) return 0; // overlong
        if (b0 == 0xF4 && b1 > 0x8F) return 0; // > U+10FFFF
        uint32_t cp = ((uint32_t)(b0 & 0x07) << 18)
            | ((uint32_t)(b1 & 0x3F) << 12)
            | ((uint32_t)(b2 & 0x3F) << 6)
            | (uint32_t)(b3 & 0x3F);
        if (cp > 0x10FFFF) return 0;
        *consumed = 4;
        *out_cp = cp;
        return 1;
    }

    return 0;
}

static int taida_utf8_encode_scalar(uint32_t cp, unsigned char out[4], size_t *out_len) {
    if (cp <= 0x7F) {
        out[0] = (unsigned char)cp;
        *out_len = 1;
        return 1;
    }
    if (cp <= 0x7FF) {
        out[0] = (unsigned char)(0xC0 | (cp >> 6));
        out[1] = (unsigned char)(0x80 | (cp & 0x3F));
        *out_len = 2;
        return 1;
    }
    if (cp >= 0xD800 && cp <= 0xDFFF) return 0;
    if (cp <= 0xFFFF) {
        out[0] = (unsigned char)(0xE0 | (cp >> 12));
        out[1] = (unsigned char)(0x80 | ((cp >> 6) & 0x3F));
        out[2] = (unsigned char)(0x80 | (cp & 0x3F));
        *out_len = 3;
        return 1;
    }
    if (cp <= 0x10FFFF) {
        out[0] = (unsigned char)(0xF0 | (cp >> 18));
        out[1] = (unsigned char)(0x80 | ((cp >> 12) & 0x3F));
        out[2] = (unsigned char)(0x80 | ((cp >> 6) & 0x3F));
        out[3] = (unsigned char)(0x80 | (cp & 0x3F));
        *out_len = 4;
        return 1;
    }
    return 0;
}

static int taida_utf8_single_scalar(const unsigned char *buf, size_t len, uint32_t *cp_out) {
    size_t consumed = 0;
    uint32_t cp = 0;
    if (!taida_utf8_decode_one(buf, len, &consumed, &cp)) return 0;
    if (consumed != len) return 0;
    *cp_out = cp;
    return 1;
}

taida_val taida_char_mold_int(taida_val value) {
    if (value < 0 || value > 0x10FFFF) return taida_lax_empty((taida_val)"");
    if (value >= 0xD800 && value <= 0xDFFF) return taida_lax_empty((taida_val)"");
    unsigned char utf8[4];
    size_t out_len = 0;
    if (!taida_utf8_encode_scalar((uint32_t)value, utf8, &out_len)) {
        return taida_lax_empty((taida_val)"");
    }
    char *out = taida_str_alloc(out_len);
    memcpy(out, utf8, out_len);
    return taida_lax_new((taida_val)out, (taida_val)"");
}

taida_val taida_char_mold_str(taida_val value) {
    const char *s = (const char*)value;
    size_t len = 0;
    if (!taida_read_cstr_len_safe(s, 4096, &len) || len == 0) {
        return taida_lax_empty((taida_val)"");
    }
    uint32_t cp = 0;
    if (!taida_utf8_single_scalar((const unsigned char*)s, len, &cp)) {
        return taida_lax_empty((taida_val)"");
    }
    return taida_char_mold_int((taida_val)cp);
}

taida_val taida_codepoint_mold_str(taida_val value) {
    const char *s = (const char*)value;
    size_t len = 0;
    if (!taida_read_cstr_len_safe(s, 4096, &len) || len == 0) {
        return taida_lax_empty(0);
    }
    uint32_t cp = 0;
    if (!taida_utf8_single_scalar((const unsigned char*)s, len, &cp)) {
        return taida_lax_empty(0);
    }
    return taida_lax_new((taida_val)cp, 0);
}

taida_val taida_bytes_mold(taida_val value, taida_val fill) {
    if (TAIDA_IS_BYTES(value)) {
        taida_val cloned = taida_bytes_clone(value);
        return taida_lax_new(cloned, taida_bytes_default_value());
    }

    if (TAIDA_IS_LIST(value)) {
        taida_val *list = (taida_val*)value;
        taida_val len = list[2];
        taida_val out = taida_bytes_new_filled(len, 0);
        taida_val *bytes = (taida_val*)out;
        for (taida_val i = 0; i < len; i++) {
            taida_val item = list[4 + i];
            if (item < 0 || item > 255) {
                return taida_lax_empty(taida_bytes_default_value());
            }
            bytes[2 + i] = item;
        }
        return taida_lax_new(out, taida_bytes_default_value());
    }

    const char *s = (const char*)value;
    size_t slen = 0;
    if (taida_read_cstr_len_safe(s, 65536, &slen)) {
        taida_val out = taida_bytes_from_raw((const unsigned char*)s, (taida_val)slen);
        return taida_lax_new(out, taida_bytes_default_value());
    }

    taida_val len = value;
    if (len < 0 || len > 10000000) return taida_lax_empty(taida_bytes_default_value());
    if (fill < 0 || fill > 255) return taida_lax_empty(taida_bytes_default_value());
    taida_val out = taida_bytes_new_filled(len, (unsigned char)fill);
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_bytes_set(taida_val bytes_ptr, taida_val idx, taida_val value) {
    if (!TAIDA_IS_BYTES(bytes_ptr)) return taida_lax_empty(taida_bytes_default_value());
    taida_val len = taida_bytes_len(bytes_ptr);
    if (idx < 0 || idx >= len) return taida_lax_empty(taida_bytes_default_value());
    if (value < 0 || value > 255) return taida_lax_empty(taida_bytes_default_value());
    taida_val out = taida_bytes_clone(bytes_ptr);
    taida_val *bytes = (taida_val*)out;
    bytes[2 + idx] = value;
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_bytes_to_list(taida_val bytes_ptr) {
    taida_val list = taida_list_new();
    if (!TAIDA_IS_BYTES(bytes_ptr)) return list;
    taida_val *bytes = (taida_val*)bytes_ptr;
    taida_val len = bytes[1];
    for (taida_val i = 0; i < len; i++) {
        list = taida_list_push(list, bytes[2 + i]);
    }
    return list;
}

static int taida_bytes_cursor_unpack(taida_val cursor_ptr, taida_val *bytes_out, taida_val *offset_out) {
    if (!TAIDA_IS_PACK(cursor_ptr)) return 0;
    taida_val *pack = (taida_val*)cursor_ptr;
    taida_val field_count = pack[1];
    if (field_count < 2) return 0;
    // A-4b: stride-3 layout: [magic+rc, fc, hash0, tag0, val0, hash1, tag1, val1, ...]
    taida_val bytes_ptr = pack[2 + 0 * 3 + 2];  // field 0 value
    taida_val offset = pack[2 + 1 * 3 + 2];      // field 1 value
    if (!TAIDA_IS_BYTES(bytes_ptr)) return 0;
    taida_val len = taida_bytes_len(bytes_ptr);
    if (offset < 0) offset = 0;
    if (offset > len) offset = len;
    *bytes_out = bytes_ptr;
    *offset_out = offset;
    return 1;
}

static taida_val taida_bytes_cursor_step(taida_val value, taida_val cursor) {
    taida_val pack = taida_pack_new(2);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_STEP_VALUE);
    taida_pack_set(pack, 0, value);
    taida_pack_set_hash(pack, 1, (taida_val)HASH_STEP_CURSOR);
    taida_pack_set(pack, 1, cursor);
    return pack;
}

taida_val taida_bytes_cursor_new(taida_val bytes_ptr, taida_val offset) {
    if (!TAIDA_IS_BYTES(bytes_ptr)) {
        bytes_ptr = taida_bytes_default_value();
    }
    taida_val len = taida_bytes_len(bytes_ptr);
    if (offset < 0) offset = 0;
    if (offset > len) offset = len;

    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_CURSOR_BYTES);
    taida_pack_set(pack, 0, bytes_ptr);
    taida_pack_set_hash(pack, 1, (taida_val)HASH_CURSOR_OFFSET);
    taida_pack_set(pack, 1, offset);
    taida_pack_set_hash(pack, 2, (taida_val)HASH_CURSOR_LENGTH);
    taida_pack_set(pack, 2, len);
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__bytes_cursor_type_str);
    return pack;
}

taida_val taida_bytes_cursor_remaining(taida_val cursor_ptr) {
    taida_val bytes_ptr = 0;
    taida_val offset = 0;
    if (!taida_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) return 0;
    return taida_bytes_len(bytes_ptr) - offset;
}

taida_val taida_bytes_cursor_take(taida_val cursor_ptr, taida_val size) {
    taida_val bytes_ptr = 0;
    taida_val offset = 0;
    if (!taida_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) {
        taida_val empty_cursor = taida_bytes_cursor_new(taida_bytes_default_value(), 0);
        taida_val def = taida_bytes_cursor_step(taida_bytes_default_value(), empty_cursor);
        return taida_lax_empty(def);
    }

    taida_val current_cursor = taida_bytes_cursor_new(bytes_ptr, offset);
    taida_val default_step = taida_bytes_cursor_step(taida_bytes_default_value(), current_cursor);
    if (size < 0) return taida_lax_empty(default_step);

    taida_val len = taida_bytes_len(bytes_ptr);
    if (offset + size > len) return taida_lax_empty(default_step);

    taida_val *src = (taida_val*)bytes_ptr;
    taida_val out = taida_bytes_new_filled(size, 0);
    taida_val *dst = (taida_val*)out;
    for (taida_val i = 0; i < size; i++) {
        dst[2 + i] = src[2 + offset + i];
    }
    taida_val next_cursor = taida_bytes_cursor_new(bytes_ptr, offset + size);
    taida_val step = taida_bytes_cursor_step(out, next_cursor);
    return taida_lax_new(step, default_step);
}

taida_val taida_bytes_cursor_u8(taida_val cursor_ptr) {
    taida_val bytes_ptr = 0;
    taida_val offset = 0;
    if (!taida_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) {
        taida_val empty_cursor = taida_bytes_cursor_new(taida_bytes_default_value(), 0);
        taida_val def = taida_bytes_cursor_step(0, empty_cursor);
        return taida_lax_empty(def);
    }

    taida_val current_cursor = taida_bytes_cursor_new(bytes_ptr, offset);
    taida_val default_step = taida_bytes_cursor_step(0, current_cursor);
    taida_val len = taida_bytes_len(bytes_ptr);
    if (offset >= len) return taida_lax_empty(default_step);

    taida_val *bytes = (taida_val*)bytes_ptr;
    taida_val value = bytes[2 + offset];
    taida_val next_cursor = taida_bytes_cursor_new(bytes_ptr, offset + 1);
    taida_val step = taida_bytes_cursor_step(value, next_cursor);
    return taida_lax_new(step, default_step);
}

taida_val taida_utf8_encode_mold(taida_val value) {
    const char *s = (const char*)value;
    size_t len = 0;
    if (!taida_read_cstr_len_safe(s, 65536, &len)) {
        return taida_lax_empty(taida_bytes_default_value());
    }
    taida_val out = taida_bytes_from_raw((const unsigned char*)s, (taida_val)len);
    return taida_lax_new(out, taida_bytes_default_value());
}

taida_val taida_utf8_decode_mold(taida_val value) {
    if (!TAIDA_IS_BYTES(value)) return taida_lax_empty((taida_val)"");
    taida_val *bytes = (taida_val*)value;
    taida_val len = bytes[1];
    // R-01: Guard against negative length — a corrupted Bytes header could
    // pass a negative len, which would be cast to a huge size_t and trigger
    // a massive malloc followed by OOM abort.
    if (len <= 0) return taida_lax_new((taida_val)taida_str_new_copy(""), (taida_val)"");
    unsigned char *raw = (unsigned char*)TAIDA_MALLOC((size_t)len, "bytes_decode");
    for (taida_val i = 0; i < len; i++) raw[i] = (unsigned char)bytes[2 + i];

    size_t pos = 0;
    while (pos < (size_t)len) {
        size_t consumed = 0;
        uint32_t cp = 0;
        if (!taida_utf8_decode_one(raw + pos, (size_t)len - pos, &consumed, &cp)) {
            free(raw);
            return taida_lax_empty((taida_val)"");
        }
        pos += consumed;
    }

    char *out = taida_str_alloc((size_t)len);
    memcpy(out, raw, (size_t)len);
    free(raw);
    return taida_lax_new((taida_val)out, (taida_val)"");
}

taida_val taida_int_neg(taida_val a) { return -a; }
double taida_float_neg(double a) { return -a; }

// Comparison runtime
taida_val taida_int_eq(taida_val a, taida_val b)  { return a == b ? 1 : 0; }
taida_val taida_int_neq(taida_val a, taida_val b) { return a != b ? 1 : 0; }
taida_val taida_str_eq(taida_val a, taida_val b)  { return (a && b) ? (strcmp((char*)a, (char*)b) == 0 ? 1 : 0) : (a == b ? 1 : 0); }
taida_val taida_str_neq(taida_val a, taida_val b) { return (a && b) ? (strcmp((char*)a, (char*)b) != 0 ? 1 : 0) : (a != b ? 1 : 0); }
static int taida_is_string_value(taida_val v) {
    if (v == 0 || v == 1 || v < 4096) return 0;
    // Check if 8-byte aligned (heap object) and has magic header
    if ((v & 0x7) == 0) {
        unsigned char vec = 0;
        uintptr_t page = (uintptr_t)v & ~((uintptr_t)4095);
        if (mincore((void*)page, 4096, &vec) == 0) {
            taida_val first = ((taida_val*)v)[0];
            if (taida_has_magic_header(first)) return 0;
        }
    }
    // If not a heap object with magic header, try reading as string
    // Use mincore to check page is mapped for the pointer itself
    unsigned char vec = 0;
    uintptr_t page = (uintptr_t)v & ~((uintptr_t)4095);
    if (mincore((void*)page, 4096, &vec) != 0) return 0;
    return 1;
}
taida_val taida_poly_eq(taida_val a, taida_val b) {
    if (taida_is_string_value(a) && taida_is_string_value(b))
        return strcmp((char*)a, (char*)b) == 0 ? 1 : 0;
    return a == b ? 1 : 0;
}
taida_val taida_poly_neq(taida_val a, taida_val b) {
    if (taida_is_string_value(a) && taida_is_string_value(b))
        return strcmp((char*)a, (char*)b) != 0 ? 1 : 0;
    return a != b ? 1 : 0;
}
// NTH-1: Helper — format a taida_val as a string for poly_add concatenation.
// If the value is a heap string, return it directly.
// If it looks like a bitcast float (outside small-integer range), format as %g.
// Otherwise format as integer.
static const char *_poly_val_to_str(taida_val v, char *buf, int bufsz) {
    if (taida_is_string_value(v)) return (const char *)v;
    // Heuristic: values outside [-1048576, 1048576] that are not valid pointers
    // are likely bitcast floats. This matches _to_double() in the float section.
    if (v < -1048576 || v > 1048576) {
        // Additional guard: skip if the value looks like a heap pointer
        if (!taida_is_string_value(v) && !taida_ptr_is_readable(v, 8)) {
            union { taida_val l; double d; } u;
            u.l = v;
            // Sanity: only treat as float if the double is finite
            if (u.d == u.d && u.d != (1.0/0.0) && u.d != -(1.0/0.0)) {
                snprintf(buf, bufsz, "%g", u.d);
                return buf;
            }
        }
    }
    snprintf(buf, bufsz, "%lld", (long long)v);
    return buf;
}

// FL-16: Polymorphic add — dispatches to str_concat or int_add at runtime
taida_val taida_poly_add(taida_val a, taida_val b) {
    if (taida_is_string_value(a) || taida_is_string_value(b)) {
        // Convert both to strings and concatenate
        char buf_a[64], buf_b[64];
        const char *sa = _poly_val_to_str(a, buf_a, sizeof(buf_a));
        const char *sb = _poly_val_to_str(b, buf_b, sizeof(buf_b));
        return taida_str_concat(sa, sb);
    }
    return a + b;
}
taida_val taida_int_lt(taida_val a, taida_val b)  { return a < b ? 1 : 0; }
taida_val taida_int_gt(taida_val a, taida_val b)  { return a > b ? 1 : 0; }
taida_val taida_int_gte(taida_val a, taida_val b) { return a >= b ? 1 : 0; }

// Boolean runtime
taida_val taida_bool_and(taida_val a, taida_val b) { return (a && b) ? 1 : 0; }
taida_val taida_bool_or(taida_val a, taida_val b)  { return (a || b) ? 1 : 0; }
taida_val taida_bool_not(taida_val a)         { return a ? 0 : 1; }

// ── Reference Counting ────────────────────────────────────
// All heap objects (Pack, List, Closure) start with refcount at [0].
// taida_retain increments refcount.
// taida_release decrements refcount and frees when it reaches 0.

static int taida_has_magic_header(taida_val tag) {
    taida_val magic = tag & TAIDA_MAGIC_MASK;
    return magic == TAIDA_LIST_MAGIC ||
           magic == TAIDA_PACK_MAGIC ||
           magic == TAIDA_HMAP_MAGIC ||
           magic == TAIDA_SET_MAGIC ||
           magic == TAIDA_ASYNC_MAGIC ||
           magic == TAIDA_CLOSURE_MAGIC ||
           magic == TAIDA_BYTES_MAGIC;
}

taida_val taida_retain(taida_ptr ptr) {
    if (ptr == 0) return 0;
    // Non-heap values are never retained.
    if (ptr > 0 && ptr < 4096) return ptr;
    if (ptr < 0) return ptr;
    if (!taida_ptr_is_readable(ptr, sizeof(taida_val))) return ptr;

    taida_val *obj = (taida_val*)ptr;
    taida_val tag = obj[0];
    if (taida_has_magic_header(tag)) {
        TAIDA_INC_RC(ptr);
        return ptr;
    }

    return ptr;
}

taida_val taida_release(taida_ptr ptr) {
    if (ptr == 0) return 0;
    // Skip non-heap values (small integers, negative values)
    if (ptr > 0 && ptr < 4096) return 0;
    if (ptr < 0) return 0;
    if (!taida_ptr_is_readable(ptr, sizeof(taida_val))) return 0;

    taida_val *obj = (taida_val*)ptr;
    taida_val tag = obj[0];
    if (taida_has_magic_header(tag)) {
        taida_val rc = TAIDA_GET_RC(ptr);
        if (rc <= 1) {
            // A-4d: Recursive release for Pack children before freeing.
            // Uses type tags (A-4a/b) to safely identify child heap objects.
            //
            // SAFETY: We only recursively release children that are KNOWN to be
            // exclusively owned by this pack. In the current Taida runtime,
            // values are immutable and sharing is pervasive, so we can only
            // safely release children in specific ownership patterns:
            //
            // 1. Closure env packs — always exclusively owned by the closure
            // 2. Pack fields tagged as PACK/LIST/CLOSURE — released recursively
            //    only if the child's own refcount allows it (the child's
            //    taida_release will check its own refcount and only free if <= 1)
            //
            // String fields (TAIDA_TAG_STR) are released via taida_str_release
            // which safely distinguishes heap strings (hidden header) from
            // static literals (no-op).
            // Recursive child release using type tags.
            // retain-on-store ensures each child stored in a Pack/List field
            // has been retained, so releasing here is balanced.
            //
            // Pack: iterate fields with stride=3 [hash, tag, value].
            // Release children tagged as PACK(4)/LIST(5)/CLOSURE(6)/STR(3).
            // STR fields use taida_str_release which is safe for both
            // heap strings (hidden header) and static literals (no-op).
            if (TAIDA_IS_PACK(ptr)) {
                taida_val count = obj[1];
                for (taida_val i = 0; i < count; i++) {
                    taida_val field_tag = obj[2 + i * 3 + 1];
                    if (field_tag == TAIDA_TAG_PACK || field_tag == TAIDA_TAG_LIST || field_tag == TAIDA_TAG_CLOSURE
                        || field_tag == TAIDA_TAG_HMAP || field_tag == TAIDA_TAG_SET) {
                        taida_val child = obj[2 + i * 3 + 2];
                        if (child > 4096) {
                            taida_release(child);
                        }
                    } else if (field_tag == TAIDA_TAG_STR) {
                        taida_val child = obj[2 + i * 3 + 2];
                        if (child > 4096) {
                            taida_str_release(child);
                        }
                    }
                }
            }
            // List elements: recursively released using elem_type_tag.
            // elem_type_tag (list[3]) is set by the checker-guaranteed
            // homogeneous type system (Step 1+2). Elements that were
            // copied from other lists have been retained (Step 3), so
            // release here is balanced.
            if (TAIDA_IS_LIST(ptr)) {
                taida_val *lobj = (taida_val*)ptr;
                taida_val llen = lobj[2];
                taida_val etag = lobj[3];
                for (taida_val i = 0; i < llen; i++) {
                    taida_list_elem_release(lobj[4 + i], etag);
                }
            }
            // Closure: release env pack (exclusively owned)
            if (TAIDA_IS_CLOSURE(ptr)) {
                taida_val env_ptr = obj[2];
                if (env_ptr > 4096) {
                    taida_release(env_ptr);
                }
            }
            // NO-2: Set — recursively release all elements using elem_type_tag
            // Same pattern as List recursive release (offset 3 = elem_type_tag).
            if (TAIDA_IS_SET(ptr)) {
                taida_val *sobj = (taida_val*)ptr;
                taida_val slen = sobj[2];
                taida_val etag = sobj[3];  // elem_type_tag
                for (taida_val i = 0; i < slen; i++) {
                    taida_list_elem_release(sobj[4 + i], etag);
                }
            }
            // NO-1: HashMap — recursively release all keys and values
            if (TAIDA_IS_HMAP(ptr)) {
                taida_val *hobj = (taida_val*)ptr;
                taida_val hcap = hobj[1];
                taida_val vtag = hobj[3];  // value_type_tag
                for (taida_val i = 0; i < hcap; i++) {
                    taida_val sh = hobj[HM_HEADER + i * 3];
                    taida_val sk = hobj[HM_HEADER + i * 3 + 1];
                    if (HM_SLOT_OCCUPIED(sh, sk)) {
                        taida_hashmap_key_release(sk);
                        taida_hashmap_val_release(hobj[HM_HEADER + i * 3 + 2], vtag);
                    }
                }
            }
            // NO-3: Async — recursively release value and error based on type tags
            if (TAIDA_IS_ASYNC(ptr)) {
                taida_val *aobj = (taida_val*)ptr;
                taida_val status = aobj[1];
                taida_val vtag = aobj[5];  // value_tag
                taida_val etag = aobj[6];  // error_tag
                if (status == 1) {
                    // fulfilled: release value
                    taida_list_elem_release(aobj[2], vtag);
                } else if (status == 2) {
                    // rejected: release error
                    taida_list_elem_release(aobj[3], etag);
                }
                // pending (status == 0): no value/error to release
                // thread_handle: if still pending with a live thread, we
                // must join before freeing (the thread writes into aobj).
                if (status == 0 && aobj[4] != 0) {
                    pthread_join((pthread_t)aobj[4], NULL);
                    aobj[4] = 0;
                    // After join, thread may have written value/error.
                    // Re-check and release.
                    taida_val new_status = aobj[1];
                    if (new_status == 1) {
                        taida_list_elem_release(aobj[2], aobj[5]);
                    } else if (new_status == 2) {
                        taida_list_elem_release(aobj[3], aobj[6]);
                    }
                }
            }
            free(obj);
            return 0;
        }
        TAIDA_DEC_RC(ptr);
        return 0;
    }

    return 0;
}

// ── Heap String helpers (A-4k) ────────────────────────────
// Hidden header layout: [magic+rc (8 bytes), len (8 bytes)] + [bytes...\0]
// The returned char* points to the bytes area (header + 16).
// Static strings (string literals, ConstStr) have no header and are
// identified by the absence of TAIDA_STR_MAGIC at ptr - 16.

// Allocate a heap string with room for `len` characters (+ \0).
// Returns pointer to the bytes area.  Caller must fill the bytes.
static char* taida_str_alloc(size_t len) {
    // M-09: Guard against size_t overflow in header+len+NUL calculation.
    // sizeof(taida_val)*2 = 16 on LP64, so len > SIZE_MAX - 17 would wrap.
    if (len > SIZE_MAX - (sizeof(taida_val) * 2 + 1)) {
        fprintf(stderr, "taida: string length overflow in taida_str_alloc: %zu\n", len);
        exit(1);
    }
    taida_val *hdr = (taida_val*)TAIDA_MALLOC(sizeof(taida_val) * 2 + len + 1, "str_alloc");
    hdr[0] = TAIDA_STR_MAGIC | 1;  // magic + refcount = 1
    hdr[1] = (taida_val)len;
    char *bytes = (char*)(hdr + 2);
    bytes[len] = '\0';
    return bytes;
}

// Copy a C string into a new heap string with hidden header.
static char* taida_str_new_copy(const char* src) {
    if (!src) {
        char *r = taida_str_alloc(0);
        r[0] = '\0';
        return r;
    }
    size_t len = strlen(src);
    char *r = taida_str_alloc(len);
    memcpy(r, src, len);
    return r;
}

// Retain a heap string (RC++).  No-op for static strings.
void taida_str_retain(taida_val ptr) {
    if (ptr == 0) return;
    if (ptr > 0 && ptr < 4096) return;
    if (ptr < 0) return;
    // Check hidden header at ptr - 16
    taida_val *hdr = ((taida_val*)ptr) - 2;
    if (!taida_ptr_is_readable((taida_val)hdr, sizeof(taida_val))) return;
    taida_val tag = hdr[0];
    if ((tag & TAIDA_MAGIC_MASK) == TAIDA_STR_MAGIC) {
        taida_val rc = tag & TAIDA_RC_MASK;
        if (rc < 255) {
            hdr[0] = (tag & TAIDA_MAGIC_MASK) | (rc + 1);
        }
    }
    // Static strings: no header → no-op
}

// Release a heap string (RC--).  Frees when RC reaches 0.
// No-op for static strings (no hidden header).
static void taida_str_release(taida_val ptr) {
    if (ptr == 0) return;
    if (ptr > 0 && ptr < 4096) return;
    if (ptr < 0) return;
    // Check hidden header at ptr - 16
    taida_val *hdr = ((taida_val*)ptr) - 2;
    if (!taida_ptr_is_readable((taida_val)hdr, sizeof(taida_val))) return;
    taida_val tag = hdr[0];
    if ((tag & TAIDA_MAGIC_MASK) == TAIDA_STR_MAGIC) {
        taida_val rc = tag & TAIDA_RC_MASK;
        if (rc <= 1) {
            free(hdr);  // Free the entire allocation (header + bytes)
        } else {
            hdr[0] = (tag & TAIDA_MAGIC_MASK) | (rc - 1);
        }
    }
    // Static strings: no header → no-op
}

// ── BuchiPack runtime ─────────────────────────────────────
// Pack layout (A-4b): [magic+rc, field_count, hash0, tag0, val0, hash1, tag1, val1, ...]
// Stride = 3 per field: [hash, type_tag, value]

taida_ptr taida_pack_new(taida_val field_count) {
    // M-01: Guard against negative field_count (taida_val is int64_t) and
    // overflow in the size calculation (2 + field_count * 3) * sizeof(taida_val).
    if (field_count < 0) {
        fprintf(stderr, "taida: invalid field_count %" PRId64 " in taida_pack_new\n", (int64_t)field_count);
        exit(1);
    }
    size_t fc = (size_t)field_count;
    size_t slots = taida_safe_add(2, taida_safe_mul(fc, 3, "pack_new fields"), "pack_new header");
    size_t alloc_size = taida_safe_mul(slots, sizeof(taida_val), "pack_new bytes");
    taida_val *pack = (taida_val*)TAIDA_MALLOC(alloc_size, "pack_new");
    pack[0] = TAIDA_PACK_MAGIC | 1;  // Magic + refcount
    pack[1] = field_count;
    // Initialize tag slots to TAIDA_TAG_INT (0) by default
    for (taida_val i = 0; i < field_count; i++) {
        pack[2 + i * 3] = 0;     // hash
        pack[2 + i * 3 + 1] = 0; // tag = INT (default)
        pack[2 + i * 3 + 2] = 0; // value
    }
    return (taida_ptr)pack;
}

taida_ptr taida_pack_set(taida_ptr pack_ptr, taida_val index, taida_val value) {
    taida_val *pack = (taida_val*)pack_ptr;
    pack[2 + index * 3 + 2] = value;
    return pack_ptr;
}

taida_ptr taida_pack_set_tag(taida_ptr pack_ptr, taida_val index, taida_val tag) {
    taida_val *pack = (taida_val*)pack_ptr;
    pack[2 + index * 3 + 1] = tag;
    return pack_ptr;
}

taida_val taida_pack_get(taida_ptr pack_ptr, taida_val field_hash) {
    taida_val *pack = (taida_val*)pack_ptr;
    taida_val count = pack[1];
    for (taida_val i = 0; i < count; i++) {
        if (pack[2 + i * 3] == field_hash) {
            return pack[2 + i * 3 + 2];
        }
    }
    return 0; // default value
}

// Get the type tag for a field by its hash. Returns TAIDA_TAG_UNKNOWN if not found.
// B11-2b: Made public (non-static) so codegen can emit calls for runtime tag lookup
// when stdout/stderr needs to format FieldAccess values with proper type display.
taida_val taida_pack_get_field_tag(taida_ptr pack_ptr, taida_val field_hash) {
    taida_val *pack = (taida_val*)pack_ptr;
    taida_val count = pack[1];
    for (taida_val i = 0; i < count; i++) {
        if (pack[2 + i * 3] == field_hash) {
            return pack[2 + i * 3 + 1];
        }
    }
    return TAIDA_TAG_UNKNOWN;
}

// Return a human-readable type name for a given type tag.
static const char *taida_tag_name(taida_val tag) {
    switch (tag) {
        case TAIDA_TAG_INT:     return "Int";
        case TAIDA_TAG_FLOAT:   return "Float";
        case TAIDA_TAG_BOOL:    return "Bool";
        case TAIDA_TAG_STR:     return "Str";
        case TAIDA_TAG_PACK:    return "BuchiPack";
        case TAIDA_TAG_LIST:    return "List";
        case TAIDA_TAG_CLOSURE: return "Closure";
        case TAIDA_TAG_HMAP:    return "HashMap";
        case TAIDA_TAG_SET:     return "Set";
        default:                return "unknown";
    }
}

// NB-14/NB-21: Runtime type detection for UNKNOWN-tagged values.
// When the compiler cannot determine the field tag (e.g., dynamic argument passing),
// we inspect the runtime value to determine its actual type.
// Note: Bool and Int are indistinguishable at the value level (both are unboxed scalars).
// Call-site arg tag propagation (taida_set/get_call_arg_tag) ensures that the pack
// field tag carries the correct Bool tag through function parameters.
static taida_val taida_runtime_detect_tag(taida_val value) {
    // Heap types can be distinguished by magic headers
    if (value == 0 || (value > 0 && value < 4096)) return TAIDA_TAG_INT;  // small scalar
    if (taida_is_list(value)) return TAIDA_TAG_LIST;
    if (taida_is_buchi_pack(value)) return TAIDA_TAG_PACK;
    if (TAIDA_IS_BYTES(value)) return TAIDA_TAG_STR;  // Bytes is string-like
    if (TAIDA_IS_CLOSURE(value)) return TAIDA_TAG_CLOSURE;
    if (taida_is_hashmap(value)) return TAIDA_TAG_HMAP;
    if (taida_is_set(value)) return TAIDA_TAG_SET;
    if (taida_is_string_value(value)) return TAIDA_TAG_STR;
    return TAIDA_TAG_INT;  // unboxed scalar fallback
}

// Format a value for error messages (parity with Interpreter/JS).
// Returns 1 if the tag was known, 0 if UNKNOWN (best-effort formatting).
static int taida_format_value(taida_val tag, taida_val value, char *buf, size_t size) {
    switch (tag) {
        case TAIDA_TAG_BOOL:
            snprintf(buf, size, "%s", value ? "true" : "false");
            return 1;
        case TAIDA_TAG_INT:
            snprintf(buf, size, "%lld", (long long)value);
            return 1;
        case TAIDA_TAG_FLOAT: {
            double d;
            memcpy(&d, &value, sizeof(double));
            snprintf(buf, size, "%g", d);
            return 1;
        }
        case TAIDA_TAG_STR: {
            size_t slen = 0;
            if (taida_read_cstr_len_safe((const char*)value, 128, &slen)) {
                snprintf(buf, size, "%s", (const char*)value);
            } else {
                snprintf(buf, size, "Str");
            }
            return 1;
        }
        case TAIDA_TAG_PACK:
            snprintf(buf, size, "BuchiPack");
            return 1;
        case TAIDA_TAG_LIST:
            snprintf(buf, size, "List");
            return 1;
        case TAIDA_TAG_CLOSURE:
            snprintf(buf, size, "Closure");
            return 1;
        default: {
            // UNKNOWN tag: use runtime type detection to format meaningfully
            taida_val detected = taida_runtime_detect_tag(value);
            if (detected != TAIDA_TAG_INT) {
                // Detected a non-scalar type; recurse with the resolved tag
                return taida_format_value(detected, value, buf, size);
            }
            // Unboxed scalar: Bool/Int indistinguishable at runtime, display as Int
            snprintf(buf, size, "%lld", (long long)value);
            return 0;
        }
    }
}

taida_val taida_pack_has_hash(taida_ptr pack_ptr, taida_val field_hash) {
    taida_val *pack = (taida_val*)pack_ptr;
    taida_val count = pack[1];
    for (taida_val i = 0; i < count; i++) {
        if (pack[2 + i * 3] == field_hash) {
            return 1;
        }
    }
    return 0;
}

taida_val taida_pack_get_idx(taida_ptr pack_ptr, taida_val index) {
    taida_val *pack = (taida_val*)pack_ptr;
    return pack[2 + index * 3 + 2];
}

taida_ptr taida_pack_set_hash(taida_ptr pack_ptr, taida_val index, taida_val hash) {
    taida_val *pack = (taida_val*)pack_ptr;
    pack[2 + index * 3] = hash;
    return pack_ptr;
}

// ── Global variable table ─────────────────────────────────
// Simple hash table for top-level variables accessed from functions.
// Key = name_hash (FNV-1a), Value = taida_val (tagged pointer or int).

#define TAIDA_GLOBAL_TABLE_SIZE 256
static taida_val taida_global_keys[TAIDA_GLOBAL_TABLE_SIZE];
static taida_val taida_global_vals[TAIDA_GLOBAL_TABLE_SIZE];
static int taida_global_used[TAIDA_GLOBAL_TABLE_SIZE];

void taida_global_set(taida_val name_hash, taida_val value) {
    unsigned int idx = (unsigned int)((uint64_t)name_hash % TAIDA_GLOBAL_TABLE_SIZE);
    for (int i = 0; i < TAIDA_GLOBAL_TABLE_SIZE; i++) {
        unsigned int slot = (idx + i) % TAIDA_GLOBAL_TABLE_SIZE;
        if (!taida_global_used[slot] || taida_global_keys[slot] == name_hash) {
            taida_global_keys[slot] = name_hash;
            taida_global_vals[slot] = value;
            taida_global_used[slot] = 1;
            return;
        }
    }
}

taida_val taida_global_get(taida_val name_hash) {
    unsigned int idx = (unsigned int)((uint64_t)name_hash % TAIDA_GLOBAL_TABLE_SIZE);
    for (int i = 0; i < TAIDA_GLOBAL_TABLE_SIZE; i++) {
        unsigned int slot = (idx + i) % TAIDA_GLOBAL_TABLE_SIZE;
        if (!taida_global_used[slot]) return 0;
        if (taida_global_keys[slot] == name_hash) return taida_global_vals[slot];
    }
    return 0;
}

// ── Closure runtime ───────────────────────────────────────
// Closure layout: [magic+refcount, fn_ptr, env_ptr]

taida_ptr taida_closure_new(taida_fn_ptr fn_ptr, taida_ptr env_ptr) {
    taida_val *closure = (taida_val*)TAIDA_MALLOC(3 * sizeof(taida_val), "closure_new");
    closure[0] = TAIDA_CLOSURE_MAGIC | 1;
    closure[1] = (taida_val)fn_ptr;
    closure[2] = (taida_val)env_ptr;
    return (taida_ptr)closure;
}

taida_fn_ptr taida_closure_get_fn(taida_ptr closure_ptr) {
    return (taida_fn_ptr)((taida_val*)closure_ptr)[1];
}

taida_ptr taida_closure_get_env(taida_ptr closure_ptr) {
    return (taida_ptr)((taida_val*)closure_ptr)[2];
}

taida_val taida_is_closure_value(taida_val ptr) {
    return TAIDA_IS_CLOSURE(ptr) ? 1 : 0;
}

typedef taida_val (*taida_cb1_fn)(taida_val);
typedef taida_val (*taida_closure_cb1_fn)(taida_val, taida_val);
typedef taida_val (*taida_cb2_fn)(taida_val, taida_val);
typedef taida_val (*taida_closure_cb2_fn)(taida_val, taida_val, taida_val);

static taida_val taida_invoke_callback1(taida_val fn_ptr, taida_val arg0) {
    if (TAIDA_IS_CLOSURE(fn_ptr)) {
        taida_val *closure = (taida_val*)fn_ptr;
        taida_closure_cb1_fn closure_fn = (taida_closure_cb1_fn)closure[1];
        taida_val env_ptr = closure[2];
        return closure_fn(env_ptr, arg0);
    }
    taida_cb1_fn fn = (taida_cb1_fn)fn_ptr;
    return fn(arg0);
}

static taida_val taida_invoke_callback2(taida_val fn_ptr, taida_val arg0, taida_val arg1) {
    if (TAIDA_IS_CLOSURE(fn_ptr)) {
        taida_val *closure = (taida_val*)fn_ptr;
        taida_closure_cb2_fn closure_fn = (taida_closure_cb2_fn)closure[1];
        taida_val env_ptr = closure[2];
        return closure_fn(env_ptr, arg0, arg1);
    }
    taida_cb2_fn fn = (taida_cb2_fn)fn_ptr;
    return fn(arg0, arg1);
}

// ── BuchiPack field call runtime ──────────────────────────
// obj.field(args) — get field from pack, then invoke as function.
// Handles both plain function pointers and closures.

typedef taida_val (*taida_cb0_fn)(void);
typedef taida_val (*taida_closure_cb0_fn)(taida_val);
typedef taida_val (*taida_cb3_fn)(taida_val, taida_val, taida_val);
typedef taida_val (*taida_closure_cb3_fn)(taida_val, taida_val, taida_val, taida_val);

taida_val taida_pack_call_field0(taida_val pack_ptr, taida_val field_hash) {
    taida_val fn_ptr = taida_pack_get(pack_ptr, field_hash);
    if (TAIDA_IS_CLOSURE(fn_ptr)) {
        taida_val *closure = (taida_val*)fn_ptr;
        taida_closure_cb0_fn closure_fn = (taida_closure_cb0_fn)closure[1];
        taida_val env_ptr = closure[2];
        return closure_fn(env_ptr);
    }
    taida_cb0_fn fn = (taida_cb0_fn)fn_ptr;
    return fn();
}

taida_val taida_pack_call_field1(taida_val pack_ptr, taida_val field_hash, taida_val arg0) {
    taida_val fn_ptr = taida_pack_get(pack_ptr, field_hash);
    if (TAIDA_IS_CLOSURE(fn_ptr)) {
        taida_val *closure = (taida_val*)fn_ptr;
        taida_closure_cb1_fn closure_fn = (taida_closure_cb1_fn)closure[1];
        taida_val env_ptr = closure[2];
        return closure_fn(env_ptr, arg0);
    }
    taida_cb1_fn fn = (taida_cb1_fn)fn_ptr;
    return fn(arg0);
}

taida_val taida_pack_call_field2(taida_val pack_ptr, taida_val field_hash, taida_val arg0, taida_val arg1) {
    taida_val fn_ptr = taida_pack_get(pack_ptr, field_hash);
    if (TAIDA_IS_CLOSURE(fn_ptr)) {
        taida_val *closure = (taida_val*)fn_ptr;
        taida_closure_cb2_fn closure_fn = (taida_closure_cb2_fn)closure[1];
        taida_val env_ptr = closure[2];
        return closure_fn(env_ptr, arg0, arg1);
    }
    taida_cb2_fn fn = (taida_cb2_fn)fn_ptr;
    return fn(arg0, arg1);
}

taida_val taida_pack_call_field3(taida_val pack_ptr, taida_val field_hash, taida_val arg0, taida_val arg1, taida_val arg2) {
    taida_val fn_ptr = taida_pack_get(pack_ptr, field_hash);
    if (TAIDA_IS_CLOSURE(fn_ptr)) {
        taida_val *closure = (taida_val*)fn_ptr;
        taida_closure_cb3_fn closure_fn = (taida_closure_cb3_fn)closure[1];
        taida_val env_ptr = closure[2];
        return closure_fn(env_ptr, arg0, arg1, arg2);
    }
    taida_cb3_fn fn = (taida_cb3_fn)fn_ptr;
    return fn(arg0, arg1, arg2);
}

// ── List runtime ──────────────────────────────────────────
// List layout: [refcount, capacity, length, elem_type_tag, item0, item1, ...]
// NO-4 RULE 1: New list builders MUST set elem_type_tag (via taida_list_set_elem_tag)
// AND retain each heap element (via taida_list_elem_retain). Direct taida_list_push
// without these two steps will cause tag loss and leak on release.

// Retain a list element based on the list's elem_type_tag.
// Used when copying elements from one list to another (shared ownership).
static void taida_list_elem_retain(taida_val elem, taida_val elem_tag) {
    if (elem_tag == TAIDA_TAG_PACK || elem_tag == TAIDA_TAG_LIST || elem_tag == TAIDA_TAG_CLOSURE
        || elem_tag == TAIDA_TAG_HMAP || elem_tag == TAIDA_TAG_SET) {
        if (elem > 4096) taida_retain(elem);
    } else if (elem_tag == TAIDA_TAG_STR) {
        if (elem > 4096) taida_str_retain(elem);
    }
    // INT, FLOAT, BOOL, UNKNOWN → no-op
}

// Release a list element based on the list's elem_type_tag.
// Used when freeing a list to release owned elements.
static void taida_list_elem_release(taida_val elem, taida_val elem_tag) {
    if (elem_tag == TAIDA_TAG_PACK || elem_tag == TAIDA_TAG_LIST || elem_tag == TAIDA_TAG_CLOSURE
        || elem_tag == TAIDA_TAG_HMAP || elem_tag == TAIDA_TAG_SET) {
        if (elem > 4096) taida_release(elem);
    } else if (elem_tag == TAIDA_TAG_STR) {
        if (elem > 4096) taida_str_release(elem);
    }
    // INT, FLOAT, BOOL, UNKNOWN → no-op (conservative: leak rather than crash)
}

taida_ptr taida_list_new(void) {
    taida_val *list = (taida_val*)TAIDA_MALLOC((4 + 16) * sizeof(taida_val), "list_new");
    list[0] = TAIDA_LIST_MAGIC | 1;   // Magic + refcount
    list[1] = 16;  // capacity
    list[2] = 0;   // length
    list[3] = TAIDA_TAG_UNKNOWN;  // elem_type_tag (unknown until set)
    return (taida_ptr)list;
}

void taida_list_set_elem_tag(taida_ptr list_ptr, taida_val tag) {
    ((taida_val*)list_ptr)[3] = tag;
}

taida_ptr taida_list_push(taida_ptr list_ptr, taida_val item) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val rc  = list[0];
    taida_val cap = list[1];
    taida_val len = list[2];
    if (len >= cap) {
        // M-05: Guard against cap * 2 overflow and (4 + new_cap) * sizeof overflow.
        taida_val new_cap = cap * 2;
        if (new_cap < cap || new_cap <= 0) {
            // Signed overflow detected — cap was already huge.
            fprintf(stderr, "taida: list capacity overflow (taida_list_push): %" PRId64 "\n", cap);
            exit(1);
        }
        size_t realloc_size = taida_safe_mul(taida_safe_add(4, (size_t)new_cap, "list_push slots"), sizeof(taida_val), "list_push bytes");
        taida_val *tmp = (taida_val*)realloc(list, realloc_size);
        if (!tmp) { fprintf(stderr, "taida: out of memory (taida_list_push)\n"); exit(1); }
        list = tmp;
        list[0] = rc;       // preserve refcount
        list[1] = new_cap;
    }
    list[4 + len] = item;
    list[2] = len + 1;
    return (taida_val)list;
}

taida_val taida_list_length(taida_val list_ptr) {
    return ((taida_val*)list_ptr)[2];
}

taida_val taida_list_get(taida_val list_ptr, taida_val index) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (index < 0 || index >= len) {
        return taida_lax_empty(0);  // OOB returns empty Lax (v0.5.0)
    }
    return taida_lax_new(list[4 + index], 0);
}

taida_val taida_list_first(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (len == 0) return taida_lax_empty(0);
    return taida_lax_new(list[4], 0);
}

taida_val taida_list_last(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (len == 0) return taida_lax_empty(0);
    return taida_lax_new(list[4 + len - 1], 0);
}

taida_val taida_list_sum(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val sum = 0;
    for (taida_val i = 0; i < len; i++) {
        sum += list[4 + i];
    }
    return sum;
}

taida_val taida_list_reverse(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = len - 1; i >= 0; i--) {
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    return new_list;
}

taida_val taida_list_contains(taida_val list_ptr, taida_val item) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        if (list[4 + i] == item) return 1;
    }
    return 0;
}

taida_val taida_list_is_empty(taida_val list_ptr) {
    return ((taida_val*)list_ptr)[2] == 0 ? 1 : 0;
}

// list.map(fn_ptr) - fn_ptr takes (taida_val) -> taida_val
taida_val taida_list_map(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val new_list = taida_list_new();
    // map may change element type, so leave elem_tag as UNKNOWN
    for (taida_val i = 0; i < len; i++) {
        taida_val result = taida_invoke_callback1(fn_ptr, list[4 + i]);
        new_list = taida_list_push(new_list, result);
    }
    return new_list;
}

// list.filter(fn_ptr) - fn_ptr takes (taida_val) -> taida_val (truthy/falsy)
taida_val taida_list_filter(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[4 + i])) {
            taida_list_elem_retain(list[4 + i], elem_tag);
            new_list = taida_list_push(new_list, list[4 + i]);
        }
    }
    return new_list;
}

// ── Bytes runtime ─────────────────────────────────────────
// Bytes layout: [magic+rc, length, b0, b1, ...] (each byte stored in taida_val slot)

static taida_val taida_bytes_new_filled(taida_val len, unsigned char fill) {
    if (len < 0) len = 0;
    // M-04: Guard against huge len values that could cause OOM or overflow
    // in (2 + len) * sizeof(taida_val). Limit to ~128M entries.
    if (len > (taida_val)(SIZE_MAX / sizeof(taida_val) - 2)) {
        fprintf(stderr, "taida: bytes length too large (taida_bytes_new_filled): %" PRId64 "\n", len);
        exit(1);
    }
    size_t alloc_size = taida_safe_mul(taida_safe_add(2, (size_t)len, "bytes_new_filled slots"), sizeof(taida_val), "bytes_new_filled bytes");
    taida_val *bytes = (taida_val*)TAIDA_MALLOC(alloc_size, "bytes_new_filled");
    bytes[0] = TAIDA_BYTES_MAGIC | 1;
    bytes[1] = len;
    for (taida_val i = 0; i < len; i++) {
        bytes[2 + i] = (taida_val)fill;
    }
    return (taida_val)bytes;
}

static taida_val taida_bytes_from_raw(const unsigned char *data, taida_val len) {
    taida_val out = taida_bytes_new_filled(len, 0);
    taida_val *bytes = (taida_val*)out;
    for (taida_val i = 0; i < len; i++) {
        bytes[2 + i] = (taida_val)data[i];
    }
    return out;
}

static taida_val taida_bytes_clone(taida_val bytes_ptr) {
    if (!TAIDA_IS_BYTES(bytes_ptr)) return taida_bytes_new_filled(0, 0);
    taida_val *src = (taida_val*)bytes_ptr;
    taida_val len = src[1];
    taida_val out = taida_bytes_new_filled(len, 0);
    taida_val *dst = (taida_val*)out;
    for (taida_val i = 0; i < len; i++) {
        dst[2 + i] = src[2 + i];
    }
    return out;
}

static taida_val taida_bytes_len(taida_val bytes_ptr) {
    if (!TAIDA_IS_BYTES(bytes_ptr)) return 0;
    return ((taida_val*)bytes_ptr)[1];
}

static taida_val taida_bytes_default_value(void) {
    return taida_bytes_new_filled(0, 0);
}

static taida_val taida_bytes_get_lax(taida_val bytes_ptr, taida_val index) {
    if (!TAIDA_IS_BYTES(bytes_ptr)) return taida_lax_empty(0);
    taida_val *bytes = (taida_val*)bytes_ptr;
    taida_val len = bytes[1];
    if (index < 0 || index >= len) return taida_lax_empty(0);
    return taida_lax_new(bytes[2 + index], 0);
}

// ── String methods ────────────────────────────────────────

taida_val taida_str_concat(const char* a, const char* b) {
    if (!a) a = "";
    if (!b) b = "";
    size_t la = strlen(a), lb = strlen(b);
    // M-10: Overflow guard on la + lb before passing to taida_str_alloc.
    size_t total_len = taida_safe_add(la, lb, "str_concat length");
    char *buf = taida_str_alloc(total_len);
    memcpy(buf, a, la);
    memcpy(buf + la, b, lb);
    return (taida_val)buf;
}

taida_val taida_str_length(const char* s) {
    if (!s) return 0;
    return (taida_val)strlen(s);
}

taida_val taida_str_char_at(const char* s, taida_val idx) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    if (idx < 0 || idx >= len) { char *r = taida_str_alloc(0); return (taida_val)r; }
    char *r = taida_str_alloc(1);
    r[0] = s[idx];
    return (taida_val)r;
}

taida_val taida_str_slice(const char* s, taida_val start, taida_val end) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    if (start < 0) start = 0;
    if (end > len) end = len;
    if (start >= end) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val slen = end - start;
    char *r = taida_str_alloc(slen);
    memcpy(r, s + start, slen);
    return (taida_val)r;
}

taida_val taida_slice_mold(taida_val value, taida_val start, taida_val end) {
    if (TAIDA_IS_BYTES(value)) {
        taida_val *bytes = (taida_val*)value;
        taida_val len = bytes[1];
        taida_val s = start;
        taida_val e = end;
        if (s < 0) s = 0;
        if (s > len) s = len;
        if (e < 0 || e > len) e = len;
        if (e < s) e = s;
        taida_val out_len = e - s;
        taida_val out = taida_bytes_new_filled(out_len, 0);
        taida_val *dst = (taida_val*)out;
        for (taida_val i = 0; i < out_len; i++) {
            dst[2 + i] = bytes[2 + s + i];
        }
        return out;
    }

    if (TAIDA_IS_LIST(value)) {
        taida_val *list = (taida_val*)value;
        taida_val len = list[2];
        taida_val elem_tag = list[3];
        taida_val s = start;
        taida_val e = end;
        if (s < 0) s = 0;
        if (s > len) s = len;
        if (e < 0 || e > len) e = len;
        if (e < s) e = s;
        taida_val out = taida_list_new();
        taida_val *ol = (taida_val*)out;
        ol[3] = elem_tag;  // propagate elem_type_tag
        for (taida_val i = s; i < e; i++) {
            taida_list_elem_retain(list[4 + i], elem_tag);
            out = taida_list_push(out, list[4 + i]);
        }
        return out;
    }

    const char *s = (const char*)value;
    if (!s) {
        char *r = taida_str_alloc(0);
        return (taida_val)r;
    }
    taida_val len = (taida_val)strlen(s);
    taida_val cs = start;
    taida_val ce = end;
    if (cs < 0) cs = 0;
    if (cs > len) cs = len;
    if (ce < 0 || ce > len) ce = len;
    if (ce < cs) ce = cs;
    return taida_str_slice(s, cs, ce);
}

taida_val taida_str_index_of(const char* s, const char* sub) {
    if (!s || !sub) return -1;
    const char *p = strstr(s, sub);
    if (!p) return -1;
    // Convert byte offset to character offset (UTF-8 aware)
    taida_val char_offset = 0;
    for (const char *c = s; c < p; ) {
        if ((*c & 0x80) == 0) c += 1;
        else if ((*c & 0xE0) == 0xC0) c += 2;
        else if ((*c & 0xF0) == 0xE0) c += 3;
        else c += 4;
        char_offset++;
    }
    return char_offset;
}

taida_val taida_str_last_index_of(const char* s, const char* sub) {
    if (!s || !sub) return -1;
    taida_val slen = (taida_val)strlen(s);
    taida_val sublen = (taida_val)strlen(sub);
    if (sublen > slen) return -1;
    for (taida_val i = slen - sublen; i >= 0; i--) {
        if (strncmp(s + i, sub, sublen) == 0) {
            // Convert byte offset to character offset (UTF-8 aware)
            taida_val char_offset = 0;
            for (const char *c = s; c < s + i; ) {
                if ((*c & 0x80) == 0) c += 1;
                else if ((*c & 0xE0) == 0xC0) c += 2;
                else if ((*c & 0xF0) == 0xE0) c += 3;
                else c += 4;
                char_offset++;
            }
            return char_offset;
        }
    }
    return -1;
}

taida_val taida_str_contains(const char* s, const char* sub) {
    if (!s || !sub) return 0;
    return strstr(s, sub) != NULL ? 1 : 0;
}

taida_val taida_str_starts_with(const char* s, const char* prefix) {
    if (!s || !prefix) return 0;
    taida_val plen = (taida_val)strlen(prefix);
    return strncmp(s, prefix, plen) == 0 ? 1 : 0;
}

taida_val taida_str_ends_with(const char* s, const char* suffix) {
    if (!s || !suffix) return 0;
    taida_val slen = (taida_val)strlen(s);
    taida_val suflen = (taida_val)strlen(suffix);
    if (suflen > slen) return 0;
    return strcmp(s + slen - suflen, suffix) == 0 ? 1 : 0;
}

taida_val taida_str_get(const char* s, taida_val idx) {
    if (!s) return taida_lax_empty((taida_val)"");
    taida_val len = (taida_val)strlen(s);
    if (idx < 0 || idx >= len) return taida_lax_empty((taida_val)"");
    char *r = taida_str_alloc(1);
    r[0] = s[idx];
    return taida_lax_new((taida_val)r, (taida_val)"");
}

taida_val taida_str_to_upper(const char* s) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    char *r = taida_str_alloc(len);
    for (taida_val i = 0; i < len; i++) {
        r[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
    }
    return (taida_val)r;
}

taida_val taida_str_to_lower(const char* s) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    char *r = taida_str_alloc(len);
    for (taida_val i = 0; i < len; i++) {
        r[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
    }
    return (taida_val)r;
}

taida_val taida_str_trim(const char* s) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    taida_val start = 0, end = len;
    while (start < len && (s[start]==' '||s[start]=='\t'||s[start]=='\n'||s[start]=='\r')) start++;
    while (end > start && (s[end-1]==' '||s[end-1]=='\t'||s[end-1]=='\n'||s[end-1]=='\r')) end--;
    taida_val slen = end - start;
    char *r = taida_str_alloc(slen);
    memcpy(r, s + start, slen);
    return (taida_val)r;
}

taida_val taida_str_split(const char* s, const char* sep) {
    if (!s) return taida_list_new();
    taida_val list = taida_list_new();
    if (!sep || strlen(sep) == 0) {
        // Split into Unicode codepoints (same logic as taida_str_chars)
        size_t slen = 0;
        if (!taida_read_cstr_len_safe(s, 65536, &slen) || slen == 0) return list;
        const unsigned char *buf = (const unsigned char*)s;
        size_t offset = 0;
        while (offset < slen) {
            size_t consumed = 0;
            uint32_t cp = 0;
            if (!taida_utf8_decode_one(buf + offset, slen - offset, &consumed, &cp) || consumed == 0) {
                char *fallback = taida_str_alloc(1);
                fallback[0] = (char)buf[offset];
                list = taida_list_push(list, (taida_val)fallback);
                offset += 1;
                continue;
            }
            unsigned char utf8[4];
            size_t out_len = 0;
            if (!taida_utf8_encode_scalar(cp, utf8, &out_len) || out_len == 0) {
                offset += consumed;
                continue;
            }
            char *ch = taida_str_alloc(out_len);
            memcpy(ch, utf8, out_len);
            list = taida_list_push(list, (taida_val)ch);
            offset += consumed;
        }
        return list;
    }
    taida_val sep_len = (taida_val)strlen(sep);
    const char *p = s;
    while (1) {
        const char *found = strstr(p, sep);
        if (!found) {
            taida_val slen = (taida_val)strlen(p);
            char *part = taida_str_alloc(slen);
            memcpy(part, p, slen);
            list = taida_list_push(list, (taida_val)part);
            break;
        }
        taida_val plen = (taida_val)(found - p);
        char *part = taida_str_alloc(plen);
        memcpy(part, p, plen);
        list = taida_list_push(list, (taida_val)part);
        p = found + sep_len;
    }
    return list;
}

taida_val taida_str_chars(const char* s) {
    taida_val list = taida_list_new();
    taida_list_set_elem_tag(list, TAIDA_TAG_STR);
    if (!s) return list;

    size_t len = 0;
    if (!taida_read_cstr_len_safe(s, 65536, &len) || len == 0) {
        return list;
    }

    const unsigned char *buf = (const unsigned char*)s;
    size_t offset = 0;
    while (offset < len) {
        size_t consumed = 0;
        uint32_t cp = 0;
        if (!taida_utf8_decode_one(buf + offset, len - offset, &consumed, &cp) || consumed == 0) {
            char *fallback = taida_str_alloc(1);
            fallback[0] = (char)buf[offset];
            list = taida_list_push(list, (taida_val)fallback);
            offset += 1;
            continue;
        }

        unsigned char utf8[4];
        size_t out_len = 0;
        if (!taida_utf8_encode_scalar(cp, utf8, &out_len) || out_len == 0) {
            offset += consumed;
            continue;
        }

        char *ch = taida_str_alloc(out_len);
        memcpy(ch, utf8, out_len);
        list = taida_list_push(list, (taida_val)ch);
        offset += consumed;
    }

    return list;
}

taida_val taida_str_replace(const char* s, const char* from, const char* to) {
    if (!s || !from || !to) {
        if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
        return (taida_val)taida_str_new_copy(s);
    }
    taida_val from_len = (taida_val)strlen(from);
    taida_val to_len = (taida_val)strlen(to);
    if (from_len == 0) {
        return (taida_val)taida_str_new_copy(s);
    }
    // Count occurrences
    taida_val count = 0;
    const char *p = s;
    while ((p = strstr(p, from)) != NULL) { count++; p += from_len; }
    taida_val s_len = (taida_val)strlen(s);
    taida_val new_len = s_len + count * (to_len - from_len);
    char *r = taida_str_alloc(new_len);
    char *dst = r;
    p = s;
    while (1) {
        const char *found = strstr(p, from);
        if (!found) {
            taida_val remaining = (taida_val)strlen(p);
            memcpy(dst, p, remaining);
            dst += remaining;
            break;
        }
        taida_val chunk = (taida_val)(found - p);
        memcpy(dst, p, chunk); dst += chunk;
        memcpy(dst, to, to_len); dst += to_len;
        p = found + from_len;
    }
    *dst = '\0';
    return (taida_val)r;
}

taida_val taida_str_to_int(const char* s) {
    if (!s) return 0;
    return atol(s);
}

taida_val taida_str_repeat(const char* s, taida_val n) {
    if (!s || n <= 0) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val slen = (taida_val)strlen(s);
    // Overflow check: slen * n must not overflow
    if (slen > 0 && n > (taida_val)(((size_t)-1) / 2) / slen) {
        // Overflow: return empty string
        char *r = taida_str_alloc(0); return (taida_val)r;
    }
    taida_val total = slen * n;
    char *r = taida_str_alloc(total);
    for (taida_val i = 0; i < n; i++) {
        memcpy(r + i * slen, s, slen);
    }
    return (taida_val)r;
}

taida_val taida_str_reverse(const char* s) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    char *r = taida_str_alloc(len);
    for (taida_val i = 0; i < len; i++) {
        r[i] = s[len - 1 - i];
    }
    return (taida_val)r;
}

taida_val taida_str_trim_start(const char* s) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    taida_val start = 0;
    while (start < len && (s[start]==' '||s[start]=='\t'||s[start]=='\n'||s[start]=='\r')) start++;
    taida_val slen = len - start;
    char *r = taida_str_alloc(slen);
    memcpy(r, s + start, slen);
    return (taida_val)r;
}

taida_val taida_str_trim_end(const char* s) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val len = (taida_val)strlen(s);
    taida_val end = len;
    while (end > 0 && (s[end-1]==' '||s[end-1]=='\t'||s[end-1]=='\n'||s[end-1]=='\r')) end--;
    char *r = taida_str_alloc(end);
    memcpy(r, s, end);
    return (taida_val)r;
}

taida_val taida_str_replace_first(const char* s, const char* from, const char* to) {
    if (!s || !from || !to) {
        if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
        return (taida_val)taida_str_new_copy(s);
    }
    taida_val from_len = (taida_val)strlen(from);
    taida_val to_len = (taida_val)strlen(to);
    if (from_len == 0) {
        return (taida_val)taida_str_new_copy(s);
    }
    const char *found = strstr(s, from);
    if (!found) {
        return (taida_val)taida_str_new_copy(s);
    }
    taida_val s_len = (taida_val)strlen(s);
    taida_val new_len = s_len - from_len + to_len;
    char *r = taida_str_alloc(new_len);
    taida_val prefix = (taida_val)(found - s);
    memcpy(r, s, prefix);
    memcpy(r + prefix, to, to_len);
    taida_val suffix = s_len - prefix - from_len;
    memcpy(r + prefix + to_len, found + from_len, suffix);
    return (taida_val)r;
}

taida_val taida_str_pad(const char* s, taida_val target_len, const char* pad_char, taida_val pad_end) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    taida_val slen = (taida_val)strlen(s);
    if (slen >= target_len) {
        return (taida_val)taida_str_new_copy(s);
    }
    taida_val pad_len = target_len - slen;
    char pc = ' ';
    if (pad_char && strlen(pad_char) > 0) pc = pad_char[0];
    char *r = taida_str_alloc(target_len);
    if (pad_end) {
        memcpy(r, s, slen);
        for (taida_val i = 0; i < pad_len; i++) r[slen + i] = pc;
    } else {
        for (taida_val i = 0; i < pad_len; i++) r[i] = pc;
        memcpy(r + pad_len, s, slen);
    }
    return (taida_val)r;
}

// ── C12-6 (FB-5): Regex support via POSIX regex.h ─────────
//
// Value representation: `Regex("pattern", "flags")` returns a
// BuchiPack with 3 fields (pattern, flags, __type). The field hashes
// used here are FNV-1a 64-bit of the field-name literals, matching
// the Rust side of the interpreter (see `regex_eval::REGEX_TYPE_TAG`)
// and the JS runtime (`__taida_is_regex`).
//
// Philosophy: construction validates the pattern eagerly; on failure
// we fall back to an empty-match / no-op result (no null / undefined
// escape path). wasm profiles link a fallback stub that treats
// Regex args as empty strings — Regex overloads are documented as
// Native/Interpreter/JS only (see design lock §C12-6).

#include <regex.h>

#define HASH_PATTERN      0x0b7a1e4bea695f89ULL
#define HASH_FLAGS        0x17a3a1a985f75aecULL
#define HASH_FULL         0x865b7478db00e67cULL
#define HASH_GROUPS       0x6039b1239580514dULL
#define HASH_START        0xee5d97ad45ad251fULL
#define HASH_REGEX_MATCH_STR 0x3b377cd3b47dd849ULL /* FNV-1a("RegexMatch"), unused — for reference */

// Detect whether `v` points to a :Regex BuchiPack. We check the pack
// magic + scan for `__type <= "Regex"`. Returns 1 on match, 0
// otherwise (or when the pointer is not a pack). Safe for NULL /
// small integer values.
static int taida_val_is_regex_pack(taida_val v) {
    if (v <= 4096) return 0;
    if (!taida_ptr_is_readable(v, sizeof(taida_val))) return 0;
    taida_val hdr = ((taida_val*)v)[0];
    if ((hdr & TAIDA_MAGIC_MASK) != TAIDA_PACK_MAGIC) return 0;
    taida_val type_val = taida_pack_get((taida_ptr)v, (taida_val)HASH___TYPE);
    if (type_val == 0) return 0;
    size_t len = 0;
    if (!taida_read_cstr_len_safe((const char*)type_val, 64, &len)) return 0;
    if (len != 5) return 0;
    return memcmp((const char*)type_val, "Regex", 5) == 0;
}

// Extract (pattern, flags) from a validated Regex pack. Returns
// pointers into pack-owned Str payloads — caller must NOT free them.
// Each output pointer falls back to "" when the field is missing.
static void taida_regex_get_fields(taida_val regex_pack,
                                    const char **pattern_out,
                                    const char **flags_out) {
    taida_val pat = taida_pack_get((taida_ptr)regex_pack, (taida_val)HASH_PATTERN);
    taida_val flg = taida_pack_get((taida_ptr)regex_pack, (taida_val)HASH_FLAGS);
    *pattern_out = pat > 4096 ? (const char*)pat : "";
    *flags_out = flg > 4096 ? (const char*)flg : "";
}

// POSIX ERE doesn't understand Perl-style meta escapes (`\d` / `\w`
// / `\s` + complements). Translate them into POSIX character classes
// before passing the pattern to `regcomp`. This keeps the surface
// semantics identical across Interpreter (Rust `regex` crate), JS
// (RegExp), and Native (POSIX regex.h).
//
// Translation table (C12B-030):
//   \d → [0-9]             \D → [^0-9]
//   \w → [0-9A-Za-z_]      \W → [^0-9A-Za-z_]
//   \s → [ \t\n\r\f\v]     \S → [^ \t\n\r\f\v]
//   \x{HH}  / \xHH         → literal byte with hex code point
//   \u{HH..} / \uHHHH      → same, encoded as UTF-8 when > 0x7F
//
// `\\` escapes are preserved so users can match a literal backslash.
// Other `\X` sequences are passed through untouched (POSIX will
// interpret `\.` / `\(` / `\)` etc. as literals).
//
// Documented gaps (C12 supported subset — see
// `docs/reference/standard_methods.md` and `.dev/C12_DESIGN.md §C12-6`):
//   - `\b` / `\B` word boundaries: POSIX ERE has no equivalent. Users
//     who need word boundaries must target Interpreter / JS only, or
//     emulate with `(^|[^A-Za-z0-9_])` / `([^A-Za-z0-9_]|$)`.
//   - `s` flag (dotall): POSIX has no direct equivalent. `.` still
//     skips `\n` on Native; Interpreter / JS honour the flag.
//
// Caller owns the returned buffer — free with `free()`.
static char *taida_regex_rewrite_pattern(const char *pat) {
    if (!pat) {
        char *empty = (char*)TAIDA_MALLOC(1, "regex_pattern empty");
        empty[0] = '\0';
        return empty;
    }
    size_t cap = strlen(pat) * 4 + 16;
    char *out = (char*)TAIDA_MALLOC(cap, "regex_pattern rewrite");
    size_t len = 0;
    #define APPEND(s, n) do { \
        size_t _n = (n); \
        while (len + _n + 1 > cap) { cap *= 2; TAIDA_REALLOC(out, cap, "regex_pattern grow"); } \
        memcpy(out + len, (s), _n); len += _n; \
    } while(0)
    /* C12B-030 helper: parse a hex escape starting at `pat[i+1]` where
       `pat[i]` is the leading `x` or `u`. Supports both bracketed
       (`\x{HH}` / `\u{HHHH}`) and fixed-width (`\xHH` 2 digits,
       `\uHHHH` 4 digits) forms. On success, encodes the code point as
       UTF-8 into out buffer and advances `*i_inout` past the escape.
       Returns 1 on success, 0 if the escape is malformed (caller
       falls back to literal pass-through). */
    #define HEX_DIGIT(ch) (((ch) >= '0' && (ch) <= '9') ? ((ch) - '0') : \
                           ((ch) >= 'a' && (ch) <= 'f') ? ((ch) - 'a' + 10) : \
                           ((ch) >= 'A' && (ch) <= 'F') ? ((ch) - 'A' + 10) : -1)
    for (size_t i = 0; pat[i]; ) {
        char c = pat[i];
        if (c == '\\' && pat[i+1]) {
            char n = pat[i+1];
            if (n == 'd') { APPEND("[0-9]", 5); i += 2; continue; }
            if (n == 'D') { APPEND("[^0-9]", 6); i += 2; continue; }
            if (n == 'w') { APPEND("[0-9A-Za-z_]", 12); i += 2; continue; }
            if (n == 'W') { APPEND("[^0-9A-Za-z_]", 13); i += 2; continue; }
            if (n == 's') { APPEND("[ \t\n\r\f\v]", 8); i += 2; continue; }
            if (n == 'S') { APPEND("[^ \t\n\r\f\v]", 9); i += 2; continue; }
            /* C12B-030: \x{...} / \u{...} / \xHH / \uHHHH hex escapes.
               Encode the code point as UTF-8 and append the raw bytes
               so POSIX ERE sees them as literal characters. Locale is
               LC_COLLATE-agnostic here because we always emit bytes
               verbatim. */
            if (n == 'x' || n == 'u') {
                size_t j = i + 2;
                uint32_t cp = 0;
                int parsed_digits = 0;
                int ok = 0;
                if (pat[j] == '{') {
                    /* Bracketed form: up to 8 hex digits. */
                    j++;
                    while (pat[j] && pat[j] != '}' && parsed_digits < 8) {
                        int d = HEX_DIGIT(pat[j]);
                        if (d < 0) break;
                        cp = (cp << 4) | (uint32_t)d;
                        parsed_digits++;
                        j++;
                    }
                    if (pat[j] == '}' && parsed_digits > 0) {
                        j++;
                        ok = 1;
                    }
                } else {
                    /* Fixed-width form: \xHH (2) or \uHHHH (4). */
                    int needed = (n == 'x') ? 2 : 4;
                    for (int k = 0; k < needed; k++) {
                        int d = HEX_DIGIT(pat[j + k]);
                        if (d < 0) { parsed_digits = 0; break; }
                        cp = (cp << 4) | (uint32_t)d;
                        parsed_digits++;
                    }
                    if (parsed_digits == needed) {
                        j += needed;
                        ok = 1;
                    }
                }
                if (ok && cp <= 0x10FFFF) {
                    /* Encode code point as UTF-8 and emit the raw bytes. */
                    char utf[4];
                    int utf_len;
                    if (cp < 0x80) {
                        utf[0] = (char)cp;
                        utf_len = 1;
                    } else if (cp < 0x800) {
                        utf[0] = (char)(0xC0 | (cp >> 6));
                        utf[1] = (char)(0x80 | (cp & 0x3F));
                        utf_len = 2;
                    } else if (cp < 0x10000) {
                        utf[0] = (char)(0xE0 | (cp >> 12));
                        utf[1] = (char)(0x80 | ((cp >> 6) & 0x3F));
                        utf[2] = (char)(0x80 | (cp & 0x3F));
                        utf_len = 3;
                    } else {
                        utf[0] = (char)(0xF0 | (cp >> 18));
                        utf[1] = (char)(0x80 | ((cp >> 12) & 0x3F));
                        utf[2] = (char)(0x80 | ((cp >> 6) & 0x3F));
                        utf[3] = (char)(0x80 | (cp & 0x3F));
                        utf_len = 4;
                    }
                    /* For bytes that have POSIX ERE meaning we need to
                       escape them. Safely wrap every byte that could be
                       a metacharacter in `[]` as a single-member class. */
                    for (int k = 0; k < utf_len; k++) {
                        unsigned char ub = (unsigned char)utf[k];
                        if (ub >= 0x80) {
                            APPEND(&utf[k], 1);
                        } else {
                            char ch = utf[k];
                            if (ch == '.' || ch == '*' || ch == '+' || ch == '?' ||
                                ch == '(' || ch == ')' || ch == '{' || ch == '}' ||
                                ch == '|' || ch == '^' || ch == '$' || ch == '[' ||
                                ch == ']' || ch == '\\' || ch == '/') {
                                char esc[2] = { '\\', ch };
                                APPEND(esc, 2);
                            } else {
                                APPEND(&utf[k], 1);
                            }
                        }
                    }
                    i = j;
                    continue;
                }
                /* Malformed hex escape — fall through and pass literally. */
            }
            // Any other backslash escape: keep as-is (including `\\`).
            APPEND(pat + i, 2);
            i += 2;
            continue;
        }
        APPEND(pat + i, 1);
        i += 1;
    }
    out[len] = '\0';
    #undef APPEND
    #undef HEX_DIGIT
    return out;
}

// Compile a POSIX regex from (pattern, flags). Flags: 'i' = icase,
// 'm' = REG_NEWLINE (line anchors), 's' = POSIX doesn't support
// dotall directly. We intentionally leave 's' unhandled at the C
// level: `.` will not cross newline on Native. This is a documented
// parity gap in the design lock; parity tests for `s` cover
// Interpreter / JS only.
//
// Returns 0 on success, non-zero on failure.
//
// NOTE: This is the "always fresh" compile path used by
// `taida_regex_new` (construction-time validation) where caching the
// probe would be semantically wrong — we must let the caller
// `regfree()` after use. The hot method paths (replace / split /
// match / search) go through `taida_regex_acquire()` below which
// caches the compiled `regex_t` across calls (C12B-036).
static int taida_regex_compile(const char *pattern, const char *flags, regex_t *out) {
    int cflags = REG_EXTENDED;
    for (const char *p = flags; p && *p; p++) {
        if (*p == 'i') cflags |= REG_ICASE;
        else if (*p == 'm') cflags |= REG_NEWLINE;
        // 's' (dotall) — not supported by POSIX ERE; silently ignored.
    }
    char *rewritten = taida_regex_rewrite_pattern(pattern);
    int rc = regcomp(out, rewritten, cflags);
    free(rewritten);
    return rc;
}

// C12B-036: POSIX `regex_t` FIFO cache shared by the hot Str-method
// paths (`replace`, `replaceAll`, `split`, `match`, `search`). Size 16
// was chosen as a pragmatic small-working-set default — large enough
// to cover typical programs that use a handful of distinct patterns
// (log parsing, tokenization, URL matching) while keeping the linear
// lookup cost negligible. Each entry owns its `regex_t` and the
// strings used to form the key; eviction calls `regfree` and `free`.
// The cache is process-wide rather than per-thread because POSIX
// `regexec` is documented thread-safe on a read-only `regex_t`, and
// the Native backend's typical workload is single-threaded (threading
// model is left to the user's code, not the runtime primitives).
//
// IMPORTANT: `taida_regex_new` (construction-time validation) still
// uses the uncached `taida_regex_compile` path to avoid polluting
// the cache with patterns that the program may never actually
// execute, and to keep probe lifetime LOCAL (we need to `regfree`
// on the error branch before throwing). Repeated construction with
// identical patterns therefore does not benefit from the cache, but
// that is intentionally off the hot path.
#define TAIDA_REGEX_CACHE_CAPACITY 16
typedef struct {
    char *pattern;    // owned copy of the input pattern (NULL => slot empty)
    char *flags;      // owned copy of the input flags   (NULL => slot empty)
    regex_t re;       // compiled regex object — valid iff active == 1
    int active;       // 0 = empty / evicted, 1 = compiled and valid
} taida_regex_cache_entry;
static taida_regex_cache_entry g_regex_cache[TAIDA_REGEX_CACHE_CAPACITY];
static int g_regex_cache_next = 0; // FIFO eviction pointer

static int _taida_str_eq_nullable(const char *a, const char *b) {
    if (a == b) return 1;
    if (!a || !b) return 0;
    return strcmp(a, b) == 0;
}

// Return a borrowed pointer to a cached `regex_t` for (pattern, flags),
// compiling and inserting on miss. On compilation failure returns NULL
// — callers fall back to their "no-match" branch (e.g. return the
// input string unchanged) just as if `taida_regex_compile` had
// returned non-zero.
//
// Lifetime: the returned pointer remains valid until the slot is
// evicted by a subsequent insertion that rolls the FIFO pointer over
// to the same index. Callers must NOT `regfree` it.
static const regex_t *taida_regex_acquire(const char *pattern, const char *flags) {
    if (!pattern) pattern = "";
    if (!flags) flags = "";
    // Linear probe for a hit. Cache is small (16) so O(n) is fine.
    for (int i = 0; i < TAIDA_REGEX_CACHE_CAPACITY; i++) {
        taida_regex_cache_entry *e = &g_regex_cache[i];
        if (!e->active) continue;
        if (_taida_str_eq_nullable(e->pattern, pattern) &&
            _taida_str_eq_nullable(e->flags, flags)) {
            return &e->re;
        }
    }
    // Miss: compile into a scratch slot, then move into the FIFO next
    // position. If that position was occupied, evict it first.
    regex_t fresh;
    if (taida_regex_compile(pattern, flags, &fresh) != 0) {
        return NULL;
    }
    int idx = g_regex_cache_next;
    g_regex_cache_next = (g_regex_cache_next + 1) % TAIDA_REGEX_CACHE_CAPACITY;
    taida_regex_cache_entry *slot = &g_regex_cache[idx];
    if (slot->active) {
        regfree(&slot->re);
        if (slot->pattern) { free(slot->pattern); slot->pattern = NULL; }
        if (slot->flags)   { free(slot->flags);   slot->flags = NULL; }
        slot->active = 0;
    }
    size_t plen = strlen(pattern);
    size_t flen = strlen(flags);
    slot->pattern = (char*)malloc(plen + 1);
    slot->flags   = (char*)malloc(flen + 1);
    if (!slot->pattern || !slot->flags) {
        // OOM on the metadata — free the freshly compiled regex and
        // bail. Callers see this as a miss that produces NULL.
        regfree(&fresh);
        if (slot->pattern) { free(slot->pattern); slot->pattern = NULL; }
        if (slot->flags)   { free(slot->flags);   slot->flags = NULL; }
        return NULL;
    }
    memcpy(slot->pattern, pattern, plen + 1);
    memcpy(slot->flags, flags, flen + 1);
    slot->re = fresh;
    slot->active = 1;
    return &slot->re;
}

// Construct a Regex BuchiPack from (pattern_str, flags_str). Field
// layout matches the interpreter / JS representation.
//
// C12B-029: Validate fail-fast at construction time — mirror the
// Interpreter (`src/interpreter/regex_eval.rs::build_regex_value`)
// and JS (`new RegExp(...)` throw) behaviour so the 3 backends share
// the same failure mode:
//   1. Reject unsupported flags (anything other than `i` / `m` / `s`).
//   2. Compile the (rewritten) pattern once; on POSIX regcomp failure,
//      throw a :RegexError via the error ceiling rather than returning
//      a usable pack that silently no-ops at the first method call.
taida_val taida_regex_new(const char *pattern_s, const char *flags_s) {
    if (!pattern_s) pattern_s = "";
    if (!flags_s) flags_s = "";
    // C12B-029: Flag validation. POSIX can only honour i/m directly,
    // but the Interpreter / JS layer accepts `s` as well. Other flags
    // must be rejected at construction time on every backend.
    for (const char *fp = flags_s; *fp; fp++) {
        char ch = *fp;
        if (ch != 'i' && ch != 'm' && ch != 's') {
            char buf[160];
            snprintf(buf, sizeof(buf),
                "Regex: unsupported flag '%c'. Supported flags: i (case-insensitive), m (multiline), s (dotall)",
                ch);
            taida_val err = taida_make_error("ValueError", buf);
            return taida_throw(err);
        }
    }
    // C12B-029: Pattern validation. Compile the rewritten pattern
    // once so that malformed input (unbalanced parens, stray
    // quantifiers, etc.) fails at `Regex(...)` instead of silently
    // returning the input string at each Str-method call site.
    {
        regex_t probe;
        int cflags = REG_EXTENDED;
        for (const char *fp = flags_s; *fp; fp++) {
            if (*fp == 'i') cflags |= REG_ICASE;
            else if (*fp == 'm') cflags |= REG_NEWLINE;
        }
        char *rewritten = taida_regex_rewrite_pattern(pattern_s);
        int rc = regcomp(&probe, rewritten ? rewritten : "", cflags);
        if (rewritten) free(rewritten);
        if (rc != 0) {
            // Mirror POSIX regerror: callable with the failing regex_t
            // to produce a descriptive message. We skip regfree on the
            // error path because regfree on a not-successfully-compiled
            // regex_t is unspecified behaviour.
            char errbuf[96];
            (void)regerror(rc, &probe, errbuf, sizeof(errbuf));
            char msg[256];
            snprintf(msg, sizeof(msg),
                "Regex: invalid pattern '%s' — %s", pattern_s, errbuf);
            taida_val err = taida_make_error("ValueError", msg);
            return taida_throw(err);
        }
        regfree(&probe);
    }
    taida_val pack = taida_pack_new(3);
    taida_pack_set_hash((taida_ptr)pack, 0, (taida_val)HASH_PATTERN);
    taida_pack_set_hash((taida_ptr)pack, 1, (taida_val)HASH_FLAGS);
    taida_pack_set_hash((taida_ptr)pack, 2, (taida_val)HASH___TYPE);
    taida_val pat_str = (taida_val)taida_str_new_copy(pattern_s);
    taida_val flg_str = (taida_val)taida_str_new_copy(flags_s);
    taida_val type_str = (taida_val)taida_str_new_copy("Regex");
    taida_pack_set((taida_ptr)pack, 0, pat_str);
    taida_pack_set_tag((taida_ptr)pack, 0, TAIDA_TAG_STR);
    taida_pack_set((taida_ptr)pack, 1, flg_str);
    taida_pack_set_tag((taida_ptr)pack, 1, TAIDA_TAG_STR);
    taida_pack_set((taida_ptr)pack, 2, type_str);
    taida_pack_set_tag((taida_ptr)pack, 2, TAIDA_TAG_STR);
    return pack;
}

// Apply Regex replace_first: build the output by copying the prefix,
// the literal replacement (no `$N` expansion), and the suffix from
// the first regex match. Returns a freshly allocated taida_str.
static taida_val taida_regex_replace_first_impl(const char *s,
                                                 const char *pattern,
                                                 const char *flags,
                                                 const char *replacement) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    if (!replacement) replacement = "";
    // C12B-036: shared compiled regex from cache; do not regfree.
    const regex_t *re = taida_regex_acquire(pattern, flags);
    if (!re) {
        return (taida_val)taida_str_new_copy(s);
    }
    regmatch_t m;
    if (regexec(re, s, 1, &m, 0) != 0 || m.rm_so < 0) {
        return (taida_val)taida_str_new_copy(s);
    }
    size_t prefix_len = (size_t)m.rm_so;
    size_t match_len = (size_t)(m.rm_eo - m.rm_so);
    size_t rep_len = strlen(replacement);
    size_t suffix_len = strlen(s) - prefix_len - match_len;
    size_t out_len = prefix_len + rep_len + suffix_len;
    char *out = taida_str_alloc(out_len);
    memcpy(out, s, prefix_len);
    memcpy(out + prefix_len, replacement, rep_len);
    memcpy(out + prefix_len + rep_len, s + prefix_len + match_len, suffix_len);
    return (taida_val)out;
}

// Apply Regex replace_all: iterate regexec with REG_NOTBOL on
// subsequent calls so `^` only anchors at the very start.
static taida_val taida_regex_replace_all_impl(const char *s,
                                               const char *pattern,
                                               const char *flags,
                                               const char *replacement) {
    if (!s) { char *r = taida_str_alloc(0); return (taida_val)r; }
    if (!replacement) replacement = "";
    // C12B-036: shared compiled regex from cache; do not regfree.
    const regex_t *re = taida_regex_acquire(pattern, flags);
    if (!re) {
        return (taida_val)taida_str_new_copy(s);
    }
    // Build output into a growing buffer.
    size_t cap = 64;
    size_t len = 0;
    char *out = (char*)TAIDA_MALLOC(cap, "regex_replace_all");
    size_t rep_len = strlen(replacement);
    const char *cursor = s;
    int eflags = 0;
    // Safety bound: number of iterations cannot exceed input byte
    // length + 1 (each zero-width match advances by one byte).
    size_t max_iters = strlen(s) + 1;
    for (size_t iter = 0; iter < max_iters; iter++) {
        regmatch_t m;
        if (regexec(re, cursor, 1, &m, eflags) != 0 || m.rm_so < 0) {
            // Copy remaining tail.
            size_t tail = strlen(cursor);
            while (len + tail + 1 > cap) { cap *= 2; TAIDA_REALLOC(out, cap, "regex_replace_all grow"); }
            memcpy(out + len, cursor, tail);
            len += tail;
            break;
        }
        size_t prefix = (size_t)m.rm_so;
        size_t match_len = (size_t)(m.rm_eo - m.rm_so);
        // Append prefix.
        while (len + prefix + 1 > cap) { cap *= 2; TAIDA_REALLOC(out, cap, "regex_replace_all grow"); }
        memcpy(out + len, cursor, prefix);
        len += prefix;
        // Append replacement.
        while (len + rep_len + 1 > cap) { cap *= 2; TAIDA_REALLOC(out, cap, "regex_replace_all grow"); }
        memcpy(out + len, replacement, rep_len);
        len += rep_len;
        // Advance cursor. On zero-width matches, bump one byte to
        // avoid an infinite loop.
        if (match_len == 0) {
            if (cursor[prefix] == '\0') break;
            while (len + 1 + 1 > cap) { cap *= 2; TAIDA_REALLOC(out, cap, "regex_replace_all grow"); }
            out[len++] = cursor[prefix];
            cursor += prefix + 1;
        } else {
            cursor += prefix + match_len;
        }
        eflags = REG_NOTBOL;
    }
    // C12B-036: compiled regex is cache-owned; do not regfree.
    // Copy into a taida-owned Str buffer.
    char *taida_out = taida_str_alloc(len);
    memcpy(taida_out, out, len);
    free(out);
    return (taida_val)taida_out;
}

// Apply Regex split: emit the text between successive matches.
static taida_val taida_regex_split_impl(const char *s,
                                         const char *pattern,
                                         const char *flags) {
    taida_val list = taida_list_new();
    taida_list_set_elem_tag(list, TAIDA_TAG_STR);
    if (!s) return list;
    // C12B-036: shared compiled regex from cache; do not regfree.
    const regex_t *re = taida_regex_acquire(pattern, flags);
    if (!re) {
        // Compile failed — return a single-element list with the
        // original string (matches interpreter fallback via `split`
        // on an unmatched pattern: whole-string list).
        char *copy = (char*)taida_str_new_copy(s);
        list = taida_list_push(list, (taida_val)copy);
        return list;
    }
    const char *cursor = s;
    int eflags = 0;
    size_t max_iters = strlen(s) + 1;
    for (size_t iter = 0; iter < max_iters; iter++) {
        regmatch_t m;
        if (regexec(re, cursor, 1, &m, eflags) != 0 || m.rm_so < 0) {
            // Tail.
            size_t tail = strlen(cursor);
            char *piece = taida_str_alloc(tail);
            memcpy(piece, cursor, tail);
            list = taida_list_push(list, (taida_val)piece);
            break;
        }
        size_t prefix = (size_t)m.rm_so;
        size_t match_len = (size_t)(m.rm_eo - m.rm_so);
        char *piece = taida_str_alloc(prefix);
        memcpy(piece, cursor, prefix);
        list = taida_list_push(list, (taida_val)piece);
        if (match_len == 0) {
            if (cursor[prefix] == '\0') break;
            cursor += prefix + 1;
        } else {
            cursor += prefix + match_len;
        }
        eflags = REG_NOTBOL;
    }
    // C12B-036: compiled regex is cache-owned; do not regfree.
    return list;
}

// Convert a byte offset within `s` to a character (codepoint) offset.
// UTF-8 aware; returns -1 if the input is malformed up to `byte_off`.
static taida_val taida_bytes_to_chars_offset(const char *s, size_t byte_off) {
    if (!s) return -1;
    size_t offset = 0;
    taida_val chars = 0;
    const unsigned char *buf = (const unsigned char*)s;
    while (offset < byte_off) {
        size_t consumed = 0;
        uint32_t cp = 0;
        if (!taida_utf8_decode_one(buf + offset, byte_off - offset, &consumed, &cp) || consumed == 0) {
            // Fallback: 1 byte = 1 "char" on malformed input.
            offset += 1;
        } else {
            offset += consumed;
        }
        chars += 1;
    }
    return chars;
}

// Build a :RegexMatch BuchiPack (`hasValue`, `full`, `groups`,
// `start`, `__type <= "RegexMatch"`). `start` is the char index of
// the first match (not the byte index), matching the JS helper.
static taida_val taida_regex_build_match_value(int matched,
                                                const char *full,
                                                taida_val start_chars,
                                                taida_val groups_list) {
    static uint64_t HASH_has_value_local = 0x9e9c6dc733414d60ULL; // HASH_HAS_VALUE
    taida_val pack = taida_pack_new(5);
    taida_pack_set_hash((taida_ptr)pack, 0, (taida_val)HASH_has_value_local);
    taida_pack_set_hash((taida_ptr)pack, 1, (taida_val)HASH_FULL);
    taida_pack_set_hash((taida_ptr)pack, 2, (taida_val)HASH_GROUPS);
    taida_pack_set_hash((taida_ptr)pack, 3, (taida_val)HASH_START);
    taida_pack_set_hash((taida_ptr)pack, 4, (taida_val)HASH___TYPE);
    taida_pack_set((taida_ptr)pack, 0, (taida_val)(matched ? 1 : 0));
    taida_pack_set_tag((taida_ptr)pack, 0, TAIDA_TAG_BOOL);
    taida_val full_str = (taida_val)taida_str_new_copy(full ? full : "");
    taida_pack_set((taida_ptr)pack, 1, full_str);
    taida_pack_set_tag((taida_ptr)pack, 1, TAIDA_TAG_STR);
    // groups list was already constructed by the caller via
    // taida_list_new / taida_list_push; retain happens implicitly
    // via our pack_set below.
    taida_pack_set((taida_ptr)pack, 2, groups_list);
    taida_pack_set_tag((taida_ptr)pack, 2, TAIDA_TAG_LIST);
    taida_pack_set((taida_ptr)pack, 3, start_chars);
    taida_pack_set_tag((taida_ptr)pack, 3, TAIDA_TAG_INT);
    taida_val type_str = (taida_val)taida_str_new_copy("RegexMatch");
    taida_pack_set((taida_ptr)pack, 4, type_str);
    taida_pack_set_tag((taida_ptr)pack, 4, TAIDA_TAG_STR);
    return pack;
}

// ── Polymorphic dispatchers — called by lowered Str methods ────
// (C12-6c): the 2nd arg is either a Str pointer or a :Regex pack.

taida_val taida_str_split_poly(const char *s, taida_val sep) {
    if (taida_val_is_regex_pack(sep)) {
        const char *pat, *flg;
        taida_regex_get_fields(sep, &pat, &flg);
        return taida_regex_split_impl(s ? s : "", pat, flg);
    }
    return taida_str_split(s, (const char*)sep);
}

taida_val taida_str_replace_first_poly(const char *s, taida_val target, const char *rep) {
    if (taida_val_is_regex_pack(target)) {
        const char *pat, *flg;
        taida_regex_get_fields(target, &pat, &flg);
        return taida_regex_replace_first_impl(s ? s : "", pat, flg, rep ? rep : "");
    }
    return taida_str_replace_first(s, (const char*)target, rep);
}

taida_val taida_str_replace_poly(const char *s, taida_val target, const char *rep) {
    if (taida_val_is_regex_pack(target)) {
        const char *pat, *flg;
        taida_regex_get_fields(target, &pat, &flg);
        return taida_regex_replace_all_impl(s ? s : "", pat, flg, rep ? rep : "");
    }
    return taida_str_replace(s, (const char*)target, rep);
}

taida_val taida_str_match_regex(const char *s, taida_val regex_pack) {
    // Build default empty-match pack up front so both failure paths
    // emit the same shape.
    if (!s || !taida_val_is_regex_pack(regex_pack)) {
        taida_val empty_list = taida_list_new();
        taida_list_set_elem_tag(empty_list, TAIDA_TAG_STR);
        return taida_regex_build_match_value(0, "", -1, empty_list);
    }
    const char *pat, *flg;
    taida_regex_get_fields(regex_pack, &pat, &flg);
    // C12B-036: shared compiled regex from cache; do not regfree.
    const regex_t *re = taida_regex_acquire(pat, flg);
    if (!re) {
        taida_val empty_list = taida_list_new();
        taida_list_set_elem_tag(empty_list, TAIDA_TAG_STR);
        return taida_regex_build_match_value(0, "", -1, empty_list);
    }
    // Allow up to 16 capture groups (design lock says no PCRE
    // look-around; 16 groups is ample for Phase 2-3 scope).
    regmatch_t matches[16];
    if (regexec(re, s, 16, matches, 0) != 0 || matches[0].rm_so < 0) {
        taida_val empty_list = taida_list_new();
        taida_list_set_elem_tag(empty_list, TAIDA_TAG_STR);
        return taida_regex_build_match_value(0, "", -1, empty_list);
    }
    size_t full_len = (size_t)(matches[0].rm_eo - matches[0].rm_so);
    char *full_buf = (char*)TAIDA_MALLOC(full_len + 1, "regex_match full");
    memcpy(full_buf, s + matches[0].rm_so, full_len);
    full_buf[full_len] = '\0';
    taida_val start_chars = taida_bytes_to_chars_offset(s, (size_t)matches[0].rm_so);
    // Groups list.
    taida_val groups_list = taida_list_new();
    taida_list_set_elem_tag(groups_list, TAIDA_TAG_STR);
    for (int i = 1; i < 16; i++) {
        if (matches[i].rm_so < 0) {
            // No more groups available — but we must keep pushing
            // empty strings for missing groups only if there are
            // *registered* groups at position i. POSIX re_nsub
            // indicates number of sub-expressions. Stop at the first
            // rm_so < 0 after re_nsub boundary.
            if ((size_t)i > re->re_nsub) break;
            char *empty = taida_str_alloc(0);
            groups_list = taida_list_push(groups_list, (taida_val)empty);
            continue;
        }
        size_t gl = (size_t)(matches[i].rm_eo - matches[i].rm_so);
        char *g = taida_str_alloc(gl);
        memcpy(g, s + matches[i].rm_so, gl);
        groups_list = taida_list_push(groups_list, (taida_val)g);
    }
    // C12B-036: compiled regex is cache-owned; do not regfree.
    taida_val out = taida_regex_build_match_value(1, full_buf, start_chars, groups_list);
    free(full_buf);
    return out;
}

taida_val taida_str_search_regex(const char *s, taida_val regex_pack) {
    if (!s || !taida_val_is_regex_pack(regex_pack)) return -1;
    const char *pat, *flg;
    taida_regex_get_fields(regex_pack, &pat, &flg);
    // C12B-036: shared compiled regex from cache; do not regfree.
    const regex_t *re = taida_regex_acquire(pat, flg);
    if (!re) return -1;
    regmatch_t m;
    if (regexec(re, s, 1, &m, 0) != 0 || m.rm_so < 0) {
        return -1;
    }
    taida_val chars = taida_bytes_to_chars_offset(s, (size_t)m.rm_so);
    return chars;
}

// ── Template string (sprintf-based) ──────────────────────
// Format: "Hello, {0}! You are {1}." with args as variadic
// We use a simpler approach: taida_template_concat builds result from parts and values
taida_val taida_str_from_int(taida_val v) {
    char tmp[32];
    snprintf(tmp, sizeof(tmp), "%" PRId64 "", v);
    return (taida_val)taida_str_new_copy(tmp);
}

taida_val taida_str_from_float(double v) {
    char tmp[64];
    snprintf(tmp, sizeof(tmp), "%g", v);
    return (taida_val)taida_str_new_copy(tmp);
}

taida_val taida_str_from_bool(taida_val v) {
    return (taida_val)taida_str_new_copy(v ? "true" : "false");
}

// ── Int methods ───────────────────────────────────────────
taida_val taida_int_abs(taida_val a) { return a < 0 ? -a : a; }

taida_val taida_int_to_str(taida_val a) {
    char tmp[32];
    snprintf(tmp, sizeof(tmp), "%" PRId64 "", a);
    return (taida_val)taida_str_new_copy(tmp);
}

taida_val taida_int_to_float(taida_val a) {
    double d = (double)a;
    taida_val result;
    memcpy(&result, &d, sizeof(taida_val));
    return result;
}

taida_val taida_int_clamp(taida_val a, taida_val lo, taida_val hi) {
    if (a < lo) return lo;
    if (a > hi) return hi;
    return a;
}

// ── Float methods ────────────────────────────────────────
double taida_float_floor(double a) { return floor(a); }
double taida_float_ceil(double a) { return ceil(a); }
double taida_float_round(double a) { return round(a); }
double taida_float_abs(double a) { return a < 0 ? -a : a; }

taida_val taida_float_to_int(double a) { return (taida_val)a; }

taida_val taida_float_to_str(double a) {
    char tmp[64];
    snprintf(tmp, sizeof(tmp), "%g", a);
    return (taida_val)taida_str_new_copy(tmp);
}

taida_val taida_float_to_fixed(double a, taida_val digits) {
    char tmp[64];
    snprintf(tmp, sizeof(tmp), "%.*f", (int)digits, a);
    return (taida_val)taida_str_new_copy(tmp);
}

double taida_float_clamp(double a, double lo, double hi) {
    if (a < lo) return lo;
    if (a > hi) return hi;
    return a;
}

// ── Num state check methods ──────────────────────────────
// For Int: isNaN=false, isInfinite=false, isFinite=true always
// For Float: need actual NaN/Inf checks

taida_val taida_int_is_positive(taida_val a) { return a > 0 ? 1 : 0; }
taida_val taida_int_is_negative(taida_val a) { return a < 0 ? 1 : 0; }
taida_val taida_int_is_zero(taida_val a) { return a == 0 ? 1 : 0; }

taida_val taida_float_is_nan(double a) { return isnan(a) ? 1 : 0; }
taida_val taida_float_is_infinite(double a) { return isinf(a) ? 1 : 0; }
taida_val taida_float_is_finite_check(double a) { return isfinite(a) ? 1 : 0; }
taida_val taida_float_is_positive(double a) { return a > 0.0 ? 1 : 0; }
taida_val taida_float_is_negative(double a) { return a < 0.0 ? 1 : 0; }
taida_val taida_float_is_zero(double a) { return a == 0.0 ? 1 : 0; }

// ── Bool methods ──────────────────────────────────────────
taida_val taida_bool_to_str(taida_val a) {
    return (taida_val)taida_str_new_copy(a ? "true" : "false");
}

taida_val taida_bool_to_int(taida_val a) { return a ? 1 : 0; }

// ── Additional List methods ──────────────────────────────
taida_val taida_list_index_of(taida_val list_ptr, taida_val item) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        if (list[4 + i] == item) return i;
    }
    return -1;
}

taida_val taida_list_last_index_of(taida_val list_ptr, taida_val item) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = len - 1; i >= 0; i--) {
        if (list[4 + i] == item) return i;
    }
    return -1;
}

taida_val taida_list_any(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[4 + i])) return 1;
    }
    return 0;
}

taida_val taida_list_all(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        if (!taida_invoke_callback1(fn_ptr, list[4 + i])) return 0;
    }
    return 1;
}

taida_val taida_list_none(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[4 + i])) return 0;
    }
    return 1;
}

taida_val taida_list_concat(taida_val list1, taida_val list2) {
    if (TAIDA_IS_BYTES(list1) && TAIDA_IS_BYTES(list2)) {
        taida_val len1 = taida_bytes_len(list1);
        taida_val len2 = taida_bytes_len(list2);
        taida_val out = taida_bytes_new_filled(len1 + len2, 0);
        taida_val *dst = (taida_val*)out;
        taida_val *a = (taida_val*)list1;
        taida_val *b = (taida_val*)list2;
        for (taida_val i = 0; i < len1; i++) dst[2 + i] = a[2 + i];
        for (taida_val i = 0; i < len2; i++) dst[2 + len1 + i] = b[2 + i];
        return out;
    }

    taida_val *l1 = (taida_val*)list1;
    taida_val *l2 = (taida_val*)list2;
    taida_val len1 = l1[2], len2 = l2[2];
    taida_val elem_tag = l1[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag from first list
    for (taida_val i = 0; i < len1; i++) {
        taida_list_elem_retain(l1[4 + i], elem_tag);
        new_list = taida_list_push(new_list, l1[4 + i]);
    }
    for (taida_val i = 0; i < len2; i++) {
        taida_list_elem_retain(l2[4 + i], elem_tag);
        new_list = taida_list_push(new_list, l2[4 + i]);
    }
    return new_list;
}

taida_val taida_list_join(taida_val list_ptr, const char* sep) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (len == 0) { char *r = taida_str_alloc(0); return (taida_val)r; }
    if (!sep) sep = "";
    size_t sep_len = strlen(sep);

    // Convert each element through the shared toString path.
    // This avoids pointer heuristics and keeps behavior consistent.
    // M-06: Overflow guard on len * sizeof + NULL check.
    size_t strs_size = taida_safe_mul((size_t)len, sizeof(const char*), "list_join strs");
    const char **strs = (const char**)TAIDA_MALLOC(strs_size, "list_join_strs");
    // M-16: Use size_t for total with overflow guards to prevent wrap-around.
    size_t total = 0;
    for (taida_val i = 0; i < len; i++) {
        strs[i] = (const char*)taida_value_to_display_string(list[4 + i]);
        total = taida_safe_add(total, strlen(strs[i]), "list_join total");
        if (i > 0) total = taida_safe_add(total, sep_len, "list_join sep");
    }

    char *r = taida_str_alloc(total);
    char *dst = r;
    for (taida_val i = 0; i < len; i++) {
        if (i > 0 && sep_len > 0) { memcpy(dst, sep, sep_len); dst += sep_len; }
        taida_val sl = (taida_val)strlen(strs[i]);
        memcpy(dst, strs[i], sl);
        dst += sl;
    }
    *dst = '\0';

    // Free temporary strings
    for (taida_val i = 0; i < len; i++) {
        taida_str_release((taida_val)strs[i]);
    }
    free(strs);

    return (taida_val)r;
}

taida_val taida_list_sort(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    if (len == 0) return new_list;
    // M-07: Overflow guard + NULL check on items allocation.
    size_t items_size = taida_safe_mul((size_t)len, sizeof(taida_val), "list_sort items");
    taida_val *items = (taida_val*)TAIDA_MALLOC(items_size, "list_sort");
    for (taida_val i = 0; i < len; i++) items[i] = list[4 + i];
    // Simple insertion sort
    for (taida_val i = 1; i < len; i++) {
        taida_val key = items[i];
        taida_val j = i - 1;
        while (j >= 0 && items[j] > key) { items[j+1] = items[j]; j--; }
        items[j+1] = key;
    }
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(items[i], elem_tag);
        new_list = taida_list_push(new_list, items[i]);
    }
    free(items);
    return new_list;
}

taida_val taida_list_unique(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl_init = (taida_val*)new_list;
    nl_init[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        // Check if already in new_list
        taida_val *nl = (taida_val*)new_list;
        taida_val nlen = nl[2];
        taida_val found = 0;
        for (taida_val j = 0; j < nlen; j++) {
            if (nl[4 + j] == item) { found = 1; break; }
        }
        if (!found) {
            taida_list_elem_retain(item, elem_tag);
            new_list = taida_list_push(new_list, item);
        }
    }
    return new_list;
}

taida_val taida_list_flatten(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val new_list = taida_list_new();
    // flatten changes nesting level, propagate inner list's elem_tag if possible
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        if (TAIDA_IS_LIST(item)) {
            taida_val *sub = (taida_val*)item;
            taida_val slen = sub[2];
            taida_val sub_tag = sub[3];
            // Propagate inner list's elem_tag to result
            if (i == 0) {
                taida_val *nl = (taida_val*)new_list;
                nl[3] = sub_tag;
            }
            for (taida_val j = 0; j < slen; j++) {
                taida_list_elem_retain(sub[4 + j], sub_tag);
                new_list = taida_list_push(new_list, sub[4 + j]);
            }
        } else {
            // Non-list element: retain using outer list's elem_tag
            taida_list_elem_retain(item, list[3]);
            new_list = taida_list_push(new_list, item);
        }
    }
    return new_list;
}

taida_val taida_list_max(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (len == 0) return taida_lax_empty(0);
    taida_val max = list[4];
    for (taida_val i = 1; i < len; i++) {
        if (list[4 + i] > max) max = list[4 + i];
    }
    return taida_lax_new(max, 0);
}

taida_val taida_list_min(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (len == 0) return taida_lax_empty(0);
    taida_val min = list[4];
    for (taida_val i = 1; i < len; i++) {
        if (list[4 + i] < min) min = list[4 + i];
    }
    return taida_lax_new(min, 0);
}

// ── Additional List mold operations ──────────────────────

taida_val taida_list_append(taida_val list_ptr, taida_val item) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    // New item: no retain (ownership transferred from caller)
    new_list = taida_list_push(new_list, item);
    return new_list;
}

taida_val taida_list_prepend(taida_val list_ptr, taida_val item) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    // New item: no retain (ownership transferred from caller)
    new_list = taida_list_push(new_list, item);
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    return new_list;
}

taida_val taida_list_sort_desc(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    if (len == 0) return new_list;
    // M-07: Overflow guard + NULL check on items allocation.
    size_t items_size = taida_safe_mul((size_t)len, sizeof(taida_val), "list_sort_desc items");
    taida_val *items = (taida_val*)TAIDA_MALLOC(items_size, "list_sort_desc");
    for (taida_val i = 0; i < len; i++) items[i] = list[4 + i];
    // Insertion sort descending
    for (taida_val i = 1; i < len; i++) {
        taida_val key = items[i];
        taida_val j = i - 1;
        while (j >= 0 && items[j] < key) { items[j+1] = items[j]; j--; }
        items[j+1] = key;
    }
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(items[i], elem_tag);
        new_list = taida_list_push(new_list, items[i]);
    }
    free(items);
    return new_list;
}

/* Sort by key extraction function: fn_ptr maps each element to a sort key,
   then sort ascending by key. Matches interpreter's Sort[list](by <= fn). */
taida_val taida_list_sort_by(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;
    if (len == 0) return new_list;
    size_t items_size = taida_safe_mul((size_t)len, sizeof(taida_val), "list_sort_by items");
    taida_val *items = (taida_val*)TAIDA_MALLOC(items_size, "list_sort_by items");
    taida_val *keys = (taida_val*)TAIDA_MALLOC(items_size, "list_sort_by keys");
    for (taida_val i = 0; i < len; i++) {
        items[i] = list[4 + i];
        keys[i] = taida_invoke_callback1(fn_ptr, items[i]);
    }
    /* Insertion sort ascending by key */
    for (taida_val i = 1; i < len; i++) {
        taida_val kkey = keys[i];
        taida_val kitem = items[i];
        taida_val j = i - 1;
        while (j >= 0 && keys[j] > kkey) {
            keys[j+1] = keys[j];
            items[j+1] = items[j];
            j--;
        }
        keys[j+1] = kkey;
        items[j+1] = kitem;
    }
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(items[i], elem_tag);
        new_list = taida_list_push(new_list, items[i]);
    }
    free(items);
    free(keys);
    return new_list;
}

taida_val taida_list_find(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        if (taida_invoke_callback1(fn_ptr, item)) {
            return taida_lax_new(item, 0);
        }
    }
    return taida_lax_empty(0);
}

taida_val taida_list_find_index(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[4 + i])) return i;
    }
    return -1;
}

taida_val taida_list_count(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val count = 0;
    for (taida_val i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[4 + i])) count++;
    }
    return count;
}

// list.fold(init, fn) — left fold: fn takes (acc, item) -> acc
taida_val taida_list_fold(taida_val list_ptr, taida_val init, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val acc = init;
    for (taida_val i = 0; i < len; i++) {
        acc = taida_invoke_callback2(fn_ptr, acc, list[4 + i]);
    }
    return acc;
}

// list.foldr(init, fn) — right fold: fn takes (acc, item) -> acc
taida_val taida_list_foldr(taida_val list_ptr, taida_val init, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val acc = init;
    for (taida_val i = len - 1; i >= 0; i--) {
        acc = taida_invoke_callback2(fn_ptr, acc, list[4 + i]);
    }
    return acc;
}

// Take[list, n]() — first n elements
taida_val taida_list_take(taida_val list_ptr, taida_val n) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val take_n = n < len ? n : len;
    if (take_n < 0) take_n = 0;
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = 0; i < take_n; i++) {
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    return new_list;
}

// TakeWhile[list, fn]() — take while fn returns truthy
taida_val taida_list_take_while(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[4 + i])) {
            taida_list_elem_retain(list[4 + i], elem_tag);
            new_list = taida_list_push(new_list, list[4 + i]);
        } else {
            break;
        }
    }
    return new_list;
}

// Drop[list, n]() — skip first n elements
taida_val taida_list_drop(taida_val list_ptr, taida_val n) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val skip = n < len ? n : len;
    if (skip < 0) skip = 0;
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    for (taida_val i = skip; i < len; i++) {
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    return new_list;
}

// DropWhile[list, fn]() — skip while fn returns truthy
taida_val taida_list_drop_while(taida_val list_ptr, taida_val fn_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = elem_tag;  // propagate elem_type_tag
    taida_val dropping = 1;
    for (taida_val i = 0; i < len; i++) {
        if (dropping && taida_invoke_callback1(fn_ptr, list[4 + i])) {
            continue;
        }
        dropping = 0;
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    return new_list;
}

// FNV-1a hashes for Zip/Enumerate BuchiPack fields
#define HASH_FIRST  0x89d7ed7f996f1d41ULL
#define HASH_SECOND 0xa49985ef4cee20bdULL
#define HASH_INDEX  0x83cf8e8f9081468bULL
#define HASH_VALUE  0x7ce4fd9430e80ceaULL

taida_val taida_list_zip(taida_val list1, taida_val list2) {
    taida_val *l1 = (taida_val*)list1;
    taida_val *l2 = (taida_val*)list2;
    taida_val len1 = l1[2], len2 = l2[2];
    taida_val min_len = len1 < len2 ? len1 : len2;
    taida_val elem_tag1 = l1[3];  // ソースリスト1の要素型タグ
    taida_val elem_tag2 = l2[3];  // ソースリスト2の要素型タグ
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = TAIDA_TAG_PACK;  // zip produces Pack elements
    for (taida_val i = 0; i < min_len; i++) {
        // Create a BuchiPack with fields: first, second
        taida_val pair = taida_pack_new(2);
        pair = taida_pack_set_hash(pair, 0, (taida_val)HASH_FIRST);
        pair = taida_pack_set(pair, 0, l1[4 + i]);
        // tag + retain for first field based on source list's elem_type_tag
        taida_pack_set_tag(pair, 0, elem_tag1);
        taida_list_elem_retain(l1[4 + i], elem_tag1);
        pair = taida_pack_set_hash(pair, 1, (taida_val)HASH_SECOND);
        pair = taida_pack_set(pair, 1, l2[4 + i]);
        // tag + retain for second field based on source list's elem_type_tag
        taida_pack_set_tag(pair, 1, elem_tag2);
        taida_list_elem_retain(l2[4 + i], elem_tag2);
        new_list = taida_list_push(new_list, pair);
    }
    return new_list;
}

taida_val taida_list_enumerate(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];  // ソースリストの要素型タグ
    taida_val new_list = taida_list_new();
    taida_val *nl = (taida_val*)new_list;
    nl[3] = TAIDA_TAG_PACK;  // enumerate produces Pack elements
    for (taida_val i = 0; i < len; i++) {
        // Create a BuchiPack with fields: index, value
        taida_val pair = taida_pack_new(2);
        pair = taida_pack_set_hash(pair, 0, (taida_val)HASH_INDEX);
        pair = taida_pack_set(pair, 0, i);
        // index は INT なのでタグはデフォルト(0)のまま、retain 不要
        pair = taida_pack_set_hash(pair, 1, (taida_val)HASH_VALUE);
        pair = taida_pack_set(pair, 1, list[4 + i]);
        // value フィールドにソースリストの elem_type_tag に基づいて tag + retain
        taida_pack_set_tag(pair, 1, elem_tag);
        taida_list_elem_retain(list[4 + i], elem_tag);
        new_list = taida_list_push(new_list, pair);
    }
    return new_list;
}

// ── HashMap runtime ──────────────────────────────────────
// HashMap layout: [tag, capacity, length, value_type_tag, (key_hash, key_ptr, value)...]
// Header slots: [0]=magic+rc, [1]=capacity, [2]=length, [3]=value_type_tag
// Entry offset = 4. Entry stride = 3: [key_hash, key_ptr, value]
// Open-addressing hash map with linear probing.
// Tombstone: hash = 1, key = 0 (marker for deleted slots).
// Ownership contract (NO-1):
//   - key is always Str: taida_str_retain on store, taida_str_release on remove/drop
//   - value uses value_type_tag: retain-on-store, release-on-remove/drop
//   - clone retains all keys+values; resize is a move (no retain/release)
//   - taida_release recursively releases all keys+values when rc<=1

// (HM_HEADER, TAIDA_HASHMAP_TOMBSTONE_HASH, HM_SLOT_* macros defined earlier)

// Retain a HashMap value based on the map's value_type_tag.
static void taida_hashmap_val_retain(taida_val val, taida_val val_tag) {
    if (val_tag == TAIDA_TAG_PACK || val_tag == TAIDA_TAG_LIST || val_tag == TAIDA_TAG_CLOSURE || val_tag == TAIDA_TAG_HMAP || val_tag == TAIDA_TAG_SET) {
        if (val > 4096) taida_retain(val);
    } else if (val_tag == TAIDA_TAG_STR) {
        if (val > 4096) taida_str_retain(val);
    }
    // INT, FLOAT, BOOL, UNKNOWN → no-op
}

// Release a HashMap value based on the map's value_type_tag.
static void taida_hashmap_val_release(taida_val val, taida_val val_tag) {
    if (val_tag == TAIDA_TAG_PACK || val_tag == TAIDA_TAG_LIST || val_tag == TAIDA_TAG_CLOSURE || val_tag == TAIDA_TAG_HMAP || val_tag == TAIDA_TAG_SET) {
        if (val > 4096) taida_release(val);
    } else if (val_tag == TAIDA_TAG_STR) {
        if (val > 4096) taida_str_release(val);
    }
    // INT, FLOAT, BOOL, UNKNOWN → no-op
}

// Retain a HashMap key (always Str).
static void taida_hashmap_key_retain(taida_val key) {
    if (key > 4096) taida_str_retain(key);
}

// Release a HashMap key (always Str).
static void taida_hashmap_key_release(taida_val key) {
    if (key > 4096) taida_str_release(key);
}

static taida_val taida_hashmap_adjust_hash(taida_val h) {
    // Avoid 0 (empty) and 1 (tombstone)
    if (h == 0) return 42424242L;
    if (h == 1) return 14141414L;
    return h;
}

// Slot helpers: HM_SLOT_* macros defined in header section above

static taida_val taida_is_hashmap(taida_val ptr) {
    return TAIDA_IS_HMAP(ptr);
}

static int taida_ptr_is_readable(taida_val ptr, size_t bytes) {
    if (ptr == 0 || ptr < 4096) return 0;
    // Taida heap objects are always 8-byte aligned.
    if (ptr & 0x7) return 0;
    if (bytes == 0) return 1;

    uintptr_t start = (uintptr_t)ptr;
    if (start > UINTPTR_MAX - (bytes - 1)) return 0;
    uintptr_t end = start + (bytes - 1);

    taida_val page_size = sysconf(_SC_PAGESIZE);
    if (page_size <= 0) page_size = 4096;
    uintptr_t step = (uintptr_t)page_size;
    uintptr_t page_mask = step - 1;
    uintptr_t page = start & ~page_mask;
    uintptr_t last_page = end & ~page_mask;

    for (;;) {
        unsigned char vec = 0;
        if (mincore((void*)page, (size_t)page_size, &vec) != 0) {
            return 0;
        }
        if (page == last_page) break;
        if (page > UINTPTR_MAX - step) return 0;
        page += step;
    }
    return 1;
}

// NB-31: Check if a taida_val is callable (function pointer or closure).
// Uses negative logic: reject values that are DEFINITELY not callable.
// Function pointers may not be 8-byte aligned, so we cannot use taida_ptr_is_readable
// as a positive gate (it requires 8-byte alignment for heap objects).
static int _taida_is_callable_impl(taida_val val) {
    // Closures are always callable
    if (TAIDA_IS_CLOSURE(val)) return 1;
    // Small non-negative integers (covers most user-facing Int values including 42, 50000)
    if (val >= 0 && val <= 65535) return 0;
    // Negative integers
    if (val < 0) return 0;
    // 8-byte aligned + readable → check for known heap data types
    if ((val & 0x7) == 0 && taida_ptr_is_readable(val, 8)) {
        taida_val magic = ((taida_val*)val)[0] & TAIDA_MAGIC_MASK;
        if (magic == TAIDA_PACK_MAGIC || magic == TAIDA_LIST_MAGIC ||
            magic == TAIDA_STR_MAGIC || magic == TAIDA_HMAP_MAGIC ||
            magic == TAIDA_SET_MAGIC || magic == TAIDA_ASYNC_MAGIC ||
            magic == TAIDA_BYTES_MAGIC) return 0;
    }
    // Assume callable: function pointer or large integer (rare edge case)
    return 1;
}

static int taida_read_cstr_len_safe(const char *s, size_t max_len, size_t *out_len) {
    if (!s) return 0;
    uintptr_t ptr = (uintptr_t)s;
    if (ptr < 4096) return 0;

    taida_val page_size = sysconf(_SC_PAGESIZE);
    if (page_size <= 0) page_size = 4096;
    uintptr_t page_mask = (uintptr_t)(page_size - 1);
    uintptr_t current_page = 0;

    for (size_t i = 0; i < max_len; i++) {
        uintptr_t addr = ptr + i;
        uintptr_t page = addr & ~page_mask;
        if (page != current_page) {
            unsigned char vec = 0;
            if (mincore((void*)page, (size_t)page_size, &vec) != 0) {
                return 0;
            }
            current_page = page;
        }
        unsigned char ch = ((const unsigned char*)s)[i];
        if (ch == 0) {
            if (out_len) *out_len = i;
            return 1;
        }
    }
    return 0;
}

// NB3-8: Get string byte length from heap header metadata when available,
// falling back to taida_read_cstr_len_safe for static strings.
// Returns 1 on success (length stored in *out_len), 0 on failure.
static int taida_str_byte_len(const char *s, size_t *out_len) {
    if (!s) return 0;
    uintptr_t ptr = (uintptr_t)s;
    if (ptr < 4096) return 0;
    // Check if this is a heap string with hidden header
    taida_val *hdr = ((taida_val*)s) - 2;
    if (taida_ptr_is_readable((taida_val)hdr, sizeof(taida_val) * 2)) {
        taida_val tag = hdr[0];
        if ((tag & TAIDA_MAGIC_MASK) == TAIDA_STR_MAGIC) {
            if (out_len) *out_len = (size_t)hdr[1];
            return 1;
        }
    }
    // Static string — fall back to NUL scan
    return taida_read_cstr_len_safe(s, 16 * 1024 * 1024, out_len);
}

static int taida_hashmap_key_valid(taida_val key_ptr) {
    // All values are valid keys in Taida.
    // Null (0) is traditionally not a key, but we can allow it as Int(0).
    return 1;
}

// FNV-1a hash for string keys (runtime computation)
taida_val taida_str_hash(taida_val str_ptr) {
    const unsigned char *s = (const unsigned char*)str_ptr;
    size_t len = 0;
    if (!taida_read_cstr_len_safe((const char*)s, 8192, &len)) return 0;

    uint64_t hash = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < len; i++) {
        hash ^= s[i];
        hash *= 0x100000001b3ULL;
    }
    return (taida_val)hash;
}

taida_val taida_value_hash(taida_val val) {
    size_t len = 0;
    taida_val h = val;
    // Check if it's a valid string pointer
    if (taida_read_cstr_len_safe((const char*)val, 8192, &len)) {
        h = taida_str_hash(val);
    }
    // Identity hash for scalars (ints/floats), or FNV-1a for strings.
    // ALWAYS adjust to avoid 0/1.
    return taida_hashmap_adjust_hash(h);
}

static int taida_hashmap_key_eq(taida_val key_a, taida_val key_b) {
    if (key_a == key_b) return 1;
    // For string comparison, ensure both are valid pointers
    if (!taida_hashmap_key_valid(key_a) || !taida_hashmap_key_valid(key_b)) return 0;

    const char *sa = (const char*)key_a;
    const char *sb = (const char*)key_b;
    size_t la = 0, lb = 0;
    if (!taida_read_cstr_len_safe(sa, 8192, &la)) return 0;
    if (!taida_read_cstr_len_safe(sb, 8192, &lb)) return 0;
    if (la != lb) return 0;
    return memcmp(sa, sb, la) == 0;
}

static taida_val taida_hashmap_new_with_cap(taida_val cap) {
    // M-02: Guard against non-positive cap and cap * 3 overflow.
    if (cap <= 0) cap = 16;
    size_t slots = taida_safe_add((size_t)HM_HEADER, taida_safe_mul((size_t)cap, 3, "hm_new_with_cap slots"), "hm_new_with_cap total");
    size_t alloc_size = taida_safe_mul(slots, sizeof(taida_val), "hm_new_with_cap bytes");
    taida_val *hm = (taida_val*)calloc(1, alloc_size);
    if (!hm) { fprintf(stderr, "taida: out of memory (taida_hashmap_new_with_cap)\n"); exit(1); }
    hm[0] = TAIDA_HMAP_MAGIC | 1;  // Magic + refcount
    hm[1] = cap;  // capacity
    hm[2] = 0;    // length
    hm[3] = TAIDA_TAG_UNKNOWN;  // value_type_tag (unknown until set)
    return (taida_val)hm;
}

void taida_hashmap_set_value_tag(taida_val hm_ptr, taida_val tag) {
    ((taida_val*)hm_ptr)[3] = tag;
}

taida_val taida_hashmap_new(void) {
    return taida_hashmap_new_with_cap(16);
}

// Internal set used by resize (does not trigger resize)
static void taida_hashmap_set_internal(taida_val *hm, taida_val cap, taida_val key_hash, taida_val key_ptr, taida_val value) {
    uint64_t uh = (uint64_t)key_hash;
    taida_val idx = (taida_val)(uh % (uint64_t)cap);
    for (taida_val i = 0; i < cap; i++) {
        taida_val slot = (idx + i) % cap;
        taida_val sh = hm[HM_HEADER + slot * 3];
        taida_val sk = hm[HM_HEADER + slot * 3 + 1];
        if (HM_SLOT_EMPTY(sh, sk)) {
            hm[HM_HEADER + slot * 3] = key_hash;
            hm[HM_HEADER + slot * 3 + 1] = key_ptr;
            hm[HM_HEADER + slot * 3 + 2] = value;
            hm[2]++;
            return;
        }
        if (sh == key_hash && taida_hashmap_key_eq(sk, key_ptr)) {
            hm[HM_HEADER + slot * 3 + 2] = value;
            return;
        }
    }
}

// Resize the hashmap to new_cap (re-hash all occupied entries)
// This is a MOVE operation — entries transfer ownership from old to new.
// No retain/release needed; the old table's raw memory is freed.
static taida_val taida_hashmap_resize(taida_val hm_ptr, taida_val new_cap) {
    taida_val *old_hm = (taida_val*)hm_ptr;
    taida_val old_cap = old_hm[1];
    taida_val new_hm_ptr = taida_hashmap_new_with_cap(new_cap);
    taida_val *new_hm = (taida_val*)new_hm_ptr;
    // NO-1: propagate value_type_tag from old to new
    new_hm[3] = old_hm[3];
    for (taida_val i = 0; i < old_cap; i++) {
        taida_val sh = old_hm[HM_HEADER + i * 3];
        taida_val sk = old_hm[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            taida_hashmap_set_internal(new_hm, new_cap, sh, sk, old_hm[HM_HEADER + i * 3 + 2]);
        }
    }
    free(old_hm);
    return new_hm_ptr;
}

taida_val taida_hashmap_set(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr, taida_val value) {
    if (!taida_hashmap_key_valid(key_ptr)) {
        return hm_ptr;
    }

    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    taida_val len = hm[2];

    // Load factor check: resize at 0.75 (F-21 fix)
    if (len * 4 >= cap * 3) {
        hm_ptr = taida_hashmap_resize(hm_ptr, cap * 2);
        hm = (taida_val*)hm_ptr;
        cap = hm[1];
    }

    taida_val val_tag = hm[3];  // value_type_tag
    uint64_t uh = (uint64_t)key_hash;
    taida_val idx = (taida_val)(uh % (uint64_t)cap);
    taida_val first_tombstone = -1;
    for (taida_val i = 0; i < cap; i++) {
        taida_val slot = (idx + i) % cap;
        taida_val sh = hm[HM_HEADER + slot * 3];
        taida_val sk = hm[HM_HEADER + slot * 3 + 1];
        if (HM_SLOT_EMPTY(sh, sk)) {
            // Insert at tombstone if we passed one, else at this empty slot
            taida_val target = (first_tombstone >= 0) ? first_tombstone : slot;
            hm[HM_HEADER + target * 3] = key_hash;
            hm[HM_HEADER + target * 3 + 1] = key_ptr;
            hm[HM_HEADER + target * 3 + 2] = value;
            // NO-1: retain-on-store for new entry
            taida_hashmap_key_retain(key_ptr);
            taida_hashmap_val_retain(value, val_tag);
            hm[2]++;
            return hm_ptr;
        }
        if (HM_SLOT_TOMBSTONE(sh, sk)) {
            if (first_tombstone < 0) first_tombstone = slot;
            continue;  // skip tombstone, keep probing
        }
        // Occupied slot: compare hash AND key (F-19 fix)
        if (sh == key_hash && taida_hashmap_key_eq(sk, key_ptr)) {
            // NO-1: release old value, retain new value on overwrite
            taida_val old_val = hm[HM_HEADER + slot * 3 + 2];
            taida_hashmap_val_release(old_val, val_tag);
            taida_hashmap_val_retain(value, val_tag);
            hm[HM_HEADER + slot * 3 + 2] = value;
            return hm_ptr;
        }
    }
    // Table full of tombstones — insert at first tombstone
    if (first_tombstone >= 0) {
        hm[HM_HEADER + first_tombstone * 3] = key_hash;
        hm[HM_HEADER + first_tombstone * 3 + 1] = key_ptr;
        hm[HM_HEADER + first_tombstone * 3 + 2] = value;
        // NO-1: retain-on-store for new entry
        taida_hashmap_key_retain(key_ptr);
        taida_hashmap_val_retain(value, val_tag);
        hm[2]++;
    }
    return hm_ptr;
}

taida_val taida_hashmap_get(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr) {
    if (!taida_hashmap_key_valid(key_ptr)) return 0;

    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    uint64_t uh = (uint64_t)key_hash;
    taida_val idx = (taida_val)(uh % (uint64_t)cap);
    for (taida_val i = 0; i < cap; i++) {
        taida_val slot = (idx + i) % cap;
        taida_val sh = hm[HM_HEADER + slot * 3];
        taida_val sk = hm[HM_HEADER + slot * 3 + 1];
        if (HM_SLOT_EMPTY(sh, sk)) return 0; // not found
        if (HM_SLOT_TOMBSTONE(sh, sk)) continue; // skip tombstone
        if (sh == key_hash && taida_hashmap_key_eq(sk, key_ptr)) return hm[HM_HEADER + slot * 3 + 2];
    }
    return 0;
}

taida_val taida_hashmap_has(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr) {
    if (!taida_hashmap_key_valid(key_ptr)) return 0;

    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    uint64_t uh = (uint64_t)key_hash;
    taida_val idx = (taida_val)(uh % (uint64_t)cap);
    for (taida_val i = 0; i < cap; i++) {
        taida_val slot = (idx + i) % cap;
        taida_val sh = hm[HM_HEADER + slot * 3];
        taida_val sk = hm[HM_HEADER + slot * 3 + 1];
        if (HM_SLOT_EMPTY(sh, sk)) return 0;
        if (HM_SLOT_TOMBSTONE(sh, sk)) continue;
        if (sh == key_hash && taida_hashmap_key_eq(sk, key_ptr)) return 1;
    }
    return 0;
}

taida_val taida_hashmap_remove(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr) {
    if (!taida_hashmap_key_valid(key_ptr)) return hm_ptr;

    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    taida_val val_tag = hm[3];  // value_type_tag
    uint64_t uh = (uint64_t)key_hash;
    taida_val idx = (taida_val)(uh % (uint64_t)cap);
    for (taida_val i = 0; i < cap; i++) {
        taida_val slot = (idx + i) % cap;
        taida_val sh = hm[HM_HEADER + slot * 3];
        taida_val sk = hm[HM_HEADER + slot * 3 + 1];
        if (HM_SLOT_EMPTY(sh, sk)) return hm_ptr;
        if (HM_SLOT_TOMBSTONE(sh, sk)) continue;
        if (sh == key_hash && taida_hashmap_key_eq(sk, key_ptr)) {
            // NO-1: release key and value before tombstoning
            taida_hashmap_key_release(sk);
            taida_hashmap_val_release(hm[HM_HEADER + slot * 3 + 2], val_tag);
            // Set tombstone marker (F-20 fix)
            hm[HM_HEADER + slot * 3] = TAIDA_HASHMAP_TOMBSTONE_HASH;
            hm[HM_HEADER + slot * 3 + 1] = 0;
            hm[HM_HEADER + slot * 3 + 2] = 0;
            hm[2]--;
            return hm_ptr;
        }
    }
    return hm_ptr;
}

taida_val taida_hashmap_keys(taida_val hm_ptr) {
    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    taida_val list = taida_list_new();
    // NO-1: keys are always Str — set elem_type_tag and retain each key
    ((taida_val*)list)[3] = TAIDA_TAG_STR;
    for (taida_val i = 0; i < cap; i++) {
        taida_val sh = hm[HM_HEADER + i * 3];
        taida_val sk = hm[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            taida_hashmap_key_retain(sk);
            list = taida_list_push(list, sk);
        }
    }
    return list;
}

taida_val taida_hashmap_values(taida_val hm_ptr) {
    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    taida_val val_tag = hm[3];  // value_type_tag
    taida_val list = taida_list_new();
    // NO-1: propagate value_type_tag to the returned list and retain each value
    ((taida_val*)list)[3] = val_tag;
    for (taida_val i = 0; i < cap; i++) {
        taida_val sh = hm[HM_HEADER + i * 3];
        taida_val sk = hm[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            taida_val v = hm[HM_HEADER + i * 3 + 2];
            taida_hashmap_val_retain(v, val_tag);
            list = taida_list_push(list, v);
        }
    }
    return list;
}

taida_val taida_hashmap_length(taida_val hm_ptr) {
    return ((taida_val*)hm_ptr)[2];
}

taida_val taida_hashmap_is_empty(taida_val hm_ptr) {
    return ((taida_val*)hm_ptr)[2] == 0 ? 1 : 0;
}

// Clone a hashmap (for immutable set/remove/merge semantics)
// Cloned entries share ownership with the original, so retain all keys+values.
static taida_val taida_hashmap_clone(taida_val hm_ptr) {
    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    taida_val val_tag = hm[3];  // value_type_tag
    // M-03: Guard against negative/overflow cap and NULL malloc result.
    if (cap < 0) {
        fprintf(stderr, "taida: invalid hashmap cap %" PRId64 " in taida_hashmap_clone\n", (int64_t)cap);
        exit(1);
    }
    size_t total = taida_safe_add((size_t)HM_HEADER, taida_safe_mul((size_t)cap, 3, "hm_clone slots"), "hm_clone total");
    size_t alloc_size = taida_safe_mul(total, sizeof(taida_val), "hm_clone bytes");
    taida_val *new_hm = (taida_val*)TAIDA_MALLOC(alloc_size, "hashmap_clone");
    memcpy(new_hm, hm, alloc_size);
    new_hm[0] = TAIDA_HMAP_MAGIC | 1;  // preserve magic + reset rc
    // Retain all keys and values in the clone (shared ownership)
    for (taida_val i = 0; i < cap; i++) {
        taida_val sh = new_hm[HM_HEADER + i * 3];
        taida_val sk = new_hm[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            taida_hashmap_key_retain(sk);
            taida_hashmap_val_retain(new_hm[HM_HEADER + i * 3 + 2], val_tag);
        }
    }
    return (taida_val)new_hm;
}

// Immutable set: clone then set
taida_val taida_hashmap_set_immut(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr, taida_val value) {
    taida_val new_hm = taida_hashmap_clone(hm_ptr);
    return taida_hashmap_set(new_hm, key_hash, key_ptr, value);
}

// Immutable remove: clone then remove
taida_val taida_hashmap_remove_immut(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr) {
    taida_val new_hm = taida_hashmap_clone(hm_ptr);
    return taida_hashmap_remove(new_hm, key_hash, key_ptr);
}

// Get returning Lax (Lax[value]() or empty Lax)
taida_val taida_hashmap_get_lax(taida_val hm_ptr, taida_val key_hash, taida_val key_ptr) {
    if (!taida_hashmap_key_valid(key_ptr)) return taida_lax_empty(0);

    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    uint64_t uh = (uint64_t)key_hash;
    taida_val idx = (taida_val)(uh % (uint64_t)cap);
    for (taida_val i = 0; i < cap; i++) {
        taida_val slot = (idx + i) % cap;
        taida_val sh = hm[HM_HEADER + slot * 3];
        taida_val sk = hm[HM_HEADER + slot * 3 + 1];
        if (HM_SLOT_EMPTY(sh, sk)) return taida_lax_empty(0);
        if (HM_SLOT_TOMBSTONE(sh, sk)) continue;
        if (sh == key_hash && taida_hashmap_key_eq(sk, key_ptr)) return taida_lax_new(hm[HM_HEADER + slot * 3 + 2], 0);
    }
    return taida_lax_empty(0);
}

// Entries: returns list of BuchiPack @(key, value)
taida_val taida_hashmap_entries(taida_val hm_ptr) {
    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];
    taida_val val_tag = hm[3];  // value_type_tag
    taida_val list = taida_list_new();
    // NO-1: entries returns List[Pack] — set elem_type_tag = PACK
    ((taida_val*)list)[3] = TAIDA_TAG_PACK;
    // FNV-1a hashes for "key" and "value"
    #define HASH_KEY   0x3dc94a19365b10ecULL
    #define HASH_VAL   0x7ce4fd9430e80ceaULL
    for (taida_val i = 0; i < cap; i++) {
        taida_val sh = hm[HM_HEADER + i * 3];
        taida_val sk = hm[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            taida_val pair = taida_pack_new(2);
            taida_pack_set_hash(pair, 0, (taida_val)HASH_KEY);
            // NO-1: tag + retain key (Str) and value fields in pair pack
            taida_pack_set_tag(pair, 0, TAIDA_TAG_STR);
            taida_hashmap_key_retain(sk);
            taida_pack_set(pair, 0, sk);
            taida_pack_set_hash(pair, 1, (taida_val)HASH_VAL);
            taida_pack_set_tag(pair, 1, val_tag);
            taida_val v = hm[HM_HEADER + i * 3 + 2];
            taida_hashmap_val_retain(v, val_tag);
            taida_pack_set(pair, 1, v);
            list = taida_list_push(list, pair);
        }
    }
    return list;
}

// Merge two hashmaps (other overwrites this)
taida_val taida_hashmap_merge(taida_val hm_ptr, taida_val other_ptr) {
    taida_val new_hm = taida_hashmap_clone(hm_ptr);
    taida_val *other = (taida_val*)other_ptr;
    taida_val cap = other[1];
    for (taida_val i = 0; i < cap; i++) {
        taida_val sh = other[HM_HEADER + i * 3];
        taida_val sk = other[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            new_hm = taida_hashmap_set(new_hm, sh, sk, other[HM_HEADER + i * 3 + 2]);
        }
    }
    return new_hm;
}

// HashMap.toString() -> "HashMap({key1: val1, key2: val2})"
taida_val taida_hashmap_to_string(taida_val hm_ptr) {
    taida_val *hm = (taida_val*)hm_ptr;
    taida_val cap = hm[1];

    size_t buf_size = 256;
    char *buf = (char*)TAIDA_MALLOC(buf_size, "hm_to_string");
    // R-03: Use offset tracking instead of strcat (O(n) per call → O(1)).
    memcpy(buf, "HashMap({", 10); /* 9 chars + '\0' */
    size_t off = 9;
    taida_val count = 0;

    for (taida_val i = 0; i < cap; i++) {
        taida_val sh = hm[HM_HEADER + i * 3];
        taida_val sk = hm[HM_HEADER + i * 3 + 1];
        if (HM_SLOT_OCCUPIED(sh, sk)) {
            taida_val value = hm[HM_HEADER + i * 3 + 2];

            taida_val key_str_ptr = taida_value_to_debug_string(sk);
            taida_val val_str_ptr = taida_value_to_debug_string(value);
            const char *key_str = (const char*)key_str_ptr;
            const char *val_str = (const char*)val_str_ptr;
            if (!key_str) key_str = "\"\"";
            if (!val_str) val_str = "0";

            size_t klen = strlen(key_str);
            size_t vlen = strlen(val_str);
            size_t needed = klen + vlen + 4;
            if (count > 0) needed += 2;
            while (off + needed + 3 > buf_size) {
                buf_size *= 2;
                TAIDA_REALLOC(buf, buf_size, "hashmap_to_string");
            }

            if (count > 0) { memcpy(buf + off, ", ", 2); off += 2; }
            memcpy(buf + off, key_str, klen); off += klen;
            memcpy(buf + off, ": ", 2); off += 2;
            memcpy(buf + off, val_str, vlen); off += vlen;
            buf[off] = '\0';

            taida_str_release(key_str_ptr);
            taida_str_release(val_str_ptr);
            count++;
        }
    }
    memcpy(buf + off, "})", 3); /* 2 chars + '\0' */
    taida_val result = (taida_val)taida_str_new_copy(buf);
    free(buf);
    return result;
}

// ── Set runtime ──────────────────────────────────────────
// NO-4 RULE 1: Set follows List pattern — elem_type_tag + retain-on-copy.
// All set mutation ops (add/remove/union/intersect/diff) MUST propagate
// elem_type_tag and retain copied elements via taida_list_elem_retain.
// Set layout: same as list but with uniqueness constraint
// We use a regular list as the backing store (linear scan for uniqueness)
// Set is tagged with a special marker in slot[0] (negative refcount area won't work)
// Instead: Set = BuchiPack @(__items: List, __type: "Set")
// But for performance, let's use a simpler approach:
// Set is just a list with uniqueness, tagged by runtime functions.
// We'll use a structure: [refcount, capacity, length, items...]
// with a tag value stored differently.
//
// Simplest approach: Set = list ptr, and all set ops maintain uniqueness.
// The set functions accept and return list ptrs.

static taida_val taida_is_set(taida_val ptr) {
    return TAIDA_IS_SET(ptr);
}

taida_val taida_set_new(void) {
    taida_val list = taida_list_new();
    // Tag the list as a Set with magic
    ((taida_val*)list)[0] = TAIDA_SET_MAGIC | 1;
    // elem_type_tag at offset 3 is already TAIDA_TAG_UNKNOWN from taida_list_new
    return list;
}

// NO-2: Set elem_type_tag setter (analogous to taida_list_set_elem_tag / taida_hashmap_set_value_tag)
void taida_set_set_elem_tag(taida_val set_ptr, taida_val tag) {
    ((taida_val*)set_ptr)[3] = tag;
}

static taida_val taida_set_contains(taida_val set_ptr, taida_val item) {
    taida_val *list = (taida_val*)set_ptr;
    taida_val len = list[2];  // length at index 2
    for (taida_val i = 0; i < len; i++) {
        if (list[4 + i] == item) return 1;
    }
    return 0;
}

taida_val taida_set_from_list(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];  // length at index 2
    taida_val elem_tag = list[3];  // NO-2: propagate elem_type_tag from source list
    taida_val new_set = taida_set_new();
    ((taida_val*)new_set)[3] = elem_tag;  // NO-2: set elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        if (!taida_set_contains(new_set, item)) {
            taida_list_elem_retain(item, elem_tag);  // NO-2: retain-on-copy
            new_set = taida_list_push(new_set, item);
        }
    }
    return new_set;
}

taida_val taida_set_add(taida_val set_ptr, taida_val item) {
    if (taida_set_contains(set_ptr, item)) {
        return set_ptr;  // Already exists, return unchanged
    }
    // Use taida_list_push for correct list manipulation
    taida_val *list = (taida_val*)set_ptr;
    taida_val len = list[2];
    taida_val elem_tag = list[3];  // NO-2: propagate elem_type_tag
    // Clone the set, then push the new item
    taida_val new_set = taida_set_new();
    ((taida_val*)new_set)[3] = elem_tag;  // NO-2: set elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(list[4 + i], elem_tag);  // NO-2: retain-on-copy
        new_set = taida_list_push(new_set, list[4 + i]);
    }
    taida_list_elem_retain(item, elem_tag);  // NO-2: retain new element
    new_set = taida_list_push(new_set, item);
    return new_set;
}

taida_val taida_set_remove(taida_val set_ptr, taida_val item) {
    taida_val *list = (taida_val*)set_ptr;
    taida_val len = list[2];  // length at index 2
    taida_val elem_tag = list[3];  // NO-2: propagate elem_type_tag
    taida_val new_set = taida_set_new();
    ((taida_val*)new_set)[3] = elem_tag;  // NO-2: set elem_type_tag
    for (taida_val i = 0; i < len; i++) {
        if (list[4 + i] != item) {
            taida_list_elem_retain(list[4 + i], elem_tag);  // NO-2: retain-on-copy
            new_set = taida_list_push(new_set, list[4 + i]);
        }
    }
    return new_set;
}

taida_val taida_set_has(taida_val set_ptr, taida_val item) {
    return taida_set_contains(set_ptr, item);
}

taida_val taida_set_size(taida_val set_ptr) {
    return ((taida_val*)set_ptr)[2];  // length at index 2
}

taida_val taida_set_is_empty(taida_val set_ptr) {
    return ((taida_val*)set_ptr)[2] == 0 ? 1 : 0;  // length at index 2
}

taida_val taida_set_to_list(taida_val set_ptr) {
    // Clone the set as a regular list (not tagged as Set)
    taida_val *list = (taida_val*)set_ptr;
    taida_val len = list[2];  // length at index 2
    taida_val elem_tag = list[3];  // NO-2: propagate elem_type_tag
    taida_val new_list = taida_list_new();  // regular list, refcount=1 (not SET tag)
    ((taida_val*)new_list)[3] = elem_tag;  // NO-2: set elem_type_tag on result list
    for (taida_val i = 0; i < len; i++) {
        taida_list_elem_retain(list[4 + i], elem_tag);  // NO-2: retain-on-copy
        new_list = taida_list_push(new_list, list[4 + i]);
    }
    return new_list;
}

taida_val taida_set_union(taida_val set_a, taida_val set_b) {
    taida_val *a = (taida_val*)set_a;
    taida_val *b = (taida_val*)set_b;
    taida_val a_len = a[2];  // length at index 2
    taida_val b_len = b[2];
    taida_val elem_tag = a[3];  // NO-2: propagate elem_type_tag from set_a
    // Start with a copy of a
    taida_val result = taida_set_new();
    ((taida_val*)result)[3] = elem_tag;  // NO-2: set elem_type_tag
    for (taida_val i = 0; i < a_len; i++) {
        taida_list_elem_retain(a[4 + i], elem_tag);  // NO-2: retain-on-copy
        result = taida_list_push(result, a[4 + i]);
    }
    // Add items from b that aren't in a
    for (taida_val i = 0; i < b_len; i++) {
        if (!taida_set_contains(result, b[4 + i])) {
            taida_list_elem_retain(b[4 + i], elem_tag);  // NO-2: retain-on-copy
            result = taida_list_push(result, b[4 + i]);
        }
    }
    return result;
}

taida_val taida_set_intersect(taida_val set_a, taida_val set_b) {
    taida_val *a = (taida_val*)set_a;
    taida_val a_len = a[2];  // length at index 2
    taida_val elem_tag = a[3];  // NO-2: propagate elem_type_tag
    taida_val result = taida_set_new();
    ((taida_val*)result)[3] = elem_tag;  // NO-2: set elem_type_tag
    for (taida_val i = 0; i < a_len; i++) {
        if (taida_set_contains(set_b, a[4 + i])) {
            taida_list_elem_retain(a[4 + i], elem_tag);  // NO-2: retain-on-copy
            result = taida_list_push(result, a[4 + i]);
        }
    }
    return result;
}

taida_val taida_set_diff(taida_val set_a, taida_val set_b) {
    taida_val *a = (taida_val*)set_a;
    taida_val a_len = a[2];  // length at index 2
    taida_val elem_tag = a[3];  // NO-2: propagate elem_type_tag
    taida_val result = taida_set_new();
    ((taida_val*)result)[3] = elem_tag;  // NO-2: set elem_type_tag
    for (taida_val i = 0; i < a_len; i++) {
        if (!taida_set_contains(set_b, a[4 + i])) {
            taida_list_elem_retain(a[4 + i], elem_tag);  // NO-2: retain-on-copy
            result = taida_list_push(result, a[4 + i]);
        }
    }
    return result;
}

// Set.toString() -> "Set({1, 2, 3})"
taida_val taida_set_to_string(taida_val set_ptr) {
    taida_val *list = (taida_val*)set_ptr;
    taida_val len = list[2];  // length at index 2
    size_t buf_size = 128;
    char *buf = (char*)TAIDA_MALLOC(buf_size, "set_to_string");
    // R-03: Use offset tracking instead of strcat (O(n) per call → O(1)).
    memcpy(buf, "Set({", 6); /* 5 chars + '\0' */
    size_t off = 5;
    for (taida_val i = 0; i < len; i++) {
        char item_str[64];
        int item_len = snprintf(item_str, sizeof(item_str), "%" PRId64 "", list[4 + i]);
        size_t needed = (size_t)item_len + (i > 0 ? 2 : 0) + 10;
        if (off + needed > buf_size) {
            buf_size *= 2;
            TAIDA_REALLOC(buf, buf_size, "set_to_string");
        }
        if (i > 0) { memcpy(buf + off, ", ", 2); off += 2; }
        memcpy(buf + off, item_str, (size_t)item_len); off += (size_t)item_len;
        buf[off] = '\0';
    }
    memcpy(buf + off, "})", 3); /* 2 chars + '\0' */
    taida_val result = (taida_val)taida_str_new_copy(buf);
    free(buf);
    return result;
}

// ── Polymorphic length ───────────────────────────────────
// .length() — works on Str (strlen) and List/Set (list[2], unchanged by elem_type_tag)
// Detection: if ptr looks like a heap object with list layout (ptr[0] small, ptr[1] >= 8),
// treat as list. Otherwise treat as string.
taida_val taida_polymorphic_length(taida_val ptr) {
    if (ptr == 0) return 0;
    if (ptr < 4096) return 0;
    // Check for HashMap
    if (taida_is_hashmap(ptr)) {
        if (!taida_ptr_is_readable(ptr, sizeof(taida_val) * 3)) return 0;
        return ((taida_val*)ptr)[2];
    }
    // Check for Set
    if (taida_is_set(ptr)) {
        if (!taida_ptr_is_readable(ptr, sizeof(taida_val) * 3)) return 0;
        return ((taida_val*)ptr)[2];
    }
    // Check for List
    if (TAIDA_IS_LIST(ptr)) {
        return ((taida_val*)ptr)[2];
    }
    // Check for Bytes
    if (TAIDA_IS_BYTES(ptr)) {
        return ((taida_val*)ptr)[1];
    }
    // Treat as string
    size_t sl = 0;
    if (taida_read_cstr_len_safe((const char*)ptr, 65536, &sl)) return (taida_val)sl;
    return 0;
}

// Polymorphic .contains(needle) — works on Str and List.
// Runtime dispatch is required for cases where static lowering cannot prove
// string type (e.g. field access inside lambda callbacks).
taida_val taida_polymorphic_contains(taida_val obj, taida_val needle) {
    if (obj == 0 || obj < 4096) return 0;
    // Check for non-string types first (list, pack, bytes, hashmap, etc.)
    if (taida_ptr_is_readable(obj, 8) && taida_has_magic_header(((taida_val*)obj)[0])) {
        return taida_list_contains(obj, needle);
    }
    // No magic header — treat as C string
    taida_val needle_str = taida_value_to_display_string(needle);
    taida_val out = taida_str_contains((const char*)obj, (const char*)needle_str);
    taida_release(needle_str);
    return out;
}

taida_val taida_polymorphic_index_of(taida_val obj, taida_val needle) {
    if (obj == 0 || obj < 4096) return -1;
    // Check for non-string types first (list, pack, bytes, hashmap, etc.)
    if (taida_ptr_is_readable(obj, 8) && taida_has_magic_header(((taida_val*)obj)[0])) {
        return taida_list_index_of(obj, needle);
    }
    // No magic header — treat as C string
    return taida_str_index_of((const char*)obj, (const char*)needle);
}

taida_val taida_polymorphic_last_index_of(taida_val obj, taida_val needle) {
    if (obj == 0 || obj < 4096) return -1;
    if (taida_ptr_is_readable(obj, 8) && taida_has_magic_header(((taida_val*)obj)[0])) {
        return taida_list_last_index_of(obj, needle);
    }
    return taida_str_last_index_of((const char*)obj, (const char*)needle);
}

// ── Polymorphic collection methods ───────────────────────
// These work on both HashMap and Set (auto-detect via tag)

// .get(key_or_index) — HashMap: hash-based lookup returning Lax, List: index-based returning Lax
taida_val taida_collection_get(taida_val ptr, taida_val item) {
    if (taida_is_hashmap(ptr)) {
        taida_val key_hash = taida_value_hash(item);
        return taida_hashmap_get_lax(ptr, key_hash, item);
    }
    if (taida_is_bytes(ptr)) {
        return taida_bytes_get_lax(ptr, item);
    }
    // List: index-based access returning Lax
    return taida_list_get(ptr, item);
}

// .has(key_or_item) — HashMap: hash-based lookup, Set: linear scan
taida_val taida_collection_has(taida_val ptr, taida_val item) {
    if (taida_is_hashmap(ptr)) {
        taida_val key_hash = taida_value_hash(item);
        return taida_hashmap_has(ptr, key_hash, item);
    }
    // Set/List: linear scan
    return taida_set_has(ptr, item);
}

// .remove(key_or_item) — HashMap: hash-based removal, Set: linear scan
taida_val taida_collection_remove(taida_val ptr, taida_val item) {
    if (taida_is_hashmap(ptr)) {
        taida_val key_hash = taida_value_hash(item);
        return taida_hashmap_remove_immut(ptr, key_hash, item);
    }
    // Set: linear scan removal
    return taida_set_remove(ptr, item);
}

// .size() — works on both HashMap and Set (both store length at ptr[2])
taida_val taida_collection_size(taida_val ptr) {
    return ((taida_val*)ptr)[2];
}

