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
