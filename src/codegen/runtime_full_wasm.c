/// runtime_full_wasm.c -- wasm-full extended runtime
///
/// wasm-min (runtime_core_wasm.c) + wasm-wasi (runtime_wasi_io.c) on top of which
/// extended runtime functions are added. Covers:
///
/// - String molds (to_upper, to_lower, trim, split, replace, etc.)
/// - Number molds (float abs/ceil/floor/round/clamp, int clamp, etc.)
/// - Extended list ops (filter, fold, find, sort, unique, etc.)
/// - HashMap/Set extensions (length, clone, keys, values, entries, etc.)
/// - JSON runtime (parse, stringify, schema_cast, etc.)
/// - Gorillax/Lax/Result extensions (map, flat_map, to_string, etc.)
/// - bytes / bitwise / char / codepoint
/// - Pack/Error/Field/Callback extensions
/// - Global get/set
///
/// This file references functions from runtime_core_wasm.c via extern.
/// runtime_core_wasm.c is NOT #included; wasm-ld resolves symbols.

#include <stdint.h>

// ---------------------------------------------------------------------------
// Forward declarations from runtime_core_wasm.c
// (linked via wasm-ld, not #include)
// ---------------------------------------------------------------------------
extern void *wasm_alloc(unsigned int size);

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
extern int64_t taida_hashmap_get_lax(int64_t hm, int64_t kh, int64_t kp);
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

// Forward declarations for functions now in runtime_core_wasm.c (WC-1)
extern int64_t taida_str_alloc(int64_t len_raw);
extern int64_t taida_str_new_copy(int64_t src_raw);
extern int64_t taida_str_to_upper(int64_t s_raw);
extern int64_t taida_str_to_lower(int64_t s_raw);
extern int64_t taida_str_trim(int64_t s_raw);
extern int64_t taida_str_trim_start(int64_t s_raw);
extern int64_t taida_str_trim_end(int64_t s_raw);
extern int64_t taida_str_split(int64_t s_raw, int64_t sep_raw);
extern int64_t taida_str_replace(int64_t s_raw, int64_t from_raw, int64_t to_raw);
extern int64_t taida_str_replace_first(int64_t s_raw, int64_t from_raw, int64_t to_raw);
extern int64_t taida_str_slice(int64_t s_raw, int64_t start_raw, int64_t end_raw);
extern int64_t taida_str_char_at(int64_t s_raw, int64_t idx_raw);
extern int64_t taida_str_repeat(int64_t s_raw, int64_t n_raw);
extern int64_t taida_str_reverse(int64_t s_raw);
extern int64_t taida_str_pad(int64_t s_raw, int64_t target_len_raw, int64_t pad_char_raw, int64_t pad_end_raw);
extern int64_t taida_str_contains(int64_t s_raw, int64_t sub_raw);
extern int64_t taida_str_starts_with(int64_t s_raw, int64_t prefix_raw);
extern int64_t taida_str_ends_with(int64_t s_raw, int64_t suffix_raw);
extern int64_t taida_str_index_of(int64_t s_raw, int64_t sub_raw);
extern int64_t taida_str_last_index_of(int64_t s_raw, int64_t sub_raw);
extern int64_t taida_str_get(int64_t s_raw, int64_t idx_raw);
extern int64_t taida_cmp_strings(int64_t a_raw, int64_t b_raw);
extern int64_t taida_slice_mold(int64_t value, int64_t start_raw, int64_t end_raw);
extern int64_t taida_char_mold_int(int64_t value);
extern int64_t taida_char_mold_str(int64_t value);
extern int64_t taida_char_to_digit(int64_t v);
extern int64_t taida_codepoint_mold_str(int64_t value);
extern int64_t taida_digit_to_char(int64_t digit);

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
// Local string helpers (no libc available in wasm32)
// ---------------------------------------------------------------------------

static int _wf_strlen(const char *s) {
    if (!s) return 0;
    int len = 0;
    while (s[len]) len++;
    return len;
}

static void _wf_memcpy(void *dst, const void *src, int len) {
    char *d = (char *)dst;
    const char *s = (const char *)src;
    for (int i = 0; i < len; i++) d[i] = s[i];
}

static int _wf_strncmp(const char *a, const char *b, int n) {
    for (int i = 0; i < n; i++) {
        if (a[i] != b[i]) return (unsigned char)a[i] - (unsigned char)b[i];
        if (a[i] == '\0') return 0;
    }
    return 0;
}

static int _wf_strcmp(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}

/// Find first occurrence of needle in haystack, or NULL.
static const char *_wf_strstr(const char *haystack, const char *needle) {
    if (!haystack || !needle) return (const char *)0;
    int nlen = _wf_strlen(needle);
    if (nlen == 0) return haystack;
    int hlen = _wf_strlen(haystack);
    if (nlen > hlen) return (const char *)0;
    for (int i = 0; i <= hlen - nlen; i++) {
        if (_wf_strncmp(haystack + i, needle, nlen) == 0)
            return haystack + i;
    }
    return (const char *)0;
}

static int _wf_is_whitespace(char c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r';
}

// ---------------------------------------------------------------------------
// WC-1: String mold/query/char functions moved to runtime_core_wasm.c
// (see extern declarations above)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// WC-2: Number mold functions moved to runtime_core_wasm.c
// (see extern declarations below)
// ---------------------------------------------------------------------------

// Forward declarations for number mold functions now in runtime_core_wasm.c (WC-2)
extern int64_t taida_float_floor(int64_t a);
extern int64_t taida_float_ceil(int64_t a);
extern int64_t taida_float_round(int64_t a);
extern int64_t taida_float_abs(int64_t a);
extern int64_t taida_float_clamp(int64_t a, int64_t lo, int64_t hi);
extern int64_t taida_float_to_fixed(int64_t a, int64_t digits_raw);
extern int64_t taida_float_is_nan(int64_t a);
extern int64_t taida_float_is_infinite(int64_t a);
extern int64_t taida_float_is_finite_check(int64_t a);
extern int64_t taida_float_is_positive(int64_t a);
extern int64_t taida_float_is_negative(int64_t a);
extern int64_t taida_float_is_zero(int64_t a);
extern int64_t taida_int_clamp(int64_t a, int64_t lo, int64_t hi);
extern int64_t taida_int_is_positive(int64_t a);
extern int64_t taida_int_is_negative(int64_t a);
extern int64_t taida_int_is_zero(int64_t a);
extern int64_t taida_int_mold_auto(int64_t v);
extern int64_t taida_int_mold_str_base(int64_t v, int64_t base);
extern int64_t taida_to_radix(int64_t value, int64_t base);

/// Int[str]() -- parse string to int, returning Lax[Int]
/// This overrides core's version which returns raw value (no Lax wrapper).
/// Linked via #define redirect: taida_int_mold_str -> _full.
int64_t taida_int_mold_str_full(int64_t v) {
    const char *s = (const char *)(intptr_t)v;
    if (!s || s[0] == '\0') return taida_lax_empty(0);
    int neg = 0;
    int i = 0;
    if (s[0] == '-') { neg = 1; i = 1; }
    else if (s[0] == '+') { i = 1; }
    int64_t acc = 0;
    int found_digit = 0;
    while (s[i] >= '0' && s[i] <= '9') {
        acc = acc * 10 + (s[i] - '0');
        found_digit = 1;
        i++;
    }
    if (found_digit && s[i] == '\0') {
        return taida_lax_new(neg ? -acc : acc, 0);
    }
    return taida_lax_empty(0); // parse failed
}

// ---------------------------------------------------------------------------
// WF-2d: Extended list operations
// ---------------------------------------------------------------------------
// WASM list layout: [capacity, length, elem_type_tag, type_marker, item0, item1, ...]
// Header offset for items = 4 (WASM_LIST_ELEMS).
// In WASM, retain/release are no-ops (bump allocator).

// Additional extern declarations for core helpers used by list ops
extern int64_t taida_list_set_elem_tag(int64_t list_ptr, int64_t tag);
extern int64_t taida_pack_set_tag(int64_t pack_ptr, int64_t index, int64_t tag);

// WASM list constants
#define WF_LIST_ELEMS 4

// FNV-1a hashes for Zip/Enumerate BuchiPack fields
#define WF_HASH_FIRST  0x89d7ed7f996f1d41ULL
#define WF_HASH_SECOND 0xa49985ef4cee20bdULL
#define WF_HASH_INDEX  0x83cf8e8f9081468bULL
#define WF_HASH_VALUE  0x7ce4fd9430e80ceaULL

// ---------------------------------------------------------------------------
// WC-3: List mold/HOF/query/callback functions moved to runtime_core_wasm.c
// (see extern declarations below)
// ---------------------------------------------------------------------------
extern int64_t taida_invoke_callback1(int64_t fn_ptr, int64_t arg0);
extern int64_t taida_invoke_callback2(int64_t fn_ptr, int64_t arg0, int64_t arg1);

// WC-3: List HOF functions moved to runtime_core_wasm.c
extern int64_t taida_list_map(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_filter(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_fold(int64_t list, int64_t init, int64_t fn_ptr);
extern int64_t taida_list_foldr(int64_t list, int64_t init, int64_t fn_ptr);
extern int64_t taida_list_find(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_find_index(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_take_while(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_drop_while(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_any(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_all(int64_t list, int64_t fn_ptr);
extern int64_t taida_list_none(int64_t list, int64_t fn_ptr);

// WC-3: List operation functions moved to runtime_core_wasm.c
extern int64_t taida_list_sort(int64_t list);
extern int64_t taida_list_sort_desc(int64_t list);
extern int64_t taida_list_unique(int64_t list);
extern int64_t taida_list_flatten(int64_t list);
extern int64_t taida_list_reverse(int64_t list);
extern int64_t taida_list_join(int64_t list, int64_t sep);
extern int64_t taida_list_concat(int64_t list_a, int64_t list_b);
extern int64_t taida_list_append(int64_t list, int64_t item);
extern int64_t taida_list_prepend(int64_t list, int64_t item);
extern int64_t taida_list_take(int64_t list, int64_t n);
extern int64_t taida_list_drop(int64_t list, int64_t n);
extern int64_t taida_list_enumerate(int64_t list);
extern int64_t taida_list_zip(int64_t list_a, int64_t list_b);
extern int64_t taida_list_to_display_string(int64_t list);

// WC-3: List query functions moved to runtime_core_wasm.c
extern int64_t taida_list_first(int64_t list);
extern int64_t taida_list_last(int64_t list);
extern int64_t taida_list_min(int64_t list);
extern int64_t taida_list_max(int64_t list);
extern int64_t taida_list_sum(int64_t list);
extern int64_t taida_list_contains(int64_t list, int64_t item);
extern int64_t taida_list_index_of(int64_t list, int64_t item);
extern int64_t taida_list_last_index_of(int64_t list, int64_t item);
extern int64_t taida_list_count(int64_t list, int64_t fn_ptr);

// WC-3: List elem retain/release moved to runtime_core_wasm.c
extern void taida_list_elem_retain(int64_t list);
extern void taida_list_elem_release(int64_t list);

// ---------------------------------------------------------------------------
// Helpers used by remaining full-only functions (kept locally after WC-3 move)
// ---------------------------------------------------------------------------

/// Helper: check if a pointer looks like a WASM list (same heuristic as core)
static int _wf_looks_like_list(int64_t ptr) {
    if (ptr == 0) return 0;
    if (ptr < 0 || ptr > 0xFFFFFFFF) return 0;
    int64_t *data = (int64_t *)(intptr_t)ptr;
    int64_t cap = data[0];
    int64_t len = data[1];
    if (cap >= 8 && cap <= 65536 && len >= 0 && len <= cap) return 1;
    return 0;
}

/// Helper: check if a value looks like a valid string pointer in WASM linear memory.
static int _wf_looks_like_string(int64_t val) {
    if (val == 0) return 0;
    if (val < 0 || val > 0xFFFFFFFF) return 0;
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)val;
    if (addr >= mem_size) return 0;
    const char *s = (const char *)(intptr_t)val;
    /* Empty string is a valid string */
    if (s[0] == '\0') return 1;
    /* Verify first few bytes are valid UTF-8/ASCII (not random garbage) */
    for (int i = 0; i < 8 && s[i]; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c < 0x20 && c != '\t' && c != '\n' && c != '\r') return 0;
    }
    return 1;
}

// ---------------------------------------------------------------------------
// WF-2e: HashMap/Set extensions
// ---------------------------------------------------------------------------

// WASM HashMap layout: [capacity, length, value_type_tag, type_marker, entries...]
// Entry stride = 3: [key_hash, key_ptr, value]
// Header = 4 slots (matching WASM_HM_HEADER)
#define WF_HM_HEADER 4
#define WF_HM_MARKER_VAL 0x484D4150LL  // must match WASM_HM_MARKER_VAL in core

// WASM Set marker: slot[3] value
#define WF_SET_MARKER_VAL 0x53455421LL  // must match WASM_SET_MARKER_VAL in core

/// Helper: check if a pointer is a WASM hashmap (heuristic)
static int _wf_is_hashmap(int64_t ptr) {
    if (ptr == 0 || ptr < 0 || ptr > 0xFFFFFFFF) return 0;
    int64_t *data = (int64_t *)(intptr_t)ptr;
    // HashMap: type_marker at slot[3] == WF_HM_MARKER_VAL
    return data[3] == WF_HM_MARKER_VAL;
}

/// Helper: check if a pointer is a WASM set (heuristic)
static int _wf_is_set(int64_t ptr) {
    if (ptr == 0 || ptr < 0 || ptr > 0xFFFFFFFF) return 0;
    int64_t *data = (int64_t *)(intptr_t)ptr;
    return data[3] == WF_SET_MARKER_VAL;
}

/// Helper: WASM Lax detection (pack with fc=4, hash0=HASH_HAS_VALUE)
#define WF_HASH_HAS_VALUE   0x9e9c6dc733414d60LL
#define WF_HASH___VALUE     0x0a7fc9f13472bbe0LL
#define WF_HASH_THROW       0x5a5fe3720c9584cfLL

static int _wf_is_valid_ptr(int64_t val, unsigned int min_bytes) {
    if (val <= 0 || val > 0xFFFFFFFF) return 0;
    unsigned int pages = __builtin_wasm_memory_size(0);
    unsigned int mem_size = pages * 65536;
    unsigned int addr = (unsigned int)val;
    if (addr + min_bytes > mem_size) return 0;
    return 1;
}

static int _wf_is_lax(int64_t val) {
    if (!_wf_is_valid_ptr(val, 104)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    if (p[0] == 4 && p[1] == WF_HASH_HAS_VALUE) return 1;
    return 0;
}

static int _wf_is_result(int64_t val) {
    if (!_wf_is_valid_ptr(val, 104)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    if (p[0] == 4 && p[1] == WF_HASH___VALUE) {
        int64_t hash2 = p[1 + 2 * 3]; // field 2 hash
        if (hash2 == WF_HASH_THROW) return 1;
    }
    return 0;
}

// Forward declare from core for polymorphic functions
extern int64_t taida_result_is_ok(int64_t result);
extern int64_t taida_result_is_error(int64_t result);
extern int64_t taida_can_throw_payload(int64_t val);

/// HashMap.length() / HashMap.size() -- return number of entries
int64_t taida_hashmap_length(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t count = 0;
    for (int64_t i = 0; i < cap; i++) {
        int64_t kh = hm[WF_HM_HEADER + i * 3];
        int64_t kp = hm[WF_HM_HEADER + i * 3 + 1];
        if (kh != 0 || kp != 0) {
            // Not empty and not tombstone (tombstone: hash=1, key=0)
            if (!(kh == 1 && kp == 0)) count++;
        }
    }
    return count;
}

/// HashMap.clone() -- deep clone a hashmap
int64_t taida_hashmap_clone(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];
    int64_t total_slots = WF_HM_HEADER + cap * 3;
    int64_t *new_hm = (int64_t *)wasm_alloc((unsigned int)(total_slots * 8));
    for (int64_t i = 0; i < total_slots; i++) {
        new_hm[i] = hm[i];
    }
    return (int64_t)(intptr_t)new_hm;
}

/// HashMap.toString() -- "HashMap({key: value, ...})"
int64_t taida_hashmap_to_string(int64_t hm_ptr) {
    int64_t *hm = (int64_t *)(intptr_t)hm_ptr;
    int64_t cap = hm[0];

    // Build "HashMap({...})"
    // First pass: collect key-value pairs
    int total_len = 10; // "HashMap({" + "})"
    int entry_count = 0;
    for (int64_t i = 0; i < cap; i++) {
        int64_t kh = hm[WF_HM_HEADER + i * 3];
        int64_t kp = hm[WF_HM_HEADER + i * 3 + 1];
        if ((kh != 0 || kp != 0) && !(kh == 1 && kp == 0)) {
            entry_count++;
        }
    }

    if (entry_count == 0) {
        char *r = (char *)wasm_alloc(13);
        _wf_memcpy(r, "HashMap({})", 12);
        r[11] = '\0';
        return (int64_t)r;
    }

    // Collect pairs as strings
    int buf_size = 256;
    char *buf = (char *)wasm_alloc((unsigned int)buf_size);
    _wf_memcpy(buf, "HashMap({", 9);
    int pos = 9;
    int first = 1;
    for (int64_t i = 0; i < cap; i++) {
        int64_t kh = hm[WF_HM_HEADER + i * 3];
        int64_t kp = hm[WF_HM_HEADER + i * 3 + 1];
        int64_t val = hm[WF_HM_HEADER + i * 3 + 2];
        if ((kh != 0 || kp != 0) && !(kh == 1 && kp == 0)) {
            if (!first) { buf[pos++] = ','; buf[pos++] = ' '; }
            first = 0;
            // Key string
            const char *ks = (const char *)(intptr_t)taida_polymorphic_to_string(kp);
            int kl = _wf_strlen(ks);
            // Value string
            const char *vs = (const char *)(intptr_t)taida_polymorphic_to_string(val);
            int vl = _wf_strlen(vs);
            // Ensure buf is large enough
            while (pos + kl + vl + 10 > buf_size) {
                buf_size *= 2;
                char *new_buf = (char *)wasm_alloc((unsigned int)buf_size);
                _wf_memcpy(new_buf, buf, pos);
                buf = new_buf;
            }
            _wf_memcpy(buf + pos, ks, kl); pos += kl;
            buf[pos++] = ':'; buf[pos++] = ' ';
            _wf_memcpy(buf + pos, vs, vl); pos += vl;
        }
    }
    buf[pos++] = '}'; buf[pos++] = ')';
    buf[pos] = '\0';
    return (int64_t)buf;
}

/// HashMap.remove(key) -> new HashMap without that key (immutable)
int64_t taida_hashmap_remove_immut(int64_t hm_ptr, int64_t key_hash, int64_t key_ptr) {
    // Clone and then remove in place
    int64_t clone = taida_hashmap_clone(hm_ptr);
    int64_t *hm = (int64_t *)(intptr_t)clone;
    int64_t cap = hm[0];
    // Find the key and tombstone it
    for (int64_t i = 0; i < cap; i++) {
        int64_t kh = hm[WF_HM_HEADER + i * 3];
        int64_t kp = hm[WF_HM_HEADER + i * 3 + 1];
        if (kh == key_hash && kp != 0) {
            // Compare key strings
            if (taida_str_eq(kp, key_ptr)) {
                // Tombstone: hash=1, key=0
                hm[WF_HM_HEADER + i * 3] = 1;
                hm[WF_HM_HEADER + i * 3 + 1] = 0;
                hm[WF_HM_HEADER + i * 3 + 2] = 0;
                break;
            }
        }
    }
    return clone;
}

/// HashMap with initial capacity
int64_t taida_hashmap_new_with_cap(int64_t cap) {
    if (cap < 8) cap = 8;
    int64_t total_slots = WF_HM_HEADER + cap * 3;
    int64_t *hm = (int64_t *)wasm_alloc((unsigned int)(total_slots * 8));
    for (int64_t i = 0; i < total_slots; i++) hm[i] = 0;
    hm[0] = cap;
    hm[1] = 0; // length
    hm[2] = -1; // value_type_tag
    hm[3] = WF_HM_MARKER_VAL;
    return (int64_t)(intptr_t)hm;
}

/// HashMap internal helpers (needed for some code paths)
int64_t taida_hashmap_adjust_hash(int64_t h) {
    // Ensure hash is never 0 or 1 (reserved for empty/tombstone)
    if (h == 0 || h == 1) return h + 2;
    return h;
}

int64_t taida_hashmap_set_internal(int64_t hm, int64_t kh, int64_t kp, int64_t v, int64_t mode) {
    // Delegate to taida_hashmap_set (mode is unused in simplified version)
    (void)mode;
    return taida_hashmap_set(hm, kh, kp, v);
}

int64_t taida_hashmap_resize(int64_t hm_ptr, int64_t new_cap) {
    int64_t *old = (int64_t *)(intptr_t)hm_ptr;
    int64_t old_cap = old[0];
    int64_t new_hm = taida_hashmap_new_with_cap(new_cap);
    // Re-insert all entries
    for (int64_t i = 0; i < old_cap; i++) {
        int64_t kh = old[WF_HM_HEADER + i * 3];
        int64_t kp = old[WF_HM_HEADER + i * 3 + 1];
        int64_t val = old[WF_HM_HEADER + i * 3 + 2];
        if ((kh != 0 || kp != 0) && !(kh == 1 && kp == 0)) {
            new_hm = taida_hashmap_set(new_hm, kh, kp, val);
        }
    }
    return new_hm;
}

int64_t taida_hashmap_key_eq(int64_t a, int64_t b) { return taida_str_eq(a, b); }
int64_t taida_hashmap_key_retain(int64_t a, int64_t b) { (void)a; (void)b; return 0; }
int64_t taida_hashmap_key_release(int64_t a, int64_t b) { (void)a; (void)b; return 0; }
int64_t taida_hashmap_val_retain(int64_t a, int64_t b) { (void)a; (void)b; return 0; }
int64_t taida_hashmap_val_release(int64_t a, int64_t b) { (void)a; (void)b; return 0; }
int64_t taida_hashmap_key_valid(int64_t v) { return v != 0 ? 1 : 0; }

// --- Set extensions ---

int64_t taida_set_contains(int64_t set_ptr, int64_t item) {
    return taida_set_has(set_ptr, item);
}

int64_t taida_set_is_empty(int64_t set_ptr) {
    int64_t *list = (int64_t *)(intptr_t)set_ptr;
    return list[1] == 0 ? 1 : 0;
}

int64_t taida_set_size(int64_t set_ptr) {
    int64_t *list = (int64_t *)(intptr_t)set_ptr;
    return list[1];
}

int64_t taida_set_to_string(int64_t set_ptr) {
    int64_t *list = (int64_t *)(intptr_t)set_ptr;
    int64_t len = list[1];
    if (len == 0) {
        char *r = (char *)wasm_alloc(8);
        _wf_memcpy(r, "Set({})", 8);
        return (int64_t)r;
    }
    // Build "Set({elem, elem, ...})"
    int buf_size = 128;
    char *buf = (char *)wasm_alloc((unsigned int)buf_size);
    _wf_memcpy(buf, "Set({", 5);
    int pos = 5;
    for (int64_t i = 0; i < len; i++) {
        if (i > 0) { buf[pos++] = ','; buf[pos++] = ' '; }
        const char *vs = (const char *)(intptr_t)taida_polymorphic_to_string(list[WF_LIST_ELEMS + i]);
        int vl = _wf_strlen(vs);
        while (pos + vl + 10 > buf_size) {
            buf_size *= 2;
            char *new_buf = (char *)wasm_alloc((unsigned int)buf_size);
            _wf_memcpy(new_buf, buf, pos);
            buf = new_buf;
        }
        _wf_memcpy(buf + pos, vs, vl); pos += vl;
    }
    buf[pos++] = '}'; buf[pos++] = ')';
    buf[pos] = '\0';
    return (int64_t)buf;
}

// ---------------------------------------------------------------------------
// WF-2f (partial): Polymorphic / type detection functions needed by WF-2e
// ---------------------------------------------------------------------------

/// Lax.getOrDefault(fallback)
static int64_t _wf_lax_get_or_default(int64_t lax_ptr, int64_t fallback) {
    if (taida_pack_get_idx(lax_ptr, 0)) {
        return taida_pack_get_idx(lax_ptr, 1); // __value
    }
    return fallback;
}

/// Result.getOrDefault(fallback) -- using result_is_error from core
static int64_t _wf_result_get_or_default(int64_t result, int64_t def) {
    if (taida_result_is_ok(result)) {
        return taida_pack_get_idx(result, 0); // __value
    }
    return def;
}

/// Polymorphic .getOrDefault(fallback) -- works on Result, Lax, raw values
int64_t taida_polymorphic_get_or_default(int64_t obj, int64_t def) {
    if (obj == 0) return def;
    // Small ints: in WASM, these are raw values (not pointers to Lax/Result)
    // But they could be valid int values (e.g. 14 from HashMap.get("age"))
    // so for small ints, return obj (they are the value itself)
    if (obj > 0 && obj < 4096) return obj;
    if (obj < 0) return obj; // negative ints are valid values
    if (_wf_is_result(obj)) return _wf_result_get_or_default(obj, def);
    if (_wf_is_lax(obj)) return _wf_lax_get_or_default(obj, def);
    // In WASM, hashmap_get returns raw values (not Lax-wrapped).
    // If we reach here, obj is a valid non-zero pointer (e.g. string from hashmap).
    // Return it as-is since it's the actual value.
    return obj;
}

/// Polymorphic .hasValue()
/// Works for Lax (hasValue at idx 0), Gorillax/RelaxedGorillax (isOk at idx 0),
/// and Result (hasValue at idx 0). All fc=4 monadic types store a boolean
/// "does this have a value" indicator at index 0.
int64_t taida_polymorphic_has_value(int64_t obj) {
    if (obj == 0 || obj < 4096) return 0;
    if (!_wf_is_valid_ptr(obj, 104)) return 0;
    int64_t *p = (int64_t *)(intptr_t)obj;
    if (p[0] == 4) return taida_pack_get_idx(obj, 0); // fc=4: Lax/Gorillax/Result
    return 0;
}

/// Polymorphic .get(key_or_index) — wasm-full override.
/// The core version returns raw values for List.get(); this version wraps in Lax
/// to match native semantics (OOB returns empty Lax, in-bounds returns Lax[value]).
/// Linked via #define redirect in generated C: taida_collection_get -> _full.
int64_t taida_collection_get_full(int64_t ptr, int64_t item) {
    if (_wf_is_hashmap(ptr)) {
        int64_t key_hash = taida_value_hash(item);
        return taida_hashmap_get_lax(ptr, key_hash, item);
    }
    /* List: index-based access returning Lax (matching native) */
    if (_wf_looks_like_list(ptr)) {
        int64_t *list = (int64_t *)(intptr_t)ptr;
        int64_t len = list[1];
        if (item < 0 || item >= len) return taida_lax_empty(0);
        return taida_lax_new(list[WF_LIST_ELEMS + item], 0);
    }
    /* String: char-at returning Lax */
    if (_wf_looks_like_string(ptr)) {
        return taida_str_get(ptr, item);
    }
    return taida_lax_empty(0);
}

/// Polymorphic .isEmpty() — wasm-full override.
/// The core version (runtime_core_wasm.c) only handles List/Set/HashMap/String.
/// This version adds Lax, Gorillax, and Result support.
/// Linked via #define redirect in generated C: taida_polymorphic_is_empty -> _full.
int64_t taida_polymorphic_is_empty_full(int64_t ptr) {
    if (ptr == 0) return 1;
    if (!_wf_is_valid_ptr(ptr, 8)) return 0;
    int64_t *p = (int64_t *)(intptr_t)ptr;
    // fc=4 monadic types: Lax/Gorillax/RelaxedGorillax/Result
    // field 0 = hasValue/isOk; isEmpty = !field0
    if (p[0] == 4) {
        if (_wf_is_result(ptr)) return taida_result_is_error(ptr);
        return taida_pack_get_idx(ptr, 0) ? 0 : 1; // Lax/Gorillax
    }
    // HashMap/Set
    if (_wf_is_hashmap(ptr)) {
        int64_t *data = (int64_t *)(intptr_t)ptr;
        return data[1] == 0 ? 1 : 0;
    }
    if (_wf_is_set(ptr)) {
        int64_t *data = (int64_t *)(intptr_t)ptr;
        return data[1] == 0 ? 1 : 0;
    }
    // List
    if (_wf_looks_like_list(ptr)) {
        int64_t *data = (int64_t *)(intptr_t)ptr;
        return data[1] == 0 ? 1 : 0;
    }
    // String
    if (_wf_looks_like_string(ptr)) {
        const char *s = (const char *)(intptr_t)ptr;
        return s[0] == '\0' ? 1 : 0;
    }
    return 0;
}

/// Polymorphic .contains()
int64_t taida_polymorphic_contains(int64_t obj, int64_t needle) {
    if (obj == 0) return 0;
    if (_wf_is_hashmap(obj)) return taida_hashmap_has(obj, taida_value_hash(needle), needle);
    if (_wf_is_set(obj)) return taida_set_has(obj, needle);
    if (_wf_looks_like_list(obj)) return taida_list_contains(obj, needle);
    // String contains — static strings can live at low addresses (data section)
    if (_wf_looks_like_string(obj)) return taida_str_contains(obj, needle);
    return 0;
}

/// Polymorphic .indexOf()
int64_t taida_polymorphic_index_of(int64_t obj, int64_t needle) {
    if (obj == 0) return -1;
    if (_wf_looks_like_list(obj)) return taida_list_index_of(obj, needle);
    if (_wf_looks_like_string(obj)) return taida_str_index_of(obj, needle);
    return -1;
}

/// Polymorphic .lastIndexOf()
int64_t taida_polymorphic_last_index_of(int64_t obj, int64_t needle) {
    if (obj == 0) return -1;
    if (_wf_looks_like_list(obj)) return taida_list_last_index_of(obj, needle);
    if (_wf_looks_like_string(obj)) return taida_str_last_index_of(obj, needle);
    return -1;
}

/// Polymorphic .map(fn)
int64_t taida_polymorphic_map(int64_t obj, int64_t fn_ptr) {
    if (obj == 0 || obj < 4096) return obj;
    if (_wf_is_result(obj)) {
        // Result.map: if success, apply fn to value
        if (taida_result_is_ok(obj)) {
            int64_t value = taida_pack_get_idx(obj, 0);
            int64_t new_val = taida_invoke_callback1(fn_ptr, value);
            return taida_result_create(new_val, 0, 0);
        }
        return obj;
    }
    if (_wf_is_lax(obj)) {
        if (!taida_pack_get_idx(obj, 0)) return obj; // empty lax
        int64_t value = taida_pack_get_idx(obj, 1);
        int64_t def = taida_pack_get_idx(obj, 2);
        int64_t result = taida_invoke_callback1(fn_ptr, value);
        return taida_lax_new(result, def);
    }
    // Default: list.map
    return taida_list_map(obj, fn_ptr);
}

/// Monadic ops — moved to runtime_core_wasm.c (WC-5d)
extern int64_t taida_monadic_field_count(int64_t val);
extern int64_t taida_monadic_flat_map(int64_t obj, int64_t fn_ptr);
extern int64_t taida_monadic_get_or_throw(int64_t obj);

/// Detect Gorillax type: 0 = unknown, 1 = Gorillax, 2 = RelaxedGorillax
/// Unlike core's version, this does not use > 4096 threshold for the __type string.
/// In wasm-full, data section strings can be at any address.
static int _wf_gorillax_type(int64_t gx) {
    int64_t *p = (int64_t *)(intptr_t)gx;
    // __type field is at index 3: p[1 + 3*3 + 2] = p[12]
    int64_t type_str = p[1 + 3 * 3 + 2];
    if (type_str > 0 && _wf_looks_like_string(type_str)) {
        const char *s = (const char *)(intptr_t)type_str;
        if (s[0] == 'R') return 2; // "RelaxedGorillax"
        if (s[0] == 'G') return 1; // "Gorillax"
    }
    return 1; // default
}

/// Gorillax/RelaxedGorillax toString with proper type detection
static int64_t _wf_gorillax_to_str(int64_t gx) {
    int64_t is_ok = taida_pack_get_idx(gx, 0);
    int gtype = _wf_gorillax_type(gx);
    const char *prefix = (gtype == 2) ? "RelaxedGorillax(" : "Gorillax(";
    int prefix_len = (gtype == 2) ? 16 : 9;
    if (is_ok) {
        int64_t value = taida_pack_get_idx(gx, 1);
        int64_t value_str = taida_polymorphic_to_string(value);
        const char *vs = (const char *)(intptr_t)value_str;
        int vlen = _wf_strlen(vs);
        int need = prefix_len + vlen + 2;
        char *buf = (char *)wasm_alloc((unsigned int)(need));
        _wf_memcpy(buf, prefix, prefix_len);
        _wf_memcpy(buf + prefix_len, vs, vlen);
        buf[prefix_len + vlen] = ')';
        buf[prefix_len + vlen + 1] = '\0';
        return (int64_t)(intptr_t)buf;
    }
    if (gtype == 2) {
        return (int64_t)(intptr_t)"RelaxedGorillax(escaped)";
    }
    return (int64_t)(intptr_t)"Gorillax(><)";
}

/// Polymorphic .toString() — wasm-full override.
/// Fixes Gorillax/RelaxedGorillax type detection that fails in core due to
/// the > 4096 address threshold for data section strings.
/// Linked via #define redirect: taida_polymorphic_to_string -> _full.
int64_t taida_polymorphic_to_string_full(int64_t obj) {
    if (obj == 0) return (int64_t)(intptr_t)"0";
    // Check Gorillax BEFORE delegating to core (core's gorillax type detection
    // has the > 4096 threshold issue)
    if (_wf_is_valid_ptr(obj, 104)) {
        int64_t *p = (int64_t *)(intptr_t)obj;
        if (p[0] == 4 && p[1] == 0x6550c1c5b98b56bfLL) { // WASM_HASH_IS_OK
            return _wf_gorillax_to_str(obj);
        }
    }
    // Delegate everything else to core
    return taida_polymorphic_to_string(obj);
}

/// Monadic .toString() — moved to runtime_core_wasm.c (WC-5d)
/// Note: wasm-full overrides taida_polymorphic_to_string via #define redirect,
/// so core's taida_monadic_to_string (which calls taida_polymorphic_to_string)
/// will automatically use taida_polymorphic_to_string_full in the full profile.
extern int64_t taida_monadic_to_string(int64_t obj);

// ===========================================================================
// WF-3a: JSON runtime (no libc)
// ===========================================================================

// ---------------------------------------------------------------------------
// Shadow field registry for wasm-full (mirrors core's _wasm_field_registry)
// ---------------------------------------------------------------------------

#define WF_FIELD_REGISTRY_MAX 256

static struct {
    int64_t hash;
    const char *name;
    int64_t type_tag;
} _wf_field_registry[WF_FIELD_REGISTRY_MAX];
static int _wf_field_registry_count = 0;

/// Register field name+type into full's shadow registry, then delegate to core.
int64_t taida_register_field_name_full(int64_t hash, int64_t name_ptr) {
    // Store in shadow registry
    for (int i = 0; i < _wf_field_registry_count; i++) {
        if (_wf_field_registry[i].hash == hash) goto skip_add;
    }
    if (_wf_field_registry_count < WF_FIELD_REGISTRY_MAX) {
        _wf_field_registry[_wf_field_registry_count].hash = hash;
        _wf_field_registry[_wf_field_registry_count].name = (const char *)(intptr_t)name_ptr;
        _wf_field_registry[_wf_field_registry_count].type_tag = -1;
        _wf_field_registry_count++;
    }
skip_add:
    // Delegate to core's original
    return taida_register_field_name(hash, name_ptr);
}

int64_t taida_register_field_type_full(int64_t hash, int64_t name_ptr, int64_t type_tag) {
    // Update shadow registry
    for (int i = 0; i < _wf_field_registry_count; i++) {
        if (_wf_field_registry[i].hash == hash) {
            _wf_field_registry[i].type_tag = type_tag;
            goto do_core;
        }
    }
    if (_wf_field_registry_count < WF_FIELD_REGISTRY_MAX) {
        _wf_field_registry[_wf_field_registry_count].hash = hash;
        _wf_field_registry[_wf_field_registry_count].name = (const char *)(intptr_t)name_ptr;
        _wf_field_registry[_wf_field_registry_count].type_tag = type_tag;
        _wf_field_registry_count++;
    }
do_core:;
    // Core's register_field_type is defined in runtime_core_wasm.c (non-static)
    extern int64_t taida_register_field_type(int64_t, int64_t, int64_t);
    return taida_register_field_type(hash, name_ptr, type_tag);
}

/// Lookup field name/type: moved to runtime_core_wasm.c (WC-4)
extern int64_t taida_lookup_field_name(int64_t hash);
extern int64_t taida_lookup_field_type(int64_t hash, int64_t name_ptr);

// ---------------------------------------------------------------------------
// Numeric helpers (no libc: manual strtod/strtol/snprintf replacements)
// ---------------------------------------------------------------------------

/// Manual string-to-long (base 10). Returns parsed value and advances *end.
static int64_t _wf_strtol(const char *s, const char **end) {
    if (!s) { if (end) *end = s; return 0; }
    int64_t result = 0;
    int neg = 0;
    const char *p = s;
    if (*p == '-') { neg = 1; p++; }
    else if (*p == '+') { p++; }
    if (*p < '0' || *p > '9') { if (end) *end = s; return 0; }
    while (*p >= '0' && *p <= '9') {
        result = result * 10 + (*p - '0');
        p++;
    }
    if (end) *end = p;
    return neg ? -result : result;
}

/// Manual string-to-double. Handles integers, decimals, and scientific notation.
static double _wf_strtod(const char *s, const char **end) {
    if (!s) { if (end) *end = s; return 0.0; }
    const char *p = s;
    double result = 0.0;
    int neg = 0;
    if (*p == '-') { neg = 1; p++; }
    else if (*p == '+') { p++; }
    if (*p < '0' || *p > '9') {
        if (*p != '.') { if (end) *end = s; return 0.0; }
    }
    // Integer part
    while (*p >= '0' && *p <= '9') {
        result = result * 10.0 + (*p - '0');
        p++;
    }
    // Fractional part
    if (*p == '.') {
        p++;
        double frac = 0.1;
        while (*p >= '0' && *p <= '9') {
            result += (*p - '0') * frac;
            frac *= 0.1;
            p++;
        }
    }
    // Exponent part
    if (*p == 'e' || *p == 'E') {
        p++;
        int exp_neg = 0;
        if (*p == '-') { exp_neg = 1; p++; }
        else if (*p == '+') { p++; }
        int exp = 0;
        while (*p >= '0' && *p <= '9') {
            exp = exp * 10 + (*p - '0');
            p++;
        }
        double factor = 1.0;
        for (int i = 0; i < exp; i++) factor *= 10.0;
        if (exp_neg) result /= factor;
        else result *= factor;
    }
    if (end) *end = p;
    return neg ? -result : result;
}

/// Manual int64_t to string. Returns bump-allocated string.
static char *_wf_i64_to_str(int64_t val) {
    char tmp[24];
    int len = 0;
    int neg = 0;
    uint64_t uval;
    if (val < 0) { neg = 1; uval = (uint64_t)(-(val + 1)) + 1; }
    else { uval = (uint64_t)val; }
    if (uval == 0) { tmp[len++] = '0'; }
    else {
        while (uval > 0) { tmp[len++] = '0' + (int)(uval % 10); uval /= 10; }
    }
    int total = neg + len;
    char *buf = (char *)wasm_alloc((unsigned int)(total + 1));
    int pos = 0;
    if (neg) buf[pos++] = '-';
    for (int i = len - 1; i >= 0; i--) buf[pos++] = tmp[i];
    buf[pos] = '\0';
    return buf;
}

/// Manual double to string (like %g). Returns bump-allocated string.
static char *_wf_double_to_str(double val) {
    // Handle special values
    if (val != val) { /* NaN */
        char *r = (char *)wasm_alloc(4); r[0]='N'; r[1]='a'; r[2]='N'; r[3]='\0'; return r;
    }
    if (val > 1e18 || val < -1e18) {
        // Very large — print as integer-ish
        int neg = val < 0;
        if (neg) val = -val;
        // Use scientific notation approximation
        int exp = 0;
        double v = val;
        while (v >= 10.0) { v /= 10.0; exp++; }
        // Simple: just format like "Xe+YY"
        char *buf = (char *)wasm_alloc(32);
        int pos = 0;
        if (neg) buf[pos++] = '-';
        int d = (int)v;
        buf[pos++] = '0' + d;
        double frac = v - d;
        if (frac > 0.0001) {
            buf[pos++] = '.';
            for (int i = 0; i < 5 && frac > 0.00001; i++) {
                frac *= 10.0;
                int fd = (int)frac;
                buf[pos++] = '0' + fd;
                frac -= fd;
            }
            // Trim trailing zeros
            while (pos > 2 && buf[pos - 1] == '0') pos--;
            if (buf[pos - 1] == '.') pos--;
        }
        buf[pos++] = 'e';
        buf[pos++] = '+';
        if (exp >= 100) { buf[pos++] = '0' + exp / 100; exp %= 100; }
        buf[pos++] = '0' + exp / 10;
        buf[pos++] = '0' + exp % 10;
        buf[pos] = '\0';
        return buf;
    }
    // Normal range
    int neg = 0;
    if (val < 0) { neg = 1; val = -val; }
    int64_t int_part = (int64_t)val;
    double frac_part = val - (double)int_part;
    // If it's effectively an integer, print without decimal
    if (frac_part < 0.0000001 && frac_part > -0.0000001) {
        char *istr = _wf_i64_to_str(neg ? -int_part : int_part);
        return istr;
    }
    // Print with decimals (%g-like: strip trailing zeros)
    char *istr = _wf_i64_to_str(int_part);
    int ilen = _wf_strlen(istr);
    char *buf = (char *)wasm_alloc((unsigned int)(ilen + 18));
    int pos = 0;
    if (neg) buf[pos++] = '-';
    for (int i = 0; i < ilen; i++) buf[pos++] = istr[i];
    buf[pos++] = '.';
    // Up to 10 decimal digits
    for (int i = 0; i < 10; i++) {
        frac_part *= 10.0;
        int d = (int)frac_part;
        if (d > 9) d = 9;
        buf[pos++] = '0' + d;
        frac_part -= d;
        if (frac_part < 0.00000001) break;
    }
    // Strip trailing zeros
    while (pos > 0 && buf[pos - 1] == '0') pos--;
    if (pos > 0 && buf[pos - 1] == '.') pos--;
    buf[pos] = '\0';
    return buf;
}

// ---------------------------------------------------------------------------
// FNV-1a hash (kept in full for Bytes cursor etc.)
// ---------------------------------------------------------------------------

/// FNV-1a hash (matches Rust side)
static uint64_t _wf_fnv1a(const char *s, int len) {
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (int i = 0; i < len; i++) {
        hash ^= (unsigned char)s[i];
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

// ---------------------------------------------------------------------------
// JSON functions: moved to runtime_core_wasm.c (WC-4)
// ---------------------------------------------------------------------------
extern int64_t taida_json_schema_cast(int64_t raw, int64_t schema);
extern int64_t taida_json_parse(int64_t str_ptr);
extern int64_t taida_json_empty(void);
extern int64_t taida_json_from_int(int64_t value);
extern int64_t taida_json_from_str(int64_t str_ptr);
extern int64_t taida_json_unmold(int64_t json_ptr);
extern int64_t taida_json_stringify(int64_t json_ptr);
extern int64_t taida_json_to_str(int64_t json_ptr);
extern int64_t taida_json_to_int(int64_t json_ptr);
extern int64_t taida_json_size(int64_t json_ptr);
extern int64_t taida_json_has(int64_t json_ptr, int64_t key_ptr);
extern int64_t taida_debug_json(int64_t json_ptr);
extern int64_t taida_debug_list(int64_t list_ptr);
extern int64_t taida_json_encode(int64_t val);
extern int64_t taida_json_pretty(int64_t val);

// ===========================================================================
// WF-3b: Lax / Result / Gorillax extensions — moved to runtime_core_wasm.c (WC-5)
// ===========================================================================
extern int64_t taida_lax_map(int64_t lax_ptr, int64_t fn_ptr);
extern int64_t taida_lax_flat_map(int64_t lax_ptr, int64_t fn_ptr);
extern int64_t taida_lax_to_string(int64_t lax_ptr);
extern int64_t taida_result_is_error_check(int64_t result);
extern int64_t taida_result_get_or_default(int64_t result, int64_t def);
extern int64_t taida_result_map(int64_t result, int64_t fn_ptr);
extern int64_t taida_result_flat_map(int64_t result, int64_t fn_ptr);
extern int64_t taida_result_get_or_throw(int64_t result);
extern int64_t taida_result_to_string(int64_t result);
extern int64_t taida_gorillax_unmold(int64_t ptr);
extern int64_t taida_gorillax_to_string(int64_t ptr);
extern int64_t taida_relaxed_gorillax_unmold(int64_t ptr);
extern int64_t taida_relaxed_gorillax_to_string(int64_t ptr);

// ===========================================================================
// WF-3c: Bitwise / Shift / Char / Codepoint
// ===========================================================================

// --- Bitwise ---
int64_t taida_bit_and(int64_t a, int64_t b) { return (int64_t)(((uint64_t)a) & ((uint64_t)b)); }
int64_t taida_bit_or(int64_t a, int64_t b) { return (int64_t)(((uint64_t)a) | ((uint64_t)b)); }
int64_t taida_bit_xor(int64_t a, int64_t b) { return (int64_t)(((uint64_t)a) ^ ((uint64_t)b)); }
int64_t taida_bit_not(int64_t x) { return (int64_t)(~((uint64_t)x)); }

int64_t taida_shift_l(int64_t x, int64_t n) {
    if (n < 0 || n > 63) return taida_lax_empty(0);
    uint64_t shifted = ((uint64_t)x) << (unsigned int)n;
    return taida_lax_new((int64_t)shifted, 0);
}

int64_t taida_shift_r(int64_t x, int64_t n) {
    if (n < 0 || n > 63) return taida_lax_empty(0);
    int64_t shifted = ((int64_t)x) >> (unsigned int)n;
    return taida_lax_new(shifted, 0);
}

int64_t taida_shift_ru(int64_t x, int64_t n) {
    if (n < 0 || n > 63) return taida_lax_empty(0);
    uint64_t shifted = ((uint64_t)x) >> (unsigned int)n;
    return taida_lax_new((int64_t)shifted, 0);
}

// --- Char / Codepoint ---

static int _wf_utf8_encode_scalar(uint32_t cp, unsigned char out[4], int *out_len) {
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

static int _wf_utf8_decode_one(const unsigned char *buf, int len, int *consumed, uint32_t *out_cp) {
    if (len == 0) return 0;
    unsigned char b0 = buf[0];
    if (b0 < 0x80) { *consumed = 1; *out_cp = (uint32_t)b0; return 1; }
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
        unsigned char b1 = buf[1], b2 = buf[2];
        if ((b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80) return 0;
        if (b0 == 0xE0 && b1 < 0xA0) return 0;
        if (b0 == 0xED && b1 >= 0xA0) return 0;
        uint32_t cp = ((uint32_t)(b0 & 0x0F) << 12) | ((uint32_t)(b1 & 0x3F) << 6) | (uint32_t)(b2 & 0x3F);
        if (cp >= 0xD800 && cp <= 0xDFFF) return 0;
        *consumed = 3; *out_cp = cp;
        return 1;
    }
    if (b0 >= 0xF0 && b0 <= 0xF4) {
        if (len < 4) return 0;
        unsigned char b1 = buf[1], b2 = buf[2], b3 = buf[3];
        if ((b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80 || (b3 & 0xC0) != 0x80) return 0;
        if (b0 == 0xF0 && b1 < 0x90) return 0;
        if (b0 == 0xF4 && b1 > 0x8F) return 0;
        uint32_t cp = ((uint32_t)(b0 & 0x07) << 18) | ((uint32_t)(b1 & 0x3F) << 12) | ((uint32_t)(b2 & 0x3F) << 6) | (uint32_t)(b3 & 0x3F);
        if (cp > 0x10FFFF) return 0;
        *consumed = 4; *out_cp = cp;
        return 1;
    }
    return 0;
}

static int _wf_utf8_single_scalar(const unsigned char *buf, int len, uint32_t *cp_out) {
    int consumed = 0;
    uint32_t cp = 0;
    if (!_wf_utf8_decode_one(buf, len, &consumed, &cp)) return 0;
    if (consumed != len) return 0;
    *cp_out = cp;
    return 1;
}

// WC-1d: Char/Codepoint functions moved to runtime_core_wasm.c
// (see extern declarations at top of file)

// ===========================================================================
// WF-3d: Pack / Error / Field / Global / Bytes basic
// ===========================================================================

// --- Pack call field (invoke pack field as function) ---
extern int64_t taida_pack_get(int64_t pack_ptr, int64_t field_hash);

int64_t taida_pack_call_field0(int64_t pack_ptr, int64_t field_hash) {
    int64_t fn_ptr = taida_pack_get(pack_ptr, field_hash);
    if (taida_is_closure_value(fn_ptr)) {
        int64_t *closure = (int64_t *)(intptr_t)fn_ptr;
        int64_t env = taida_closure_get_env(fn_ptr);
        typedef int64_t (*cb)(int64_t);
        return ((cb)(intptr_t)taida_closure_get_fn(fn_ptr))(env);
    }
    typedef int64_t (*cb0)(void);
    return ((cb0)(intptr_t)fn_ptr)();
}

int64_t taida_pack_call_field1(int64_t pack_ptr, int64_t field_hash, int64_t a) {
    int64_t fn_ptr = taida_pack_get(pack_ptr, field_hash);
    return taida_invoke_callback1(fn_ptr, a);
}

int64_t taida_pack_call_field2(int64_t pack_ptr, int64_t field_hash, int64_t a, int64_t b) {
    int64_t fn_ptr = taida_pack_get(pack_ptr, field_hash);
    return taida_invoke_callback2(fn_ptr, a, b);
}

int64_t taida_pack_call_field3(int64_t pack_ptr, int64_t field_hash, int64_t a, int64_t b, int64_t c) {
    int64_t fn_ptr = taida_pack_get(pack_ptr, field_hash);
    if (taida_is_closure_value(fn_ptr)) {
        int64_t env = taida_closure_get_env(fn_ptr);
        typedef int64_t (*cb)(int64_t, int64_t, int64_t, int64_t);
        return ((cb)(intptr_t)taida_closure_get_fn(fn_ptr))(env, a, b, c);
    }
    typedef int64_t (*cb3)(int64_t, int64_t, int64_t);
    return ((cb3)(intptr_t)fn_ptr)(a, b, c);
}

/// Pack.toString()
int64_t taida_pack_to_display_string(int64_t pack_ptr) {
    return taida_polymorphic_to_string(pack_ptr);
}

/// make_io_error(msg) -> Error BuchiPack
int64_t taida_make_io_error(int64_t msg) {
    return taida_make_error(
        (int64_t)(intptr_t)"IOError",
        msg);
}

/// retain_and_tag_field -- no-op in WASM (no RC)
int64_t taida_retain_and_tag_field(int64_t val, int64_t tag) {
    (void)val; (void)tag;
    return 0;
}

// --- Global get/set ---
#define WF_GLOBAL_TABLE_SIZE 64
static int64_t _wf_global_keys[WF_GLOBAL_TABLE_SIZE];
static int64_t _wf_global_vals[WF_GLOBAL_TABLE_SIZE];
static int _wf_global_used[WF_GLOBAL_TABLE_SIZE];

int64_t taida_global_set(int64_t key, int64_t val) {
    unsigned int idx = (unsigned int)((uint64_t)key % WF_GLOBAL_TABLE_SIZE);
    for (int i = 0; i < WF_GLOBAL_TABLE_SIZE; i++) {
        unsigned int slot = (idx + (unsigned int)i) % WF_GLOBAL_TABLE_SIZE;
        if (!_wf_global_used[slot] || _wf_global_keys[slot] == key) {
            _wf_global_keys[slot] = key;
            _wf_global_vals[slot] = val;
            _wf_global_used[slot] = 1;
            return 0;
        }
    }
    return 0;
}

int64_t taida_global_get(int64_t key) {
    unsigned int idx = (unsigned int)((uint64_t)key % WF_GLOBAL_TABLE_SIZE);
    for (int i = 0; i < WF_GLOBAL_TABLE_SIZE; i++) {
        unsigned int slot = (idx + (unsigned int)i) % WF_GLOBAL_TABLE_SIZE;
        if (!_wf_global_used[slot]) return 0;
        if (_wf_global_keys[slot] == key) return _wf_global_vals[slot];
    }
    return 0;
}

// --- Bytes runtime ---
// Bytes layout (WASM): [BYTES_MAGIC, len, byte0, byte1, ...]
// No RC header in WASM -- bump allocator.
#define WF_BYTES_MAGIC 0x5441494442595400LL  // "TAIDBYT\0"

int64_t taida_bytes_default_value(void) {
    int64_t *bytes = (int64_t *)wasm_alloc(3 * 8);
    bytes[0] = WF_BYTES_MAGIC;
    bytes[1] = 0;  // len
    bytes[2] = 0;
    return (int64_t)(intptr_t)bytes;
}

static int _wf_is_bytes(int64_t val) {
    if (!_wf_is_valid_ptr(val, 16)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    return (p[0] & 0xFFFFFFFFFFFFFF00LL) == WF_BYTES_MAGIC;
}

int64_t taida_bytes_len(int64_t bytes_ptr) {
    if (!_wf_is_bytes(bytes_ptr)) return 0;
    return ((int64_t *)(intptr_t)bytes_ptr)[1];
}

int64_t taida_bytes_new_filled(int64_t len, int64_t fill) {
    if (len < 0) len = 0;
    int64_t *bytes = (int64_t *)wasm_alloc((unsigned int)((2 + len) * 8));
    bytes[0] = WF_BYTES_MAGIC;
    bytes[1] = len;
    for (int64_t i = 0; i < len; i++) bytes[2 + i] = fill;
    return (int64_t)(intptr_t)bytes;
}

int64_t taida_bytes_from_raw(int64_t ptr, int64_t len) {
    if (len < 0) len = 0;
    const unsigned char *data = (const unsigned char *)(intptr_t)ptr;
    int64_t *bytes = (int64_t *)wasm_alloc((unsigned int)((2 + len) * 8));
    bytes[0] = WF_BYTES_MAGIC;
    bytes[1] = len;
    for (int64_t i = 0; i < len; i++) bytes[2 + i] = (int64_t)data[i];
    return (int64_t)(intptr_t)bytes;
}

int64_t taida_bytes_clone(int64_t bytes_ptr) {
    if (!_wf_is_bytes(bytes_ptr)) return taida_bytes_default_value();
    int64_t *src = (int64_t *)(intptr_t)bytes_ptr;
    int64_t len = src[1];
    int64_t *dst = (int64_t *)wasm_alloc((unsigned int)((2 + len) * 8));
    for (int64_t i = 0; i < 2 + len; i++) dst[i] = src[i];
    return (int64_t)(intptr_t)dst;
}

int64_t taida_bytes_get_lax(int64_t bytes_ptr, int64_t idx) {
    if (!_wf_is_bytes(bytes_ptr)) return taida_lax_empty(0);
    int64_t len = ((int64_t *)(intptr_t)bytes_ptr)[1];
    if (idx < 0 || idx >= len) return taida_lax_empty(0);
    int64_t val = ((int64_t *)(intptr_t)bytes_ptr)[2 + idx];
    return taida_lax_new(val, 0);
}

int64_t taida_bytes_set(int64_t bytes_ptr, int64_t idx, int64_t val) {
    if (!_wf_is_bytes(bytes_ptr)) return taida_lax_empty(taida_bytes_default_value());
    int64_t len = ((int64_t *)(intptr_t)bytes_ptr)[1];
    if (idx < 0 || idx >= len) return taida_lax_empty(taida_bytes_default_value());
    if (val < 0 || val > 255) return taida_lax_empty(taida_bytes_default_value());
    int64_t out = taida_bytes_clone(bytes_ptr);
    ((int64_t *)(intptr_t)out)[2 + idx] = val;
    return taida_lax_new(out, taida_bytes_default_value());
}

int64_t taida_bytes_to_list(int64_t bytes_ptr) {
    int64_t list = taida_list_new();
    if (!_wf_is_bytes(bytes_ptr)) return list;
    int64_t *bytes = (int64_t *)(intptr_t)bytes_ptr;
    int64_t len = bytes[1];
    for (int64_t i = 0; i < len; i++) {
        list = taida_list_push(list, bytes[2 + i]);
    }
    return list;
}

/* Minimal string buffer for Bytes display (JSON buf was moved to core in WC-4) */
typedef struct { char *buf; int len; int cap; } _wf_strbuf;
static void _wf_sb_init(_wf_strbuf *sb) {
    sb->cap = 256; sb->buf = (char *)wasm_alloc(sb->cap); sb->len = 0;
    if (sb->buf) sb->buf[0] = '\0';
}
static void _wf_sb_append(_wf_strbuf *sb, const char *s) {
    int slen = _wf_strlen(s);
    if (sb->len + slen + 1 > sb->cap) {
        int new_cap = sb->cap;
        while (sb->len + slen + 1 > new_cap) new_cap *= 2;
        char *nb = (char *)wasm_alloc((unsigned int)new_cap);
        if (!nb) return;
        for (int i = 0; i < sb->len; i++) nb[i] = sb->buf[i];
        sb->buf = nb; sb->cap = new_cap;
    }
    for (int i = 0; i < slen; i++) sb->buf[sb->len + i] = s[i];
    sb->len += slen; sb->buf[sb->len] = '\0';
}

int64_t taida_bytes_to_display_string(int64_t bytes_ptr) {
    if (!_wf_is_bytes(bytes_ptr)) {
        return taida_str_new_copy((int64_t)(intptr_t)"Bytes()");
    }
    int64_t *bytes = (int64_t *)(intptr_t)bytes_ptr;
    int64_t len = bytes[1];
    _wf_strbuf jb;
    _wf_sb_init(&jb);
    _wf_sb_append(&jb, "Bytes([");
    for (int64_t i = 0; i < len; i++) {
        if (i > 0) _wf_sb_append(&jb, ", ");
        char *s = _wf_i64_to_str(bytes[2 + i]);
        _wf_sb_append(&jb, s);
    }
    _wf_sb_append(&jb, "])");
    return (int64_t)(intptr_t)jb.buf;
}

int64_t taida_bytes_mold(int64_t value, int64_t fill) {
    if (_wf_is_bytes(value)) {
        int64_t cloned = taida_bytes_clone(value);
        return taida_lax_new(cloned, taida_bytes_default_value());
    }
    if (_wf_looks_like_list(value)) {
        int64_t *list = (int64_t *)(intptr_t)value;
        int64_t len = list[1];
        int64_t out = taida_bytes_new_filled(len, 0);
        int64_t *ob = (int64_t *)(intptr_t)out;
        for (int64_t i = 0; i < len; i++) {
            int64_t item = list[4 + i];
            if (item < 0 || item > 255) {
                return taida_lax_empty(taida_bytes_default_value());
            }
            ob[2 + i] = item;
        }
        return taida_lax_new(out, taida_bytes_default_value());
    }
    // String
    if (_wf_looks_like_string(value)) {
        const char *s = (const char *)(intptr_t)value;
        int slen = _wf_strlen(s);
        int64_t out = taida_bytes_from_raw((int64_t)(intptr_t)s, (int64_t)slen);
        return taida_lax_new(out, taida_bytes_default_value());
    }
    // Integer length
    int64_t len = value;
    if (len < 0 || len > 10000000) return taida_lax_empty(taida_bytes_default_value());
    if (fill < 0 || fill > 255) return taida_lax_empty(taida_bytes_default_value());
    int64_t out = taida_bytes_new_filled(len, (int64_t)(unsigned char)fill);
    return taida_lax_new(out, taida_bytes_default_value());
}

// --- Bytes cursor ---
#define WF_HASH_CURSOR_BYTES   0xb66db66da4a4c5a2LL
#define WF_HASH_CURSOR_OFFSET  0x1234567890abcdefLL
#define WF_HASH_CURSOR_LENGTH  0xfedcba0987654321LL
#define WF_HASH_STEP_VALUE     0x0a7fc9f13472bbe0LL  /* FNV-1a("__value") */
#define WF_HASH_STEP_CURSOR    0xb66db66da4a4c5a3LL

static int _wf_bytes_cursor_unpack(int64_t cursor_ptr, int64_t *bytes_out, int64_t *offset_out) {
    // Cursor is a Pack with fields: [bytes, offset, length, __type]
    if (!_wf_is_valid_ptr(cursor_ptr, 8)) return 0;
    int64_t *pack = (int64_t *)(intptr_t)cursor_ptr;
    int64_t fc = pack[0];
    if (fc < 2) return 0;
    int64_t bytes_ptr = pack[1 + 0 * 3 + 2]; // field 0 value
    int64_t offset = pack[1 + 1 * 3 + 2];     // field 1 value
    if (!_wf_is_bytes(bytes_ptr)) return 0;
    int64_t len = taida_bytes_len(bytes_ptr);
    if (offset < 0) offset = 0;
    if (offset > len) offset = len;
    *bytes_out = bytes_ptr;
    *offset_out = offset;
    return 1;
}

static int64_t _wf_bytes_cursor_step(int64_t value, int64_t cursor) {
    int64_t pack = taida_pack_new(2);
    taida_pack_set_hash(pack, 0, WF_HASH_STEP_VALUE);
    taida_pack_set(pack, 0, value);
    taida_pack_set_hash(pack, 1, WF_HASH_STEP_CURSOR);
    taida_pack_set(pack, 1, cursor);
    return pack;
}

int64_t taida_bytes_cursor_new(int64_t bytes_ptr, int64_t offset) {
    if (!_wf_is_bytes(bytes_ptr)) bytes_ptr = taida_bytes_default_value();
    int64_t len = taida_bytes_len(bytes_ptr);
    if (offset < 0) offset = 0;
    if (offset > len) offset = len;
    int64_t pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, WF_HASH_CURSOR_BYTES);
    taida_pack_set(pack, 0, bytes_ptr);
    taida_pack_set_hash(pack, 1, WF_HASH_CURSOR_OFFSET);
    taida_pack_set(pack, 1, offset);
    taida_pack_set_hash(pack, 2, WF_HASH_CURSOR_LENGTH);
    taida_pack_set(pack, 2, len);
    uint64_t type_hash = _wf_fnv1a("__type", 6);
    taida_pack_set_hash(pack, 3, (int64_t)type_hash);
    taida_pack_set(pack, 3, (int64_t)(intptr_t)"BytesCursor");
    return pack;
}

int64_t taida_bytes_cursor_u8(int64_t cursor_ptr) {
    int64_t bytes_ptr = 0, offset = 0;
    if (!_wf_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) {
        int64_t empty_cursor = taida_bytes_cursor_new(taida_bytes_default_value(), 0);
        return taida_lax_empty(_wf_bytes_cursor_step(0, empty_cursor));
    }
    int64_t cur = taida_bytes_cursor_new(bytes_ptr, offset);
    int64_t def_step = _wf_bytes_cursor_step(0, cur);
    int64_t len = taida_bytes_len(bytes_ptr);
    if (offset >= len) return taida_lax_empty(def_step);
    int64_t val = ((int64_t *)(intptr_t)bytes_ptr)[2 + offset];
    int64_t next = taida_bytes_cursor_new(bytes_ptr, offset + 1);
    return taida_lax_new(_wf_bytes_cursor_step(val, next), def_step);
}

int64_t taida_bytes_cursor_take(int64_t cursor_ptr, int64_t size) {
    int64_t bytes_ptr = 0, offset = 0;
    if (!_wf_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) {
        int64_t empty_cursor = taida_bytes_cursor_new(taida_bytes_default_value(), 0);
        return taida_lax_empty(_wf_bytes_cursor_step(taida_bytes_default_value(), empty_cursor));
    }
    int64_t cur = taida_bytes_cursor_new(bytes_ptr, offset);
    int64_t def_step = _wf_bytes_cursor_step(taida_bytes_default_value(), cur);
    if (size < 0) return taida_lax_empty(def_step);
    int64_t len = taida_bytes_len(bytes_ptr);
    if (offset + size > len) return taida_lax_empty(def_step);
    int64_t *src = (int64_t *)(intptr_t)bytes_ptr;
    int64_t out = taida_bytes_new_filled(size, 0);
    int64_t *dst = (int64_t *)(intptr_t)out;
    for (int64_t i = 0; i < size; i++) dst[2 + i] = src[2 + offset + i];
    int64_t next = taida_bytes_cursor_new(bytes_ptr, offset + size);
    return taida_lax_new(_wf_bytes_cursor_step(out, next), def_step);
}

int64_t taida_bytes_cursor_step(int64_t cursor_ptr, int64_t n) {
    int64_t bytes_ptr = 0, offset = 0;
    if (!_wf_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) {
        return taida_bytes_cursor_new(taida_bytes_default_value(), 0);
    }
    return taida_bytes_cursor_new(bytes_ptr, offset + n);
}

int64_t taida_bytes_cursor_remaining(int64_t cursor_ptr) {
    int64_t bytes_ptr = 0, offset = 0;
    if (!_wf_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) return 0;
    return taida_bytes_len(bytes_ptr) - offset;
}

int64_t taida_bytes_cursor_unpack(int64_t cursor_ptr, int64_t schema) {
    (void)schema;
    int64_t bytes_ptr = 0, offset = 0;
    if (!_wf_bytes_cursor_unpack(cursor_ptr, &bytes_ptr, &offset)) return 0;
    return bytes_ptr;
}

// --- u16/u32 encode/decode molds ---
int64_t taida_u16be_mold(int64_t value) {
    if (value < 0 || value > 65535) return taida_lax_empty(taida_bytes_default_value());
    unsigned char raw[2];
    uint16_t n = (uint16_t)value;
    raw[0] = (unsigned char)((n >> 8) & 0xFF);
    raw[1] = (unsigned char)(n & 0xFF);
    int64_t out = taida_bytes_from_raw((int64_t)(intptr_t)raw, 2);
    return taida_lax_new(out, taida_bytes_default_value());
}

int64_t taida_u16le_mold(int64_t value) {
    if (value < 0 || value > 65535) return taida_lax_empty(taida_bytes_default_value());
    unsigned char raw[2];
    uint16_t n = (uint16_t)value;
    raw[0] = (unsigned char)(n & 0xFF);
    raw[1] = (unsigned char)((n >> 8) & 0xFF);
    int64_t out = taida_bytes_from_raw((int64_t)(intptr_t)raw, 2);
    return taida_lax_new(out, taida_bytes_default_value());
}

int64_t taida_u32be_mold(int64_t value) {
    if (value < 0 || value > 4294967295LL) return taida_lax_empty(taida_bytes_default_value());
    unsigned char raw[4];
    uint32_t n = (uint32_t)value;
    raw[0] = (unsigned char)((n >> 24) & 0xFF);
    raw[1] = (unsigned char)((n >> 16) & 0xFF);
    raw[2] = (unsigned char)((n >> 8) & 0xFF);
    raw[3] = (unsigned char)(n & 0xFF);
    int64_t out = taida_bytes_from_raw((int64_t)(intptr_t)raw, 4);
    return taida_lax_new(out, taida_bytes_default_value());
}

int64_t taida_u32le_mold(int64_t value) {
    if (value < 0 || value > 4294967295LL) return taida_lax_empty(taida_bytes_default_value());
    unsigned char raw[4];
    uint32_t n = (uint32_t)value;
    raw[0] = (unsigned char)(n & 0xFF);
    raw[1] = (unsigned char)((n >> 8) & 0xFF);
    raw[2] = (unsigned char)((n >> 16) & 0xFF);
    raw[3] = (unsigned char)((n >> 24) & 0xFF);
    int64_t out = taida_bytes_from_raw((int64_t)(intptr_t)raw, 4);
    return taida_lax_new(out, taida_bytes_default_value());
}

int64_t taida_u16be_decode_mold(int64_t value) {
    if (!_wf_is_bytes(value)) return taida_lax_empty(0);
    if (taida_bytes_len(value) < 2) return taida_lax_empty(0);
    int64_t *b = (int64_t *)(intptr_t)value;
    uint16_t n = (uint16_t)(((unsigned)b[2] << 8) | (unsigned)b[3]);
    return taida_lax_new((int64_t)n, 0);
}

int64_t taida_u16le_decode_mold(int64_t value) {
    if (!_wf_is_bytes(value)) return taida_lax_empty(0);
    if (taida_bytes_len(value) < 2) return taida_lax_empty(0);
    int64_t *b = (int64_t *)(intptr_t)value;
    uint16_t n = (uint16_t)((unsigned)b[2] | ((unsigned)b[3] << 8));
    return taida_lax_new((int64_t)n, 0);
}

int64_t taida_u32be_decode_mold(int64_t value) {
    if (!_wf_is_bytes(value)) return taida_lax_empty(0);
    if (taida_bytes_len(value) < 4) return taida_lax_empty(0);
    int64_t *b = (int64_t *)(intptr_t)value;
    uint32_t n = ((uint32_t)b[2] << 24) | ((uint32_t)b[3] << 16) | ((uint32_t)b[4] << 8) | (uint32_t)b[5];
    return taida_lax_new((int64_t)n, 0);
}

int64_t taida_u32le_decode_mold(int64_t value) {
    if (!_wf_is_bytes(value)) return taida_lax_empty(0);
    if (taida_bytes_len(value) < 4) return taida_lax_empty(0);
    int64_t *b = (int64_t *)(intptr_t)value;
    uint32_t n = (uint32_t)b[2] | ((uint32_t)b[3] << 8) | ((uint32_t)b[4] << 16) | ((uint32_t)b[5] << 24);
    return taida_lax_new((int64_t)n, 0);
}

int64_t taida_uint8_mold(int64_t v) {
    int64_t parsed = v;
    if (_wf_looks_like_string(v)) {
        const char *s = (const char *)(intptr_t)v;
        const char *end;
        parsed = _wf_strtol(s, &end);
        if (*end != '\0') parsed = v;
    }
    if (parsed < 0 || parsed > 255) return taida_lax_empty(0);
    return taida_lax_new(parsed, 0);
}

int64_t taida_uint8_mold_float(int64_t v) {
    double d = _to_double(v);
    if (d != d) return taida_lax_empty(0); // NaN
    if (d < 0.0 || d > 255.0) return taida_lax_empty(0);
    // Check integer
    double fl = (double)(int64_t)d;
    if (fl != d) return taida_lax_empty(0);
    return taida_lax_new((int64_t)d, 0);
}

// --- UTF-8 molds ---
int64_t taida_utf8_encode_mold(int64_t value) {
    const char *s = (const char *)(intptr_t)value;
    if (!s || !_wf_looks_like_string(value)) {
        return taida_lax_empty(taida_bytes_default_value());
    }
    int slen = _wf_strlen(s);
    int64_t out = taida_bytes_from_raw((int64_t)(intptr_t)s, (int64_t)slen);
    return taida_lax_new(out, taida_bytes_default_value());
}

int64_t taida_utf8_decode_mold(int64_t value) {
    if (!_wf_is_bytes(value)) return taida_lax_empty(taida_str_alloc(0));
    int64_t *bytes = (int64_t *)(intptr_t)value;
    int64_t len = bytes[1];
    // Extract raw bytes
    unsigned char *raw = (unsigned char *)wasm_alloc((unsigned int)len);
    for (int64_t i = 0; i < len; i++) raw[i] = (unsigned char)bytes[2 + i];
    // Validate UTF-8
    int pos = 0;
    while (pos < (int)len) {
        int consumed = 0;
        uint32_t cp = 0;
        if (!_wf_utf8_decode_one(raw + pos, (int)len - pos, &consumed, &cp)) {
            return taida_lax_empty(taida_str_alloc(0));
        }
        pos += consumed;
    }
    char *out = (char *)wasm_alloc((unsigned int)(len + 1));
    for (int64_t i = 0; i < len; i++) out[i] = (char)raw[i];
    out[len] = '\0';
    return taida_lax_new((int64_t)(intptr_t)out, taida_str_alloc(0));
}

int64_t taida_utf8_encode_scalar(int64_t v) {
    return taida_utf8_encode_mold(v);
}

int64_t taida_utf8_decode_one(int64_t v) {
    return taida_utf8_decode_mold(v);
}

int64_t taida_utf8_single_scalar(int64_t v) {
    return taida_codepoint_mold_str(v);
}

// --- Type detection stubs (for prototypes that are declared) ---
int64_t taida_is_string_value(int64_t val) { return _wf_looks_like_string(val); }
int64_t taida_is_list(int64_t val) { return _wf_looks_like_list(val); }
int64_t taida_is_hashmap(int64_t val) { return _wf_is_hashmap(val); }
int64_t taida_is_set(int64_t val) { return _wf_is_set(val); }
int64_t taida_is_buchi_pack(int64_t val) {
    if (!_wf_is_valid_ptr(val, 8)) return 0;
    int64_t *p = (int64_t *)(intptr_t)val;
    int64_t fc = p[0];
    if (fc > 0 && fc < 200) {
        int64_t h = p[1];
        if (h > 0x10000 || h < 0) return 1;
    }
    return 0;
}
int64_t taida_is_molten(int64_t val) { return 0; /* Molten is JS-only */ }
int64_t taida_is_bytes(int64_t val) { return _wf_is_bytes(val); }
int64_t taida_is_async(int64_t val) { return 0; /* no async in WASM */ }
int64_t taida_detect_value_tag(int64_t val) { return 0; }
int64_t taida_detect_gorillax_type(int64_t val) { return 0; }
int64_t taida_bool_to_int(int64_t v) { return v ? 1 : 0; }
int64_t taida_bool_to_str(int64_t v) {
    return taida_str_new_copy((int64_t)(intptr_t)(v ? "true" : "false"));
}
int64_t taida_value_to_display_string(int64_t val) { return taida_polymorphic_to_string(val); }
int64_t taida_value_to_debug_string(int64_t val) { return taida_polymorphic_to_string(val); }
int64_t taida_has_magic_header(int64_t val) { return 0; }
int64_t taida_ptr_is_readable(int64_t val) { return _wf_is_valid_ptr(val, 1); }
int64_t taida_read_cstr_len_safe(int64_t ptr, int64_t max) {
    if (!_wf_is_valid_ptr(ptr, 1)) return 0;
    const char *s = (const char *)(intptr_t)ptr;
    int len = 0;
    while (len < (int)max && s[len]) len++;
    if (len >= (int)max) return 0;
    return (int64_t)len;
}
