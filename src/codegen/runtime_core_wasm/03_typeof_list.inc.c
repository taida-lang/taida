/* ── RC no-ops (wasm-min ではヒープなし) ── */

void taida_retain(int64_t val) { (void)val; }
void taida_release(int64_t val) { (void)val; }
void taida_str_retain(int64_t val) { (void)val; }

/* ── typeof: compile-time tag + runtime heuristic ── */

int64_t taida_typeof(int64_t val, int64_t tag) {
    if (val != 0 && val >= WASM_MIN_HEAP_ADDR) {
        if (_is_wasm_hashmap(val)) return (int64_t)(intptr_t)"HashMap";
        if (_is_wasm_set(val)) return (int64_t)(intptr_t)"Set";
        if (_wasm_is_result(val)) return (int64_t)(intptr_t)"Result";
        if (_wasm_is_lax(val)) return (int64_t)(intptr_t)"Lax";
        if (_looks_like_pack(val)) return (int64_t)(intptr_t)"BuchiPack";
        if (_looks_like_list(val)) return (int64_t)(intptr_t)"List";
        if (_looks_like_string(val)) return (int64_t)(intptr_t)"Str";
    }
    switch (tag) {
        case 1: return (int64_t)(intptr_t)"Float";
        case 2: return (int64_t)(intptr_t)"Bool";
        case 3: return (int64_t)(intptr_t)"Str";
        case 4: return (int64_t)(intptr_t)"BuchiPack";
        case 5: return (int64_t)(intptr_t)"List";
        case 6: return (int64_t)(intptr_t)"Closure";
        default: return (int64_t)(intptr_t)"Int";
    }
}

/* =========================================================================
 * WC-2a: Float mold functions (prelude — all profiles)
 * ========================================================================= */

/// Floor[f]() -- floor(x), returns float (bit-punned int64_t)
int64_t taida_float_floor(int64_t a) {
    double d = _to_double(a);
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
static int _wc_write_uint64(char *buf, uint64_t val) {
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
    if (d != d) {
        char *r = (char *)wasm_alloc(4);
        r[0] = 'N'; r[1] = 'a'; r[2] = 'N'; r[3] = '\0';
        return (int64_t)r;
    }

    int negative = 0;
    if (d < 0.0) { negative = 1; d = -d; }

    // Check infinity
    double zero_test = d * 0.0;
    if (zero_test != 0.0 || (d > 0.0 && d == d + d)) {
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
    rounded = (double)(long long)(rounded + 0.5);
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
    pos += _wc_write_uint64(buf + pos, int_part);
    if (digits > 0) {
        buf[pos++] = '.';
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

/* =========================================================================
 * WC-2b: Int extended mold functions (prelude — all profiles)
 * ========================================================================= */

int64_t taida_int_clamp(int64_t a, int64_t lo, int64_t hi) {
    if (a < lo) return lo;
    if (a > hi) return hi;
    return a;
}

int64_t taida_int_is_positive(int64_t a) { return a > 0 ? 1 : 0; }
int64_t taida_int_is_negative(int64_t a) { return a < 0 ? 1 : 0; }
int64_t taida_int_is_zero(int64_t a) { return a == 0 ? 1 : 0; }

// ── Int mold auto / str_base ─────────────────────────────

/// digit_to_char (local) -- 0-9 -> '0'-'9', 10-35 -> 'a'-'z'
static int64_t _wc_digit_to_char(int64_t digit) {
    return (digit < 10) ? ('0' + digit) : ('a' + (digit - 10));
}

/// char_to_digit (local) -- '0'-'9' -> 0-9, 'a'-'z' -> 10-35, 'A'-'Z' -> 10-35, else -1
static int _wc_char_to_digit(int c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'z') return c - 'a' + 10;
    if (c >= 'A' && c <= 'Z') return c - 'A' + 10;
    return -1;
}

/// Int[v]() auto-detect: tries to distinguish int, string, other
int64_t taida_int_mold_auto(int64_t v) {
    if (v == 0) return taida_lax_new(0, 0);
    if (v < 0 || v < WASM_MIN_HEAP_ADDR) return taida_lax_new(v, 0);

    const char *s = (const char *)(intptr_t)v;
    char c = s[0];
    if (c == '-' || c == '+' || (c >= '0' && c <= '9')) {
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
        int d = _wc_char_to_digit((unsigned char)s[i]);
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
        tmp[pos++] = (char)_wc_digit_to_char((int64_t)rem);
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

/* ── WC-3d: Public callback invoke helpers (list HOF depends on these) ── */
/* These wrap the same logic as _wasm_invoke_callback1 (static, used by Cage)
   but with public linkage so runtime_full_wasm.c can also call them. */

int64_t taida_invoke_callback1(int64_t fn_ptr, int64_t arg0) {
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

int64_t taida_invoke_callback2(int64_t fn_ptr, int64_t arg0, int64_t arg1) {
    if (taida_is_closure_value(fn_ptr)) {
        int64_t *closure = (int64_t *)(intptr_t)fn_ptr;
        /* Closure with 2 user args: call with env + arg0 + arg1 */
        typedef int64_t (*closure_fn2_t)(int64_t, int64_t, int64_t);
        closure_fn2_t func = (closure_fn2_t)(intptr_t)closure[1];
        return func(closure[2], arg0, arg1);
    }
    typedef int64_t (*fn_t)(int64_t, int64_t);
    fn_t func = (fn_t)(intptr_t)fn_ptr;
    return func(arg0, arg1);
}

/* ── WC-3: Hash constants for enumerate/zip (FNV-1a hashes) ── */
#define WASM_HASH_FIRST  0x89d7ed7f996f1d41ULL  /* FNV-1a("first") */
#define WASM_HASH_SECOND 0xa49985ef4cee20bdULL  /* FNV-1a("second") */
#define WASM_HASH_INDEX  0x83cf8e8f9081468bULL  /* FNV-1a("index") */
#define WASM_HASH_VALUE2 0x7ce4fd9430e80ceaULL  /* FNV-1a("value") -- suffixed to avoid conflict with WASM_HASH___VALUE */

/* ── WC-3a: List HOF functions (all profiles) ── */

int64_t taida_list_map(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t new_list = taida_list_new();
    /* map may change element type, so leave elem_tag as UNKNOWN */
    for (int64_t i = 0; i < len; i++) {
        int64_t result = taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i]);
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
        if (taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) {
            new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
        }
    }
    return new_list;
}

int64_t taida_list_fold(int64_t list_ptr, int64_t init, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t acc = init;
    for (int64_t i = 0; i < len; i++) {
        acc = taida_invoke_callback2(fn_ptr, acc, list[WASM_LIST_ELEMS + i]);
    }
    return acc;
}

int64_t taida_list_foldr(int64_t list_ptr, int64_t init, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t acc = init;
    for (int64_t i = len - 1; i >= 0; i--) {
        acc = taida_invoke_callback2(fn_ptr, acc, list[WASM_LIST_ELEMS + i]);
    }
    return acc;
}

int64_t taida_list_find(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        int64_t item = list[WASM_LIST_ELEMS + i];
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
        if (taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) return i;
    }
    return -1;
}

int64_t taida_list_take_while(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) {
            new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
        } else {
            break;
        }
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
        if (dropping && taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) {
            continue;
        }
        dropping = 0;
        new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
    }
    return new_list;
}

int64_t taida_list_any(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) return 1;
    }
    return 0;
}

int64_t taida_list_all(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (!taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) return 0;
    }
    return 1;
}

int64_t taida_list_none(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) return 0;
    }
    return 1;
}

/* ── WC-3b: List operation functions (all profiles) ── */

int64_t taida_list_sort(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    /* Copy items into temp array (on bump allocator) */
    int64_t *items = (int64_t *)wasm_alloc((unsigned int)(len * 8));
    for (int64_t i = 0; i < len; i++) items[i] = list[WASM_LIST_ELEMS + i];
    /* Insertion sort ascending */
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
    for (int64_t i = 0; i < len; i++) items[i] = list[WASM_LIST_ELEMS + i];
    /* Insertion sort descending */
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

/* Sort by key extraction function: fn_ptr maps each element to a sort key,
   then sort ascending by key. Matches interpreter's Sort[list](by <= fn). */
int64_t taida_list_sort_by(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    if (len == 0) return new_list;
    /* Allocate parallel arrays: items and keys */
    int64_t *items = (int64_t *)wasm_alloc((unsigned int)(len * 8));
    int64_t *keys = (int64_t *)wasm_alloc((unsigned int)(len * 8));
    for (int64_t i = 0; i < len; i++) {
        items[i] = list[WASM_LIST_ELEMS + i];
        keys[i] = taida_invoke_callback1(fn_ptr, items[i]);
    }
    /* Insertion sort ascending by key */
    for (int64_t i = 1; i < len; i++) {
        int64_t kkey = keys[i];
        int64_t kitem = items[i];
        int64_t j = i - 1;
        while (j >= 0 && keys[j] > kkey) {
            keys[j+1] = keys[j];
            items[j+1] = items[j];
            j--;
        }
        keys[j+1] = kkey;
        items[j+1] = kitem;
    }
    for (int64_t i = 0; i < len; i++) {
        new_list = taida_list_push(new_list, items[i]);
    }
    return new_list;
}

int64_t taida_list_unique(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl_init = (int64_t *)(intptr_t)new_list;
    nl_init[2] = elem_tag;
    for (int64_t i = 0; i < len; i++) {
        int64_t item = list[WASM_LIST_ELEMS + i];
        /* Check if already in new_list */
        int64_t *nl = (int64_t *)(intptr_t)new_list;
        int64_t nlen = nl[1];
        int64_t found = 0;
        for (int64_t j = 0; j < nlen; j++) {
            if (nl[WASM_LIST_ELEMS + j] == item) { found = 1; break; }
        }
        if (!found) {
            new_list = taida_list_push(new_list, item);
        }
    }
    return new_list;
}

int64_t taida_list_flatten(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t new_list = taida_list_new();
    for (int64_t i = 0; i < len; i++) {
        int64_t item = list[WASM_LIST_ELEMS + i];
        if (_looks_like_list(item)) {
            int64_t *sub = (int64_t *)(intptr_t)item;
            int64_t slen = sub[1];
            /* Propagate inner list's elem_tag to result */
            if (i == 0) {
                int64_t *nl = (int64_t *)(intptr_t)new_list;
                nl[2] = sub[2];
            }
            for (int64_t j = 0; j < slen; j++) {
                new_list = taida_list_push(new_list, sub[WASM_LIST_ELEMS + j]);
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
        new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
    }
    return new_list;
}

int64_t taida_list_join(int64_t list_ptr, int64_t sep_raw) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_str_alloc(0);
    const char *sep = (const char *)(intptr_t)sep_raw;
    if (!sep) sep = "";
    int sep_len = _wf_strlen(sep);

    /* Convert each element through polymorphic_to_string */
    const char **strs = (const char **)wasm_alloc((unsigned int)(len * sizeof(const char *)));
    int total = 0;
    for (int64_t i = 0; i < len; i++) {
        strs[i] = (const char *)(intptr_t)taida_polymorphic_to_string(list[WASM_LIST_ELEMS + i]);
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

int64_t taida_list_concat(int64_t list1, int64_t list2) {
    int64_t *l1 = (int64_t *)(intptr_t)list1;
    int64_t *l2 = (int64_t *)(intptr_t)list2;
    int64_t len1 = l1[1], len2 = l2[1];
    int64_t elem_tag = l1[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = elem_tag;
    for (int64_t i = 0; i < len1; i++) {
        new_list = taida_list_push(new_list, l1[WASM_LIST_ELEMS + i]);
    }
    for (int64_t i = 0; i < len2; i++) {
        new_list = taida_list_push(new_list, l2[WASM_LIST_ELEMS + i]);
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
        new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
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
        new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
    }
    return new_list;
}

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
        new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
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
        new_list = taida_list_push(new_list, list[WASM_LIST_ELEMS + i]);
    }
    return new_list;
}

/* C24-B (2026-04-23): Register zip / enumerate pair-pack field names
   into `_wasm_field_registry` so `_wasm_pack_to_string_full` resolves
   `first` / `second` / `index` / `value` (previously unregistered →
   NULL → every pair rendered as `@()`, which then trapped on the
   recursive full-form walk because the outer list's `elem_type_tag`
   = WASM_TAG_PACK forced the pair through tagged fast-path rendering).
   Idempotent — follows C23B-009's `taida_hashmap_entries` pattern
   (registers inside the helper body rather than at startup because
   the field names are only meaningful on the zip / enumerate path). */
static void _wasm_register_zip_enumerate_field_names(void) {
    taida_register_field_name((int64_t)WASM_HASH_FIRST,  (int64_t)(intptr_t)"first");
    taida_register_field_name((int64_t)WASM_HASH_SECOND, (int64_t)(intptr_t)"second");
    taida_register_field_name((int64_t)WASM_HASH_INDEX,  (int64_t)(intptr_t)"index");
    taida_register_field_name((int64_t)WASM_HASH_VALUE2, (int64_t)(intptr_t)"value");
}

int64_t taida_list_enumerate(int64_t list_ptr) {
    _wasm_register_zip_enumerate_field_names();
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    /* C24-B: propagate source list's elem_type_tag to the pair's value
       slot so primitives render through the tagged fast-path. The
       `index` slot is always INT (tag 0), no explicit stamping needed
       since `taida_pack_new` zero-initialises tags to INT. */
    int64_t elem_tag = list[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = WASM_TAG_PACK;  /* C24-B: enumerate produces Pack elements */
    for (int64_t i = 0; i < len; i++) {
        int64_t pair = taida_pack_new(2);
        taida_pack_set_hash(pair, 0, (int64_t)WASM_HASH_INDEX);
        taida_pack_set(pair, 0, i);
        taida_pack_set_tag(pair, 0, WASM_TAG_INT);
        taida_pack_set_hash(pair, 1, (int64_t)WASM_HASH_VALUE2);
        taida_pack_set(pair, 1, list[WASM_LIST_ELEMS + i]);
        taida_pack_set_tag(pair, 1, elem_tag);
        new_list = taida_list_push(new_list, pair);
    }
    return new_list;
}

int64_t taida_list_zip(int64_t list1, int64_t list2) {
    _wasm_register_zip_enumerate_field_names();
    int64_t *l1 = (int64_t *)(intptr_t)list1;
    int64_t *l2 = (int64_t *)(intptr_t)list2;
    int64_t len1 = l1[1], len2 = l2[1];
    int64_t min_len = len1 < len2 ? len1 : len2;
    /* C24-B: propagate each source list's elem_type_tag to its pair
       slot so primitives in either position render through tagged
       fast-path dispatch. */
    int64_t elem_tag1 = l1[2];
    int64_t elem_tag2 = l2[2];
    int64_t new_list = taida_list_new();
    int64_t *nl = (int64_t *)(intptr_t)new_list;
    nl[2] = WASM_TAG_PACK;  /* C24-B: zip produces Pack elements */
    for (int64_t i = 0; i < min_len; i++) {
        int64_t pair = taida_pack_new(2);
        taida_pack_set_hash(pair, 0, (int64_t)WASM_HASH_FIRST);
        taida_pack_set(pair, 0, l1[WASM_LIST_ELEMS + i]);
        taida_pack_set_tag(pair, 0, elem_tag1);
        taida_pack_set_hash(pair, 1, (int64_t)WASM_HASH_SECOND);
        taida_pack_set(pair, 1, l2[WASM_LIST_ELEMS + i]);
        taida_pack_set_tag(pair, 1, elem_tag2);
        new_list = taida_list_push(new_list, pair);
    }
    return new_list;
}

int64_t taida_list_to_display_string(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) {
        char *result = (char *)wasm_alloc(4);
        _wf_memcpy(result, "@[]", 4);
        return (int64_t)result;
    }
    /* Build "@[elem, elem, ...]" */
    const char **strs = (const char **)wasm_alloc((unsigned int)(len * sizeof(const char *)));
    int total = 3; /* "@[" + "]" */
    for (int64_t i = 0; i < len; i++) {
        strs[i] = (const char *)(intptr_t)taida_polymorphic_to_string(list[WASM_LIST_ELEMS + i]);
        total += _wf_strlen(strs[i]);
        if (i > 0) total += 2; /* ", " */
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

/* ── WC-3c: List query functions (all profiles) ── */

int64_t taida_list_first(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    return taida_lax_new(list[WASM_LIST_ELEMS], 0);
}

int64_t taida_list_last(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    return taida_lax_new(list[WASM_LIST_ELEMS + len - 1], 0);
}

int64_t taida_list_min(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    int64_t min_val = list[WASM_LIST_ELEMS];
    for (int64_t i = 1; i < len; i++) {
        if (list[WASM_LIST_ELEMS + i] < min_val) min_val = list[WASM_LIST_ELEMS + i];
    }
    return taida_lax_new(min_val, 0);
}

int64_t taida_list_max(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    if (len == 0) return taida_lax_empty(0);
    int64_t max_val = list[WASM_LIST_ELEMS];
    for (int64_t i = 1; i < len; i++) {
        if (list[WASM_LIST_ELEMS + i] > max_val) max_val = list[WASM_LIST_ELEMS + i];
    }
    return taida_lax_new(max_val, 0);
}

int64_t taida_list_sum(int64_t list_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t sum = 0;
    for (int64_t i = 0; i < len; i++) {
        sum += list[WASM_LIST_ELEMS + i];
    }
    return sum;
}

int64_t taida_list_contains(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (list[WASM_LIST_ELEMS + i] == item) return 1;
    }
    return 0;
}

int64_t taida_list_index_of(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = 0; i < len; i++) {
        if (list[WASM_LIST_ELEMS + i] == item) return i;
    }
    return -1;
}

int64_t taida_list_last_index_of(int64_t list_ptr, int64_t item) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    for (int64_t i = len - 1; i >= 0; i--) {
        if (list[WASM_LIST_ELEMS + i] == item) return i;
    }
    return -1;
}

int64_t taida_list_count(int64_t list_ptr, int64_t fn_ptr) {
    int64_t *list = (int64_t *)(intptr_t)list_ptr;
    int64_t len = list[1];
    int64_t count = 0;
    for (int64_t i = 0; i < len; i++) {
        if (taida_invoke_callback1(fn_ptr, list[WASM_LIST_ELEMS + i])) count++;
    }
    return count;
}

/* ── List elem retain/release (no-ops in WASM) ── */
void taida_list_elem_retain(int64_t list) { (void)list; }
void taida_list_elem_release(int64_t list) { (void)list; }

