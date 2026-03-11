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

// Forward declarations for functions defined later in this file
int64_t taida_str_get(int64_t s_raw, int64_t idx_raw);

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
// WF-2b: String allocation helpers (WASM bump allocator versions)
// ---------------------------------------------------------------------------

/// Allocate a NUL-terminated string buffer of `len` bytes (+ 1 for NUL).
/// Uses bump allocator. No hidden header needed (no RC in WASM).
int64_t taida_str_alloc(int64_t len_raw) {
    int len = (int)len_raw;
    if (len < 0) len = 0;
    char *buf = (char *)wasm_alloc((unsigned int)(len + 1));
    buf[len] = '\0';
    return (int64_t)buf;
}

/// Copy a NUL-terminated string into a newly allocated buffer.
int64_t taida_str_new_copy(int64_t src_raw) {
    const char *src = (const char *)src_raw;
    if (!src) {
        char *r = (char *)wasm_alloc(1);
        r[0] = '\0';
        return (int64_t)r;
    }
    int len = _wf_strlen(src);
    char *r = (char *)wasm_alloc((unsigned int)(len + 1));
    _wf_memcpy(r, src, len);
    r[len] = '\0';
    return (int64_t)r;
}

/// Release a string. No-op in WASM (bump allocator, no free).
void taida_str_release(int64_t s) {
    (void)s;
}

// ---------------------------------------------------------------------------
// WF-2b: String mold implementations
// ---------------------------------------------------------------------------

/// Upper[str]() -- convert ASCII lowercase to uppercase
int64_t taida_str_to_upper(int64_t s_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    char *r = (char *)wasm_alloc((unsigned int)(len + 1));
    for (int i = 0; i < len; i++) {
        r[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
    }
    r[len] = '\0';
    return (int64_t)r;
}

/// Lower[str]() -- convert ASCII uppercase to lowercase
int64_t taida_str_to_lower(int64_t s_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    char *r = (char *)wasm_alloc((unsigned int)(len + 1));
    for (int i = 0; i < len; i++) {
        r[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
    }
    r[len] = '\0';
    return (int64_t)r;
}

/// Trim[str]() -- strip leading and trailing whitespace
int64_t taida_str_trim(int64_t s_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    int start = 0, end = len;
    while (start < len && _wf_is_whitespace(s[start])) start++;
    while (end > start && _wf_is_whitespace(s[end - 1])) end--;
    int slen = end - start;
    char *r = (char *)wasm_alloc((unsigned int)(slen + 1));
    _wf_memcpy(r, s + start, slen);
    r[slen] = '\0';
    return (int64_t)r;
}

/// TrimStart[str]() -- strip leading whitespace
int64_t taida_str_trim_start(int64_t s_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    int start = 0;
    while (start < len && _wf_is_whitespace(s[start])) start++;
    int slen = len - start;
    char *r = (char *)wasm_alloc((unsigned int)(slen + 1));
    _wf_memcpy(r, s + start, slen);
    r[slen] = '\0';
    return (int64_t)r;
}

/// TrimEnd[str]() -- strip trailing whitespace
int64_t taida_str_trim_end(int64_t s_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    int end = len;
    while (end > 0 && _wf_is_whitespace(s[end - 1])) end--;
    char *r = (char *)wasm_alloc((unsigned int)(end + 1));
    _wf_memcpy(r, s, end);
    r[end] = '\0';
    return (int64_t)r;
}

/// Split[str, sep]() -- split string by separator, return list of strings.
/// If sep is empty, splits into individual characters.
int64_t taida_str_split(int64_t s_raw, int64_t sep_raw) {
    const char *s = (const char *)s_raw;
    const char *sep = (const char *)sep_raw;
    if (!s) return taida_list_new();
    int64_t list = taida_list_new();
    if (!sep || _wf_strlen(sep) == 0) {
        // Split into individual characters
        int len = _wf_strlen(s);
        for (int i = 0; i < len; i++) {
            char *c = (char *)wasm_alloc(2);
            c[0] = s[i];
            c[1] = '\0';
            list = taida_list_push(list, (int64_t)c);
        }
        return list;
    }
    int sep_len = _wf_strlen(sep);
    const char *p = s;
    while (1) {
        const char *found = _wf_strstr(p, sep);
        if (!found) {
            int slen = _wf_strlen(p);
            char *part = (char *)wasm_alloc((unsigned int)(slen + 1));
            _wf_memcpy(part, p, slen);
            part[slen] = '\0';
            list = taida_list_push(list, (int64_t)part);
            break;
        }
        int plen = (int)(found - p);
        char *part = (char *)wasm_alloc((unsigned int)(plen + 1));
        _wf_memcpy(part, p, plen);
        part[plen] = '\0';
        list = taida_list_push(list, (int64_t)part);
        p = found + sep_len;
    }
    return list;
}

/// Replace[str, from, to](all <= true) -- replace all occurrences
int64_t taida_str_replace(int64_t s_raw, int64_t from_raw, int64_t to_raw) {
    const char *s = (const char *)s_raw;
    const char *from = (const char *)from_raw;
    const char *to = (const char *)to_raw;
    if (!s || !from || !to) {
        if (!s) { return taida_str_alloc(0); }
        return taida_str_new_copy(s_raw);
    }
    int from_len = _wf_strlen(from);
    int to_len = _wf_strlen(to);
    if (from_len == 0) {
        return taida_str_new_copy(s_raw);
    }
    // Count occurrences
    int count = 0;
    const char *p = s;
    while ((p = _wf_strstr(p, from)) != (const char *)0) { count++; p += from_len; }
    int s_len = _wf_strlen(s);
    int new_len = s_len + count * (to_len - from_len);
    char *r = (char *)wasm_alloc((unsigned int)(new_len + 1));
    char *dst = r;
    p = s;
    while (1) {
        const char *found = _wf_strstr(p, from);
        if (!found) {
            int remaining = _wf_strlen(p);
            _wf_memcpy(dst, p, remaining);
            dst += remaining;
            break;
        }
        int chunk = (int)(found - p);
        _wf_memcpy(dst, p, chunk); dst += chunk;
        _wf_memcpy(dst, to, to_len); dst += to_len;
        p = found + from_len;
    }
    *dst = '\0';
    return (int64_t)r;
}

/// ReplaceFirst[str, from, to]() -- replace first occurrence only
int64_t taida_str_replace_first(int64_t s_raw, int64_t from_raw, int64_t to_raw) {
    const char *s = (const char *)s_raw;
    const char *from = (const char *)from_raw;
    const char *to = (const char *)to_raw;
    if (!s || !from || !to) {
        if (!s) { return taida_str_alloc(0); }
        return taida_str_new_copy(s_raw);
    }
    int from_len = _wf_strlen(from);
    int to_len = _wf_strlen(to);
    if (from_len == 0) {
        return taida_str_new_copy(s_raw);
    }
    const char *found = _wf_strstr(s, from);
    if (!found) {
        return taida_str_new_copy(s_raw);
    }
    int s_len = _wf_strlen(s);
    int new_len = s_len - from_len + to_len;
    char *r = (char *)wasm_alloc((unsigned int)(new_len + 1));
    int prefix = (int)(found - s);
    _wf_memcpy(r, s, prefix);
    _wf_memcpy(r + prefix, to, to_len);
    int suffix = s_len - prefix - from_len;
    _wf_memcpy(r + prefix + to_len, found + from_len, suffix);
    r[new_len] = '\0';
    return (int64_t)r;
}

/// Slice[str](start, end) -- extract substring from start to end
int64_t taida_str_slice(int64_t s_raw, int64_t start_raw, int64_t end_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    int start = (int)start_raw;
    int end = (int)end_raw;
    /* Native normalizes negative end to len (e.g. end=-1 means "to end of string") */
    if (end < 0) end = len;
    if (start < 0) start = 0;
    if (end > len) end = len;
    if (start >= end) { return taida_str_alloc(0); }
    int slen = end - start;
    char *r = (char *)wasm_alloc((unsigned int)(slen + 1));
    _wf_memcpy(r, s + start, slen);
    r[slen] = '\0';
    return (int64_t)r;
}

/// CharAt[str, index]() -- extract single character at index
int64_t taida_str_char_at(int64_t s_raw, int64_t idx_raw) {
    const char *s = (const char *)s_raw;
    int idx = (int)idx_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    if (idx < 0 || idx >= len) { return taida_str_alloc(0); }
    char *r = (char *)wasm_alloc(2);
    r[0] = s[idx];
    r[1] = '\0';
    return (int64_t)r;
}

/// Repeat[str, n]() -- repeat string n times
int64_t taida_str_repeat(int64_t s_raw, int64_t n_raw) {
    const char *s = (const char *)s_raw;
    int n = (int)n_raw;
    if (!s || n <= 0) { return taida_str_alloc(0); }
    int slen = _wf_strlen(s);
    if (slen == 0) { return taida_str_alloc(0); }
    int total = slen * n;
    char *r = (char *)wasm_alloc((unsigned int)(total + 1));
    for (int i = 0; i < n; i++) {
        _wf_memcpy(r + i * slen, s, slen);
    }
    r[total] = '\0';
    return (int64_t)r;
}

/// Reverse[str]() -- reverse characters
int64_t taida_str_reverse(int64_t s_raw) {
    const char *s = (const char *)s_raw;
    if (!s) { return taida_str_alloc(0); }
    int len = _wf_strlen(s);
    char *r = (char *)wasm_alloc((unsigned int)(len + 1));
    for (int i = 0; i < len; i++) {
        r[i] = s[len - 1 - i];
    }
    r[len] = '\0';
    return (int64_t)r;
}

/// Pad[str, target_len](padChar, padEnd) -- pad string to target length
int64_t taida_str_pad(int64_t s_raw, int64_t target_len_raw, int64_t pad_char_raw, int64_t pad_end_raw) {
    const char *s = (const char *)s_raw;
    int target_len = (int)target_len_raw;
    const char *pad_char = (const char *)pad_char_raw;
    int pad_end = (int)pad_end_raw;
    if (!s) { return taida_str_alloc(0); }
    int slen = _wf_strlen(s);
    if (slen >= target_len) {
        return taida_str_new_copy(s_raw);
    }
    int pad_len = target_len - slen;
    char pc = ' ';
    if (pad_char && _wf_strlen(pad_char) > 0) pc = pad_char[0];
    char *r = (char *)wasm_alloc((unsigned int)(target_len + 1));
    if (pad_end) {
        _wf_memcpy(r, s, slen);
        for (int i = 0; i < pad_len; i++) r[slen + i] = pc;
    } else {
        for (int i = 0; i < pad_len; i++) r[i] = pc;
        _wf_memcpy(r + pad_len, s, slen);
    }
    r[target_len] = '\0';
    return (int64_t)r;
}

/// str.contains(sub) -- check if string contains substring
int64_t taida_str_contains(int64_t s_raw, int64_t sub_raw) {
    const char *s = (const char *)s_raw;
    const char *sub = (const char *)sub_raw;
    if (!s || !sub) return 0;
    return _wf_strstr(s, sub) != (const char *)0 ? 1 : 0;
}

/// str.startsWith(prefix) -- check if string starts with prefix
int64_t taida_str_starts_with(int64_t s_raw, int64_t prefix_raw) {
    const char *s = (const char *)s_raw;
    const char *prefix = (const char *)prefix_raw;
    if (!s || !prefix) return 0;
    int plen = _wf_strlen(prefix);
    return _wf_strncmp(s, prefix, plen) == 0 ? 1 : 0;
}

/// str.endsWith(suffix) -- check if string ends with suffix
int64_t taida_str_ends_with(int64_t s_raw, int64_t suffix_raw) {
    const char *s = (const char *)s_raw;
    const char *suffix = (const char *)suffix_raw;
    if (!s || !suffix) return 0;
    int slen = _wf_strlen(s);
    int suflen = _wf_strlen(suffix);
    if (suflen > slen) return 0;
    return _wf_strcmp(s + slen - suflen, suffix) == 0 ? 1 : 0;
}

/// str.indexOf(sub) -- find first index of substring, or -1
int64_t taida_str_index_of(int64_t s_raw, int64_t sub_raw) {
    const char *s = (const char *)s_raw;
    const char *sub = (const char *)sub_raw;
    if (!s || !sub) return -1;
    const char *p = _wf_strstr(s, sub);
    if (!p) return -1;
    return (int64_t)(p - s);
}

/// str.lastIndexOf(sub) -- find last index of substring, or -1
int64_t taida_str_last_index_of(int64_t s_raw, int64_t sub_raw) {
    const char *s = (const char *)s_raw;
    const char *sub = (const char *)sub_raw;
    if (!s || !sub) return -1;
    int slen = _wf_strlen(s);
    int sublen = _wf_strlen(sub);
    if (sublen > slen) return -1;
    for (int i = slen - sublen; i >= 0; i--) {
        if (_wf_strncmp(s + i, sub, sublen) == 0) return (int64_t)i;
    }
    return -1;
}

/// str.get(index) -- get character at index as Lax[Str]
int64_t taida_str_get(int64_t s_raw, int64_t idx_raw) {
    const char *s = (const char *)s_raw;
    int idx = (int)idx_raw;
    if (!s) return taida_lax_empty((int64_t)"");
    int len = _wf_strlen(s);
    if (idx < 0 || idx >= len) return taida_lax_empty((int64_t)"");
    char *r = (char *)wasm_alloc(2);
    r[0] = s[idx];
    r[1] = '\0';
    return taida_lax_new((int64_t)r, (int64_t)"");
}

/// cmp_strings -- comparator for sorting string pointers
int64_t taida_cmp_strings(int64_t a_raw, int64_t b_raw) {
    const char *a = (const char *)a_raw;
    const char *b = (const char *)b_raw;
    if (!a && !b) return 0;
    if (!a) return -1;
    if (!b) return 1;
    return (int64_t)_wf_strcmp(a, b);
}

/// Slice mold -- polymorphic slice for Str, List, Bytes
int64_t taida_slice_mold(int64_t value, int64_t start_raw, int64_t end_raw) {
    // For now, delegate to taida_str_slice for string values.
    // List/Bytes slice will be added in WF-2d/WF-3c.
    // Native uses type tags to distinguish; in WASM we check the value heuristically.
    // Since compile_str_molds.td only uses string Slice, this is sufficient for WF-2b.
    return taida_str_slice(value, start_raw, end_raw);
}

// ---------------------------------------------------------------------------
// WF-2c: Number mold implementations
// ---------------------------------------------------------------------------
// In WASM, all parameters are int64_t (bit-punned for floats).
// Use _to_double() / _d2l() for conversion.

// ── Float math molds ─────────────────────────────────────

/// Floor[f]() -- floor(x), returns float (bit-punned int64_t)
int64_t taida_float_floor(int64_t a) {
    double d = _to_double(a);
    // Manual floor: truncate toward negative infinity
    double t = (double)(long long)d;
    if (t > d) t -= 1.0;
    return _d2l(t);
}

/// Ceil[f]() -- ceil(x), returns float (bit-punned int64_t)
int64_t taida_float_ceil(int64_t a) {
    double d = _to_double(a);
    double t = (double)(long long)d;
    if (t < d) t += 1.0;
    return _d2l(t);
}

/// Round[f]() -- round(x) to nearest, ties away from zero
int64_t taida_float_round(int64_t a) {
    double d = _to_double(a);
    // round half away from zero
    double t;
    if (d >= 0.0) {
        t = (double)(long long)(d + 0.5);
    } else {
        t = (double)(long long)(d - 0.5);
    }
    return _d2l(t);
}

/// Abs[f]() -- absolute value of float
int64_t taida_float_abs(int64_t a) {
    double d = _to_double(a);
    return _d2l(d < 0.0 ? -d : d);
}

/// Clamp[f, lo, hi]() -- clamp float to range [lo, hi]
int64_t taida_float_clamp(int64_t a, int64_t lo, int64_t hi) {
    double da = _to_double(a);
    double dlo = _to_double(lo);
    double dhi = _to_double(hi);
    if (da < dlo) return lo;
    if (da > dhi) return hi;
    return a;
}

// --- Manual float-to-string helper for ToFixed ---

/// Write the integer part of |val| into buf, return number of chars written.
static int _wf_write_uint64(char *buf, uint64_t val) {
    if (val == 0) { buf[0] = '0'; return 1; }
    char tmp[20];
    int n = 0;
    while (val > 0) {
        tmp[n++] = '0' + (int)(val % 10);
        val /= 10;
    }
    for (int i = 0; i < n; i++) buf[i] = tmp[n - 1 - i];
    return n;
}

/// ToFixed[f, digits]() -- format float to string with N decimal places
int64_t taida_float_to_fixed(int64_t a, int64_t digits_raw) {
    double d = _to_double(a);
    int digits = (int)digits_raw;
    if (digits < 0) digits = 0;
    if (digits > 20) digits = 20;

    // Handle NaN
    // NaN: d != d
    if (d != d) {
        char *r = (char *)wasm_alloc(4);
        r[0] = 'N'; r[1] = 'a'; r[2] = 'N'; r[3] = '\0';
        return (int64_t)r;
    }

    int negative = 0;
    if (d < 0.0) { negative = 1; d = -d; }

    // Check infinity: d > 1e18 && d == d * 2 (heuristic)
    // Actually: infinity is when d * 0.0 != 0.0
    double zero_test = d * 0.0;
    if (zero_test != 0.0 || (d > 0.0 && d == d + d)) {
        // infinity
        if (negative) {
            char *r = (char *)wasm_alloc(5);
            r[0] = '-'; r[1] = 'i'; r[2] = 'n'; r[3] = 'f'; r[4] = '\0';
            return (int64_t)r;
        } else {
            char *r = (char *)wasm_alloc(4);
            r[0] = 'i'; r[1] = 'n'; r[2] = 'f'; r[3] = '\0';
            return (int64_t)r;
        }
    }

    // Round to `digits` decimal places
    double multiplier = 1.0;
    for (int i = 0; i < digits; i++) multiplier *= 10.0;
    double rounded = d * multiplier;
    // Round half away from zero
    rounded = (double)(long long)(rounded + 0.5);
    // Now convert integer part and fractional part
    uint64_t total = (uint64_t)rounded;
    uint64_t int_part = total;
    uint64_t frac_part = 0;
    if (digits > 0) {
        uint64_t divisor = (uint64_t)multiplier;
        int_part = total / divisor;
        frac_part = total % divisor;
    }

    char buf[80];
    int pos = 0;
    if (negative) buf[pos++] = '-';
    pos += _wf_write_uint64(buf + pos, int_part);
    if (digits > 0) {
        buf[pos++] = '.';
        // Write frac_part with leading zeros
        for (int i = digits - 1; i >= 0; i--) {
            uint64_t p = 1;
            for (int j = 0; j < i; j++) p *= 10;
            int digit = (int)((frac_part / p) % 10);
            buf[pos++] = '0' + digit;
        }
    }
    buf[pos] = '\0';

    char *r = (char *)wasm_alloc((unsigned int)(pos + 1));
    _wf_memcpy(r, buf, pos + 1);
    return (int64_t)r;
}

// ── Float state check methods ────────────────────────────

/// isNaN -- NaN != NaN
int64_t taida_float_is_nan(int64_t a) {
    double d = _to_double(a);
    return d != d ? 1 : 0;
}

/// isInfinite -- d * 0 != 0 and not NaN
int64_t taida_float_is_infinite(int64_t a) {
    double d = _to_double(a);
    if (d != d) return 0;  // NaN is not infinite
    double z = d * 0.0;
    return z != 0.0 ? 1 : 0;
}

/// isFinite -- not NaN and not infinite
int64_t taida_float_is_finite_check(int64_t a) {
    double d = _to_double(a);
    if (d != d) return 0;  // NaN
    double z = d * 0.0;
    if (z != 0.0) return 0;  // infinity
    return 1;
}

int64_t taida_float_is_positive(int64_t a) {
    double d = _to_double(a);
    return d > 0.0 ? 1 : 0;
}

int64_t taida_float_is_negative(int64_t a) {
    double d = _to_double(a);
    return d < 0.0 ? 1 : 0;
}

int64_t taida_float_is_zero(int64_t a) {
    double d = _to_double(a);
    return d == 0.0 ? 1 : 0;
}

// ── Int methods ──────────────────────────────────────────

int64_t taida_int_clamp(int64_t a, int64_t lo, int64_t hi) {
    if (a < lo) return lo;
    if (a > hi) return hi;
    return a;
}

int64_t taida_int_is_positive(int64_t a) { return a > 0 ? 1 : 0; }
int64_t taida_int_is_negative(int64_t a) { return a < 0 ? 1 : 0; }
int64_t taida_int_is_zero(int64_t a) { return a == 0 ? 1 : 0; }

// ── Int mold auto / str_base ─────────────────────────────

/// digit_to_char -- 0-9 -> '0'-'9', 10-35 -> 'a'-'z'
static int64_t _wf_digit_to_char(int64_t digit) {
    return (digit < 10) ? ('0' + digit) : ('a' + (digit - 10));
}

/// char_to_digit -- '0'-'9' -> 0-9, 'a'-'z' -> 10-35, 'A'-'Z' -> 10-35, else -1
static int _wf_char_to_digit(int c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'z') return c - 'a' + 10;
    if (c >= 'A' && c <= 'Z') return c - 'A' + 10;
    return -1;
}

/// Int[v]() auto-detect: tries to distinguish int, string, other
int64_t taida_int_mold_auto(int64_t v) {
    // Simple heuristic: if v looks like a small int, return it
    // If v is 0, return Lax(0, 0)
    if (v == 0) return taida_lax_new(0, 0);
    if (v < 0 || v < 4096) return taida_lax_new(v, 0);

    // Try to read as string
    const char *s = (const char *)(intptr_t)v;
    // Safety: check first byte is printable ASCII
    // In WASM linear memory, all valid pointers are readable
    char c = s[0];
    if (c == '-' || c == '+' || (c >= '0' && c <= '9')) {
        // Try parsing as integer string
        int neg = 0;
        int i = 0;
        if (c == '-') { neg = 1; i = 1; }
        else if (c == '+') { i = 1; }
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
    }

    // Otherwise treat as raw int value
    return taida_lax_new(v, 0);
}

/// Int[str, base]() -- parse string in given base
int64_t taida_int_mold_str_base(int64_t v, int64_t base) {
    if (base < 2 || base > 36) return taida_lax_empty(0);
    const char *s = (const char *)(intptr_t)v;
    if (!s || s[0] == '\0') return taida_lax_empty(0);
    int len = _wf_strlen(s);

    int negative = 0;
    int i = 0;
    if (s[0] == '-') {
        negative = 1;
        i = 1;
        if (len == 1) return taida_lax_empty(0);
    }

    uint64_t acc = 0;
    for (; i < len; i++) {
        int d = _wf_char_to_digit((unsigned char)s[i]);
        if (d < 0 || d >= (int)base) return taida_lax_empty(0);
        acc = acc * (uint64_t)base + (uint64_t)d;
    }

    int64_t out;
    if (negative) {
        out = -(int64_t)acc;
    } else {
        out = (int64_t)acc;
    }
    return taida_lax_new(out, 0);
}

// ── to_radix ─────────────────────────────────────────────

int64_t taida_digit_to_char(int64_t digit) {
    return _wf_digit_to_char(digit);
}

int64_t taida_char_to_digit_fn(int64_t v) {
    return _wf_char_to_digit((int)v);
}

/// ToRadix[value, base]() -- convert int to string in given base
int64_t taida_to_radix(int64_t value, int64_t base) {
    if (base < 2 || base > 36) return taida_lax_empty((int64_t)"");
    if (value == 0) {
        char *out = (char *)wasm_alloc(2);
        out[0] = '0';
        out[1] = '\0';
        return taida_lax_new((int64_t)out, (int64_t)"");
    }

    uint64_t mag = value < 0
        ? (uint64_t)(-(value + 1)) + 1
        : (uint64_t)value;
    char tmp[70];
    int pos = 0;
    while (mag > 0) {
        uint64_t rem = mag % (uint64_t)base;
        tmp[pos++] = (char)_wf_digit_to_char((int64_t)rem);
        mag /= (uint64_t)base;
    }
    if (value < 0) tmp[pos++] = '-';

    char *out = (char *)wasm_alloc((unsigned int)(pos + 1));
    for (int i = 0; i < pos; i++) {
        out[i] = tmp[pos - 1 - i];
    }
    out[pos] = '\0';
    return taida_lax_new((int64_t)out, (int64_t)"");
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

// --- Callback invoke ---

/// Invoke a callback (plain function or closure) with 1 argument.
int64_t taida_invoke_callback1(int64_t fn_ptr, int64_t arg0) {
    if (taida_is_closure_value(fn_ptr)) {
        int64_t *closure = (int64_t *)(intptr_t)fn_ptr;
        int64_t user_arity = closure[3];
        if (user_arity == 0) {
            // Zero-param lambda: call with env only, ignore arg0
            typedef int64_t (*closure_fn0_t)(int64_t);
            closure_fn0_t func = (closure_fn0_t)(intptr_t)closure[1];
            return func(closure[2]);
        }
        // 1+ param lambda: call with env + arg0
        typedef int64_t (*closure_fn1_t)(int64_t, int64_t);
        closure_fn1_t func = (closure_fn1_t)(intptr_t)closure[1];
        return func(closure[2], arg0);
    }
    typedef int64_t (*fn_t)(int64_t);
    fn_t func = (fn_t)(intptr_t)fn_ptr;
    return func(arg0);
}

/// Invoke a callback (plain function or closure) with 2 arguments.
int64_t taida_invoke_callback2(int64_t fn_ptr, int64_t arg0, int64_t arg1) {
    if (taida_is_closure_value(fn_ptr)) {
        int64_t *closure = (int64_t *)(intptr_t)fn_ptr;
        // Closure with 2 user args: call with env + arg0 + arg1
        typedef int64_t (*closure_fn2_t)(int64_t, int64_t, int64_t);
        closure_fn2_t func = (closure_fn2_t)(intptr_t)closure[1];
        return func(closure[2], arg0, arg1);
    }
    typedef int64_t (*fn_t)(int64_t, int64_t);
    fn_t func = (fn_t)(intptr_t)fn_ptr;
    return func(arg0, arg1);
}

// --- List map/filter ---

int64_t taida_list_map(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t new_list = taida_list_new();
    // map may change element type, so leave elem_tag as UNKNOWN
    for (int64_t i = 0; i < len; i++) {
        int64_t result = taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i]);
        new_list = taida_list_push(new_list, result);
    }
    return new_list;
}

int64_t taida_list_filter(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) {
            new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
        }
    }
    return new_list;
}

// --- fold / foldr ---

int64_t taida_list_fold(int64_t list_ptr, int64_t init, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t acc = init;
    for (int64_t i = 0; i < len; i++) {
        acc = taida_invoke_callback2(fn_ptr, acc, list[WF_LIST_ELEMS + i]);
    }
    return acc;
}

int64_t taida_list_foldr(int64_t list_ptr, int64_t init, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t acc = init;
    for (int64_t i = len - 1; i >= 0; i--) {
        acc = taida_invoke_callback2(fn_ptr, acc, list[WF_LIST_ELEMS + i]);
    }
    return acc;
}

// --- find / find_index ---

int64_t taida_list_find(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        int64_t item = list[WF_LIST_ELEMS + i];
        if (taida_invoke_callback1(fn_ptr, item)) {
            return taida_lax_new(item, 0);
        }
    }
    return taida_lax_empty(0);
}

int64_t taida_list_find_index(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) return i;
    }
    return -1;
}

// --- index_of / last_index_of / contains ---

int64_t taida_list_index_of(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (list[WF_LIST_ELEMS + i] == item) return i;
    }
    return -1;
}

int64_t taida_list_last_index_of(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = len - 1; i >= 0; i--) {
        if (list[WF_LIST_ELEMS + i] == item) return i;
    }
    return -1;
}

int64_t taida_list_contains(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (list[WF_LIST_ELEMS + i] == item) return 1;
    }
    return 0;
}

// --- first / last / min / max / sum ---

int64_t taida_list_first(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    return taida_lax_new(list[WF_LIST_ELEMS], 0);
}

int64_t taida_list_last(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    return taida_lax_new(list[WF_LIST_ELEMS + len - 1], 0);
}

int64_t taida_list_min(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    int64_t min_val = list[WF_LIST_ELEMS];
    for (int64_t i = 1; i < len; i++) {
        if (list[WF_LIST_ELEMS + i] < min_val) min_val = list[WF_LIST_ELEMS + i];
    }
    return taida_lax_new(min_val, 0);
}

int64_t taida_list_max(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    int64_t max_val = list[WF_LIST_ELEMS];
    for (int64_t i = 1; i < len; i++) {
        if (list[WF_LIST_ELEMS + i] > max_val) max_val = list[WF_LIST_ELEMS + i];
    }
    return taida_lax_new(max_val, 0);
}

int64_t taida_list_sum(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t sum = 0;
    for (int64_t i = 0; i < len; i++) {
        sum += list[WF_LIST_ELEMS + i];
    }
    return sum;
}

// --- sort / sort_desc ---

int64_t taida_list_sort(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    // Copy items into temp array (on bump allocator)
    int64_t *items = (int64_t *)wasm_alloc((unsigned int)(len * 8));
    for (int64_t i = 0; i < len; i++) items[i] = list[WF_LIST_ELEMS + i];
    // Insertion sort ascending
    for (int64_t i = 1; i < len; i++) {
        int64_t key = items[i];
        int64_t j = i - 1;
        while (j >= 0 && items[j] > key) { items[j+1] = items[j]; j--; }
        items[j+1] = key;
    }
    for (int64_t i = 0; i < len; i++) {
        new_list = taida_list_push(new_list, items[i]);
    }
    return new_list;
}

int64_t taida_list_sort_desc(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    int64_t *items = (int64_t *)wasm_alloc((unsigned int)(len * 8));
    for (int64_t i = 0; i < len; i++) items[i] = list[WF_LIST_ELEMS + i];
    // Insertion sort descending
    for (int64_t i = 1; i < len; i++) {
        int64_t key = items[i];
        int64_t j = i - 1;
        while (j >= 0 && items[j] < key) { items[j+1] = items[j]; j--; }
        items[j+1] = key;
    }
    for (int64_t i = 0; i < len; i++) {
        new_list = taida_list_push(new_list, items[i]);
    }
    return new_list;
}

// --- unique / flatten / reverse ---

int64_t taida_list_unique(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl_init = (int64_t *)(intptr_t)new_list;
    nl_init[2] = elem_tag;
    for (int64_t i = 0; i < len; i++) {
        int64_t item = list[WF_LIST_ELEMS + i];
        // Check if already in new_list
        int64_t *nl = (int64_t *)(intptr_t)new_list;
        int64_t nlen = nl[1];
        int64_t found = 0;
        for (int64_t j = 0; j < nlen; j++) {
            if (nl[WF_LIST_ELEMS + j] == item) { found = 1; break; }
        }
        if (!found) {
            new_list = taida_list_push(new_list, item);
        }
    }
    return new_list;
}

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
/// Same heuristic as _looks_like_string in runtime_core_wasm.c (which is static).
/// Static strings in the data section can have low addresses, so we do NOT use
/// an address threshold (e.g. < 4096) — instead we validate memory bounds and
/// check that the first bytes are plausible ASCII/UTF-8.
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

int64_t taida_list_flatten(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t new_list = taida_list_new();
    for (int64_t i = 0; i < len; i++) {
        int64_t item = list[WF_LIST_ELEMS + i];
        if (_wf_looks_like_list(item)) {
            int64_t *sub = (int64_t *)(intptr_t)item;
            int64_t slen = sub[1];
            // Propagate inner list's elem_tag to result
            if (i == 0) {
                int64_t *nl = (int64_t *)(intptr_t)new_list;
                nl[2] = sub[2];
            }
            for (int64_t j = 0; j < slen; j++) {
                new_list = taida_list_push(new_list, sub[WF_LIST_ELEMS + j]);
            }
        } else {
            new_list = taida_list_push(new_list, item);
        }
    }
    return new_list;
}

int64_t taida_list_reverse(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = len - 1; i >= 0; i--) {
        new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
    }
    return new_list;
}

// --- join ---

int64_t taida_list_join(int64_t list_ptr, int64_t sep_raw) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_str_alloc(0);
    const char *sep = (const char *)(intptr_t)sep_raw;
    if (!sep) sep = "";
    int sep_len = _wf_strlen(sep);

    // Convert each element through polymorphic_to_string
    // Allocate a temp pointer array on bump allocator
    const char **strs = (const char **)wasm_alloc((unsigned int)(len * sizeof(const char *)));
    int total = 0;
    for (int64_t i = 0; i < len; i++) {
        strs[i] = (const char *)(intptr_t)taida_polymorphic_to_string(list[WF_LIST_ELEMS + i]);
        total += _wf_strlen(strs[i]);
        if (i > 0) total += sep_len;
    }

    char *r = (char *)wasm_alloc((unsigned int)(total + 1));
    char *dst = r;
    for (int64_t i = 0; i < len; i++) {
        if (i > 0 && sep_len > 0) { _wf_memcpy(dst, sep, sep_len); dst += sep_len; }
        int sl = _wf_strlen(strs[i]);
        _wf_memcpy(dst, strs[i], sl);
        dst += sl;
    }
    *dst = '\0';
    return (int64_t)r;
}

// --- concat / append / prepend ---

int64_t taida_list_concat(int64_t list1, int64_t list2) {
    int64_t *l1 = (int64_t *)(intptr_t)list1;
    int64_t *l2 = (int64_t *)(intptr_t)list2;
    int64_t len1 = l1[1], len2 = l2[1];
    int64_t elem_tag = l1[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < len1; i++) {
        new_list = taida_list_push(new_list, l1[WF_LIST_ELEMS + i]);
    }
    for (int64_t i = 0; i < len2; i++) {
        new_list = taida_list_push(new_list, l2[WF_LIST_ELEMS + i]);
    }
    return new_list;
}

int64_t taida_list_append(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < len; i++) {
        new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
    }
    new_list = taida_list_push(new_list, item);
    return new_list;
}

int64_t taida_list_prepend(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    new_list = taida_list_push(new_list, item);
    for (int64_t i = 0; i < len; i++) {
        new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
    }
    return new_list;
}

// --- count ---

int64_t taida_list_count(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t count = 0;
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) count++;
    }
    return count;
}

// --- take / take_while / drop / drop_while ---

int64_t taida_list_take(int64_t list_ptr, int64_t n) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t take_n = n < len ? n : len;
    if (take_n < 0) take_n = 0;
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < take_n; i++) {
        new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
    }
    return new_list;
}

int64_t taida_list_take_while(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) {
            new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
        } else {
            break;
        }
    }
    return new_list;
}

int64_t taida_list_drop(int64_t list_ptr, int64_t n) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t skip = n < len ? n : len;
    if (skip < 0) skip = 0;
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = skip; i < len; i++) {
        new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
    }
    return new_list;
}

int64_t taida_list_drop_while(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    int64_t dropping = 1;
    for (int64_t i = 0; i < len; i++) {
        if (dropping && taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) {
            continue;
        }
        dropping = 0;
        new_list = taida_list_push(new_list, list[WF_LIST_ELEMS + i]);
    }
    return new_list;
}

// --- enumerate / zip ---

int64_t taida_list_enumerate(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t new_list = taida_list_new();
    for (int64_t i = 0; i < len; i++) {
        int64_t pair = taida_pack_new(2);
        taida_pack_set_hash(pair, 0, (int64_t)WF_HASH_INDEX);
        taida_pack_set(pair, 0, i);
        taida_pack_set_hash(pair, 1, (int64_t)WF_HASH_VALUE);
        taida_pack_set(pair, 1, list[WF_LIST_ELEMS + i]);
        new_list = taida_list_push(new_list, pair);
    }
    return new_list;
}

int64_t taida_list_zip(int64_t list1, int64_t list2) {
    int64_t *l1 = (int64_t *)(intptr_t)list1;
    int64_t *l2 = (int64_t *)(intptr_t)list2;
    int64_t len1 = l1[1], len2 = l2[1];
    int64_t min_len = len1 < len2 ? len1 : len2;
    int64_t new_list = taida_list_new();
    for (int64_t i = 0; i < min_len; i++) {
        int64_t pair = taida_pack_new(2);
        taida_pack_set_hash(pair, 0, (int64_t)WF_HASH_FIRST);
        taida_pack_set(pair, 0, l1[WF_LIST_ELEMS + i]);
        taida_pack_set_hash(pair, 1, (int64_t)WF_HASH_SECOND);
        taida_pack_set(pair, 1, l2[WF_LIST_ELEMS + i]);
        new_list = taida_list_push(new_list, pair);
    }
    return new_list;
}

// --- any / all / none ---

int64_t taida_list_any(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) return 1;
    }
    return 0;
}

int64_t taida_list_all(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (!taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) return 0;
    }
    return 1;
}

int64_t taida_list_none(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WF_LIST_ELEMS + i])) return 0;
    }
    return 1;
}

// --- to_display_string ---

int64_t taida_list_to_display_string(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) {
        char *r = (char *)wasm_alloc(3);
        r[0] = '@'; r[1] = '['; r[2] = ']';
        char *result = (char *)wasm_alloc(4);
        _wf_memcpy(result, "@[]", 4);
        return (int64_t)result;
    }
    // Build "@[elem, elem, ...]"
    // First pass: convert all elements to strings
    const char **strs = (const char **)wasm_alloc((unsigned int)(len * sizeof(const char *)));
    int total = 3; // "@[" + "]"
    for (int64_t i = 0; i < len; i++) {
        strs[i] = (const char *)(intptr_t)taida_polymorphic_to_string(list[WF_LIST_ELEMS + i]);
        total += _wf_strlen(strs[i]);
        if (i > 0) total += 2; // ", "
    }
    char *r = (char *)wasm_alloc((unsigned int)(total + 1));
    r[0] = '@'; r[1] = '[';
    char *dst = r + 2;
    for (int64_t i = 0; i < len; i++) {
        if (i > 0) { dst[0] = ','; dst[1] = ' '; dst += 2; }
        int sl = _wf_strlen(strs[i]);
        _wf_memcpy(dst, strs[i], sl);
        dst += sl;
    }
    *dst++ = ']';
    *dst = '\0';
    return (int64_t)r;
}

// --- elem_retain / elem_release (no-ops in WASM) ---

void taida_list_elem_retain(int64_t list) { (void)list; }
void taida_list_elem_release(int64_t list) { (void)list; }

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
int64_t taida_polymorphic_has_value(int64_t obj) {
    if (obj == 0 || obj < 4096) return 0;
    if (_wf_is_lax(obj)) return taida_pack_get_idx(obj, 0);
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
/// This version adds Lax and Result support.
/// Linked via #define redirect in generated C: taida_polymorphic_is_empty -> _full.
int64_t taida_polymorphic_is_empty_full(int64_t ptr) {
    if (ptr == 0) return 1;
    // Lax: field 0 = hasValue; isEmpty = !hasValue
    if (_wf_is_lax(ptr)) return taida_pack_get_idx(ptr, 0) ? 0 : 1;
    // Result: isEmpty if error state (isError)
    if (_wf_is_result(ptr)) return taida_result_is_error(ptr);
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

/// Monadic field_count (for dispatch)
int64_t taida_monadic_field_count(int64_t val) {
    if (val == 0 || val < 4096) return 0;
    if (_wf_is_result(val)) return 3;
    if (_wf_is_lax(val)) return 4;
    return 0;
}

/// Monadic .flatMap(fn)
int64_t taida_monadic_flat_map(int64_t obj, int64_t fn_ptr) {
    if (obj == 0 || obj < 4096) return obj;
    if (_wf_is_result(obj)) {
        if (!taida_result_is_ok(obj)) return obj;
        int64_t value = taida_pack_get_idx(obj, 0);
        return taida_invoke_callback1(fn_ptr, value);
    }
    if (_wf_is_lax(obj)) {
        if (!taida_pack_get_idx(obj, 0)) return obj;
        int64_t value = taida_pack_get_idx(obj, 1);
        return taida_invoke_callback1(fn_ptr, value);
    }
    return obj;
}

/// Monadic .getOrThrow()
int64_t taida_monadic_get_or_throw(int64_t obj) {
    if (obj == 0 || obj < 4096) return obj;
    if (_wf_is_result(obj)) {
        if (taida_result_is_ok(obj)) return taida_pack_get_idx(obj, 0);
        int64_t throw_val = taida_pack_get_idx(obj, 2);
        if (taida_can_throw_payload(throw_val)) return taida_throw(throw_val);
        int64_t error = taida_make_error(
            (int64_t)(intptr_t)"ResultError",
            (int64_t)(intptr_t)"Result predicate failed");
        return taida_throw(error);
    }
    if (_wf_is_lax(obj)) return taida_lax_unmold(obj);
    return obj;
}

/// Monadic .toString()
int64_t taida_monadic_to_string(int64_t obj) {
    return taida_polymorphic_to_string(obj);
}

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

/// Lookup field name from shadow registry
int64_t taida_lookup_field_name(int64_t hash) {
    for (int i = 0; i < _wf_field_registry_count; i++) {
        if (_wf_field_registry[i].hash == hash)
            return (int64_t)(intptr_t)_wf_field_registry[i].name;
    }
    return 0;
}

/// Lookup field type tag from shadow registry
int64_t taida_lookup_field_type(int64_t hash, int64_t name_ptr) {
    (void)name_ptr;
    for (int i = 0; i < _wf_field_registry_count; i++) {
        if (_wf_field_registry[i].hash == hash)
            return _wf_field_registry[i].type_tag;
    }
    return -1;
}

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
// JSON value types and parser
// ---------------------------------------------------------------------------

enum {
    WF_JSON_NULL = 0,
    WF_JSON_INT,
    WF_JSON_FLOAT,
    WF_JSON_STRING,
    WF_JSON_BOOL,
    WF_JSON_ARRAY,
    WF_JSON_OBJECT
};

typedef struct wf_json_array wf_json_array;
typedef struct wf_json_obj wf_json_obj;
typedef struct wf_json_obj_entry wf_json_obj_entry;

typedef struct {
    int type;
    int64_t int_val;
    double float_val;
    char *str_val;
    wf_json_array *arr;
    wf_json_obj *obj;
} wf_json_val;

struct wf_json_array {
    wf_json_val *items;
    int count;
    int cap;
};

struct wf_json_obj_entry {
    char *key;
    wf_json_val value;
};

struct wf_json_obj {
    wf_json_obj_entry *entries;
    int count;
    int cap;
};

// Forward declarations
static wf_json_val _wf_json_parse_value(const char **p);
static void _wf_json_skip_ws(const char **p);
static int64_t _wf_json_apply_schema(wf_json_val *jval, const char **desc);

/// FNV-1a hash (matches Rust side)
static uint64_t _wf_fnv1a(const char *s, int len) {
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (int i = 0; i < len; i++) {
        hash ^= (unsigned char)s[i];
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

static void _wf_json_skip_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\n' || **p == '\r') (*p)++;
}

/// Parse a JSON string, handling escape sequences. Returns bump-allocated string.
static char *_wf_json_parse_string_raw(const char **p) {
    if (**p != '"') return (char *)0;
    (*p)++;
    // First pass: compute length
    const char *scan = *p;
    int len = 0;
    while (*scan && *scan != '"') {
        if (*scan == '\\') { scan++; if (*scan) scan++; }
        else scan++;
        len++;
    }
    // Allocate and copy with escape handling
    char *buf = (char *)wasm_alloc((unsigned int)(len + 1));
    int out = 0;
    while (**p && **p != '"') {
        if (**p == '\\') {
            (*p)++;
            switch (**p) {
                case '"': buf[out++] = '"'; break;
                case '\\': buf[out++] = '\\'; break;
                case '/': buf[out++] = '/'; break;
                case 'n': buf[out++] = '\n'; break;
                case 't': buf[out++] = '\t'; break;
                case 'r': buf[out++] = '\r'; break;
                case 'b': buf[out++] = '\b'; break;
                case 'f': buf[out++] = '\f'; break;
                default: buf[out++] = **p; break;
            }
            (*p)++;
        } else {
            buf[out++] = **p;
            (*p)++;
        }
    }
    buf[out] = '\0';
    if (**p == '"') (*p)++;
    return buf;
}

static wf_json_val _wf_json_parse_string(const char **p) {
    wf_json_val v;
    v.type = WF_JSON_STRING;
    v.str_val = _wf_json_parse_string_raw(p);
    v.arr = (wf_json_array *)0;
    v.obj = (wf_json_obj *)0;
    v.int_val = 0;
    v.float_val = 0.0;
    return v;
}

static wf_json_val _wf_json_parse_number(const char **p) {
    wf_json_val v;
    v.str_val = (char *)0; v.arr = (wf_json_array *)0; v.obj = (wf_json_obj *)0;
    const char *end;
    double d = _wf_strtod(*p, &end);
    // Check if integer (no decimal point or exponent)
    int is_int = 1;
    const char *scan = *p;
    if (*scan == '-') scan++;
    while (scan < end) {
        if (*scan == '.' || *scan == 'e' || *scan == 'E') { is_int = 0; break; }
        scan++;
    }
    *p = end;
    if (is_int && d >= -9007199254740992.0 && d <= 9007199254740992.0) {
        v.type = WF_JSON_INT;
        v.int_val = (int64_t)d;
        v.float_val = d;
    } else {
        v.type = WF_JSON_FLOAT;
        v.float_val = d;
        v.int_val = (int64_t)d;
    }
    return v;
}

static wf_json_val _wf_json_parse_array(const char **p) {
    wf_json_val v;
    v.type = WF_JSON_ARRAY;
    v.str_val = (char *)0; v.obj = (wf_json_obj *)0;
    v.int_val = 0; v.float_val = 0.0;
    v.arr = (wf_json_array *)wasm_alloc(sizeof(wf_json_array));
    v.arr->count = 0;
    v.arr->cap = 4;
    v.arr->items = (wf_json_val *)wasm_alloc((unsigned int)(4 * sizeof(wf_json_val)));
    (*p)++; // skip '['
    _wf_json_skip_ws(p);
    if (**p == ']') { (*p)++; return v; }
    while (**p) {
        wf_json_val item = _wf_json_parse_value(p);
        if (v.arr->count >= v.arr->cap) {
            int new_cap = v.arr->cap * 2;
            wf_json_val *new_items = (wf_json_val *)wasm_alloc(
                (unsigned int)(new_cap * sizeof(wf_json_val)));
            for (int i = 0; i < v.arr->count; i++) new_items[i] = v.arr->items[i];
            v.arr->items = new_items;
            v.arr->cap = new_cap;
        }
        v.arr->items[v.arr->count++] = item;
        _wf_json_skip_ws(p);
        if (**p == ',') { (*p)++; _wf_json_skip_ws(p); }
        else break;
    }
    if (**p == ']') (*p)++;
    return v;
}

static wf_json_val _wf_json_parse_object(const char **p) {
    wf_json_val v;
    v.type = WF_JSON_OBJECT;
    v.str_val = (char *)0; v.arr = (wf_json_array *)0;
    v.int_val = 0; v.float_val = 0.0;
    v.obj = (wf_json_obj *)wasm_alloc(sizeof(wf_json_obj));
    v.obj->count = 0;
    v.obj->cap = 8;
    v.obj->entries = (wf_json_obj_entry *)wasm_alloc(
        (unsigned int)(8 * sizeof(wf_json_obj_entry)));
    (*p)++; // skip '{'
    _wf_json_skip_ws(p);
    if (**p == '}') { (*p)++; return v; }
    while (**p) {
        _wf_json_skip_ws(p);
        char *key = _wf_json_parse_string_raw(p);
        _wf_json_skip_ws(p);
        if (**p == ':') (*p)++;
        _wf_json_skip_ws(p);
        wf_json_val val = _wf_json_parse_value(p);
        if (v.obj->count >= v.obj->cap) {
            int new_cap = v.obj->cap * 2;
            wf_json_obj_entry *new_entries = (wf_json_obj_entry *)wasm_alloc(
                (unsigned int)(new_cap * sizeof(wf_json_obj_entry)));
            for (int i = 0; i < v.obj->count; i++) new_entries[i] = v.obj->entries[i];
            v.obj->entries = new_entries;
            v.obj->cap = new_cap;
        }
        v.obj->entries[v.obj->count].key = key;
        v.obj->entries[v.obj->count].value = val;
        v.obj->count++;
        _wf_json_skip_ws(p);
        if (**p == ',') { (*p)++; _wf_json_skip_ws(p); }
        else break;
    }
    if (**p == '}') (*p)++;
    return v;
}

static wf_json_val _wf_json_parse_value(const char **p) {
    _wf_json_skip_ws(p);
    wf_json_val v;
    v.str_val = (char *)0; v.arr = (wf_json_array *)0; v.obj = (wf_json_obj *)0;
    v.int_val = 0; v.float_val = 0.0;
    if (**p == '"') return _wf_json_parse_string(p);
    if (**p == '{') return _wf_json_parse_object(p);
    if (**p == '[') return _wf_json_parse_array(p);
    if (**p == 't' && _wf_strncmp(*p, "true", 4) == 0) {
        *p += 4; v.type = WF_JSON_BOOL; v.int_val = 1; return v;
    }
    if (**p == 'f' && _wf_strncmp(*p, "false", 5) == 0) {
        *p += 5; v.type = WF_JSON_BOOL; v.int_val = 0; return v;
    }
    if (**p == 'n' && _wf_strncmp(*p, "null", 4) == 0) {
        *p += 4; v.type = WF_JSON_NULL; v.int_val = 0; return v;
    }
    if (**p == '-' || (**p >= '0' && **p <= '9')) return _wf_json_parse_number(p);
    // Parse error
    v.type = WF_JSON_NULL; v.int_val = 0;
    return v;
}

// --- JSON object field lookup ---
static wf_json_val *_wf_json_obj_get(wf_json_obj *obj, const char *key) {
    if (!obj) return (wf_json_val *)0;
    for (int i = 0; i < obj->count; i++) {
        if (_wf_strcmp(obj->entries[i].key, key) == 0) {
            return &obj->entries[i].value;
        }
    }
    return (wf_json_val *)0;
}

// --- Schema helpers ---

static int _wf_schema_find_closing_brace(const char *desc) {
    int depth = 1;
    int i = 0;
    while (desc[i] && depth > 0) {
        if (desc[i] == '{') depth++;
        if (desc[i] == '}') depth--;
        if (depth > 0) i++;
    }
    return i;
}

// --- Default values from schema ---
static int64_t _wf_json_default_value_for_desc(const char *desc) {
    if (!desc || !*desc) return 0;
    switch (desc[0]) {
        case 'i': return 0;
        case 'f': return _d2l(0.0);
        case 's': {
            char *empty = (char *)wasm_alloc(1);
            empty[0] = '\0';
            return (int64_t)(intptr_t)empty;
        }
        case 'b': return 0;
        case 'T': {
            wf_json_val null_val;
            null_val.type = WF_JSON_NULL;
            null_val.str_val = (char *)0; null_val.arr = (wf_json_array *)0;
            null_val.obj = (wf_json_obj *)0;
            null_val.int_val = 0; null_val.float_val = 0.0;
            return _wf_json_apply_schema(&null_val, &desc);
        }
        case 'L': {
            return taida_list_new();
        }
        default: return 0;
    }
}

// --- Convert JSON value to typed value ---
static int64_t _wf_json_to_int(wf_json_val *jv) {
    if (!jv) return 0;
    switch (jv->type) {
        case WF_JSON_INT: return jv->int_val;
        case WF_JSON_FLOAT: return (int64_t)jv->float_val;
        case WF_JSON_BOOL: return jv->int_val;
        case WF_JSON_STRING: {
            if (!jv->str_val) return 0;
            const char *end;
            int64_t r = _wf_strtol(jv->str_val, &end);
            if (*end != '\0') return 0;
            return r;
        }
        default: return 0;
    }
}

static int64_t _wf_json_to_float(wf_json_val *jv) {
    if (!jv) return _d2l(0.0);
    switch (jv->type) {
        case WF_JSON_FLOAT: return _d2l(jv->float_val);
        case WF_JSON_INT: return _d2l((double)jv->int_val);
        case WF_JSON_BOOL: return _d2l(jv->int_val ? 1.0 : 0.0);
        case WF_JSON_STRING: {
            if (!jv->str_val) return _d2l(0.0);
            const char *end;
            double r = _wf_strtod(jv->str_val, &end);
            if (*end != '\0') return _d2l(0.0);
            return _d2l(r);
        }
        default: return _d2l(0.0);
    }
}

static int64_t _wf_json_to_str(wf_json_val *jv) {
    if (!jv) return taida_str_alloc(0);
    switch (jv->type) {
        case WF_JSON_STRING: {
            if (!jv->str_val) return taida_str_alloc(0);
            return taida_str_new_copy((int64_t)(intptr_t)jv->str_val);
        }
        case WF_JSON_INT: {
            char *s = _wf_i64_to_str(jv->int_val);
            return (int64_t)(intptr_t)s;
        }
        case WF_JSON_FLOAT: {
            char *s = _wf_double_to_str(jv->float_val);
            return (int64_t)(intptr_t)s;
        }
        case WF_JSON_BOOL: {
            const char *src = jv->int_val ? "true" : "false";
            return taida_str_new_copy((int64_t)(intptr_t)src);
        }
        case WF_JSON_NULL:
            return taida_str_alloc(0);
        default:
            return taida_str_alloc(0);
    }
}

static int64_t _wf_json_to_bool(wf_json_val *jv) {
    if (!jv) return 0;
    switch (jv->type) {
        case WF_JSON_BOOL: return jv->int_val;
        case WF_JSON_INT: return jv->int_val != 0 ? 1 : 0;
        case WF_JSON_FLOAT: return jv->float_val != 0.0 ? 1 : 0;
        case WF_JSON_STRING: return (jv->str_val && jv->str_val[0]) ? 1 : 0;
        case WF_JSON_NULL: return 0;
        default: return 0;
    }
}

// --- Apply schema descriptor to JSON value ---
static int64_t _wf_json_apply_schema(wf_json_val *jval, const char **desc) {
    if (!desc || !*desc || !**desc) return 0;
    const char *d = *desc;

    switch (d[0]) {
        case 'i': {
            *desc = d + 1;
            if (!jval || jval->type == WF_JSON_NULL) return 0;
            return _wf_json_to_int(jval);
        }
        case 'f': {
            *desc = d + 1;
            if (!jval || jval->type == WF_JSON_NULL) return _d2l(0.0);
            return _wf_json_to_float(jval);
        }
        case 's': {
            *desc = d + 1;
            if (!jval || jval->type == WF_JSON_NULL) {
                return taida_str_alloc(0);
            }
            return _wf_json_to_str(jval);
        }
        case 'b': {
            *desc = d + 1;
            if (!jval || jval->type == WF_JSON_NULL) return 0;
            return _wf_json_to_bool(jval);
        }
        case 'T': {
            // T{TypeName|field1:desc,field2:desc,...}
            if (d[1] != '{') { *desc = d + 1; return 0; }
            d += 2; // skip "T{"
            char type_name[256];
            int tn_len = 0;
            while (*d && *d != '|' && tn_len < 255) { type_name[tn_len++] = *d; d++; }
            type_name[tn_len] = '\0';
            if (*d == '|') d++;

            // Count fields
            int field_count = 0;
            {
                const char *scan = d;
                if (*scan && *scan != '}') field_count = 1;
                int depth = 0;
                while (*scan && !(*scan == '}' && depth == 0)) {
                    if (*scan == '{') depth++;
                    if (*scan == '}') depth--;
                    if (*scan == ',' && depth == 0) field_count++;
                    scan++;
                }
            }

            // +1 for __type field
            int64_t pack = taida_pack_new(field_count + 1);

            int idx = 0;
            while (*d && *d != '}') {
                // Read field name
                char fname[256];
                int fn_len = 0;
                while (*d && *d != ':' && *d != '}' && fn_len < 255) { fname[fn_len++] = *d; d++; }
                fname[fn_len] = '\0';
                if (*d == ':') d++;

                // FNV-1a hash
                uint64_t hash = _wf_fnv1a(fname, fn_len);
                taida_pack_set_hash(pack, idx, (int64_t)hash);

                // Look up field in JSON object
                wf_json_val *field_jval = (wf_json_val *)0;
                if (jval && jval->type == WF_JSON_OBJECT) {
                    field_jval = _wf_json_obj_get(jval->obj, fname);
                }

                int64_t field_val = _wf_json_apply_schema(field_jval, &d);
                taida_pack_set(pack, idx, field_val);
                idx++;

                if (*d == ',') d++;
            }
            if (*d == '}') d++;

            // Add __type field
            uint64_t type_hash = _wf_fnv1a("__type", 6);
            taida_pack_set_hash(pack, idx, (int64_t)type_hash);
            char *type_str = (char *)wasm_alloc((unsigned int)(tn_len + 1));
            _wf_memcpy(type_str, type_name, tn_len + 1);
            taida_pack_set(pack, idx, (int64_t)(intptr_t)type_str);

            *desc = d;
            return pack;
        }
        case 'L': {
            // L{desc}
            if (d[1] != '{') { *desc = d + 1; return taida_list_new(); }
            d += 2; // skip "L{"
            int inner_len = _wf_schema_find_closing_brace(d);
            char *inner_desc = (char *)wasm_alloc((unsigned int)(inner_len + 1));
            _wf_memcpy(inner_desc, d, inner_len);
            inner_desc[inner_len] = '\0';

            int64_t list = taida_list_new();

            if (jval && jval->type == WF_JSON_ARRAY && jval->arr) {
                for (int i = 0; i < jval->arr->count; i++) {
                    const char *elem_desc = inner_desc;
                    int64_t elem = _wf_json_apply_schema(&jval->arr->items[i], &elem_desc);
                    list = taida_list_push(list, elem);
                }
            }

            d += inner_len;
            if (*d == '}') d++;
            *desc = d;
            return list;
        }
        default: {
            *desc = d + 1;
            return 0;
        }
    }
}

// ---------------------------------------------------------------------------
// WF-3a: Public JSON API functions
// ---------------------------------------------------------------------------

/// JSON[raw, Schema]() -> Lax[T]
int64_t taida_json_schema_cast(int64_t raw_ptr, int64_t schema_ptr) {
    const char *raw = (const char *)(intptr_t)raw_ptr;
    const char *schema = (const char *)(intptr_t)schema_ptr;

    if (!raw || !schema) {
        int64_t def = _wf_json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    const char *p = raw;
    _wf_json_skip_ws(&p);
    if (!*p) {
        int64_t def = _wf_json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    const char *before_parse = p;
    wf_json_val jval = _wf_json_parse_value(&p);

    if (p == before_parse) {
        int64_t def = _wf_json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    _wf_json_skip_ws(&p);
    if (*p != '\0') {
        int64_t def = _wf_json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    const char *desc = schema;
    int64_t result = _wf_json_apply_schema(&jval, &desc);
    int64_t def = _wf_json_default_value_for_desc(schema);
    return taida_lax_new(result, def);
}

/// taida_json_parse: copy raw JSON string
int64_t taida_json_parse(int64_t str_ptr) {
    const char *src = (const char *)(intptr_t)str_ptr;
    if (!src) src = "{}";
    int len = _wf_strlen(src);
    char *buf = (char *)wasm_alloc((unsigned int)(len + 1));
    _wf_memcpy(buf, src, len + 1);
    return (int64_t)(intptr_t)buf;
}

/// taida_json_empty: return "{}"
int64_t taida_json_empty(void) {
    char *buf = (char *)wasm_alloc(3);
    buf[0] = '{'; buf[1] = '}'; buf[2] = '\0';
    return (int64_t)(intptr_t)buf;
}

/// taida_json_from_int: serialize int as JSON string
int64_t taida_json_from_int(int64_t value) {
    char *s = _wf_i64_to_str(value);
    return (int64_t)(intptr_t)s;
}

/// taida_json_from_str: wrap string in quotes
int64_t taida_json_from_str(int64_t str_ptr) {
    const char *src = (const char *)(intptr_t)str_ptr;
    if (!src) src = "";
    int src_len = _wf_strlen(src);
    int new_len = src_len + 2;
    char *buf = (char *)wasm_alloc((unsigned int)(new_len + 1));
    buf[0] = '"';
    _wf_memcpy(buf + 1, src, src_len);
    buf[new_len - 1] = '"';
    buf[new_len] = '\0';
    return (int64_t)(intptr_t)buf;
}

/// taida_json_unmold: copy JSON string
int64_t taida_json_unmold(int64_t json_ptr) {
    const char *src = (const char *)(intptr_t)json_ptr;
    if (!src) return taida_str_alloc(0);
    int len = _wf_strlen(src);
    char *buf = (char *)wasm_alloc((unsigned int)(len + 1));
    _wf_memcpy(buf, src, len + 1);
    return (int64_t)(intptr_t)buf;
}

/// taida_json_stringify: same as unmold
int64_t taida_json_stringify(int64_t json_ptr) {
    return taida_json_unmold(json_ptr);
}

/// taida_json_to_str: same as unmold
int64_t taida_json_to_str(int64_t json_ptr) {
    return taida_json_unmold(json_ptr);
}

/// taida_json_to_int: parse JSON as integer
int64_t taida_json_to_int(int64_t json_ptr) {
    const char *data = (const char *)(intptr_t)json_ptr;
    if (!data) return 0;
    const char *end;
    return _wf_strtol(data, &end);
}

/// taida_json_size: length of JSON string
int64_t taida_json_size(int64_t json_ptr) {
    const char *data = (const char *)(intptr_t)json_ptr;
    if (!data) return 0;
    return (int64_t)_wf_strlen(data);
}

/// taida_json_has: check if key substring exists
int64_t taida_json_has(int64_t json_ptr, int64_t key_ptr) {
    const char *json_data = (const char *)(intptr_t)json_ptr;
    const char *key_data = (const char *)(intptr_t)key_ptr;
    if (!json_data || !key_data) return 0;
    return _wf_strstr(json_data, key_data) ? 1 : 0;
}

/// taida_debug_json: print JSON debug info to stdout
int64_t taida_debug_json(int64_t json_ptr) {
    // In WASM, we use fd_write. Build a simple output.
    const char *data = (const char *)(intptr_t)json_ptr;
    // Construct "JSON(...)\n" string and write to stdout
    const char *prefix = "JSON(";
    const char *suffix = ")\n";
    const char *body = data ? data : "null";
    int plen = _wf_strlen(prefix);
    int blen = _wf_strlen(body);
    int slen = _wf_strlen(suffix);
    int total = plen + blen + slen;
    char *buf = (char *)wasm_alloc((unsigned int)(total + 1));
    _wf_memcpy(buf, prefix, plen);
    _wf_memcpy(buf + plen, body, blen);
    _wf_memcpy(buf + plen + blen, suffix, slen);
    buf[total] = '\0';
    // Use WASI fd_write to output
    extern int fd_write(int fd, const void *iovs, int iovs_len, int *nwritten)
        __attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")));
    struct { const char *buf; int len; } iov = { buf, total };
    int nwritten;
    fd_write(1, &iov, 1, &nwritten);
    return 0;
}

/// taida_debug_list: print list debug info to stdout
int64_t taida_debug_list(int64_t list_ptr) {
    int64_t str = taida_list_to_display_string(list_ptr);
    const char *s = (const char *)(intptr_t)str;
    int len = _wf_strlen(s);
    extern int fd_write(int fd, const void *iovs, int iovs_len, int *nwritten)
        __attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")));
    char *buf = (char *)wasm_alloc((unsigned int)(len + 1));
    _wf_memcpy(buf, s, len);
    buf[len] = '\n';
    struct { const char *buf; int len; } iov = { buf, len + 1 };
    int nwritten;
    fd_write(1, &iov, 1, &nwritten);
    return 0;
}

// ---------------------------------------------------------------------------
// WF-3a: jsonEncode / jsonPretty (WASM type-detection based serializer)
// ---------------------------------------------------------------------------

// Dynamic string buffer for JSON serialization
typedef struct {
    char *buf;
    int len;
    int cap;
} _wf_json_buf;

static void _wf_jb_init(_wf_json_buf *jb) {
    jb->cap = 256;
    jb->buf = (char *)wasm_alloc(jb->cap);
    jb->len = 0;
    if (jb->buf) jb->buf[0] = '\0';
}

static void _wf_jb_ensure(_wf_json_buf *jb, int needed) {
    if (jb->len + needed + 1 > jb->cap) {
        int new_cap = jb->cap;
        while (jb->len + needed + 1 > new_cap) new_cap *= 2;
        char *new_buf = (char *)wasm_alloc((unsigned int)new_cap);
        if (!new_buf) return;
        for (int i = 0; i < jb->len; i++) new_buf[i] = jb->buf[i];
        new_buf[jb->len] = '\0';
        jb->buf = new_buf;
        jb->cap = new_cap;
    }
}

static void _wf_jb_append(_wf_json_buf *jb, const char *s) {
    int slen = _wf_strlen(s);
    _wf_jb_ensure(jb, slen);
    for (int i = 0; i < slen; i++) jb->buf[jb->len + i] = s[i];
    jb->len += slen;
    jb->buf[jb->len] = '\0';
}

static void _wf_jb_append_char(_wf_json_buf *jb, char c) {
    _wf_jb_ensure(jb, 1);
    jb->buf[jb->len] = c;
    jb->len++;
    jb->buf[jb->len] = '\0';
}

static void _wf_jb_append_escaped_str(_wf_json_buf *jb, const char *s) {
    _wf_jb_append_char(jb, '"');
    if (s) {
        const char *p = s;
        while (*p) {
            switch (*p) {
                case '"':  _wf_jb_append(jb, "\\\""); break;
                case '\\': _wf_jb_append(jb, "\\\\"); break;
                case '\n': _wf_jb_append(jb, "\\n"); break;
                case '\r': _wf_jb_append(jb, "\\r"); break;
                case '\t': _wf_jb_append(jb, "\\t"); break;
                default:   _wf_jb_append_char(jb, *p); break;
            }
            p++;
        }
    }
    _wf_jb_append_char(jb, '"');
}

static void _wf_jb_append_indent(_wf_json_buf *jb, int indent, int depth) {
    if (indent <= 0) return;
    _wf_jb_append_char(jb, '\n');
    for (int i = 0; i < indent * depth; i++) {
        _wf_jb_append_char(jb, ' ');
    }
}

// Forward declare
static void _wf_json_serialize_typed(_wf_json_buf *jb, int64_t val, int indent, int depth, int type_hint);

// WASM type detection helpers: reuse _wf_looks_like_list, _wf_looks_like_string,
// _wf_is_hashmap, _wf_is_set, _wf_is_result, _wf_is_lax defined earlier in this file.
extern int64_t taida_list_to_display_string(int64_t list);

// Helper: serialize pack fields as JSON object (alphabetically sorted)
static void _wf_json_serialize_pack_fields(_wf_json_buf *jb, int64_t *pack, int64_t fc, int indent, int depth) {
    typedef struct { const char *name; int64_t val; int type_hint; } _WfJsonField;
    _WfJsonField fields[100];
    int nfields = 0;
    for (int64_t i = 0; i < fc && nfields < 100; i++) {
        int64_t field_hash = pack[1 + i * 3];     // WASM pack layout: [fc, hash0, tag0, val0, ...]
        int64_t field_val = pack[1 + i * 3 + 2];
        int64_t fname_ptr = taida_lookup_field_name(field_hash);
        const char *fname = (const char *)(intptr_t)fname_ptr;
        if (!fname) continue;
        // Skip __ fields
        if (fname[0] == '_' && fname[1] == '_') continue;
        int64_t ftype = taida_lookup_field_type(field_hash, 0);
        fields[nfields].name = fname;
        fields[nfields].val = field_val;
        fields[nfields].type_hint = (int)ftype;
        nfields++;
    }
    // Sort alphabetically (insertion sort)
    for (int i = 1; i < nfields; i++) {
        _WfJsonField tmp = fields[i];
        int j = i - 1;
        while (j >= 0 && _wf_strcmp(fields[j].name, tmp.name) > 0) {
            fields[j + 1] = fields[j];
            j--;
        }
        fields[j + 1] = tmp;
    }
    _wf_jb_append_char(jb, '{');
    for (int i = 0; i < nfields; i++) {
        if (i > 0) _wf_jb_append_char(jb, ',');
        if (indent > 0) _wf_jb_append_indent(jb, indent, depth + 1);
        _wf_jb_append_escaped_str(jb, fields[i].name);
        _wf_jb_append_char(jb, ':');
        if (indent > 0) _wf_jb_append_char(jb, ' ');
        _wf_json_serialize_typed(jb, fields[i].val, indent, depth + 1, fields[i].type_hint);
    }
    if (indent > 0 && nfields > 0) _wf_jb_append_indent(jb, indent, depth);
    _wf_jb_append_char(jb, '}');
}

static void _wf_json_serialize_typed(_wf_json_buf *jb, int64_t val, int indent, int depth, int type_hint) {
    // Bool type hint: 0/1 -> false/true
    if (type_hint == 4) {
        _wf_jb_append(jb, val ? "true" : "false");
        return;
    }
    // Null/Unit
    if (val == 0) {
        if (type_hint == 3) { // Str
            _wf_jb_append(jb, "\"\"");
        } else {
            _wf_jb_append(jb, "{}");
        }
        return;
    }
    // Integer hint
    if (type_hint == 1 || type_hint == 2) {
        char *num = _wf_i64_to_str(val);
        _wf_jb_append(jb, num);
        return;
    }
    // String hint
    if (type_hint == 3) {
        const char *s = (const char *)(intptr_t)val;
        _wf_jb_append_escaped_str(jb, s);
        return;
    }

    // No type hint (0 or -1): heuristic detection

    // Negative integer or out-of-range for wasm32 pointer
    if (val < 0 || val > 0xFFFFFFFF) {
        char *num = _wf_i64_to_str(val);
        _wf_jb_append(jb, num);
        return;
    }

    // Very small positive values (< 256) are definitely integers, not pointers.
    // Data section strings typically start at higher addresses.
    if (val > 0 && val < 256) {
        char *num = _wf_i64_to_str(val);
        _wf_jb_append(jb, num);
        return;
    }

    // Check HashMap
    if (_wf_is_hashmap(val)) {
        int64_t *hm = (int64_t *)(intptr_t)val;
        int64_t cap = hm[0];
        _wf_jb_append_char(jb, '{');
        int64_t count = 0;
        for (int64_t i = 0; i < cap; i++) {
            int64_t sh = hm[WF_HM_HEADER + i * 3];
            int64_t sk = hm[WF_HM_HEADER + i * 3 + 1];
            if (sh != 0 && !(sh == 1 && sk == 0)) { // occupied (not empty, not tombstone)
                if (count > 0) _wf_jb_append_char(jb, ',');
                if (indent > 0) _wf_jb_append_indent(jb, indent, depth + 1);
                const char *key_str = (const char *)(intptr_t)sk;
                if (!key_str) key_str = "";
                _wf_jb_append_escaped_str(jb, key_str);
                _wf_jb_append_char(jb, ':');
                if (indent > 0) _wf_jb_append_char(jb, ' ');
                _wf_json_serialize_typed(jb, hm[WF_HM_HEADER + i * 3 + 2], indent, depth + 1, 0);
                count++;
            }
        }
        if (indent > 0 && count > 0) _wf_jb_append_indent(jb, indent, depth);
        _wf_jb_append_char(jb, '}');
        return;
    }

    // Check Set (looks like list with set marker at slot[3])
    if (_wf_is_set(val)) {
        int64_t *list = (int64_t *)(intptr_t)val;
        int64_t list_len = list[1];
        _wf_jb_append_char(jb, '[');
        for (int64_t i = 0; i < list_len; i++) {
            if (i > 0) _wf_jb_append_char(jb, ',');
            if (indent > 0) _wf_jb_append_indent(jb, indent, depth + 1);
            _wf_json_serialize_typed(jb, list[4 + i], indent, depth + 1, 0);
        }
        if (indent > 0 && list_len > 0) _wf_jb_append_indent(jb, indent, depth);
        _wf_jb_append_char(jb, ']');
        return;
    }

    // Check monadic types (Result, Lax)
    if (_wf_is_result(val) || _wf_is_lax(val)) {
        int64_t *pack = (int64_t *)(intptr_t)val;
        int64_t fc = pack[0];
        _wf_json_serialize_pack_fields(jb, pack, fc, indent, depth);
        return;
    }

    // Check List
    if (_wf_looks_like_list(val) && !_wf_is_hashmap(val) && !_wf_is_set(val)) {
        int64_t *list = (int64_t *)(intptr_t)val;
        int64_t list_len = list[1];
        _wf_jb_append_char(jb, '[');
        for (int64_t i = 0; i < list_len; i++) {
            if (i > 0) _wf_jb_append_char(jb, ',');
            if (indent > 0) _wf_jb_append_indent(jb, indent, depth + 1);
            _wf_json_serialize_typed(jb, list[4 + i], indent, depth + 1, 0);
        }
        if (indent > 0 && list_len > 0) _wf_jb_append_indent(jb, indent, depth);
        _wf_jb_append_char(jb, ']');
        return;
    }

    // Check BuchiPack (any size with valid pointer)
    if (_wf_is_valid_ptr(val, 8)) {
        int64_t *obj = (int64_t *)(intptr_t)val;
        int64_t fc = obj[0];
        if (fc > 0 && fc < 200) {
            // Verify it looks like a pack (check that hash values are large)
            int64_t hash0 = obj[1]; // first field hash
            if (hash0 > 0x10000 || hash0 < 0) {
                _wf_json_serialize_pack_fields(jb, obj, fc, indent, depth);
                return;
            }
        }
    }

    // String pointer
    if (_wf_looks_like_string(val)) {
        _wf_jb_append_escaped_str(jb, (const char *)(intptr_t)val);
        return;
    }

    // Default: integer
    char *num = _wf_i64_to_str(val);
    _wf_jb_append(jb, num);
}

/// jsonEncode: serialize value as JSON (compact)
int64_t taida_json_encode(int64_t val) {
    _wf_json_buf jb;
    _wf_jb_init(&jb);
    _wf_json_serialize_typed(&jb, val, 0, 0, 0);
    return (int64_t)(intptr_t)jb.buf;
}

/// jsonPretty: serialize value as JSON (indented)
int64_t taida_json_pretty(int64_t val) {
    _wf_json_buf jb;
    _wf_jb_init(&jb);
    _wf_json_serialize_typed(&jb, val, 2, 0, 0);
    return (int64_t)(intptr_t)jb.buf;
}

// ===========================================================================
// WF-3b: Lax / Result / Gorillax extensions
// ===========================================================================

/// Result.isError() check (matches native taida_result_is_error_check)
static int64_t _wf_result_is_error_check(int64_t result) {
    int64_t throw_val = taida_pack_get_idx(result, 2);  // throw
    int64_t pred = taida_pack_get_idx(result, 1);        // __predicate
    int64_t value = taida_pack_get_idx(result, 0);       // __value
    if (throw_val != 0) {
        if (pred != 0) {
            int64_t pred_result = taida_invoke_callback1(pred, value);
            if (!pred_result) return 1;
            return 0;
        }
        return 1;
    }
    if (pred != 0) {
        int64_t pred_result = taida_invoke_callback1(pred, value);
        return pred_result ? 0 : 1;
    }
    return 0;
}

/// Result.isError() — public wrapper
int64_t taida_result_is_error_check(int64_t result) {
    return _wf_result_is_error_check(result);
}

/// Result.getOrDefault(fallback)
int64_t taida_result_get_or_default(int64_t result, int64_t def) {
    if (!_wf_result_is_error_check(result)) return taida_pack_get_idx(result, 0);
    return def;
}

/// Result.map(fn)
int64_t taida_result_map(int64_t result, int64_t fn_ptr) {
    if (_wf_result_is_error_check(result)) return result;
    int64_t value = taida_pack_get_idx(result, 0);
    int64_t new_val = taida_invoke_callback1(fn_ptr, value);
    return taida_result_create(new_val, 0, 0);
}

/// Result.flatMap(fn)
int64_t taida_result_flat_map(int64_t result, int64_t fn_ptr) {
    if (_wf_result_is_error_check(result)) return result;
    int64_t value = taida_pack_get_idx(result, 0);
    return taida_invoke_callback1(fn_ptr, value);
}

/// Result.getOrThrow()
int64_t taida_result_get_or_throw(int64_t result) {
    if (!_wf_result_is_error_check(result)) {
        return taida_pack_get_idx(result, 0);
    }
    int64_t throw_val = taida_pack_get_idx(result, 2);
    if (taida_can_throw_payload(throw_val)) {
        return taida_throw(throw_val);
    }
    int64_t error = taida_make_error(
        (int64_t)(intptr_t)"ResultError",
        (int64_t)(intptr_t)"Result predicate failed");
    return taida_throw(error);
}

/// Result.toString()
int64_t taida_result_to_string(int64_t result) {
    if (!_wf_result_is_error_check(result)) {
        int64_t value = taida_pack_get_idx(result, 0);
        int64_t value_str = taida_polymorphic_to_string(value);
        const char *vs = (const char *)(intptr_t)value_str;
        int vlen = _wf_strlen(vs);
        int need = vlen + 10;
        char *buf = (char *)wasm_alloc((unsigned int)(need + 1));
        _wf_memcpy(buf, "Result(", 7);
        _wf_memcpy(buf + 7, vs, vlen);
        buf[7 + vlen] = ')';
        buf[7 + vlen + 1] = '\0';
        return (int64_t)(intptr_t)buf;
    }
    int64_t throw_val = taida_pack_get_idx(result, 2);
    if (throw_val == 0) {
        return taida_str_new_copy((int64_t)(intptr_t)"Result(throw <= error)");
    }
    int64_t err_disp = taida_polymorphic_to_string(throw_val);
    const char *es = (const char *)(intptr_t)err_disp;
    int elen = _wf_strlen(es);
    int need = elen + 24;
    char *buf = (char *)wasm_alloc((unsigned int)(need + 1));
    _wf_memcpy(buf, "Result(throw <= ", 16);
    _wf_memcpy(buf + 16, es, elen);
    buf[16 + elen] = ')';
    buf[16 + elen + 1] = '\0';
    return (int64_t)(intptr_t)buf;
}

/// Lax.map(fn)
int64_t taida_lax_map(int64_t lax_ptr, int64_t fn_ptr) {
    if (!taida_pack_get_idx(lax_ptr, 0)) {
        int64_t def = taida_pack_get_idx(lax_ptr, 2);
        return taida_lax_empty(def);
    }
    int64_t value = taida_pack_get_idx(lax_ptr, 1);
    int64_t def = taida_pack_get_idx(lax_ptr, 2);
    int64_t result = taida_invoke_callback1(fn_ptr, value);
    return taida_lax_new(result, def);
}

/// Lax.flatMap(fn)
int64_t taida_lax_flat_map(int64_t lax_ptr, int64_t fn_ptr) {
    if (!taida_pack_get_idx(lax_ptr, 0)) {
        int64_t def = taida_pack_get_idx(lax_ptr, 2);
        return taida_lax_empty(def);
    }
    int64_t value = taida_pack_get_idx(lax_ptr, 1);
    return taida_invoke_callback1(fn_ptr, value);
}

/// Lax.toString()
int64_t taida_lax_to_string(int64_t lax_ptr) {
    int64_t val = taida_pack_get_idx(lax_ptr, 1);
    int64_t def = taida_pack_get_idx(lax_ptr, 2);
    int64_t rendered = taida_pack_get_idx(lax_ptr, 0)
        ? taida_polymorphic_to_string(val)
        : taida_polymorphic_to_string(def);
    const char *rs = (const char *)(intptr_t)rendered;
    int rlen = _wf_strlen(rs);
    int need = rlen + 24;
    char *buf = (char *)wasm_alloc((unsigned int)(need + 1));
    if (taida_pack_get_idx(lax_ptr, 0)) {
        _wf_memcpy(buf, "Lax(", 4);
        _wf_memcpy(buf + 4, rs, rlen);
        buf[4 + rlen] = ')';
        buf[4 + rlen + 1] = '\0';
    } else {
        _wf_memcpy(buf, "Lax(default: ", 13);
        _wf_memcpy(buf + 13, rs, rlen);
        buf[13 + rlen] = ')';
        buf[13 + rlen + 1] = '\0';
    }
    return (int64_t)(intptr_t)buf;
}

/// Gorillax.unmold()
int64_t taida_gorillax_unmold(int64_t ptr) {
    if (taida_pack_get_idx(ptr, 0)) {
        return taida_pack_get_idx(ptr, 1);
    }
    // GORILLA — terminate
    extern int fd_write(int fd, const void *iovs, int iovs_len, int *nwritten)
        __attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")));
    const char *msg = "><\n";
    struct { const char *buf; int len; } iov = { msg, 3 };
    int nwritten;
    fd_write(2, &iov, 1, &nwritten);  // stderr
    extern void proc_exit(int code)
        __attribute__((import_module("wasi_snapshot_preview1"), import_name("proc_exit")));
    proc_exit(1);
    return 0;
}

/// Gorillax.toString()
int64_t taida_gorillax_to_string(int64_t ptr) {
    if (taida_pack_get_idx(ptr, 0)) {
        int64_t value = taida_pack_get_idx(ptr, 1);
        char *vs = _wf_i64_to_str(value);
        int vlen = _wf_strlen(vs);
        int need = vlen + 12;
        char *buf = (char *)wasm_alloc((unsigned int)(need + 1));
        _wf_memcpy(buf, "Gorillax(", 9);
        _wf_memcpy(buf + 9, vs, vlen);
        buf[9 + vlen] = ')';
        buf[9 + vlen + 1] = '\0';
        return (int64_t)(intptr_t)buf;
    }
    return taida_str_new_copy((int64_t)(intptr_t)"Gorillax(><)");
}

/// RelaxedGorillax.unmold()
int64_t taida_relaxed_gorillax_unmold(int64_t ptr) {
    if (taida_pack_get_idx(ptr, 0)) {
        return taida_pack_get_idx(ptr, 1);
    }
    int64_t error = taida_make_error(
        (int64_t)(intptr_t)"RelaxedGorillaEscaped",
        (int64_t)(intptr_t)"Relaxed gorilla escaped");
    return taida_throw(error);
}

/// RelaxedGorillax.toString()
int64_t taida_relaxed_gorillax_to_string(int64_t ptr) {
    if (taida_pack_get_idx(ptr, 0)) {
        int64_t value = taida_pack_get_idx(ptr, 1);
        char *vs = _wf_i64_to_str(value);
        int vlen = _wf_strlen(vs);
        int need = vlen + 20;
        char *buf = (char *)wasm_alloc((unsigned int)(need + 1));
        _wf_memcpy(buf, "RelaxedGorillax(", 16);
        _wf_memcpy(buf + 16, vs, vlen);
        buf[16 + vlen] = ')';
        buf[16 + vlen + 1] = '\0';
        return (int64_t)(intptr_t)buf;
    }
    return taida_str_new_copy((int64_t)(intptr_t)"RelaxedGorillax(escaped)");
}

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

/// Char[int_codepoint]() -> Lax[Str]
int64_t taida_char_mold_int(int64_t value) {
    if (value < 0 || value > 0x10FFFF) return taida_lax_empty(taida_str_alloc(0));
    if (value >= 0xD800 && value <= 0xDFFF) return taida_lax_empty(taida_str_alloc(0));
    unsigned char utf8[4];
    int out_len = 0;
    if (!_wf_utf8_encode_scalar((uint32_t)value, utf8, &out_len)) {
        return taida_lax_empty(taida_str_alloc(0));
    }
    char *out = (char *)wasm_alloc((unsigned int)(out_len + 1));
    for (int i = 0; i < out_len; i++) out[i] = (char)utf8[i];
    out[out_len] = '\0';
    return taida_lax_new((int64_t)(intptr_t)out, taida_str_alloc(0));
}

/// Char[str]() -> Lax[Str] (extract single codepoint)
int64_t taida_char_mold_str(int64_t value) {
    const char *s = (const char *)(intptr_t)value;
    if (!s) return taida_lax_empty(taida_str_alloc(0));
    int len = _wf_strlen(s);
    if (len == 0) return taida_lax_empty(taida_str_alloc(0));
    uint32_t cp = 0;
    if (!_wf_utf8_single_scalar((const unsigned char *)s, len, &cp)) {
        return taida_lax_empty(taida_str_alloc(0));
    }
    return taida_char_mold_int((int64_t)cp);
}

/// Char.toDigit() -> int (-1 for non-digit)
int64_t taida_char_to_digit(int64_t v) {
    int c = (int)v;
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'z') return c - 'a' + 10;
    if (c >= 'A' && c <= 'Z') return c - 'A' + 10;
    return -1;
}

/// Codepoint[str]() -> Lax[Int]
int64_t taida_codepoint_mold_str(int64_t value) {
    const char *s = (const char *)(intptr_t)value;
    if (!s) return taida_lax_empty(0);
    int len = _wf_strlen(s);
    if (len == 0) return taida_lax_empty(0);
    uint32_t cp = 0;
    if (!_wf_utf8_single_scalar((const unsigned char *)s, len, &cp)) {
        return taida_lax_empty(0);
    }
    return taida_lax_new((int64_t)cp, 0);
}

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

int64_t taida_bytes_to_display_string(int64_t bytes_ptr) {
    if (!_wf_is_bytes(bytes_ptr)) {
        return taida_str_new_copy((int64_t)(intptr_t)"Bytes()");
    }
    int64_t *bytes = (int64_t *)(intptr_t)bytes_ptr;
    int64_t len = bytes[1];
    // "Bytes([0, 1, 2, ...])"
    _wf_json_buf jb;
    _wf_jb_init(&jb);
    _wf_jb_append(&jb, "Bytes([");
    for (int64_t i = 0; i < len; i++) {
        if (i > 0) _wf_jb_append(&jb, ", ");
        char *s = _wf_i64_to_str(bytes[2 + i]);
        _wf_jb_append(&jb, s);
    }
    _wf_jb_append(&jb, "])");
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
