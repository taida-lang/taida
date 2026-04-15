// ── Error ceiling (setjmp/longjmp) ───────────────────────
// Uses setjmp/longjmp for error catching. The key function is
// taida_error_try_call which wraps setjmp and calls a function pointer.
#include <setjmp.h>

static __thread jmp_buf __taida_error_jmp[64];
static __thread taida_val __taida_error_val[64];
static __thread taida_val __taida_try_result[64];
static __thread int __taida_error_depth = 0;

taida_val taida_error_ceiling_push(void) {
    if (__taida_error_depth >= 64) {
        fprintf(stderr, "Error: maximum error handling depth exceeded (64)\n");
        exit(1);
    }
    int depth = __taida_error_depth++;
    return (taida_val)depth;
}

void taida_error_ceiling_pop(void) {
    if (__taida_error_depth > 0) __taida_error_depth--;
}

taida_val taida_throw(taida_val error_val) {
    if (__taida_error_depth > 0) {
        int depth = __taida_error_depth - 1;
        __taida_error_val[depth] = error_val;
        longjmp(__taida_error_jmp[depth], 1);
    }
    // No error ceiling: gorilla — print the actual error message
    taida_val msg = taida_throw_to_display_string(error_val);
    if (msg != 0) {
        fprintf(stderr, "Runtime error: %s\n", (const char*)msg);
    } else {
        fprintf(stderr, "Unhandled error (no error ceiling)\n");
    }
    exit(1);
    return 0;
}

// Try to execute a function pointer; if it throws, return 1 and store error.
// This wraps setjmp so the jmp_buf lives in THIS function's stack frame.
// fn_ptr: pointer to a 1-arg function (env_ptr) returning taida_val
// env_ptr: environment pack containing captured variables from parent scope
// Returns: 0 if fn completed normally, 1 if an error was thrown
taida_val taida_error_try_call(taida_val fn_ptr, taida_val env_ptr, taida_val depth) {
    typedef taida_val (*fn_t)(taida_val);
    fn_t func = (fn_t)fn_ptr;
    if (setjmp(__taida_error_jmp[(int)depth]) == 0) {
        __taida_try_result[(int)depth] = func(env_ptr);
        return 0; // normal completion
    } else {
        return 1; // error caught
    }
}

// Get the return value of the last successful try_call at the given depth
taida_val taida_error_try_get_result(taida_val depth) {
    return __taida_try_result[(int)depth];
}

// Legacy: for backward compat with existing IR that calls setjmp directly.
// This won't work properly from Cranelift code but is kept for reference.
taida_val taida_error_setjmp(taida_val depth) {
    return (taida_val)setjmp(__taida_error_jmp[(int)depth]);
}

taida_val taida_error_get_value(taida_val depth) {
    return __taida_error_val[(int)depth];
}

// RCB-101: Inheritance parent registry for error type filtering in |==
// Dynamic array — grows as needed to handle projects with many type hierarchies.
// NB2-7: Protected by mutex — realloc during registration could cause dangling
// pointers if a worker thread reads while the main thread grows the arrays.
static taida_val *__taida_type_parent_child = NULL;
static taida_val *__taida_type_parent_parent = NULL;
static int __taida_type_parent_count = 0;
static int __taida_type_parent_cap = 0;
static pthread_mutex_t __taida_type_parent_mutex = PTHREAD_MUTEX_INITIALIZER;

// Register an inheritance parent: child IS-A parent
void taida_register_type_parent(taida_val child_str, taida_val parent_str) {
    pthread_mutex_lock(&__taida_type_parent_mutex);
    if (__taida_type_parent_count >= __taida_type_parent_cap) {
        int new_cap = __taida_type_parent_cap == 0 ? 64 : __taida_type_parent_cap * 2;
        // Allocate both new arrays first, then copy + swap atomically.
        // This avoids stale pointers if one allocation fails.
        taida_val *new_child = (taida_val*)malloc(sizeof(taida_val) * new_cap);
        taida_val *new_parent = (taida_val*)malloc(sizeof(taida_val) * new_cap);
        if (!new_child || !new_parent) {
            free(new_child);
            free(new_parent);
            fprintf(stderr, "Warning: type parent registry allocation failed\n");
            pthread_mutex_unlock(&__taida_type_parent_mutex);
            return;
        }
        if (__taida_type_parent_count > 0) {
            memcpy(new_child, __taida_type_parent_child, sizeof(taida_val) * __taida_type_parent_count);
            memcpy(new_parent, __taida_type_parent_parent, sizeof(taida_val) * __taida_type_parent_count);
        }
        free(__taida_type_parent_child);
        free(__taida_type_parent_parent);
        __taida_type_parent_child = new_child;
        __taida_type_parent_parent = new_parent;
        __taida_type_parent_cap = new_cap;
    }
    __taida_type_parent_child[__taida_type_parent_count] = child_str;
    __taida_type_parent_parent[__taida_type_parent_count] = parent_str;
    __taida_type_parent_count++;
    pthread_mutex_unlock(&__taida_type_parent_mutex);
}

// Find the parent type string for a given child type string.
// Returns 0 if not found.
// NB2-7: Protected by mutex for safe concurrent reads during handler execution.
static taida_val taida_find_parent_type(taida_val child_str) {
    pthread_mutex_lock(&__taida_type_parent_mutex);
    taida_val result = 0;
    for (int i = 0; i < __taida_type_parent_count; i++) {
        if (taida_str_eq(__taida_type_parent_child[i], child_str)) {
            result = __taida_type_parent_parent[i];
            break;
        }
    }
    pthread_mutex_unlock(&__taida_type_parent_mutex);
    return result;
}

// Check if thrown_type IS-A handler_type by walking the inheritance chain.
// handler_type_str and thrown_type_str are C string pointers.
// Returns 1 if match, 0 if not.
taida_val taida_error_type_matches(taida_val error_val, taida_val handler_type_str) {
    // "Error" catches everything
    const char *handler_s = (const char*)handler_type_str;
    if (handler_s && strcmp(handler_s, "Error") == 0) return 1;

    // Get the thrown type from __type field of the BuchiPack.
    // Fall back to "type" field if __type is absent (legacy errors).
    taida_val thrown_type_str = 0;
    if (taida_is_buchi_pack(error_val)) {
        if (taida_pack_has_hash(error_val, (taida_val)HASH___TYPE)) {
            thrown_type_str = taida_pack_get(error_val, (taida_val)HASH___TYPE);
        } else if (taida_pack_has_hash(error_val, (taida_val)HASH_TYPE)) {
            thrown_type_str = taida_pack_get(error_val, (taida_val)HASH_TYPE);
        }
    }
    // RCB-101 fix: unknown type must NOT be catch-all.  Only the "Error"
    // handler (checked above) catches everything.  A typed handler like
    // |== e: MyError should not match an error with no type information.
    if (thrown_type_str == 0) return 0;

    // Walk inheritance chain
    taida_val current = thrown_type_str;
    for (int i = 0; i < 64; i++) {
        if (taida_str_eq(current, handler_type_str)) return 1;
        taida_val parent = taida_find_parent_type(current);
        if (parent == 0) break;
        current = parent;
    }
    return 0;
}

// B11B-015: Runtime type check for TypeIs with named types.
// Gets __type from the BuchiPack and walks the inheritance chain.
// Returns 1 (true) or 0 (false).
taida_val taida_typeis_named(taida_val val, taida_val expected_type_str) {
    if (!taida_is_buchi_pack(val)) return 0;
    taida_val type_str = 0;
    if (taida_pack_has_hash(val, (taida_val)HASH___TYPE)) {
        type_str = taida_pack_get(val, (taida_val)HASH___TYPE);
    }
    if (type_str == 0) return 0;
    // Direct match
    if (taida_str_eq(type_str, expected_type_str)) return 1;
    // Walk inheritance chain
    taida_val current = type_str;
    for (int i = 0; i < 64; i++) {
        taida_val parent = taida_find_parent_type(current);
        if (parent == 0) break;
        if (taida_str_eq(parent, expected_type_str)) return 1;
        current = parent;
    }
    return 0;
}

// RCB-101: Check error type and re-throw if it does not match.
// Called at the start of error ceiling handler arm.
// If the type matches, returns the error_val unchanged.
// If it does not match, calls taida_throw(error_val) which longjmps (never returns).
taida_val taida_error_type_check_or_rethrow(taida_val error_val, taida_val handler_type_str) {
    if (taida_error_type_matches(error_val, handler_type_str)) {
        return error_val;
    }
    // Re-throw: this longjmps to the next outer error ceiling
    taida_throw(error_val);
    return 0; // unreachable
}

taida_val taida_cage_apply(taida_val cage_value, taida_val fn_ptr) {
    if (fn_ptr == 0) {
        taida_val error = taida_make_error("CageError", "Cage second argument must be a function");
        return taida_gorillax_err(error);
    }

    taida_val depth = taida_error_ceiling_push();
    if (setjmp(__taida_error_jmp[(int)depth]) == 0) {
        taida_val result = taida_invoke_callback1(fn_ptr, cage_value);
        taida_error_ceiling_pop();
        return taida_gorillax_new(result);
    }

    taida_val error = taida_error_get_value(depth);
    taida_error_ceiling_pop();
    if (error == 0) {
        error = taida_make_error("CageError", "Cage function failed");
    }
    return taida_gorillax_err(error);
}

// ── Result[T, P] (v0.8.0 redesign — predicate support) ───
// Optional abolished in v0.8.0 — use Lax[T] instead.
// Result: operation mold — BuchiPack @(__value: T, __predicate: P, throw: Error, __type: "Result")
//   Layout: [refcount, field_count=4, hash0(__value), val0, hash1(__predicate), val1, hash2(throw), val2, hash3(__type), val3("Result")]
//   field 0: __value
//   field 1: __predicate (0 = no predicate, non-zero = function pointer)
//   field 2: throw (0 = Unit = success, non-zero = error)
//   field 3: __type ("Result" string)

// FNV-1a hashes for Result fields
#define HASH___TYPE            0x84d2d84b631f799bULL  // "__type"
#define HASH_RES___VALUE       0x0a7fc9f13472bbe0ULL  // "__value"
#define HASH_RES___PREDICATE   0x15592af3c2291540ULL  // "__predicate"
#define HASH_RES_THROW         0x5a5fe3720c9584cfULL  // "throw"

static const char __result_type_str[] = "Result";

// Throw payload must be type-confirmed to avoid pointer-guess heuristics.
static int taida_can_throw_payload(taida_val val) {
    if (val == 0) return 0;
    if (TAIDA_IS_PACK(val) || TAIDA_IS_LIST(val) || TAIDA_IS_HMAP(val) || TAIDA_IS_SET(val) || TAIDA_IS_ASYNC(val)) {
        return 1;
    }
    size_t sl = 0;
    return taida_read_cstr_len_safe((const char*)val, 65536, &sl);
}

// ── Result constructors ──

// Result[value, predicate](throw <= error) — create Result with optional predicate
taida_val taida_result_create(taida_val value, taida_val throw_val, taida_val predicate) {
    taida_val pack = taida_pack_new(4);
    taida_pack_set_hash(pack, 0, (taida_val)HASH_RES___VALUE);
    taida_pack_set(pack, 0, value);
    // retain-on-store: value が Pack/List/Closure の場合 retain
    // value の型は不明なので magic header で判定
    if (value > 4096 && taida_ptr_is_readable(value, sizeof(taida_val))) {
        taida_val vtag = ((taida_val*)value)[0] & TAIDA_MAGIC_MASK;
        if (vtag == TAIDA_PACK_MAGIC || vtag == TAIDA_LIST_MAGIC || vtag == TAIDA_CLOSURE_MAGIC) {
            taida_retain(value);
            // value の型タグも設定
            if (vtag == TAIDA_PACK_MAGIC) taida_pack_set_tag(pack, 0, TAIDA_TAG_PACK);
            else if (vtag == TAIDA_LIST_MAGIC) taida_pack_set_tag(pack, 0, TAIDA_TAG_LIST);
            else taida_pack_set_tag(pack, 0, TAIDA_TAG_CLOSURE);
        }
    }
    taida_pack_set_hash(pack, 1, (taida_val)HASH_RES___PREDICATE);
    taida_pack_set(pack, 1, predicate);  // 0 = no predicate, non-zero = function pointer
    if (predicate != 0) {
        taida_pack_set_tag(pack, 1, TAIDA_TAG_CLOSURE);
        taida_retain(predicate);  // retain-on-store: closure child
    }
    taida_pack_set_hash(pack, 2, (taida_val)HASH_RES_THROW);
    taida_pack_set(pack, 2, throw_val);  // 0 = success (Unit), non-zero = error
    if (throw_val != 0) {
        taida_pack_set_tag(pack, 2, TAIDA_TAG_PACK);
        taida_retain(throw_val);  // retain-on-store: pack child
    }
    taida_pack_set_hash(pack, 3, (taida_val)HASH___TYPE);
    taida_pack_set(pack, 3, (taida_val)__result_type_str);
    // __result_type_str is static - leave tag as INT(0)
    return pack;
}

// Helper: check if Result has error
// 1. If throw is set (not 0), it's an error — UNLESS predicate passes
// 2. If predicate exists, evaluate P(value) — true = success, false = error
// 3. No predicate + no throw = success (backward compatible)
static taida_val taida_result_is_error_check(taida_val result) {
    taida_val throw_val = taida_pack_get_idx(result, 2);  // throw
    taida_val pred = taida_pack_get_idx(result, 1);  // __predicate
    taida_val value = taida_pack_get_idx(result, 0);  // __value

    if (throw_val != 0) {
        // If predicate exists, evaluate it even when throw is set
        if (pred != 0) {
            taida_val pred_result = taida_invoke_callback1(pred, value);
            if (!pred_result) return 1;  // predicate failed — error
            return 0;  // predicate passed even though throw was set — success
        }
        return 1;  // throw set, no predicate — error
    }
    if (pred != 0) {
        taida_val pred_result = taida_invoke_callback1(pred, value);
        return pred_result ? 0 : 1;
    }
    return 0;  // no throw, no predicate — success
}

taida_val taida_result_is_ok(taida_val result) {
    return taida_result_is_error_check(result) ? 0 : 1;
}

taida_val taida_result_get_or_default(taida_val result, taida_val def) {
    if (!taida_result_is_error_check(result)) return taida_pack_get_idx(result, 0);
    return def;
}

taida_val taida_result_is_error(taida_val result) {
    return taida_result_is_error_check(result);
}

// ── Result methods (map, flatMap, mapError, getOrThrow, isError, toString) ──

// Result.map(fn) — if success, apply fn to __value
taida_val taida_result_map(taida_val result, taida_val fn_ptr) {
    if (taida_result_is_error_check(result)) {
        return result;  // Error: return as-is
    }
    taida_val value = taida_pack_get_idx(result, 0);  // __value
    taida_val new_val = taida_invoke_callback1(fn_ptr, value);
    return taida_result_create(new_val, 0, 0);  // success, no predicate
}

// Result.flatMap(fn) — if success, apply fn (which should return Result)
taida_val taida_result_flat_map(taida_val result, taida_val fn_ptr) {
    if (taida_result_is_error_check(result)) {
        return result;
    }
    taida_val value = taida_pack_get_idx(result, 0);  // __value
    taida_val new_result = taida_invoke_callback1(fn_ptr, value);
    return new_result;
}

// Result.mapError(fn) — if error, apply fn to throw value
taida_val taida_result_map_error(taida_val result, taida_val fn_ptr) {
    if (!taida_result_is_error_check(result)) {
        return result;  // Success: return as-is
    }
    taida_val throw_val = taida_pack_get_idx(result, 2);  // throw (shifted from idx 1 to idx 2)
    // Extract the error message string to pass to the mapping function
    // (matching interpreter: passes display string, not the Error BuchiPack)
    taida_val err_display = taida_throw_to_display_string(throw_val);
    taida_val mapped_str = taida_invoke_callback1(fn_ptr, err_display);
    // Wrap the mapped result back into an Error BuchiPack
    const char *new_msg = (const char*)mapped_str;
    size_t sl = 0;
    if (taida_read_cstr_len_safe(new_msg, 65536, &sl)) {
        taida_val new_error = taida_make_error("ResultError", new_msg);
        taida_str_release(mapped_str);
        taida_str_release(err_display);
        return taida_result_create(0, new_error, 0);
    }
    // Fallback: use mapped value as-is
    taida_str_release(err_display);
    return taida_result_create(0, mapped_str, 0);
}

// Result.getOrThrow() — if success return __value, otherwise throw
taida_val taida_result_get_or_throw(taida_val result) {
    if (!taida_result_is_error_check(result)) {
        return taida_pack_get_idx(result, 0);  // __value
    }
    taida_val throw_val = taida_pack_get_idx(result, 2);  // throw (shifted to idx 2)
    if (taida_can_throw_payload(throw_val)) {
        return taida_throw(throw_val);
    }
    // Fallback: create a generic error
    taida_val error = taida_make_error("ResultError", "Result predicate failed");
    return taida_throw(error);
}

// Result.toString() — "Result(value)" or "Result(throw <= ...)"
// Helper: render a throw value for display.
// TF-16: BuchiPack errors — extract the "message" field value
// (matching interpreter: shows just the message string, not the full pack structure).
static taida_val taida_throw_to_display_string(taida_val throw_val) {
    if (throw_val == 0) return (taida_val)taida_str_new_copy("error");
    // If it's a BuchiPack (Error TypeDef), extract the "message" field
    if (taida_is_buchi_pack(throw_val)) {
        if (taida_pack_has_hash(throw_val, (taida_val)HASH_MESSAGE)) {
            taida_val msg = taida_pack_get(throw_val, (taida_val)HASH_MESSAGE);
            if (msg != 0) {
                size_t sl = 0;
                if (taida_read_cstr_len_safe((const char*)msg, 65536, &sl)) {
                    return (taida_val)taida_str_new_copy((const char*)msg);
                }
            }
        }
        // Fallback: render full pack structure for non-message packs
        return taida_value_to_display_string(throw_val);
    }
    // String error message
    const char *s = (const char*)throw_val;
    size_t sl = 0;
    if (taida_read_cstr_len_safe(s, 65536, &sl)) {
        return (taida_val)taida_str_new_copy(s);
    }
    return taida_value_to_display_string(throw_val);
}

taida_val taida_result_to_string(taida_val result) {
    if (!taida_result_is_error_check(result)) {
        taida_val value = taida_pack_get_idx(result, 0);  // __value
        taida_val value_str = taida_value_to_display_string(value);
        const char *value_cstr = (const char*)value_str;
        size_t value_len = strlen(value_cstr);
        size_t need = value_len + 10;
        char *buf = taida_str_alloc(need);
        snprintf(buf, need + 1, "Result(%s)", value_cstr);
        taida_str_release(value_str);
        return (taida_val)buf;
    }
    taida_val throw_val = taida_pack_get_idx(result, 2);  // throw (shifted to idx 2)
    if (throw_val == 0) {
        return (taida_val)taida_str_new_copy("Result(throw <= error)");
    }
    taida_val err_disp = taida_throw_to_display_string(throw_val);
    const char *err_str = (const char*)err_disp;
    size_t elen = strlen(err_str);
    size_t need = elen + 24;
    char *buf = taida_str_alloc(need);
    snprintf(buf, need + 1, "Result(throw <= %s)", err_str);
    taida_str_release(err_disp);
    return (taida_val)buf;
}

// ── Lax methods (map, flatMap) ──────────────────────────────

// Lax.map(fn) — if hasValue, apply fn to __value and return new Lax
taida_val taida_lax_map(taida_val lax_ptr, taida_val fn_ptr) {
    if (!taida_pack_get_idx(lax_ptr, 0)) {
        // Empty Lax: return empty with same default
        taida_val def = taida_pack_get_idx(lax_ptr, 2);
        return taida_lax_empty(def);
    }
    taida_val value = taida_pack_get_idx(lax_ptr, 1);
    taida_val def = taida_pack_get_idx(lax_ptr, 2);
    taida_val result = taida_invoke_callback1(fn_ptr, value);
    return taida_lax_new(result, def);
}

// Lax.flatMap(fn) — if hasValue, apply fn (which should return Lax)
taida_val taida_lax_flat_map(taida_val lax_ptr, taida_val fn_ptr) {
    if (!taida_pack_get_idx(lax_ptr, 0)) {
        taida_val def = taida_pack_get_idx(lax_ptr, 2);
        return taida_lax_empty(def);
    }
    taida_val value = taida_pack_get_idx(lax_ptr, 1);
    taida_val result = taida_invoke_callback1(fn_ptr, value);
    // flatMap expects fn to return Lax — return directly
    return result;
}

// Lax.toString() — "Lax(value)" or "Lax(default: value)"
taida_val taida_lax_to_string(taida_val lax_ptr) {
    taida_val val = taida_pack_get_idx(lax_ptr, 1);
    taida_val def = taida_pack_get_idx(lax_ptr, 2);
    taida_val rendered = taida_pack_get_idx(lax_ptr, 0)
        ? taida_value_to_display_string(val)
        : taida_value_to_display_string(def);
    const char *rs = (const char*)rendered;
    size_t need = strlen(rs) + 24;
    char *buf = taida_str_alloc(need);
    if (taida_pack_get_idx(lax_ptr, 0)) {
        snprintf(buf, need + 1, "Lax(%s)", rs);
    } else {
        snprintf(buf, need + 1, "Lax(default: %s)", rs);
    }
    taida_str_release(rendered);
    return (taida_val)buf;
}

// ── Polymorphic monadic dispatch ──────────────────────────
// These functions detect the type at runtime and dispatch to the correct impl.
// Type detection uses BuchiPack field_count + first field hash:
//   - field_count == 4, hash0 == HASH_RES___VALUE → Result (__value, __predicate, throw, __type)
//   - field_count == 4, hash0 == HASH_HAS_VALUE   → Lax (hasValue, __value, __default, __type)
//   - otherwise → List (check via capacity/length heuristic)
// Note: Optional (fc==2) was abolished in v0.8.0.
// taida_monadic_field_count returns stable type IDs:
//   3 = Result (for backward compat with all dispatch code)
//   4 = Lax/Gorillax/RelaxedGorillax

static int taida_is_list(taida_val ptr) {
    return TAIDA_IS_LIST(ptr);
}

static int taida_is_bytes(taida_val ptr) {
    return TAIDA_IS_BYTES(ptr);
}

static int taida_monadic_field_count(taida_val ptr) {
    if (!taida_ptr_is_readable(ptr, sizeof(taida_val) * 3)) return 0;
    taida_val *obj = (taida_val*)ptr;
    taida_val fc = obj[1];
    // Both Result and Lax are now fc=4; distinguish by hash0
    if (fc == 4) {
        taida_val hash0 = obj[2];
        if (hash0 > 0x10000 || hash0 < 0) {
            // Result (fc=4, hash0=HASH_RES___VALUE) → return 3 for compat
            if (hash0 == (taida_val)HASH_RES___VALUE) return 3;
            // Lax/Gorillax/RelaxedGorillax (fc=4, hash0=HASH_HAS_VALUE) → return 4
            if (hash0 == (taida_val)HASH_HAS_VALUE) return 4;
        }
    }
    return 0;
}

// ── Async pthread support ────────────────────────────────────
// Thread argument: passed to pthread entry, stores callback + result pointer.
typedef struct {
    taida_val fn_ptr;
    taida_val arg;               // callback argument
    taida_val *async_obj;        // back-pointer to Async object (writes value/status)
} taida_thread_arg;

// NO-3: Detect the type tag of a runtime value by inspecting its magic header.
// Returns TAIDA_TAG_* constant. Used by thread entry to set value_tag dynamically.
static taida_val taida_detect_value_tag(taida_val val) {
    if (val == 0) return TAIDA_TAG_INT;
    if (val > 0 && val < 4096) return TAIDA_TAG_INT;  // small integer
    if (val < 0) return TAIDA_TAG_INT;  // negative integer (or float-as-bits, but conservative)
    if (!taida_ptr_is_readable(val, sizeof(taida_val))) return TAIDA_TAG_INT;
    taida_val *obj = (taida_val*)val;
    taida_val magic = obj[0] & TAIDA_MAGIC_MASK;
    if (magic == TAIDA_PACK_MAGIC) return TAIDA_TAG_PACK;
    if (magic == TAIDA_LIST_MAGIC) return TAIDA_TAG_LIST;
    if (magic == TAIDA_CLOSURE_MAGIC) return TAIDA_TAG_CLOSURE;
    if (magic == TAIDA_HMAP_MAGIC) return TAIDA_TAG_HMAP;
    if (magic == TAIDA_SET_MAGIC) return TAIDA_TAG_SET;
    if (magic == TAIDA_ASYNC_MAGIC) return TAIDA_TAG_PACK;  // Async uses PACK tag for retain/release
    if (magic == TAIDA_STR_MAGIC) return TAIDA_TAG_STR;
    // Check hidden-header String: ptr-16 may contain STR_MAGIC.
    // Same pattern as taida_str_release.
    {
        taida_val *hdr = ((taida_val*)val) - 2;
        if (taida_ptr_is_readable((taida_val)hdr, sizeof(taida_val))) {
            taida_val htag = hdr[0] & TAIDA_MAGIC_MASK;
            if (htag == TAIDA_STR_MAGIC) return TAIDA_TAG_STR;
        }
    }
    // Could be a raw char* or an integer pointer.
    // Conservative: return UNKNOWN to avoid misidentifying ints as pointers.
    return TAIDA_TAG_UNKNOWN;
}

// pthread entry point: call the function, write result into the Async object.
static void* taida_thread_entry(void* raw) {
    taida_thread_arg *ta = (taida_thread_arg*)raw;
    taida_val result = taida_invoke_callback1(ta->fn_ptr, ta->arg);
    // NO-3: detect value type and store tag for recursive release on drop.
    // Move semantics: the callback result is transferred to the Async object.
    taida_val vtag = taida_detect_value_tag(result);
    ta->async_obj[2] = result;   // write value
    ta->async_obj[5] = vtag;     // set value_tag
    __atomic_thread_fence(__ATOMIC_RELEASE);  // barrier: ensure value+tag visible before status
    ta->async_obj[1] = 1;        // mark fulfilled (must be last — signals to readers)
    free(ta);
    return NULL;
}

// Detect Async value: [ASYNC_MAGIC, status, value, error, thread_handle, value_tag, error_tag]
// Uses a magic number in slot[0] for unambiguous identification.
static int taida_is_async(taida_val ptr) {
    return TAIDA_IS_ASYNC(ptr);
}

// Detect BuchiPack of any size (fc >= 1, with FNV-1a hash check)
static int taida_is_buchi_pack(taida_val ptr) {
    return TAIDA_IS_PACK(ptr);
}

// Forward declare recursive value-to-display-string
// NO-4 RULE 2: These functions return heap-allocated strings via taida_str_new_copy
// or taida_str_alloc. The CALLER is responsible for calling taida_str_release on
// the returned value after use. Intermediate strings generated during recursive
// formatting (e.g., item_str in list display) are released within the function.
static taida_val taida_value_to_display_string(taida_val val);
static taida_val taida_value_to_debug_string(taida_val val);

// Convert a list to display string: @[item1, item2, ...]
static taida_val taida_list_to_display_string(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val list_len = list[2];
    size_t cap = 64;
    size_t len = 0;
    char *buf = (char*)TAIDA_MALLOC(cap, "list_to_string");
    buf[0] = '\0';
    // Append "@["
    { const char *s = "@["; size_t sl = 2; while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, s, sl); len += sl; buf[len] = '\0'; }
    for (taida_val i = 0; i < list_len; i++) {
        if (i > 0) {
            const char *s = ", "; size_t sl = 2; while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, s, sl); len += sl; buf[len] = '\0';
        }
        taida_val item = list[4 + i];
        taida_val item_str = taida_value_to_debug_string(item);
        const char *is = (const char*)item_str;
        if (is) {
            size_t sl = strlen(is); while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, is, sl); len += sl; buf[len] = '\0';
        }
        taida_str_release(item_str);
    }
    // Append "]"
    while (len + 2 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
    buf[len++] = ']'; buf[len] = '\0';
    taida_val result = (taida_val)taida_str_new_copy(buf);
    free(buf);
    return result;
}

static taida_val taida_bytes_to_display_string(taida_val bytes_ptr) {
    if (!TAIDA_IS_BYTES(bytes_ptr)) {
        return (taida_val)taida_str_new_copy("Bytes[@[]]");
    }
    taida_val *bytes = (taida_val*)bytes_ptr;
    taida_val len_bytes = bytes[1];
    size_t cap = 64;
    size_t len = 0;
    char *buf = (char*)TAIDA_MALLOC(cap, "bytes_to_string");
    buf[0] = '\0';
    const char *prefix = "Bytes[@[";
    size_t pl = strlen(prefix);
    while (len + pl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
    memcpy(buf + len, prefix, pl);
    len += pl;
    buf[len] = '\0';

    for (taida_val i = 0; i < len_bytes; i++) {
        if (i > 0) {
            const char *sep = ", ";
            size_t sl = 2;
            while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
            memcpy(buf + len, sep, sl);
            len += sl;
            buf[len] = '\0';
        }
        char nbuf[8];
        int wrote = snprintf(nbuf, sizeof(nbuf), "%" PRId64 "", bytes[2 + i]);
        if (wrote < 0) wrote = 0;
        size_t sl = (size_t)wrote;
        while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
        memcpy(buf + len, nbuf, sl);
        len += sl;
        buf[len] = '\0';
    }

    const char *suffix = "]]";
    size_t sl = 2;
    while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
    memcpy(buf + len, suffix, sl);
    len += sl;
    buf[len] = '\0';
    taida_val result = (taida_val)taida_str_new_copy(buf);
    free(buf);
    return result;
}

// Convert a BuchiPack to display string: @(field <= value, ...)
static taida_val taida_pack_to_display_string(taida_val pack_ptr) {
    taida_val *pack = (taida_val*)pack_ptr;
    taida_val fc = pack[1];
    size_t cap = 128;
    size_t len = 0;
    char *buf = (char*)TAIDA_MALLOC(cap, "pack_to_display_string");
    buf[0] = '\0';
    // Append "@("
    { const char *s = "@("; size_t sl = 2; while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, s, sl); len += sl; buf[len] = '\0'; }
    int count = 0;
    for (taida_val i = 0; i < fc; i++) {
        taida_val field_hash = pack[2 + i * 3];
        taida_val field_val = pack[2 + i * 3 + 2];
        const char *fname = taida_lookup_field_name(field_hash);
        if (!fname) continue;
        // Skip internal __ fields for display
        if (fname[0] == '_' && fname[1] == '_') continue;
        if (count > 0) {
            const char *s = ", "; size_t sl = 2; while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, s, sl); len += sl; buf[len] = '\0';
        }
        // Append "fieldname <= "
        size_t nlen = strlen(fname);
        while (len + nlen + 5 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
        memcpy(buf + len, fname, nlen); len += nlen;
        memcpy(buf + len, " <= ", 4); len += 4;
        buf[len] = '\0';
        // Check if field is Bool via registry
        int ftype = taida_lookup_field_type(field_hash);
        if (ftype == 4) {
            // Bool: display as true/false
            const char *bv = field_val ? "true" : "false";
            size_t sl = strlen(bv); while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, bv, sl); len += sl; buf[len] = '\0';
        } else {
            // Append value (debug string: strings are quoted)
            taida_val val_str = taida_value_to_debug_string(field_val);
            const char *vs = (const char*)val_str;
            if (vs) {
                size_t sl = strlen(vs); while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); } memcpy(buf + len, vs, sl); len += sl; buf[len] = '\0';
            }
            taida_str_release(val_str);
        }
        count++;
    }
    // Append ")"
    while (len + 2 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string"); }
    buf[len++] = ')'; buf[len] = '\0';
    taida_val result = (taida_val)taida_str_new_copy(buf);
    free(buf);
    return result;
}

// TF-15: Pack to display string with ALL fields (including __ internal fields).
// Matches interpreter's to_display_string() for BuchiPack which shows all fields.
static taida_val taida_pack_to_display_string_full(taida_val pack_ptr) {
    taida_val *pack = (taida_val*)pack_ptr;
    taida_val fc = pack[1];
    size_t cap = 128;
    size_t len = 0;
    char *buf = (char*)TAIDA_MALLOC(cap, "pack_to_display_string_full");
    buf[0] = '\0';
    // Append "@("
    { const char *s = "@("; size_t sl = 2; while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string_full"); } memcpy(buf + len, s, sl); len += sl; buf[len] = '\0'; }
    int count = 0;
    for (taida_val i = 0; i < fc; i++) {
        taida_val field_hash = pack[2 + i * 3];
        taida_val field_val = pack[2 + i * 3 + 2];
        const char *fname = taida_lookup_field_name(field_hash);
        if (!fname) continue;
        // NOTE: Unlike taida_pack_to_display_string, we do NOT skip __ fields
        if (count > 0) {
            const char *s = ", "; size_t sl = 2; while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string_full"); } memcpy(buf + len, s, sl); len += sl; buf[len] = '\0';
        }
        // Append "fieldname <= "
        size_t nlen = strlen(fname);
        while (len + nlen + 5 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string_full"); }
        memcpy(buf + len, fname, nlen); len += nlen;
        memcpy(buf + len, " <= ", 4); len += 4;
        buf[len] = '\0';
        // Check if field is Bool via registry
        int ftype = taida_lookup_field_type(field_hash);
        if (ftype == 4) {
            const char *bv = field_val ? "true" : "false";
            size_t sl = strlen(bv); while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string_full"); } memcpy(buf + len, bv, sl); len += sl; buf[len] = '\0';
        } else {
            taida_val val_str = taida_value_to_debug_string(field_val);
            const char *vs = (const char*)val_str;
            if (vs) {
                size_t sl = strlen(vs); while (len + sl + 1 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string_full"); } memcpy(buf + len, vs, sl); len += sl; buf[len] = '\0';
            }
            taida_str_release(val_str);
        }
        count++;
    }
    while (len + 2 > cap) { cap *= 2; TAIDA_REALLOC(buf, cap, "to_string_full"); }
    buf[len++] = ')'; buf[len] = '\0';
    taida_val result = (taida_val)taida_str_new_copy(buf);
    free(buf);
    return result;
}

// Convert any Taida value to its display string (like interpreter's to_display_string)
static taida_val taida_value_to_display_string(taida_val val) {
    if (val == 0) {
        return (taida_val)taida_str_new_copy("0");
    }
    // Precise object checks using magics first.
    if (taida_is_hashmap(val)) return taida_hashmap_to_string(val);
    if (taida_is_set(val)) return taida_set_to_string(val);
    if (taida_is_async(val)) return taida_async_to_string(val);
    if (taida_is_list(val)) return taida_list_to_display_string(val);
    if (taida_is_bytes(val)) return taida_bytes_to_display_string(val);

    // Check for BuchiPack (including monadic types)
    if (taida_is_buchi_pack(val)) {
        int fc = taida_monadic_field_count(val);
        if (fc == 3) return taida_result_to_string(val);
        if (fc == 4) {
            int gtype = taida_detect_gorillax_type(val);
            if (gtype == 1) return taida_gorillax_to_string(val);
            if (gtype == 2) return taida_relaxed_gorillax_to_string(val);
            return taida_lax_to_string(val);
        }
        return taida_pack_to_display_string(val);
    }

    // Check if it's a safely readable string (char*).
    const char *s = (const char*)val;
    size_t sl = 0;
    if (taida_read_cstr_len_safe(s, 65536, &sl)) {
        char *r = taida_str_alloc(sl);
        memcpy(r, s, sl);
        return (taida_val)r;
    }
    // Fallback: it's an integer.
    char tmp[32]; snprintf(tmp, sizeof(tmp), "%" PRId64 "", val); return (taida_val)taida_str_new_copy(tmp);
}

// Convert value to debug string (strings are quoted, everything else like display)
static taida_val taida_value_to_debug_string(taida_val val) {
    if (val == 0) {
        return (taida_val)taida_str_new_copy("0");
    }
    // Check for objects first using magics
    if (taida_is_hashmap(val)) return taida_hashmap_to_string(val);
    if (taida_is_set(val)) return taida_set_to_string(val);
    if (taida_is_async(val)) return taida_async_to_string(val);
    if (taida_is_list(val)) return taida_list_to_display_string(val);
    if (taida_is_bytes(val)) return taida_bytes_to_display_string(val);
    if (taida_is_buchi_pack(val)) {
        int fc = taida_monadic_field_count(val);
        if (fc == 3) return taida_result_to_string(val);
        if (fc == 4) return taida_lax_to_string(val);
        return taida_pack_to_display_string(val);
    }

    // Check for string (quoted in debug output)
    const char *s = (const char*)val;
    size_t sl = 0;
    if (taida_read_cstr_len_safe(s, 65536, &sl)) {
        char *r = taida_str_alloc(sl + 2);
        r[0] = '"';
        memcpy(r + 1, s, sl);
        r[sl + 1] = '"';
        return (taida_val)r;
    }
    // Fallback: integer
    char tmp[32]; snprintf(tmp, sizeof(tmp), "%" PRId64 "", val); return (taida_val)taida_str_new_copy(tmp);
}

// Polymorphic .getOrDefault(fallback) — works on Result, Lax
taida_val taida_polymorphic_get_or_default(taida_val obj, taida_val def) {
    if (obj == 0 || obj < 4096) return def;
    // Check Async first (before monadic, since Async has different layout)
    if (taida_is_async(obj)) return taida_async_get_or_default(obj, def);
    int fc = taida_monadic_field_count(obj);
    if (fc == 3) return taida_result_get_or_default(obj, def);    // Result
    if (fc == 4) return taida_lax_get_or_default(obj, def);       // Lax
    return def;
}

// Polymorphic .hasValue() — works on Lax
taida_val taida_polymorphic_has_value(taida_val obj) {
    if (obj == 0 || obj < 4096) return 0;
    int fc = taida_monadic_field_count(obj);
    if (fc == 4) return taida_pack_get_idx(obj, 0);     // Lax: hasValue field
    return 0;
}

// Polymorphic .isEmpty() — works on List, Lax
taida_val taida_polymorphic_is_empty(taida_val obj) {
    if (obj == 0 || obj < 4096) return 1;
    // Check for HashMap
    if (taida_is_hashmap(obj)) return taida_hashmap_is_empty(obj);
    // Check for Set (uses same layout as list, so list_is_empty works)
    if (taida_is_set(obj)) return taida_set_is_empty(obj);
    if (taida_is_bytes(obj)) return taida_bytes_len(obj) == 0 ? 1 : 0;
    int fc = taida_monadic_field_count(obj);
    if (fc == 4) return taida_pack_get_idx(obj, 0) ? 0 : 1;  // Lax
    // Default: treat as list
    return taida_list_is_empty(obj);
}

// Polymorphic .toString() — works on Int, Float, Bool, Result, Lax, HashMap, Set, List, BuchiPack
taida_val taida_polymorphic_to_string(taida_val obj) {
    // RCB-222: Check for user-defined toString method on BuchiPack types.
    // If the pack has a function field named "toString", call it instead of
    // formatting as @(field <= value, ...). This matches the Interpreter's
    // type_methods dispatch behavior.
    if (taida_is_buchi_pack(obj)) {
        // FNV-1a hash of "toString"
        const taida_val toString_hash = 0xc5c8cdb28370e485ULL;
        taida_val fn_ptr = taida_pack_get(obj, toString_hash);
        if (fn_ptr != 0 && (TAIDA_IS_CLOSURE(fn_ptr) || taida_ptr_is_readable(fn_ptr, 1))) {
            // Check if it looks like a function (closure or function pointer)
            if (TAIDA_IS_CLOSURE(fn_ptr)) {
                taida_val *closure = (taida_val*)fn_ptr;
                taida_closure_cb0_fn closure_fn = (taida_closure_cb0_fn)closure[1];
                taida_val env_ptr = closure[2];
                return closure_fn(env_ptr);
            }
            // Plain function pointer — but we need to distinguish from non-function values.
            // Function pointers are in code segment, not heap. We cannot reliably distinguish
            // them from string pointers at runtime, so only dispatch closures here.
            // Non-closure toString fields (e.g., string values) fall through to default display.
        }
    }
    return taida_value_to_display_string(obj);
}

// TF-15: stdout display — renders BuchiPacks with ALL fields (including __ internal fields)
// matching the interpreter's to_display_string() behavior.
// .toString() methods use taida_polymorphic_to_string which produces Lax(...)/Result(...) forms.
taida_val taida_stdout_display_string(taida_val obj) {
    if (obj == 0) return (taida_val)taida_str_new_copy("0");
    if (taida_is_buchi_pack(obj)) {
        return taida_pack_to_display_string_full(obj);
    }
    return taida_value_to_display_string(obj);
}

// typeof(value, tag) — returns type name as a string.
// tag is a compile-time hint: 0=Int, 1=Float, 2=Bool, 3=Str, 4=Pack, 5=List, 6=Closure.
// For heap objects the tag is ignored and runtime detection is used.
taida_val taida_typeof(taida_val val, taida_val tag) {
    // For non-zero heap pointers, detect at runtime via magic headers
    if (val != 0 && val >= 4096) {
        if (taida_is_hashmap(val)) return (taida_val)taida_str_new_copy("HashMap");
        if (taida_is_set(val)) return (taida_val)taida_str_new_copy("Set");
        if (taida_is_async(val)) return (taida_val)taida_str_new_copy("Async");
        if (taida_is_list(val)) return (taida_val)taida_str_new_copy("List");
        if (taida_is_bytes(val)) return (taida_val)taida_str_new_copy("Bytes");
        if (taida_is_buchi_pack(val)) {
            int fc = taida_monadic_field_count(val);
            if (fc == 3) return (taida_val)taida_str_new_copy("Result");
            if (fc == 4) {
                int gtype = taida_detect_gorillax_type(val);
                if (gtype == 1) return (taida_val)taida_str_new_copy("Gorillax");
                if (gtype == 2) return (taida_val)taida_str_new_copy("RelaxedGorillax");
                return (taida_val)taida_str_new_copy("Lax");
            }
            return (taida_val)taida_str_new_copy("BuchiPack");
        }
        // Check if it's a string pointer
        const char *s = (const char*)val;
        size_t sl = 0;
        if (taida_read_cstr_len_safe(s, 65536, &sl)) {
            return (taida_val)taida_str_new_copy("Str");
        }
    }
    // For scalars, use the compile-time tag
    switch (tag) {
        case 1: return (taida_val)taida_str_new_copy("Float");
        case 2: return (taida_val)taida_str_new_copy("Bool");
        case 3: return (taida_val)taida_str_new_copy("Str");
        case 4: return (taida_val)taida_str_new_copy("BuchiPack");
        case 5: return (taida_val)taida_str_new_copy("List");
        case 6: return (taida_val)taida_str_new_copy("Closure");
        default: return (taida_val)taida_str_new_copy("Int");
    }
}

// Polymorphic .map(fn) — works on List, Result, Lax, Async
taida_val taida_polymorphic_map(taida_val obj, taida_val fn_ptr) {
    if (obj == 0 || obj < 4096) return obj;
    // Check Async first (before monadic, since Async has different layout)
    if (taida_is_async(obj)) return taida_async_map(obj, fn_ptr);
    int fc = taida_monadic_field_count(obj);
    if (fc == 3) return taida_result_map(obj, fn_ptr);
    if (fc == 4) return taida_lax_map(obj, fn_ptr);
    // Default: treat as list
    return taida_list_map(obj, fn_ptr);
}

// Polymorphic .flatMap(fn) — works on Result, Lax
taida_val taida_monadic_flat_map(taida_val obj, taida_val fn_ptr) {
    if (obj == 0 || obj < 4096) return obj;
    int fc = taida_monadic_field_count(obj);
    if (fc == 3) return taida_result_flat_map(obj, fn_ptr);
    if (fc == 4) return taida_lax_flat_map(obj, fn_ptr);
    return obj;  // fallback
}

// Polymorphic .getOrThrow() — works on Result
taida_val taida_monadic_get_or_throw(taida_val obj) {
    if (obj == 0 || obj < 4096) return obj;
    int fc = taida_monadic_field_count(obj);
    if (fc == 3) return taida_result_get_or_throw(obj);
    // Lax doesn't have getOrThrow — fall back to unmold
    if (fc == 4) return taida_lax_unmold(obj);
    return obj;
}

// Polymorphic .toString() — works on Result, Lax
taida_val taida_monadic_to_string(taida_val obj) {
    if (obj == 0 || obj < 4096) {
        char tmp[32];
        snprintf(tmp, 32, "%" PRId64 "", obj);
        return (taida_val)taida_str_new_copy(tmp);
    }
    int fc = taida_monadic_field_count(obj);
    if (fc == 3) return taida_result_to_string(obj);
    if (fc == 4) {
        int gtype = taida_detect_gorillax_type(obj);
        if (gtype == 1) return taida_gorillax_to_string(obj);
        if (gtype == 2) return taida_relaxed_gorillax_to_string(obj);
        return taida_lax_to_string(obj);
    }
    // Fallback: treat as int
    char tmp[32];
    snprintf(tmp, 32, "%" PRId64 "", obj);
    return (taida_val)taida_str_new_copy(tmp);
}

// ── Async methods ────────────────────────────────────────
taida_val taida_async_map(taida_val async_ptr, taida_val fn_ptr) {
    taida_val *obj = (taida_val*)async_ptr;
    // Join thread if pending
    if (obj[1] == 0) taida_async_join(async_ptr);
    if (obj[1] != 1) return async_ptr; // not fulfilled, return as-is
    taida_val new_val = taida_invoke_callback1(fn_ptr, obj[2]);
    // NO-3: detect type of mapped value and create tagged async
    taida_val vtag = taida_detect_value_tag(new_val);
    return taida_async_ok_tagged(new_val, vtag);
}

taida_val taida_async_get_or_default(taida_val async_ptr, taida_val def) {
    taida_val *obj = (taida_val*)async_ptr;
    // Join thread if pending
    if (obj[1] == 0) taida_async_join(async_ptr);
    if (obj[1] == 1) return obj[2]; // fulfilled
    return def;
}

// ── Async runtime ─────────────────────────────────────────
// NO-4 RULE 1: Async producers MUST use taida_async_ok_tagged (not taida_async_ok)
// to set value_tag. Legacy taida_async_ok uses UNKNOWN tag (conservative leak).
// NO-3: Async layout: [ASYNC_MAGIC, status, value, error, thread_handle, value_tag, error_tag]
//   status: 0=pending, 1=fulfilled, 2=rejected
//   thread_handle: 0 = no thread, otherwise pthread_t cast to taida_val
//   value_tag: type tag for value (TAIDA_TAG_* constant, -1 = UNKNOWN)
//   error_tag: type tag for error (usually TAIDA_TAG_PACK from taida_make_error)

taida_val taida_async_ok_tagged(taida_val value, taida_val value_tag) {
    // M-14: TAIDA_MALLOC ensures NULL check + OOM diagnostic.
    taida_val *obj = (taida_val*)TAIDA_MALLOC(7 * sizeof(taida_val), "async_ok_tagged");
    obj[0] = TAIDA_ASYNC_MAGIC | 1;  // magic + refcount
    obj[1] = 1;  // fulfilled
    obj[2] = value;
    obj[3] = 0;  // no error
    obj[4] = 0;  // no thread
    obj[5] = value_tag;
    obj[6] = TAIDA_TAG_UNKNOWN;  // no error
    // NO-3: move semantics — caller transfers ownership of value to Async.
    // Async release will call taida_list_elem_release on value.
    // If the value is shared, the caller must retain before calling this.
    return (taida_val)obj;
}

taida_val taida_async_ok(taida_val value) {
    // Legacy wrapper: uses UNKNOWN tag (conservative — no retain/release for children)
    // M-14: TAIDA_MALLOC ensures NULL check + OOM diagnostic.
    taida_val *obj = (taida_val*)TAIDA_MALLOC(7 * sizeof(taida_val), "async_ok");
    obj[0] = TAIDA_ASYNC_MAGIC | 1;  // magic + refcount
    obj[1] = 1;  // fulfilled
    obj[2] = value;
    obj[3] = 0;  // no error
    obj[4] = 0;  // no thread
    obj[5] = TAIDA_TAG_UNKNOWN;  // value_tag unknown
    obj[6] = TAIDA_TAG_UNKNOWN;  // no error
    return (taida_val)obj;
}

taida_val taida_async_err(taida_val error) {
    // M-14: TAIDA_MALLOC ensures NULL check + OOM diagnostic.
    taida_val *obj = (taida_val*)TAIDA_MALLOC(7 * sizeof(taida_val), "async_err");
    obj[0] = TAIDA_ASYNC_MAGIC | 1;  // magic + refcount
    obj[1] = 2;  // rejected
    obj[2] = 0;  // no value
    obj[3] = error;
    obj[4] = 0;  // no thread
    obj[5] = TAIDA_TAG_UNKNOWN;  // no value
    obj[6] = TAIDA_TAG_PACK;    // error is always a Pack (from taida_make_error)
    // NO-3: move semantics — caller transfers ownership of error to Async.
    return (taida_val)obj;
}

// NO-3: Set value_tag on an existing Async object (for lowering to call after taida_async_ok)
void taida_async_set_value_tag(taida_val async_ptr, taida_val tag) {
    ((taida_val*)async_ptr)[5] = tag;
}

// Join a pending Async's thread (if any). After this call, status is no longer Pending.
static void taida_async_join(taida_val async_ptr) {
    taida_val *obj = (taida_val*)async_ptr;
    if (obj[1] != 0) return;              // not pending — nothing to join
    taida_val th = obj[4];
    if (th != 0) {
        pthread_join((pthread_t)th, NULL);
        obj[4] = 0;                       // clear thread handle
        // Thread entry already set status + value
    }
}

taida_val taida_async_unmold(taida_val async_ptr) {
    if (async_ptr == 0) return 0;
    taida_val *obj = (taida_val*)async_ptr;
    // If pending with a thread, join it first
    if (obj[1] == 0) {
        taida_async_join(async_ptr);
    }
    taida_val status = obj[1];
    if (status == 1) return obj[2];       // fulfilled → value
    if (status == 2) {                    // rejected → throw (catchable by |==)
        taida_val error = obj[3];
        if (taida_can_throw_payload(error)) {
            return taida_throw(error);
        }
        taida_val err = taida_make_error("AsyncError", "Async rejected");
        return taida_throw(err);
    }
    return 0;                              // pending (no thread) → Unit
}

// ── Async spawn (pthread-based) ──────────────────────────────

// Spawn a function in a background pthread. Returns Async[pending] with thread_handle.
taida_val taida_async_spawn(taida_val fn_ptr, taida_val arg) {
    taida_thread_arg *ta = (taida_thread_arg*)TAIDA_MALLOC(sizeof(taida_thread_arg), "async_spawn_arg");
    taida_val *obj = (taida_val*)TAIDA_MALLOC(7 * sizeof(taida_val), "async_spawn");
    obj[0] = TAIDA_ASYNC_MAGIC | 1; // Magic + initial refcount
    obj[1] = 0;   // status: pending
    obj[2] = 0;   // no value yet
    obj[3] = 0;   // no error
    obj[4] = 0;   // thread handle (set below)
    obj[5] = TAIDA_TAG_UNKNOWN;  // value_tag (set when resolved)
    obj[6] = TAIDA_TAG_UNKNOWN;  // error_tag (set when rejected)

    ta->fn_ptr = fn_ptr;
    ta->arg = arg;
    ta->async_obj = obj;

    pthread_t thread;
    pthread_create(&thread, NULL, taida_thread_entry, ta);
    obj[4] = (taida_val)thread;

    return (taida_val)obj;
}

taida_val taida_async_cancel(taida_val async_ptr) {
    if (async_ptr == 0) {
        taida_val err = taida_make_error("CancelledError", "Async operation cancelled");
        return taida_async_err(err);
    }
    if (!TAIDA_IS_ASYNC(async_ptr)) {
        // NO-3: detect value type for ownership tracking
        taida_val vtag = taida_detect_value_tag(async_ptr);
        return taida_async_ok_tagged(async_ptr, vtag);
    }

    taida_val *obj = (taida_val*)async_ptr;
    if (obj[1] != 0) {
        // Fulfilled/rejected async values are already resolved.
        return async_ptr;
    }

    taida_val th = obj[4];
    if (th != 0) {
        // Best-effort cancellation for pending pthread tasks.
        pthread_cancel((pthread_t)th);
        pthread_detach((pthread_t)th);
    }
    taida_val err = taida_make_error("CancelledError", "Async operation cancelled");
    return taida_async_err(err);
}

// ── Async aggregation ────────────────────────────────────────

// All[asyncList]() — join all pending threads, collect all fulfilled values.
// If any element is rejected, throw the error.
taida_val taida_async_all(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    // First pass: join all pending threads
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        if (TAIDA_IS_ASYNC(item)) {
            taida_async_join(item);
        }
    }
    // Second pass: collect values, retaining each element and tracking elem_type_tag.
    taida_val result_list = taida_list_new();
    taida_val unified_tag = TAIDA_TAG_UNKNOWN;
    int tag_set = 0;
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        taida_val val;
        taida_val vtag;
        if (TAIDA_IS_ASYNC(item)) {
            taida_val *obj = (taida_val*)item;
            taida_val status = obj[1];
            if (status == 2) {
                taida_val error = obj[3];
                // Release partially built result_list before throwing
                taida_release(result_list);
                if (taida_can_throw_payload(error)) {
                    return taida_throw(error);
                }
                taida_val err = taida_make_error("AsyncError", "All: async rejected");
                return taida_throw(err);
            }
            val = obj[2];
            vtag = obj[5];  // value_tag from source Async
        } else {
            val = item;
            vtag = taida_detect_value_tag(item);
        }
        // QF-58: retain element before pushing (source Async still owns it)
        taida_list_elem_retain(val, vtag);
        result_list = taida_list_push(result_list, val);
        // Track unified elem_type_tag
        if (!tag_set) {
            unified_tag = vtag;
            tag_set = 1;
        } else if (unified_tag != vtag) {
            unified_tag = TAIDA_TAG_UNKNOWN;  // heterogeneous → UNKNOWN
        }
    }
    // QF-58: set elem_type_tag on result list
    taida_list_set_elem_tag(result_list, unified_tag);
    // NO-3: result is always a List
    return taida_async_ok_tagged(result_list, TAIDA_TAG_LIST);
}

// Race[asyncList]() — join all pending threads, return the first fulfilled value.
taida_val taida_async_race(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    if (len == 0) {
        // Matches Interpreter behavior: Race[@[]] -> Async(@())
        return taida_async_ok_tagged(taida_pack_new(0), TAIDA_TAG_PACK);
    }
    // Join all pending threads (simple approach: join all, pick first)
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        if (TAIDA_IS_ASYNC(item)) {
            taida_async_join(item);
        }
    }
    taida_val first = list[4];
    if (TAIDA_IS_ASYNC(first)) {
        taida_val *obj = (taida_val*)first;
        taida_val status = obj[1];
        if (status == 2) {
            taida_val error = obj[3];
            if (taida_can_throw_payload(error)) {
                return taida_throw(error);
            }
            taida_val err = taida_make_error("AsyncError", "Race: async rejected");
            return taida_throw(err);
        }
        // NO-3: propagate value_tag from the source Async.
        // Retain because source Async still owns obj[2] and will release on drop.
        taida_list_elem_retain(obj[2], obj[5]);
        return taida_async_ok_tagged(obj[2], obj[5]);
    }
    // NO-3: non-async element — detect its type.
    // The element is borrowed from the input list; retain for new Async ownership.
    taida_val ftag = taida_detect_value_tag(first);
    taida_list_elem_retain(first, ftag);
    return taida_async_ok_tagged(first, ftag);
}

// Generic unmold: detect whether this is a Result, Lax, or Async at runtime
// Optional abolished in v0.8.0 — use Lax[T] instead.
// Result:   BuchiPack fc=4, hash0=HASH_RES___VALUE → evaluate predicate, check throw, return __value or throw
// Lax:      BuchiPack fc=4, hash0=HASH_HAS_VALUE → lax_unmold
// Async:    [ASYNC_MAGIC, status, value, error, thread_handle, value_tag, error_tag]
taida_val taida_generic_unmold(taida_val ptr) {
    if (ptr == 0) return 0;

    if (taida_is_molten(ptr)) {
        taida_val error = taida_make_error(
            "TypeError",
            "Cannot unmold Molten directly. Molten can only be used inside Cage."
        );
        return taida_throw(error);
    }
    
    // Check for BuchiPack (monadic types) using magic
    if (TAIDA_IS_PACK(ptr)) {
        taida_val *obj = (taida_val*)ptr;
        taida_val field_count = obj[1];
        taida_val hash0 = obj[2];

        // Result (fc=4, hash0=HASH_RES___VALUE): evaluate predicate + check throw
        if (field_count == 4 && hash0 == (taida_val)HASH_RES___VALUE) {
        taida_val value = taida_pack_get_idx(ptr, 0);       // __value
        taida_val pred = taida_pack_get_idx(ptr, 1);         // __predicate
        taida_val throw_val = taida_pack_get_idx(ptr, 2);    // throw

        // If throw is set explicitly, check predicate first
        if (throw_val != 0) {
            if (pred != 0) {
                taida_val pred_result = taida_invoke_callback1(pred, value);
                if (!pred_result) {
                    // Predicate failed — throw the error
                    if (taida_can_throw_payload(throw_val)) return taida_throw(throw_val);
                    taida_val error = taida_make_error("ResultError", "Result predicate failed");
                    return taida_throw(error);
                }
                // Predicate passed even with throw set — return value
                return value;
            }
            // No predicate, throw is set — throw
            if (taida_can_throw_payload(throw_val)) return taida_throw(throw_val);
            taida_val error = taida_make_error("ResultError", "Result error");
            return taida_throw(error);
        }

        // Evaluate predicate if present (no throw set)
        if (pred != 0) {
            taida_val pred_result = taida_invoke_callback1(pred, value);
            if (pred_result) return value;  // success
            // Predicate failed — throw default error
            taida_val error = taida_make_error("ResultError", "Result predicate failed");
            return taida_throw(error);
        }

        // No predicate, no throw — success
        return value;
    }

    // Lax/Gorillax/RelaxedGorillax (fc=4, hash0=HASH_HAS_VALUE)
    if (field_count == 4 && hash0 == (taida_val)HASH_HAS_VALUE) {
        int gtype = taida_detect_gorillax_type(ptr);
        if (gtype == 1) return taida_gorillax_unmold(ptr);
        if (gtype == 2) return taida_relaxed_gorillax_unmold(ptr);
        return taida_lax_unmold(ptr);
    }

    // TODO mold unmold — check __type tag and extract via unm/default/sol/value channels.
    // The `unm` channel is returned when present (priority: unm > __default > sol > __value).
    if (taida_pack_has_hash(ptr, (taida_val)HASH___TYPE)) {
        taida_val type_ptr = taida_pack_get(ptr, (taida_val)HASH___TYPE);
        int is_todo = 0;
        if (type_ptr == (taida_val)__todo_type_str) {
            is_todo = 1;
        } else if (type_ptr > 4096) {
            const char *type_str = (const char*)type_ptr;
            size_t len = 0;
            if (taida_read_cstr_len_safe(type_str, 32, &len) &&
                len == 4 && memcmp(type_str, "TODO", 4) == 0) {
                is_todo = 1;
            }
        }
        if (is_todo) {
            if (taida_pack_has_hash(ptr, (taida_val)HASH_TODO_UNM)) {
                return taida_pack_get(ptr, (taida_val)HASH_TODO_UNM);
            }
            if (taida_pack_has_hash(ptr, (taida_val)HASH___DEFAULT)) {
                return taida_pack_get(ptr, (taida_val)HASH___DEFAULT);
            }
            if (taida_pack_has_hash(ptr, (taida_val)HASH_TODO_SOL)) {
                return taida_pack_get(ptr, (taida_val)HASH_TODO_SOL);
            }
            if (taida_pack_has_hash(ptr, (taida_val)HASH___VALUE)) {
                return taida_pack_get(ptr, (taida_val)HASH___VALUE);
            }
            return taida_pack_new(0);
        }
    }

    // Custom mold default unmold:
    // pack with first field __type and a __value field.
    if (hash0 == (taida_val)HASH___TYPE &&
        taida_pack_has_hash(ptr, (taida_val)HASH___VALUE)) {
        return taida_pack_get(ptr, (taida_val)HASH___VALUE);
    }
    }

    // Check if this is an Async: [ASYNC_MAGIC, status, value, error, thread_handle, value_tag, error_tag]
    if (TAIDA_IS_ASYNC(ptr)) {
        return taida_async_unmold(ptr);
    }
    // Not a monadic type or Async — return as-is (e.g., list, string, plain value)
    return ptr;
}

taida_val taida_async_is_pending(taida_val async_ptr) {
    return ((taida_val*)async_ptr)[1] == 0 ? 1 : 0;
}

taida_val taida_async_is_fulfilled(taida_val async_ptr) {
    return ((taida_val*)async_ptr)[1] == 1 ? 1 : 0;
}

taida_val taida_async_is_rejected(taida_val async_ptr) {
    return ((taida_val*)async_ptr)[1] == 2 ? 1 : 0;
}

taida_val taida_async_get_value(taida_val async_ptr) {
    return ((taida_val*)async_ptr)[2];
}

taida_val taida_async_get_error(taida_val async_ptr) {
    return ((taida_val*)async_ptr)[3];
}

// Async toString — format like interpreter: Async[fulfilled: value] / Async[rejected: error] / Async[pending]
static taida_val taida_async_to_string(taida_val async_ptr) {
    taida_val *obj = (taida_val*)async_ptr;
    taida_val status = obj[1];
    char tmp[256];
    if (status == 1) {
        taida_val value = obj[2];
        taida_val val_str = taida_value_to_display_string(value);
        snprintf(tmp, sizeof(tmp), "Async[fulfilled: %s]", (const char*)val_str);
        taida_str_release(val_str);
    } else if (status == 2) {
        taida_val error = obj[3];
        taida_val err_str = taida_value_to_display_string(error);
        snprintf(tmp, sizeof(tmp), "Async[rejected: %s]", (const char*)err_str);
        taida_str_release(err_str);
    } else {
        memcpy(tmp, "Async[pending]", 15); /* 14 chars + '\0' */
    }
    return (taida_val)taida_str_new_copy(tmp);
}

// ── Debug for list ────────────────────────────────────────
taida_val taida_debug_list(taida_val list_ptr) {
    taida_val *list = (taida_val*)list_ptr;
    taida_val len = list[2];
    printf("@[");
    for (taida_val i = 0; i < len; i++) {
        if (i > 0) printf(", ");
        printf("%" PRId64 "", list[4 + i]);
    }
    printf("]\n");
    return 0;
}

// ── JSON Molten Iron runtime ──────────────────────────────
// JSON is an opaque primitive. To use JSON data, it must be cast through
// a schema using JSON[raw, Schema](). The schema is resolved at compile
// time and passed as a descriptor string.
//
// Schema descriptor format:
//   "i" = Int (default 0)
//   "f" = Float (default 0.0)
//   "s" = Str (default "")
//   "b" = Bool (default false)
//   "T{TypeName|field1:desc,field2:desc,...}" = TypeDef (BuchiPack)
//   "L{desc}" = List of elements
//
// The runtime parses JSON, interprets the schema descriptor, and constructs
// a Lax[BuchiPack] with proper FNV-1a hashes.

// --- Minimal JSON parser (recursive descent) ---

// JSON value types
#define JSON_NULL    0
#define JSON_BOOL    1
#define JSON_INT     2
#define JSON_FLOAT   3
#define JSON_STRING  4
#define JSON_ARRAY   5
#define JSON_OBJECT  6

typedef struct {
    int type;
    taida_val int_val;
    double float_val;
    char *str_val;        // for strings (heap-allocated)
    struct json_array *arr;  // for arrays
    struct json_obj *obj;    // for objects
} json_val;

typedef struct json_array {
    json_val *items;
    int count;
    int cap;
} json_array;

typedef struct json_obj_entry {
    char *key;
    json_val value;
} json_obj_entry;

typedef struct json_obj {
    json_obj_entry *entries;
    int count;
    int cap;
} json_obj;

// Forward declarations
static json_val json_parse_value(const char **p);
static void json_skip_ws(const char **p);
static json_val json_default_for_desc(const char *desc);
static taida_val json_apply_schema(json_val *jval, const char **desc);

// FNV-1a hash (matches Rust side)
static uint64_t fnv1a(const char *s, int len) {
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (int i = 0; i < len; i++) {
        hash ^= (unsigned char)s[i];
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

static void json_skip_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\n' || **p == '\r') (*p)++;
}

static char *json_parse_string_raw(const char **p) {
    if (**p != '"') return NULL;
    (*p)++;  // skip opening quote
    // Find end of string (handle escape sequences)
    const char *start = *p;
    int len = 0;
    const char *scan = *p;
    while (*scan && *scan != '"') {
        if (*scan == '\\') { scan++; if (*scan) scan++; }
        else scan++;
        len++;
    }
    // Allocate and copy with escape handling
    char *buf = (char*)TAIDA_MALLOC(len + 1, "json_parse_str");
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
    if (**p == '"') (*p)++;  // skip closing quote
    return buf;
}

static json_val json_parse_string(const char **p) {
    json_val v;
    v.type = JSON_STRING;
    v.str_val = json_parse_string_raw(p);
    v.arr = NULL; v.obj = NULL;
    return v;
}

static json_val json_parse_number(const char **p) {
    json_val v;
    v.str_val = NULL; v.arr = NULL; v.obj = NULL;
    char *end;
    double d = strtod(*p, &end);
    // Check if it's an integer (no decimal point or exponent)
    int is_int = 1;
    const char *scan = *p;
    if (*scan == '-') scan++;
    while (scan < end) {
        if (*scan == '.' || *scan == 'e' || *scan == 'E') { is_int = 0; break; }
        scan++;
    }
    *p = end;
    if (is_int && d >= -9007199254740992.0 && d <= 9007199254740992.0) {
        v.type = JSON_INT;
        v.int_val = (taida_val)d;
        v.float_val = d;
    } else {
        v.type = JSON_FLOAT;
        v.float_val = d;
        v.int_val = (taida_val)d;
    }
    return v;
}

static json_val json_parse_array(const char **p) {
    json_val v;
    v.type = JSON_ARRAY;
    v.str_val = NULL; v.obj = NULL;
    // M-14: TAIDA_MALLOC ensures NULL check + OOM diagnostic.
    v.arr = (json_array*)TAIDA_MALLOC(sizeof(json_array), "json_array");
    v.arr->count = 0;
    v.arr->cap = 4;
    v.arr->items = (json_val*)TAIDA_MALLOC(4 * sizeof(json_val), "json_array_items");
    (*p)++;  // skip '['
    json_skip_ws(p);
    if (**p == ']') { (*p)++; return v; }
    while (**p) {
        json_val item = json_parse_value(p);
        if (v.arr->count >= v.arr->cap) {
            v.arr->cap *= 2;
            json_val *_tmp = (json_val*)realloc(v.arr->items, v.arr->cap * sizeof(json_val));
            if (!_tmp) { fprintf(stderr, "taida: out of memory (json_array)\n"); exit(1); }
            v.arr->items = _tmp;
        }
        v.arr->items[v.arr->count++] = item;
        json_skip_ws(p);
        if (**p == ',') { (*p)++; json_skip_ws(p); }
        else break;
    }
    if (**p == ']') (*p)++;
    return v;
}

static json_val json_parse_object(const char **p) {
    json_val v;
    v.type = JSON_OBJECT;
    v.str_val = NULL; v.arr = NULL;
    // M-14: TAIDA_MALLOC ensures NULL check + OOM diagnostic.
    v.obj = (json_obj*)TAIDA_MALLOC(sizeof(json_obj), "json_obj");
    v.obj->count = 0;
    v.obj->cap = 8;
    v.obj->entries = (json_obj_entry*)TAIDA_MALLOC(8 * sizeof(json_obj_entry), "json_obj_entries");
    (*p)++;  // skip '{'
    json_skip_ws(p);
    if (**p == '}') { (*p)++; return v; }
    while (**p) {
        json_skip_ws(p);
        char *key = json_parse_string_raw(p);
        json_skip_ws(p);
        if (**p == ':') (*p)++;
        json_skip_ws(p);
        json_val val = json_parse_value(p);
        if (v.obj->count >= v.obj->cap) {
            v.obj->cap *= 2;
            json_obj_entry *_tmp = (json_obj_entry*)realloc(v.obj->entries, v.obj->cap * sizeof(json_obj_entry));
            if (!_tmp) { fprintf(stderr, "taida: out of memory (json_object)\n"); exit(1); }
            v.obj->entries = _tmp;
        }
        v.obj->entries[v.obj->count].key = key;
        v.obj->entries[v.obj->count].value = val;
        v.obj->count++;
        json_skip_ws(p);
        if (**p == ',') { (*p)++; json_skip_ws(p); }
        else break;
    }
    if (**p == '}') (*p)++;
    return v;
}

static json_val json_parse_value(const char **p) {
    json_skip_ws(p);
    json_val v;
    v.str_val = NULL; v.arr = NULL; v.obj = NULL;
    if (**p == '"') return json_parse_string(p);
    if (**p == '{') return json_parse_object(p);
    if (**p == '[') return json_parse_array(p);
    if (**p == 't' && strncmp(*p, "true", 4) == 0) {
        *p += 4; v.type = JSON_BOOL; v.int_val = 1; return v;
    }
    if (**p == 'f' && strncmp(*p, "false", 5) == 0) {
        *p += 5; v.type = JSON_BOOL; v.int_val = 0; return v;
    }
    if (**p == 'n' && strncmp(*p, "null", 4) == 0) {
        *p += 4; v.type = JSON_NULL; v.int_val = 0; return v;
    }
    if (**p == '-' || (**p >= '0' && **p <= '9')) return json_parse_number(p);
    // Parse error: return null
    v.type = JSON_NULL; v.int_val = 0;
    return v;
}

// --- JSON object field lookup ---
static json_val *json_obj_get(json_obj *obj, const char *key) {
    if (!obj) return NULL;
    for (int i = 0; i < obj->count; i++) {
        if (strcmp(obj->entries[i].key, key) == 0) {
            return &obj->entries[i].value;
        }
    }
    return NULL;
}

// --- Schema descriptor parsing ---

// Parse a field name from schema descriptor. Returns length consumed.
// Reads until ':' or ',' or '}' or end of string.
static int schema_read_name(const char *desc, char *buf, int buf_size) {
    int i = 0;
    while (desc[i] && desc[i] != ':' && desc[i] != ',' && desc[i] != '}' && desc[i] != '|' && i < buf_size - 1) {
        buf[i] = desc[i];
        i++;
    }
    buf[i] = '\0';
    return i;
}

// Find matching closing brace, accounting for nesting
static int schema_find_closing_brace(const char *desc) {
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
static taida_val json_default_value_for_desc(const char *desc) {
    if (!desc || !*desc) return 0;
    switch (desc[0]) {
        case 'i': return 0;
        case 'f': return _d2l(0.0);
        case 's': {
            char *empty = (char*)TAIDA_MALLOC(1, "json_default_str");
            empty[0] = '\0';
            return (taida_val)empty;
        }
        case 'b': return 0;
        case 'T': {
            // Create default BuchiPack for TypeDef
            json_val null_val;
            null_val.type = JSON_NULL;
            null_val.str_val = NULL; null_val.arr = NULL; null_val.obj = NULL;
            return json_apply_schema(&null_val, &desc);
        }
        case 'L': {
            // Empty list
            return taida_list_new();
        }
        default: return 0;
    }
}

// --- Convert JSON value to typed value using schema ---
// Returns a taida_val (int, float-as-bitcast, string pointer, or BuchiPack pointer)

static taida_val json_to_int(json_val *jv) {
    if (!jv) return 0;
    switch (jv->type) {
        case JSON_INT: return jv->int_val;
        case JSON_FLOAT: return (taida_val)jv->float_val;
        case JSON_BOOL: return jv->int_val;
        case JSON_STRING: {
            if (!jv->str_val) return 0;
            char *end;
            taida_val r = strtol(jv->str_val, &end, 10);
            if (*end != '\0') return 0;
            return r;
        }
        default: return 0;
    }
}

static taida_val json_to_float(json_val *jv) {
    if (!jv) return _d2l(0.0);
    switch (jv->type) {
        case JSON_FLOAT: return _d2l(jv->float_val);
        case JSON_INT: return _d2l((double)jv->int_val);
        case JSON_BOOL: return _d2l(jv->int_val ? 1.0 : 0.0);
        case JSON_STRING: {
            if (!jv->str_val) return _d2l(0.0);
            char *end;
            double r = strtod(jv->str_val, &end);
            if (*end != '\0') return _d2l(0.0);
            return _d2l(r);
        }
        default: return _d2l(0.0);
    }
}

static taida_val json_to_str(json_val *jv) {
    if (!jv) { return (taida_val)taida_str_alloc(0); }
    switch (jv->type) {
        case JSON_STRING: {
            if (!jv->str_val) { return (taida_val)taida_str_alloc(0); }
            return (taida_val)taida_str_new_copy(jv->str_val);
        }
        case JSON_INT: {
            char buf[32]; snprintf(buf, sizeof(buf), "%" PRId64 "", jv->int_val);
            return (taida_val)taida_str_new_copy(buf);
        }
        case JSON_FLOAT: {
            char buf[64]; snprintf(buf, sizeof(buf), "%g", jv->float_val);
            return (taida_val)taida_str_new_copy(buf);
        }
        case JSON_BOOL: {
            return (taida_val)taida_str_new_copy(jv->int_val ? "true" : "false");
        }
        case JSON_NULL: {
            return (taida_val)taida_str_alloc(0);
        }
        default: {
            char *e = (char*)TAIDA_MALLOC(1, "json_default_empty"); e[0]='\0'; return (taida_val)e;
        }
    }
}

static taida_val json_to_bool(json_val *jv) {
    if (!jv) return 0;
    switch (jv->type) {
        case JSON_BOOL: return jv->int_val;
        case JSON_INT: return jv->int_val != 0 ? 1 : 0;
        case JSON_FLOAT: return jv->float_val != 0.0 ? 1 : 0;
        case JSON_STRING: return (jv->str_val && jv->str_val[0]) ? 1 : 0;
        case JSON_NULL: return 0;
        default: return 0;
    }
}

// Apply a schema descriptor to a JSON value, constructing the appropriate native value.
// Returns: taida_val (the native value — int, float-bitcast, string ptr, BuchiPack ptr, or list ptr)
// The desc pointer is advanced past the consumed descriptor.
static taida_val json_apply_schema(json_val *jval, const char **desc) {
    if (!desc || !*desc || !**desc) return 0;
    const char *d = *desc;

    switch (d[0]) {
        case 'i': {
            *desc = d + 1;
            if (!jval || jval->type == JSON_NULL) return 0;
            return json_to_int(jval);
        }
        case 'f': {
            *desc = d + 1;
            if (!jval || jval->type == JSON_NULL) return _d2l(0.0);
            return json_to_float(jval);
        }
        case 's': {
            *desc = d + 1;
            if (!jval || jval->type == JSON_NULL) {
                char *e = (char*)TAIDA_MALLOC(1, "json_null_str"); e[0]='\0'; return (taida_val)e;
            }
            return json_to_str(jval);
        }
        case 'b': {
            *desc = d + 1;
            if (!jval || jval->type == JSON_NULL) return 0;
            return json_to_bool(jval);
        }
        case 'T': {
            // T{TypeName|field1:desc,field2:desc,...}
            // Parse type name
            if (d[1] != '{') { *desc = d + 1; return 0; }
            d += 2;  // skip "T{"
            // Read type name (until '|')
            char type_name[256];
            int tn_len = 0;
            while (*d && *d != '|' && tn_len < 255) { type_name[tn_len++] = *d; d++; }
            type_name[tn_len] = '\0';
            if (*d == '|') d++;

            // Count fields first
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
            taida_val pack = taida_pack_new(field_count + 1);

            // Parse each field and apply schema
            int idx = 0;
            while (*d && *d != '}') {
                // Read field name
                char fname[256];
                int fn_len = 0;
                while (*d && *d != ':' && *d != '}' && fn_len < 255) { fname[fn_len++] = *d; d++; }
                fname[fn_len] = '\0';
                if (*d == ':') d++;

                // Compute FNV-1a hash for field name
                uint64_t hash = fnv1a(fname, fn_len);
                taida_pack_set_hash(pack, idx, (taida_val)hash);

                // Look up this field in JSON object
                json_val *field_jval = NULL;
                if (jval && jval->type == JSON_OBJECT) {
                    field_jval = json_obj_get(jval->obj, fname);
                }

                // Apply sub-schema to field value
                taida_val field_val = json_apply_schema(field_jval, &d);
                taida_pack_set(pack, idx, field_val);
                idx++;

                if (*d == ',') d++;
            }
            if (*d == '}') d++;

            // Add __type field
            uint64_t type_hash = fnv1a("__type", 6);
            taida_pack_set_hash(pack, idx, (taida_val)type_hash);
            char *type_str = (char*)TAIDA_MALLOC(tn_len + 1, "json_type_str");
            memcpy(type_str, type_name, tn_len + 1);
            taida_pack_set(pack, idx, (taida_val)type_str);

            *desc = d;
            return pack;
        }
        case 'L': {
            // L{desc}
            if (d[1] != '{') { *desc = d + 1; return taida_list_new(); }
            d += 2;  // skip "L{"
            // Find closing brace
            int inner_len = schema_find_closing_brace(d);
            // Make a copy of the inner descriptor for repeated use
            char *inner_desc = (char*)TAIDA_MALLOC(inner_len + 1, "json_inner_desc");
            memcpy(inner_desc, d, inner_len);
            inner_desc[inner_len] = '\0';

            taida_val list = taida_list_new();

            if (jval && jval->type == JSON_ARRAY && jval->arr) {
                for (int i = 0; i < jval->arr->count; i++) {
                    const char *elem_desc = inner_desc;
                    taida_val elem = json_apply_schema(&jval->arr->items[i], &elem_desc);
                    list = taida_list_push(list, elem);
                }
            }
            // else: non-array or null -> empty list

            free(inner_desc);
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

// Main entry point: JSON[raw, Schema]() -> Lax[T]
// raw_ptr: C string (the raw JSON)
// schema_ptr: C string (the schema descriptor)
// Returns: Lax BuchiPack (hasValue=true if parse succeeds, false on error)
taida_val taida_json_schema_cast(taida_val raw_ptr, taida_val schema_ptr) {
    const char *raw = (const char *)raw_ptr;
    const char *schema = (const char *)schema_ptr;

    if (!raw || !schema) {
        taida_val def = json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    // Parse JSON
    const char *p = raw;
    json_skip_ws(&p);
    if (!*p) {
        // Empty string -> parse error
        taida_val def = json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    const char *before_parse = p;
    json_val jval = json_parse_value(&p);

    // Detect parse error: if parser didn't advance, or the input wasn't
    // valid JSON (non-null value that didn't consume input)
    if (p == before_parse) {
        // Parser didn't consume anything -> parse error
        taida_val def = json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    // Check if there's trailing non-whitespace (malformed JSON)
    json_skip_ws(&p);
    if (*p != '\0') {
        // Trailing garbage -> parse error
        taida_val def = json_default_value_for_desc(schema);
        return taida_lax_empty(def);
    }

    // Apply schema
    const char *desc = schema;
    taida_val result = json_apply_schema(&jval, &desc);

    // Compute default for same schema
    taida_val def = json_default_value_for_desc(schema);

    return taida_lax_new(result, def);
}

// Legacy JSON functions (kept for backward compat with older tests)
taida_val taida_json_parse(taida_val str_ptr) {
    const char *src = (const char*)str_ptr;
    if (!src) src = "{}";
    size_t len = strlen(src);
    char *buf = (char*)TAIDA_MALLOC(len + 1, "json_parse");
    memcpy(buf, src, len + 1);
    return (taida_val)buf;
}

taida_val taida_json_empty(void) {
    char *buf = (char*)TAIDA_MALLOC(3, "json_empty");
    buf[0] = '{'; buf[1] = '}'; buf[2] = '\0';
    return (taida_val)buf;
}

taida_val taida_json_from_int(taida_val value) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%" PRId64 "", value);
    size_t len = strlen(buf);
    char *result = (char*)TAIDA_MALLOC(len + 1, "json_from_int");
    memcpy(result, buf, len + 1);
    return (taida_val)result;
}

taida_val taida_json_from_str(taida_val str_ptr) {
    const char *src = (const char*)str_ptr;
    if (!src) src = "";
    size_t src_len = strlen(src);
    size_t new_len = src_len + 2;
    char *buf = (char*)TAIDA_MALLOC(new_len + 1, "json_from_str");
    buf[0] = '"';
    memcpy(buf + 1, src, src_len);
    buf[new_len - 1] = '"';
    buf[new_len] = '\0';
    return (taida_val)buf;
}

taida_val taida_json_unmold(taida_val json_ptr) {
    const char *src = (const char*)json_ptr;
    if (!src) { char *e = (char*)TAIDA_MALLOC(1, "json_unmold_empty"); e[0]='\0'; return (taida_val)e; }
    size_t len = strlen(src);
    char *buf = (char*)TAIDA_MALLOC(len + 1, "json_unmold");
    memcpy(buf, src, len + 1);
    return (taida_val)buf;
}

taida_val taida_json_stringify(taida_val json_ptr) {
    return taida_json_unmold(json_ptr);
}

taida_val taida_json_to_str(taida_val json_ptr) {
    return taida_json_unmold(json_ptr);
}

taida_val taida_json_to_int(taida_val json_ptr) {
    const char *data = (const char*)json_ptr;
    if (!data) return 0;
    return atol(data);
}

taida_val taida_json_size(taida_val json_ptr) {
    const char *data = (const char*)json_ptr;
    if (!data) return 0;
    return (taida_val)strlen(data);
}

taida_val taida_json_has(taida_val json_ptr, taida_val key_ptr) {
    const char *json_data = (const char*)json_ptr;
    const char *key_data = (const char*)key_ptr;
    if (!json_data || !key_data) return 0;
    return strstr(json_data, key_data) != NULL ? 1 : 0;
}

taida_val taida_debug_json(taida_val json_ptr) {
    const char *data = (const char*)json_ptr;
    if (data) printf("JSON(%s)\n", data);
    else printf("JSON(null)\n");
    return 0;
}

// ── stdlib math (native) ──────────────────────────────────
// Values may be integer (small values stored directly) or float (f64 bits in taida_val).
// We use a heuristic: the Taida lowering emits ConstFloat for known float literals
// and ConstInt for integer literals. Integer values in math context should be
// converted to double before computation.
//
// Convention: Math functions receive "tagged" longs. If the value was originally
// an integer (from ConstInt), the lowering inserts a taida_int_to_float call.
// For now, we use a bit-pattern heuristic as a fallback.

static double _l2d(taida_val v) { union { taida_val l; double d; } u; u.l = v; return u.d; }
static taida_val _d2l(double v) { union { taida_val l; double d; } u; u.d = v; return u.l; }

// Smart conversion: if the bit pattern represents a "reasonable" f64, use it as-is.
// If it looks like a small integer (-1M..1M), convert from integer.
// This heuristic handles both ConstFloat (bitcast) and ConstInt paths.
static double _to_double(taida_val v) {
    // If v is a small integer (common case for literals like 16, 100, etc.)
    // f64 encoding of small integers has specific bit patterns
    // Quick check: if |v| < 2^20 (about 1M), it's likely a plain integer
    if (v >= -1048576 && v <= 1048576) {
        return (double)v;
    }
    // Otherwise treat as f64 bit pattern
    return _l2d(v);
}

// taida_math_* functions removed (std dissolution)

// Float arithmetic (values stored as f64 bits in taida_val)
taida_val taida_float_add(taida_val a, taida_val b) { return _d2l(_to_double(a) + _to_double(b)); }
taida_val taida_float_sub(taida_val a, taida_val b) { return _d2l(_to_double(a) - _to_double(b)); }
taida_val taida_float_mul(taida_val a, taida_val b) { return _d2l(_to_double(a) * _to_double(b)); }
// taida_float_div removed — use Div[x, y]() mold instead

// ── Field Name Registry (for jsonEncode) ──────────────────
// Global hash -> name table for BuchiPack field name lookup.
// Populated by taida_register_field_name() calls emitted at compile time.

#define FIELD_REGISTRY_CAP 256
// type_tag: 0=unknown, 1=Int, 2=Float, 3=Str, 4=Bool
static struct { taida_val hash; const char *name; int type_tag; } __field_registry[FIELD_REGISTRY_CAP];
static int __field_registry_len = 0;

taida_val taida_register_field_name(taida_val hash, taida_val name_ptr) {
    // Check for duplicate
    for (int i = 0; i < __field_registry_len; i++) {
        if (__field_registry[i].hash == hash) return 0;
    }
    if (__field_registry_len < FIELD_REGISTRY_CAP) {
        __field_registry[__field_registry_len].hash = hash;
        __field_registry[__field_registry_len].name = (const char*)name_ptr;
        __field_registry[__field_registry_len].type_tag = 0;
        __field_registry_len++;
    }
    return 0;
}

// Extended version: register field with type tag
taida_val taida_register_field_type(taida_val hash, taida_val name_ptr, taida_val type_tag) {
    for (int i = 0; i < __field_registry_len; i++) {
        if (__field_registry[i].hash == hash) {
            __field_registry[i].type_tag = (int)type_tag;
            return 0;
        }
    }
    if (__field_registry_len < FIELD_REGISTRY_CAP) {
        __field_registry[__field_registry_len].hash = hash;
        __field_registry[__field_registry_len].name = (const char*)name_ptr;
        __field_registry[__field_registry_len].type_tag = (int)type_tag;
        __field_registry_len++;
    }
    return 0;
}

static const char* taida_lookup_field_name(taida_val hash) {
    for (int i = 0; i < __field_registry_len; i++) {
        if (__field_registry[i].hash == hash) return __field_registry[i].name;
    }
    return NULL;
}

static int taida_lookup_field_type(taida_val hash) {
    for (int i = 0; i < __field_registry_len; i++) {
        if (__field_registry[i].hash == hash) return __field_registry[i].type_tag;
    }
    return 0; // unknown
}

// ── jsonEncode / jsonPretty (native) ──────────────────────
// Recursive serialization of Taida values to JSON string.
// Uses runtime type detection (same heuristics as polymorphic dispatch).

static void json_append(char **buf, size_t *cap, size_t *len, const char *s) {
    size_t slen = strlen(s);
    while (*len + slen + 1 > *cap) {
        *cap *= 2;
        TAIDA_REALLOC(*buf, *cap, "json_stringify");
    }
    memcpy(*buf + *len, s, slen);
    *len += slen;
    (*buf)[*len] = '\0';
}

static void json_append_char(char **buf, size_t *cap, size_t *len, char c) {
    if (*len + 2 > *cap) {
        *cap *= 2;
        TAIDA_REALLOC(*buf, *cap, "json_stringify");
    }
    (*buf)[*len] = c;
    *len += 1;
    (*buf)[*len] = '\0';
}

// Escape a string for JSON output
static void json_append_escaped_str(char **buf, size_t *cap, size_t *len, const char *s) {
    json_append_char(buf, cap, len, '"');
    if (s) {
        for (const char *p = s; *p; p++) {
            switch (*p) {
                case '"':  json_append(buf, cap, len, "\\\""); break;
                case '\\': json_append(buf, cap, len, "\\\\"); break;
                case '\n': json_append(buf, cap, len, "\\n"); break;
                case '\r': json_append(buf, cap, len, "\\r"); break;
                case '\t': json_append(buf, cap, len, "\\t"); break;
                default:   json_append_char(buf, cap, len, *p); break;
            }
        }
    }
    json_append_char(buf, cap, len, '"');
}

// Forward declare: recursive serialization
// type_hint: 0=unknown, 1=Int, 2=Float, 3=Str, 4=Bool
static void json_serialize_typed(char **buf, size_t *cap, size_t *len, taida_val val, int indent, int depth, int type_hint);

// Append indentation (for pretty mode, indent > 0)
static void json_append_indent(char **buf, size_t *cap, size_t *len, int indent, int depth) {
    if (indent <= 0) return;
    json_append_char(buf, cap, len, '\n');
    for (int i = 0; i < indent * depth; i++) {
        json_append_char(buf, cap, len, ' ');
    }
}

// Helper: serialize a BuchiPack's fields as JSON object
// Fields are sorted alphabetically (matching interpreter/JS behavior).
// All __ fields are skipped (__type, __value, __default, __entries, __items).
static void json_serialize_pack_fields(char **buf, size_t *cap, size_t *len, taida_val *pack, taida_val fc, int indent, int depth) {
    // Collect visible fields: (name, val, type_hint, index for stable sort)
    typedef struct { const char *name; taida_val val; int type_hint; } JsonField;
    JsonField fields[100];
    int nfields = 0;
    for (taida_val i = 0; i < fc && nfields < 100; i++) {
        taida_val field_hash = pack[2 + i * 3];
        taida_val field_val = pack[2 + i * 3 + 2];
        const char *fname = taida_lookup_field_name(field_hash);
        if (!fname) continue;
        // Skip all __ fields (__type, __value, __default, __entries, __items)
        if (fname[0] == '_' && fname[1] == '_') {
            continue;
        }
        int ftype = taida_lookup_field_type(field_hash);
        fields[nfields].name = fname;
        fields[nfields].val = field_val;
        fields[nfields].type_hint = ftype;
        nfields++;
    }
    // Sort fields alphabetically by name (insertion sort — nfields is small)
    for (int i = 1; i < nfields; i++) {
        JsonField tmp = fields[i];
        int j = i - 1;
        while (j >= 0 && strcmp(fields[j].name, tmp.name) > 0) {
            fields[j + 1] = fields[j];
            j--;
        }
        fields[j + 1] = tmp;
    }
    // Serialize
    json_append_char(buf, cap, len, '{');
    for (int i = 0; i < nfields; i++) {
        if (i > 0) json_append_char(buf, cap, len, ',');
        if (indent > 0) json_append_indent(buf, cap, len, indent, depth + 1);
        json_append_escaped_str(buf, cap, len, fields[i].name);
        json_append_char(buf, cap, len, ':');
        if (indent > 0) json_append_char(buf, cap, len, ' ');
        json_serialize_typed(buf, cap, len, fields[i].val, indent, depth + 1, fields[i].type_hint);
    }
    if (indent > 0 && nfields > 0) json_append_indent(buf, cap, len, indent, depth);
    json_append_char(buf, cap, len, '}');
}

static void json_serialize_typed(char **buf, size_t *cap, size_t *len, taida_val val, int indent, int depth, int type_hint) {
    // Bool type hint: serialize 0/1 as false/true
    if (type_hint == 4) {
        json_append(buf, cap, len, val ? "true" : "false");
        return;
    }

    // Null/Unit
    if (val == 0) {
        if (type_hint == 3) { // Str
            json_append(buf, cap, len, "\"\"");
        } else {
            json_append(buf, cap, len, "{}");
        }
        return;
    }

    // Integer hints: always serialize as number
    if (type_hint == 1 || type_hint == 2) { // Int or Float
        char num[32];
        snprintf(num, sizeof(num), "%" PRId64 "", val);
        json_append(buf, cap, len, num);
        return;
    }
    // String hint: always treat as string pointer
    if (type_hint == 3) {
        const char *s = (const char*)val;
        json_append_escaped_str(buf, cap, len, s);
        return;
    }

    // No type hint (type_hint == 0): heuristic-based detection
    // Small integer (not a heap pointer)
    if (val > 0 && val < 4096) {
        char num[32];
        snprintf(num, sizeof(num), "%" PRId64 "", val);
        json_append(buf, cap, len, num);
        return;
    }
    if (val < 0) {
        char num[32];
        snprintf(num, sizeof(num), "%" PRId64 "", val);
        json_append(buf, cap, len, num);
        return;
    }

    // Check for HashMap
    if (taida_is_hashmap(val)) {
        taida_val *hm = (taida_val*)val;
        taida_val hm_cap = hm[1];
        json_append_char(buf, cap, len, '{');
        taida_val count = 0;
        for (taida_val i = 0; i < hm_cap; i++) {
            taida_val slot_hash = hm[HM_HEADER + i * 3];
            taida_val slot_key = hm[HM_HEADER + i * 3 + 1];
            if (HM_SLOT_OCCUPIED(slot_hash, slot_key)) {
                if (count > 0) json_append_char(buf, cap, len, ',');
                if (indent > 0) json_append_indent(buf, cap, len, indent, depth + 1);
                const char *key_str = (const char*)slot_key;
                if (!key_str) key_str = "";
                json_append_escaped_str(buf, cap, len, key_str);
                json_append_char(buf, cap, len, ':');
                if (indent > 0) json_append_char(buf, cap, len, ' ');
                json_serialize_typed(buf, cap, len, hm[HM_HEADER + i * 3 + 2], indent, depth + 1, 0);
                count++;
            }
        }
        if (indent > 0 && count > 0) json_append_indent(buf, cap, len, indent, depth);
        json_append_char(buf, cap, len, '}');
        return;
    }

    // Check for Set
    if (taida_is_set(val)) {
        taida_val *list = (taida_val*)val;
        taida_val list_len = list[2];
        json_append_char(buf, cap, len, '[');
        for (taida_val i = 0; i < list_len; i++) {
            if (i > 0) json_append_char(buf, cap, len, ',');
            if (indent > 0) json_append_indent(buf, cap, len, indent, depth + 1);
            json_serialize_typed(buf, cap, len, list[4 + i], indent, depth + 1, 0);
        }
        if (indent > 0 && list_len > 0) json_append_indent(buf, cap, len, indent, depth);
        json_append_char(buf, cap, len, ']');
        return;
    }

    // Check for BuchiPack (monadic types: Result, Lax)
    int fc = taida_monadic_field_count(val);
    if (fc > 0) {
        taida_val *pack = (taida_val*)val;
        // Use actual field_count from pack, not the type ID from monadic_field_count
        taida_val real_fc = pack[1];
        json_serialize_pack_fields(buf, cap, len, pack, real_fc, indent, depth);
        return;
    }

    // Check for List (before general BuchiPack since list detection is more specific)
    if (taida_is_list(val)) {
        taida_val *list = (taida_val*)val;
        taida_val list_len = list[2];
        json_append_char(buf, cap, len, '[');
        for (taida_val i = 0; i < list_len; i++) {
            if (i > 0) json_append_char(buf, cap, len, ',');
            if (indent > 0) json_append_indent(buf, cap, len, indent, depth + 1);
            json_serialize_typed(buf, cap, len, list[4 + i], indent, depth + 1, 0);
        }
        if (indent > 0 && list_len > 0) json_append_indent(buf, cap, len, indent, depth);
        json_append_char(buf, cap, len, ']');
        return;
    }

    // Check for BuchiPack (any size, including user-defined types)
    if (taida_is_buchi_pack(val)) {
        taida_val *obj = (taida_val*)val;
        taida_val obj_fc = obj[1];
        json_serialize_pack_fields(buf, cap, len, obj, obj_fc, indent, depth);
        return;
    }

    // Default: only serialize as string when safely readable.
    size_t str_len = 0;
    if (taida_read_cstr_len_safe((const char*)val, 65536, &str_len)) {
        json_append_escaped_str(buf, cap, len, (const char*)val);
    } else {
        // Not a safe C-string pointer — treat as integer
        char num[32];
        snprintf(num, sizeof(num), "%" PRId64 "", val);
        json_append(buf, cap, len, num);
    }
}

taida_val taida_json_encode(taida_val val) {
    size_t cap = 256;
    size_t len = 0;
    char *buf = (char*)TAIDA_MALLOC(cap, "json_encode");
    buf[0] = '\0';
    json_serialize_typed(&buf, &cap, &len, val, 0, 0, 0);
    return (taida_val)buf;
}

taida_val taida_json_pretty(taida_val val) {
    size_t cap = 256;
    size_t len = 0;
    char *buf = (char*)TAIDA_MALLOC(cap, "json_pretty");
    buf[0] = '\0';
    json_serialize_typed(&buf, &cap, &len, val, 2, 0, 0);
    return (taida_val)buf;
}

// ── stdlib I/O (native) ───────────────────────────────────

taida_val taida_time_now_ms(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_REALTIME, &ts) != 0) {
        return (taida_val)time(NULL) * 1000L;
    }
    int64_t ms = (int64_t)ts.tv_sec * 1000LL + (int64_t)(ts.tv_nsec / 1000000L);
    if (ms > INT64_MAX) return INT64_MAX;
    if (ms < INT64_MIN) return INT64_MIN;
    return (taida_val)ms;
}

static taida_val taida_time_sleep_task(taida_val ms) {
    struct timespec req;
    req.tv_sec = (time_t)(ms / 1000);
    req.tv_nsec = (taida_val)((ms % 1000) * 1000000L);
    while (nanosleep(&req, &req) == -1 && errno == EINTR) {
    }
    return taida_pack_new(0);
}

taida_val taida_time_sleep(taida_val ms) {
    const taida_val max_sleep_ms = 2147483647L;
    if (ms < 0 || ms > max_sleep_ms) {
        char msg[160];
        snprintf(msg, sizeof(msg), "sleep: ms must be in range 0..=%" PRId64 ", got %" PRId64 "", max_sleep_ms, ms);
        return taida_async_err(taida_make_error("RangeError", msg));
    }
    return taida_async_spawn((taida_val)taida_time_sleep_task, ms);
}

// ── SHA-256 prelude function (builtin, no external dependency) ─────────
typedef struct {
    uint32_t state[8];
    uint64_t total_len;
    unsigned char block[64];
    size_t block_len;
} taida_sha256_ctx;

static const uint32_t TAIDA_SHA256_K[64] = {
    0x428a2f98U, 0x71374491U, 0xb5c0fbcfU, 0xe9b5dba5U,
    0x3956c25bU, 0x59f111f1U, 0x923f82a4U, 0xab1c5ed5U,
    0xd807aa98U, 0x12835b01U, 0x243185beU, 0x550c7dc3U,
    0x72be5d74U, 0x80deb1feU, 0x9bdc06a7U, 0xc19bf174U,
    0xe49b69c1U, 0xefbe4786U, 0x0fc19dc6U, 0x240ca1ccU,
    0x2de92c6fU, 0x4a7484aaU, 0x5cb0a9dcU, 0x76f988daU,
    0x983e5152U, 0xa831c66dU, 0xb00327c8U, 0xbf597fc7U,
    0xc6e00bf3U, 0xd5a79147U, 0x06ca6351U, 0x14292967U,
    0x27b70a85U, 0x2e1b2138U, 0x4d2c6dfcU, 0x53380d13U,
    0x650a7354U, 0x766a0abbU, 0x81c2c92eU, 0x92722c85U,
    0xa2bfe8a1U, 0xa81a664bU, 0xc24b8b70U, 0xc76c51a3U,
    0xd192e819U, 0xd6990624U, 0xf40e3585U, 0x106aa070U,
    0x19a4c116U, 0x1e376c08U, 0x2748774cU, 0x34b0bcb5U,
    0x391c0cb3U, 0x4ed8aa4aU, 0x5b9cca4fU, 0x682e6ff3U,
    0x748f82eeU, 0x78a5636fU, 0x84c87814U, 0x8cc70208U,
    0x90befffaU, 0xa4506cebU, 0xbef9a3f7U, 0xc67178f2U
};

static uint32_t taida_sha256_rotr(uint32_t x, uint32_t n) {
    return (x >> n) | (x << (32 - n));
}

static void taida_sha256_init(taida_sha256_ctx *ctx) {
    ctx->state[0] = 0x6a09e667U;
    ctx->state[1] = 0xbb67ae85U;
    ctx->state[2] = 0x3c6ef372U;
    ctx->state[3] = 0xa54ff53aU;
    ctx->state[4] = 0x510e527fU;
    ctx->state[5] = 0x9b05688cU;
    ctx->state[6] = 0x1f83d9abU;
    ctx->state[7] = 0x5be0cd19U;
    ctx->total_len = 0;
    ctx->block_len = 0;
}

static void taida_sha256_transform(taida_sha256_ctx *ctx, const unsigned char block[64]) {
    uint32_t w[64];
    for (int i = 0; i < 16; i++) {
        int j = i * 4;
        w[i] = ((uint32_t)block[j] << 24) |
               ((uint32_t)block[j + 1] << 16) |
               ((uint32_t)block[j + 2] << 8) |
               (uint32_t)block[j + 3];
    }
    for (int i = 16; i < 64; i++) {
        uint32_t s0 = taida_sha256_rotr(w[i - 15], 7) ^ taida_sha256_rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        uint32_t s1 = taida_sha256_rotr(w[i - 2], 17) ^ taida_sha256_rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }

    uint32_t a = ctx->state[0];
    uint32_t b = ctx->state[1];
    uint32_t c = ctx->state[2];
    uint32_t d = ctx->state[3];
    uint32_t e = ctx->state[4];
    uint32_t f = ctx->state[5];
    uint32_t g = ctx->state[6];
    uint32_t h = ctx->state[7];

    for (int i = 0; i < 64; i++) {
        uint32_t s1 = taida_sha256_rotr(e, 6) ^ taida_sha256_rotr(e, 11) ^ taida_sha256_rotr(e, 25);
        uint32_t ch = (e & f) ^ ((~e) & g);
        uint32_t temp1 = h + s1 + ch + TAIDA_SHA256_K[i] + w[i];
        uint32_t s0 = taida_sha256_rotr(a, 2) ^ taida_sha256_rotr(a, 13) ^ taida_sha256_rotr(a, 22);
        uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
        uint32_t temp2 = s0 + maj;

        h = g;
        g = f;
        f = e;
        e = d + temp1;
        d = c;
        c = b;
        b = a;
        a = temp1 + temp2;
    }

    ctx->state[0] += a;
    ctx->state[1] += b;
    ctx->state[2] += c;
    ctx->state[3] += d;
    ctx->state[4] += e;
    ctx->state[5] += f;
    ctx->state[6] += g;
    ctx->state[7] += h;
}

static void taida_sha256_update(taida_sha256_ctx *ctx, const unsigned char *data, size_t len) {
    if (!data || len == 0) return;
    ctx->total_len += (uint64_t)len;
    size_t pos = 0;
    while (pos < len) {
        size_t need = 64 - ctx->block_len;
        size_t take = (len - pos < need) ? (len - pos) : need;
        memcpy(ctx->block + ctx->block_len, data + pos, take);
        ctx->block_len += take;
        pos += take;
        if (ctx->block_len == 64) {
            taida_sha256_transform(ctx, ctx->block);
            ctx->block_len = 0;
        }
    }
}

static void taida_sha256_final(taida_sha256_ctx *ctx, unsigned char out[32]) {
    uint64_t bit_len = ctx->total_len * 8ULL;

    ctx->block[ctx->block_len++] = 0x80;
    if (ctx->block_len > 56) {
        while (ctx->block_len < 64) ctx->block[ctx->block_len++] = 0;
        taida_sha256_transform(ctx, ctx->block);
        ctx->block_len = 0;
    }
    while (ctx->block_len < 56) ctx->block[ctx->block_len++] = 0;

    for (int i = 0; i < 8; i++) {
        ctx->block[56 + i] = (unsigned char)(bit_len >> (56 - i * 8));
    }
    taida_sha256_transform(ctx, ctx->block);

    for (int i = 0; i < 8; i++) {
        out[i * 4] = (unsigned char)(ctx->state[i] >> 24);
        out[i * 4 + 1] = (unsigned char)(ctx->state[i] >> 16);
        out[i * 4 + 2] = (unsigned char)(ctx->state[i] >> 8);
        out[i * 4 + 3] = (unsigned char)(ctx->state[i]);
    }
}

static taida_val taida_sha256_hex_from_bytes(const unsigned char *data, size_t len) {
    taida_sha256_ctx ctx;
    unsigned char digest[32];
    static const char hex[] = "0123456789abcdef";
    char *out = taida_str_alloc(64);
    taida_sha256_init(&ctx);
    taida_sha256_update(&ctx, data, len);
    taida_sha256_final(&ctx, digest);
    for (int i = 0; i < 32; i++) {
        out[i * 2] = hex[(digest[i] >> 4) & 0x0f];
        out[i * 2 + 1] = hex[digest[i] & 0x0f];
    }
    return (taida_val)out;
}

taida_val taida_sha256(taida_val value) {
    if (TAIDA_IS_BYTES(value)) {
        taida_val len = taida_bytes_len(value);
        if (len <= 0) return taida_sha256_hex_from_bytes(NULL, 0);
        // M-08: Cap Bytes length to 256MB to prevent OOM from huge positive len.
        if (len > (taida_val)(256 * 1024 * 1024)) {
            return taida_sha256_hex_from_bytes(NULL, 0);
        }
        taida_val *bytes = (taida_val*)value;
        unsigned char *raw = (unsigned char*)TAIDA_MALLOC((size_t)len, "sha256_bytes");
        for (taida_val i = 0; i < len; i++) raw[i] = (unsigned char)bytes[2 + i];
        taida_val out = taida_sha256_hex_from_bytes(raw, (size_t)len);
        free(raw);
        return out;
    }

    taida_val display = taida_value_to_display_string(value);
    const char *s = (const char*)display;
    size_t slen = 0;
    if (!taida_read_cstr_len_safe(s, 1 << 20, &slen)) {
        taida_str_release(display);
        return taida_sha256_hex_from_bytes(NULL, 0);
    }
    taida_val out = taida_sha256_hex_from_bytes((const unsigned char*)s, slen);
    taida_str_release(display);
    return out;
}

taida_val taida_io_stdin(taida_val prompt_ptr) {
    // Print prompt if provided
    const char *prompt = (const char*)prompt_ptr;
    if (prompt && prompt[0] != '\0') {
        printf("%s", prompt);
        fflush(stdout);
    }
    // Read a line from stdin
    char line[4096];
    if (fgets(line, sizeof(line), stdin) == NULL) {
        // EOF or error — return empty string
        return (taida_val)taida_str_alloc(0);
    }
    // Strip trailing newline
    size_t slen = strlen(line);
    if (slen > 0 && line[slen - 1] == '\n') {
        line[slen - 1] = '\0';
        slen--;
        if (slen > 0 && line[slen - 1] == '\r') {
            line[slen - 1] = '\0';
            slen--;
        }
    }
    char *r = taida_str_alloc(slen);
    memcpy(r, line, slen);
    return (taida_val)r;
}

// C12-5 (FB-18): stdout / stderr return the UTF-8 byte length of the payload
// as Int so that `n <= stdout("hi")` binds `n = 2`. The trailing newline added
// for display is NOT counted — callers see the payload size they supplied.
// Parity: interpreter and JS runtime use the same semantics (content length
// via Rust `String::len()` / JS UTF-8 byte length, newline excluded).
taida_val taida_io_stdout(taida_val val_ptr) {
    // For now, treat val as a string pointer
    const char *s = (const char*)val_ptr;
    if (s) {
        printf("%s\n", s);
        return (taida_val)strlen(s);
    }
    return 0;
}

// B11-2a: Type-tagged stdout — resolves Bool display parity (FB-3).
// When the compiler knows the argument type at emit time, it passes
// a compile-time tag so that Bool prints "true"/"false" instead of "1"/"0".
// Only Bool needs special handling; all other types (Str, Int, Float,
// Pack, List, etc.) are correctly handled by taida_polymorphic_to_string.
// C12-5: returns bytes written (Int), see taida_io_stdout above.
taida_val taida_io_stdout_with_tag(taida_val val, taida_val tag) {
    const char *s = NULL;
    char bool_buf[6];
    size_t bytes = 0;
    if ((int)tag == TAIDA_TAG_BOOL) {
        s = val ? "true" : "false";
        bytes = strlen(s);
        memcpy(bool_buf, s, bytes);
        bool_buf[bytes] = '\0';
        printf("%s\n", bool_buf);
        return (taida_val)bytes;
    }
    taida_val str = taida_polymorphic_to_string(val);
    s = (const char*)str;
    if (s) {
        printf("%s\n", s);
        return (taida_val)strlen(s);
    }
    return 0;
}

// C12-5: bytes written as Int.
taida_val taida_io_stderr(taida_val val_ptr) {
    const char *s = (const char*)val_ptr;
    if (s) {
        fprintf(stderr, "%s\n", s);
        return (taida_val)strlen(s);
    }
    return 0;
}

// B11-2a: Type-tagged stderr — mirrors taida_io_stdout_with_tag for stderr.
// C12-5: returns bytes written (Int).
taida_val taida_io_stderr_with_tag(taida_val val, taida_val tag) {
    const char *s = NULL;
    if ((int)tag == TAIDA_TAG_BOOL) {
        s = val ? "true" : "false";
        fprintf(stderr, "%s\n", s);
        return (taida_val)strlen(s);
    }
    taida_val str = taida_polymorphic_to_string(val);
    s = (const char*)str;
    if (s) {
        fprintf(stderr, "%s\n", s);
        return (taida_val)strlen(s);
    }
    return 0;
}

