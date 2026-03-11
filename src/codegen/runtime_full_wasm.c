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

/// Polymorphic .contains()
int64_t taida_polymorphic_contains(int64_t obj, int64_t needle) {
    if (obj == 0 || obj < 4096) return 0;
    if (_wf_is_hashmap(obj)) return taida_hashmap_has(obj, taida_value_hash(needle), needle);
    if (_wf_is_set(obj)) return taida_set_has(obj, needle);
    if (_wf_looks_like_list(obj)) return taida_list_contains(obj, needle);
    // String contains
    return taida_str_contains(obj, needle);
}

/// Polymorphic .indexOf()
int64_t taida_polymorphic_index_of(int64_t obj, int64_t needle) {
    if (obj == 0 || obj < 4096) return -1;
    if (_wf_looks_like_list(obj)) return taida_list_index_of(obj, needle);
    return taida_str_index_of(obj, needle);
}

/// Polymorphic .lastIndexOf()
int64_t taida_polymorphic_last_index_of(int64_t obj, int64_t needle) {
    if (obj == 0 || obj < 4096) return -1;
    if (_wf_looks_like_list(obj)) return taida_list_last_index_of(obj, needle);
    return taida_str_last_index_of(obj, needle);
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
