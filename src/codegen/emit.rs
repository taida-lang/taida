use cranelift_codegen::ir::types;
/// Taida IR → CLIF IR 変換（Cranelift Emission）
use cranelift_codegen::ir::{self as clif, AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::HashMap;
use target_lexicon::Triple;

use super::ir::*;

fn runtime_func_signature(name: &str) -> (Vec<clif::Type>, Vec<clif::Type>) {
    match name {
        "taida_debug_int" => (vec![types::I64], vec![types::I64]),
        "taida_debug_float" => (vec![types::F64], vec![types::I64]),
        "taida_debug_bool" => (vec![types::I64], vec![types::I64]),
        "taida_debug_str" => (vec![types::I64], vec![types::I64]),
        "taida_gorilla" => (vec![], vec![]),
        "taida_int_add" | "taida_int_sub" | "taida_int_mul" => {
            (vec![types::I64, types::I64], vec![types::I64])
        }
        "taida_div_mold" | "taida_mod_mold" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_bit_and" | "taida_bit_or" | "taida_bit_xor" => {
            (vec![types::I64, types::I64], vec![types::I64])
        }
        "taida_shift_l" | "taida_shift_r" | "taida_shift_ru" => {
            (vec![types::I64, types::I64], vec![types::I64])
        }
        "taida_to_radix" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_bit_not" => (vec![types::I64], vec![types::I64]),
        "taida_int_neg" => (vec![types::I64], vec![types::I64]),
        "taida_float_neg" => (vec![types::F64], vec![types::F64]),
        "taida_int_eq" | "taida_int_neq" | "taida_str_eq" | "taida_str_neq" | "taida_poly_eq"
        | "taida_poly_neq" | "taida_int_lt" | "taida_int_gt" | "taida_int_gte" => {
            (vec![types::I64, types::I64], vec![types::I64])
        }
        "taida_bool_and" | "taida_bool_or" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_bool_not" => (vec![types::I64], vec![types::I64]),
        // ぶちパック操作
        "taida_pack_new" => (vec![types::I64], vec![types::I64]), // (field_count) -> ptr
        "taida_pack_set" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (ptr, index, value)
        "taida_pack_get" => (vec![types::I64, types::I64], vec![types::I64]), // (ptr, field_hash) -> value
        "taida_pack_get_idx" => (vec![types::I64, types::I64], vec![types::I64]), // (ptr, index) -> value
        "taida_pack_set_hash" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (ptr, index, hash)
        "taida_pack_set_tag" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (ptr, index, tag)
        // BuchiPack field call (field get + invoke)
        "taida_pack_call_field0" => (vec![types::I64, types::I64], vec![types::I64]), // (pack, hash) -> result
        "taida_pack_call_field1" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (pack, hash, arg0)
        "taida_pack_call_field2" => (
            vec![types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ), // (pack, hash, arg0, arg1)
        "taida_pack_call_field3" => (
            vec![types::I64, types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ), // (pack, hash, arg0, arg1, arg2)
        // クロージャ操作
        "taida_closure_new" => (vec![types::I64, types::I64], vec![types::I64]), // (fn_ptr, env_ptr) -> closure_ptr
        "taida_closure_get_fn" => (vec![types::I64], vec![types::I64]), // (closure_ptr) -> fn_ptr
        "taida_closure_get_env" => (vec![types::I64], vec![types::I64]), // (closure_ptr) -> env_ptr
        "taida_is_closure_value" => (vec![types::I64], vec![types::I64]), // (callable) -> bool
        // グローバル変数テーブル
        "taida_global_set" => (vec![types::I64, types::I64], vec![]), // (name_hash, value) -> void
        "taida_global_get" => (vec![types::I64], vec![types::I64]),   // (name_hash) -> value
        // リスト操作
        "taida_list_new" => (vec![], vec![types::I64]),
        "taida_list_set_elem_tag" => (vec![types::I64, types::I64], vec![]), // (list, tag) -> void
        "taida_list_push" => (vec![types::I64, types::I64], vec![types::I64]), // (list, item) -> list
        "taida_list_length" => (vec![types::I64], vec![types::I64]),
        "taida_list_get" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_first" => (vec![types::I64], vec![types::I64]),
        "taida_list_last" => (vec![types::I64], vec![types::I64]),
        "taida_list_sum" => (vec![types::I64], vec![types::I64]),
        "taida_list_reverse" => (vec![types::I64], vec![types::I64]),
        "taida_list_contains" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_is_empty" => (vec![types::I64], vec![types::I64]),
        "taida_list_map" => (vec![types::I64, types::I64], vec![types::I64]), // (list, fn_ptr) -> list
        "taida_list_filter" => (vec![types::I64, types::I64], vec![types::I64]),
        // 型変換
        "taida_int_abs" => (vec![types::I64], vec![types::I64]),
        "taida_int_to_str" => (vec![types::I64], vec![types::I64]),
        "taida_int_to_float" => (vec![types::I64], vec![types::I64]),
        "taida_int_clamp" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        // String operations
        "taida_str_concat" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_length" => (vec![types::I64], vec![types::I64]),
        "taida_str_char_at" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_slice" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_str_index_of" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_to_upper" => (vec![types::I64], vec![types::I64]),
        "taida_str_to_lower" => (vec![types::I64], vec![types::I64]),
        "taida_str_trim" => (vec![types::I64], vec![types::I64]),
        "taida_str_split" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_replace" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_str_to_int" => (vec![types::I64], vec![types::I64]),
        "taida_str_repeat" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_reverse" => (vec![types::I64], vec![types::I64]),
        "taida_str_trim_start" => (vec![types::I64], vec![types::I64]),
        "taida_str_trim_end" => (vec![types::I64], vec![types::I64]),
        "taida_str_replace_first" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_str_pad" => (
            vec![types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ),
        "taida_str_from_int" => (vec![types::I64], vec![types::I64]),
        "taida_str_from_float" => (vec![types::F64], vec![types::I64]),
        "taida_str_from_bool" => (vec![types::I64], vec![types::I64]),
        // Str state check methods
        "taida_str_contains" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_starts_with" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_ends_with" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_last_index_of" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_str_get" => (vec![types::I64, types::I64], vec![types::I64]),
        // Str refcount
        "taida_str_retain" => (vec![types::I64], vec![]), // (ptr) -> void
        // Float operations
        "taida_float_floor" => (vec![types::F64], vec![types::F64]),
        "taida_float_ceil" => (vec![types::F64], vec![types::F64]),
        "taida_float_round" => (vec![types::F64], vec![types::F64]),
        "taida_float_abs" => (vec![types::F64], vec![types::F64]),
        "taida_float_to_int" => (vec![types::F64], vec![types::I64]),
        "taida_float_to_str" => (vec![types::F64], vec![types::I64]),
        "taida_float_to_fixed" => (vec![types::F64, types::I64], vec![types::I64]),
        "taida_float_clamp" => (vec![types::F64, types::F64, types::F64], vec![types::F64]),
        // Num state check methods (Int)
        "taida_int_is_positive" => (vec![types::I64], vec![types::I64]),
        "taida_int_is_negative" => (vec![types::I64], vec![types::I64]),
        "taida_int_is_zero" => (vec![types::I64], vec![types::I64]),
        // Num state check methods (Float)
        "taida_float_is_nan" => (vec![types::F64], vec![types::I64]),
        "taida_float_is_infinite" => (vec![types::F64], vec![types::I64]),
        "taida_float_is_finite_check" => (vec![types::F64], vec![types::I64]),
        "taida_float_is_positive" => (vec![types::F64], vec![types::I64]),
        "taida_float_is_negative" => (vec![types::F64], vec![types::I64]),
        "taida_float_is_zero" => (vec![types::F64], vec![types::I64]),
        // Bool operations
        "taida_bool_to_str" => (vec![types::I64], vec![types::I64]),
        "taida_bool_to_int" => (vec![types::I64], vec![types::I64]),
        // Additional List operations
        "taida_list_index_of" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_last_index_of" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_any" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_all" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_none" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_concat" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_join" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_sort" => (vec![types::I64], vec![types::I64]),
        "taida_list_unique" => (vec![types::I64], vec![types::I64]),
        "taida_list_flatten" => (vec![types::I64], vec![types::I64]),
        "taida_list_max" => (vec![types::I64], vec![types::I64]),
        "taida_list_min" => (vec![types::I64], vec![types::I64]),
        // List mold operations
        "taida_list_append" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_prepend" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_sort_desc" => (vec![types::I64], vec![types::I64]),
        "taida_list_find" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_find_index" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_count" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_fold" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (list, init, fn_ptr) -> value
        "taida_list_foldr" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (list, init, fn_ptr) -> value
        "taida_list_take" => (vec![types::I64, types::I64], vec![types::I64]), // (list, n) -> list
        "taida_list_take_while" => (vec![types::I64, types::I64], vec![types::I64]), // (list, fn_ptr) -> list
        "taida_list_drop" => (vec![types::I64, types::I64], vec![types::I64]), // (list, n) -> list
        "taida_list_drop_while" => (vec![types::I64, types::I64], vec![types::I64]), // (list, fn_ptr) -> list
        "taida_list_zip" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_list_enumerate" => (vec![types::I64], vec![types::I64]),
        // HashMap operations
        "taida_hashmap_new" => (vec![], vec![types::I64]),
        "taida_hashmap_set_value_tag" => (vec![types::I64, types::I64], vec![]),
        "taida_hashmap_set" => (
            vec![types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ),
        "taida_hashmap_set_immut" => (
            vec![types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ),
        "taida_hashmap_get" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_hashmap_get_lax" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_hashmap_has" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_hashmap_remove" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_hashmap_remove_immut" => {
            (vec![types::I64, types::I64, types::I64], vec![types::I64])
        }
        "taida_hashmap_keys" => (vec![types::I64], vec![types::I64]),
        "taida_hashmap_values" => (vec![types::I64], vec![types::I64]),
        "taida_hashmap_length" => (vec![types::I64], vec![types::I64]),
        "taida_hashmap_is_empty" => (vec![types::I64], vec![types::I64]),
        "taida_hashmap_entries" => (vec![types::I64], vec![types::I64]),
        "taida_hashmap_merge" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_hashmap_to_string" => (vec![types::I64], vec![types::I64]),
        "taida_str_hash" => (vec![types::I64], vec![types::I64]),
        "taida_value_hash" => (vec![types::I64], vec![types::I64]),
        // Set operations
        "taida_set_new" => (vec![], vec![types::I64]),
        "taida_set_set_elem_tag" => (vec![types::I64, types::I64], vec![]), // NO-2: (set, tag) -> void
        "taida_set_from_list" => (vec![types::I64], vec![types::I64]),
        "taida_set_add" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_set_remove" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_set_has" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_set_size" => (vec![types::I64], vec![types::I64]),
        "taida_set_is_empty" => (vec![types::I64], vec![types::I64]),
        "taida_set_to_list" => (vec![types::I64], vec![types::I64]),
        "taida_set_union" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_set_intersect" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_set_diff" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_set_to_string" => (vec![types::I64], vec![types::I64]),
        // Polymorphic collection methods
        "taida_collection_get" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_collection_has" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_collection_remove" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_collection_size" => (vec![types::I64], vec![types::I64]),
        "taida_polymorphic_length" => (vec![types::I64], vec![types::I64]),
        // Error ceiling
        "taida_error_ceiling_push" => (vec![], vec![types::I64]),
        "taida_error_ceiling_pop" => (vec![], vec![]),
        "taida_throw" => (vec![types::I64], vec![types::I64]),
        "taida_error_setjmp" => (vec![types::I64], vec![types::I64]),
        "taida_error_try_call" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_error_get_value" => (vec![types::I64], vec![types::I64]),
        "taida_error_try_get_result" => (vec![types::I64], vec![types::I64]),
        // Result[T, P] (v0.8.0 redesign — predicate support) — Optional abolished
        "taida_result_create" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_result_is_ok" => (vec![types::I64], vec![types::I64]),
        "taida_result_is_error" => (vec![types::I64], vec![types::I64]),
        "taida_result_get_or_default" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_result_get_or_throw" => (vec![types::I64], vec![types::I64]),
        "taida_result_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_result_flat_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_result_map_error" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_result_to_string" => (vec![types::I64], vec![types::I64]),
        // Lax methods (map, flatMap, toString)
        "taida_lax_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_lax_flat_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_lax_to_string" => (vec![types::I64], vec![types::I64]),
        // Polymorphic monadic dispatch
        "taida_polymorphic_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_polymorphic_contains" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_polymorphic_index_of" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_polymorphic_last_index_of" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_polymorphic_get_or_default" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_polymorphic_has_value" => (vec![types::I64], vec![types::I64]),
        "taida_polymorphic_is_empty" => (vec![types::I64], vec![types::I64]),
        "taida_polymorphic_to_string" => (vec![types::I64], vec![types::I64]),
        "taida_monadic_flat_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_monadic_get_or_throw" => (vec![types::I64], vec![types::I64]),
        // Async methods
        "taida_async_map" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_async_get_or_default" => (vec![types::I64, types::I64], vec![types::I64]),
        // Debug
        "taida_debug_list" => (vec![types::I64], vec![types::I64]),
        // 参照カウント
        "taida_retain" => (vec![types::I64], vec![types::I64]),
        "taida_release" => (vec![types::I64], vec![types::I64]),
        // Async
        "taida_async_ok" => (vec![types::I64], vec![types::I64]),
        "taida_async_ok_tagged" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_async_err" => (vec![types::I64], vec![types::I64]),
        "taida_async_set_value_tag" => (vec![types::I64, types::I64], vec![]), // NO-3: (async, tag) -> void
        "taida_async_unmold" => (vec![types::I64], vec![types::I64]),
        "taida_async_is_pending" => (vec![types::I64], vec![types::I64]),
        "taida_async_is_fulfilled" => (vec![types::I64], vec![types::I64]),
        "taida_async_is_rejected" => (vec![types::I64], vec![types::I64]),
        "taida_async_get_value" => (vec![types::I64], vec![types::I64]),
        "taida_async_get_error" => (vec![types::I64], vec![types::I64]),
        "taida_async_all" => (vec![types::I64], vec![types::I64]),
        "taida_async_race" => (vec![types::I64], vec![types::I64]),
        "taida_async_spawn" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_async_cancel" => (vec![types::I64], vec![types::I64]),
        // Lax
        "taida_lax_new" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_lax_empty" => (vec![types::I64], vec![types::I64]),
        "taida_lax_has_value" => (vec![types::I64], vec![types::I64]),
        "taida_lax_get_or_default" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_lax_unmold" => (vec![types::I64], vec![types::I64]),
        "taida_lax_is_empty" => (vec![types::I64], vec![types::I64]),
        "taida_generic_unmold" => (vec![types::I64], vec![types::I64]),
        // Gorillax / RelaxedGorillax
        "taida_gorillax_new" => (vec![types::I64], vec![types::I64]),
        "taida_molten_new" => (vec![], vec![types::I64]),
        "taida_stub_new" => (vec![types::I64], vec![types::I64]),
        "taida_todo_new" => (
            vec![types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ),
        "taida_cage_apply" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_gorillax_unmold" => (vec![types::I64], vec![types::I64]),
        "taida_gorillax_relax" => (vec![types::I64], vec![types::I64]),
        "taida_gorillax_to_string" => (vec![types::I64], vec![types::I64]),
        "taida_relaxed_gorillax_unmold" => (vec![types::I64], vec![types::I64]),
        "taida_relaxed_gorillax_to_string" => (vec![types::I64], vec![types::I64]),
        // Type conversion molds (Str/Int/Float/Bool) — all return Lax (I64 ptr)
        "taida_str_mold_int" => (vec![types::I64], vec![types::I64]),
        "taida_str_mold_float" => (vec![types::F64], vec![types::I64]),
        "taida_str_mold_bool" => (vec![types::I64], vec![types::I64]),
        "taida_str_mold_str" => (vec![types::I64], vec![types::I64]),
        "taida_int_mold_int" => (vec![types::I64], vec![types::I64]),
        "taida_int_mold_float" => (vec![types::F64], vec![types::I64]),
        "taida_int_mold_str" => (vec![types::I64], vec![types::I64]),
        "taida_int_mold_auto" => (vec![types::I64], vec![types::I64]),
        "taida_int_mold_str_base" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_int_mold_bool" => (vec![types::I64], vec![types::I64]),
        "taida_float_mold_int" => (vec![types::I64], vec![types::I64]),
        "taida_float_mold_float" => (vec![types::F64], vec![types::I64]),
        "taida_float_mold_str" => (vec![types::I64], vec![types::I64]),
        "taida_float_mold_bool" => (vec![types::I64], vec![types::I64]),
        "taida_bool_mold_int" => (vec![types::I64], vec![types::I64]),
        "taida_bool_mold_float" => (vec![types::F64], vec![types::I64]),
        "taida_bool_mold_str" => (vec![types::I64], vec![types::I64]),
        "taida_bool_mold_bool" => (vec![types::I64], vec![types::I64]),
        "taida_uint8_mold" => (vec![types::I64], vec![types::I64]),
        "taida_uint8_mold_float" => (vec![types::F64], vec![types::I64]),
        "taida_u16be_mold" | "taida_u16le_mold" | "taida_u32be_mold" | "taida_u32le_mold" => {
            (vec![types::I64], vec![types::I64])
        }
        "taida_u16be_decode_mold"
        | "taida_u16le_decode_mold"
        | "taida_u32be_decode_mold"
        | "taida_u32le_decode_mold" => (vec![types::I64], vec![types::I64]),
        "taida_bytes_mold" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_bytes_set" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        "taida_bytes_to_list" => (vec![types::I64], vec![types::I64]),
        "taida_bytes_cursor_new" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_bytes_cursor_remaining" => (vec![types::I64], vec![types::I64]),
        "taida_bytes_cursor_take" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_bytes_cursor_u8" => (vec![types::I64], vec![types::I64]),
        "taida_char_mold_int" => (vec![types::I64], vec![types::I64]),
        "taida_char_mold_str" => (vec![types::I64], vec![types::I64]),
        "taida_codepoint_mold_str" => (vec![types::I64], vec![types::I64]),
        "taida_utf8_encode_mold" => (vec![types::I64], vec![types::I64]),
        "taida_utf8_decode_mold" => (vec![types::I64], vec![types::I64]),
        "taida_slice_mold" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        // JSON — Molten Iron (schema-based casting)
        "taida_json_schema_cast" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_json_parse" => (vec![types::I64], vec![types::I64]),
        "taida_json_from_int" => (vec![types::I64], vec![types::I64]),
        "taida_json_from_str" => (vec![types::I64], vec![types::I64]),
        "taida_json_unmold" => (vec![types::I64], vec![types::I64]),
        "taida_json_stringify" => (vec![types::I64], vec![types::I64]),
        "taida_json_to_str" => (vec![types::I64], vec![types::I64]),
        "taida_json_to_int" => (vec![types::I64], vec![types::I64]),
        "taida_json_size" => (vec![types::I64], vec![types::I64]),
        "taida_json_has" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_json_empty" => (vec![], vec![types::I64]),
        "taida_debug_json" => (vec![types::I64], vec![types::I64]),
        // stdlib math (all operate on F64 encoded as I64 via bitcast)
        // These take I64 (boxed float) and return I64 (boxed float)
        "taida_math_sqrt" => (vec![types::I64], vec![types::I64]),
        "taida_math_pow" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_math_abs" => (vec![types::I64], vec![types::I64]),
        "taida_math_sin" => (vec![types::I64], vec![types::I64]),
        "taida_math_cos" => (vec![types::I64], vec![types::I64]),
        "taida_math_tan" => (vec![types::I64], vec![types::I64]),
        "taida_math_asin" => (vec![types::I64], vec![types::I64]),
        "taida_math_acos" => (vec![types::I64], vec![types::I64]),
        "taida_math_atan" => (vec![types::I64], vec![types::I64]),
        "taida_math_atan2" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_math_log" => (vec![types::I64], vec![types::I64]),
        "taida_math_log10" => (vec![types::I64], vec![types::I64]),
        "taida_math_exp" => (vec![types::I64], vec![types::I64]),
        "taida_math_floor" => (vec![types::I64], vec![types::I64]),
        "taida_math_ceil" => (vec![types::I64], vec![types::I64]),
        "taida_math_round" => (vec![types::I64], vec![types::I64]),
        "taida_math_truncate" => (vec![types::I64], vec![types::I64]),
        "taida_math_max" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_math_min" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_math_clamp" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        // Float arithmetic
        "taida_float_add" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_float_sub" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_float_mul" => (vec![types::I64, types::I64], vec![types::I64]),
        // taida_float_div removed — use Div[x, y]() mold
        // stdlib I/O
        "taida_io_stdout" => (vec![types::I64], vec![types::I64]),
        "taida_io_stderr" => (vec![types::I64], vec![types::I64]),
        "taida_io_stdin" => (vec![types::I64], vec![types::I64]),
        "taida_sha256" => (vec![types::I64], vec![types::I64]),
        // time prelude
        "taida_time_now_ms" => (vec![], vec![types::I64]),
        "taida_time_sleep" => (vec![types::I64], vec![types::I64]),
        // jsonEncode / jsonPretty
        "taida_json_encode" => (vec![types::I64], vec![types::I64]),
        "taida_json_pretty" => (vec![types::I64], vec![types::I64]),
        // Field name registry
        "taida_register_field_name" => (vec![types::I64, types::I64], vec![types::I64]),
        "taida_register_field_type" => (vec![types::I64, types::I64, types::I64], vec![types::I64]),
        // taida-lang/os package — input molds
        "taida_os_read" => (vec![types::I64], vec![types::I64]), // (path) -> Lax[Str]
        "taida_os_read_bytes" => (vec![types::I64], vec![types::I64]), // (path) -> Lax[Bytes]
        "taida_os_list_dir" => (vec![types::I64], vec![types::I64]), // (path) -> Lax[@[Str]]
        "taida_os_stat" => (vec![types::I64], vec![types::I64]), // (path) -> Lax[@(size,modified,isDir)]
        "taida_os_exists" => (vec![types::I64], vec![types::I64]), // (path) -> Bool
        "taida_os_env_var" => (vec![types::I64], vec![types::I64]), // (name) -> Lax[Str]
        // taida-lang/os package — side-effect functions
        "taida_os_write_file" => (vec![types::I64, types::I64], vec![types::I64]), // (path, content) -> Result
        "taida_os_write_bytes" => (vec![types::I64, types::I64], vec![types::I64]), // (path, content) -> Result
        "taida_os_append_file" => (vec![types::I64, types::I64], vec![types::I64]), // (path, content) -> Result
        "taida_os_remove" => (vec![types::I64], vec![types::I64]), // (path) -> Result
        "taida_os_create_dir" => (vec![types::I64], vec![types::I64]), // (path) -> Result
        "taida_os_rename" => (vec![types::I64, types::I64], vec![types::I64]), // (from, to) -> Result
        "taida_os_run" => (vec![types::I64, types::I64], vec![types::I64]), // (program, args_list) -> Gorillax
        "taida_os_exec_shell" => (vec![types::I64], vec![types::I64]),      // (command) -> Gorillax
        // taida-lang/os package — query function
        "taida_os_all_env" => (vec![], vec![types::I64]), // () -> HashMap[Str, Str]
        "taida_os_argv" => (vec![], vec![types::I64]),    // () -> @[Str]
        // taida-lang/os package — Phase 2: async APIs
        "taida_os_read_async" => (vec![types::I64], vec![types::I64]), // (path) -> Async[Lax[Str]]
        "taida_os_http_get" => (vec![types::I64], vec![types::I64]), // (url) -> Async[Lax[@(...)]]
        "taida_os_http_post" => (vec![types::I64, types::I64], vec![types::I64]), // (url, body) -> Async[Lax[@(...)]]
        "taida_os_http_request" => (
            vec![types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ), // (method, url, headers, body) -> Async[Lax[@(...)]]
        "taida_os_dns_resolve" => (vec![types::I64, types::I64], vec![types::I64]), // (host, timeoutMs) -> Async[Result]
        "taida_os_tcp_connect" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (host, port, timeoutMs) -> Async[Result]
        "taida_os_tcp_listen" => (vec![types::I64, types::I64], vec![types::I64]), // (port, timeoutMs) -> Async[Result]
        "taida_os_tcp_accept" => (vec![types::I64, types::I64], vec![types::I64]), // (listener, timeoutMs) -> Async[Result]
        "taida_os_socket_send" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (socket, data, timeoutMs) -> Async[Result]
        "taida_os_socket_send_all" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (socket, data, timeoutMs) -> Async[Result]
        "taida_os_socket_recv" => (vec![types::I64, types::I64], vec![types::I64]), // (socket, timeoutMs) -> Async[Lax[Str]]
        "taida_os_socket_send_bytes" => {
            (vec![types::I64, types::I64, types::I64], vec![types::I64])
        } // (socket, data, timeoutMs) -> Async[Result]
        "taida_os_socket_recv_bytes" => (vec![types::I64, types::I64], vec![types::I64]), // (socket, timeoutMs) -> Async[Lax[Bytes]]
        "taida_os_socket_recv_exact" => {
            (vec![types::I64, types::I64, types::I64], vec![types::I64])
        } // (socket, size, timeoutMs) -> Async[Lax[Bytes]]
        "taida_os_udp_bind" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (host, port, timeoutMs) -> Async[Result]
        "taida_os_udp_send_to" => (
            vec![types::I64, types::I64, types::I64, types::I64, types::I64],
            vec![types::I64],
        ), // (socket, host, port, data, timeoutMs) -> Async[Result]
        "taida_os_udp_recv_from" => (vec![types::I64, types::I64], vec![types::I64]), // (socket, timeoutMs) -> Async[Lax[@(...)]]
        "taida_os_socket_close" => (vec![types::I64], vec![types::I64]), // (socket) -> Async[Result]
        "taida_os_listener_close" => (vec![types::I64], vec![types::I64]), // (listener) -> Async[Result]
        // taida-lang/pool package
        "taida_pool_create" => (vec![types::I64], vec![types::I64]), // (config) -> Result
        "taida_pool_acquire" => (vec![types::I64, types::I64], vec![types::I64]), // (pool, timeoutMs) -> Async[Result]
        "taida_pool_release" => (vec![types::I64, types::I64, types::I64], vec![types::I64]), // (pool, token, resource) -> Result
        "taida_pool_close" => (vec![types::I64], vec![types::I64]), // (pool) -> Async[Result]
        "taida_pool_health" => (vec![types::I64], vec![types::I64]), // (pool) -> @(open,idle,inUse,waiting)
        _ => panic!("unknown runtime function: {}", name),
    }
}

pub struct Emitter {
    pub module: ObjectModule,
    builder_ctx: FunctionBuilderContext,
    ctx: cranelift_codegen::Context,
    declared_funcs: HashMap<String, FuncId>,
    string_constants: Vec<(cranelift_module::DataId, Vec<u8>)>,
    user_func_sigs: HashMap<String, (Vec<clif::Type>, Vec<clif::Type>)>,
}

/// emit_function 内で使うコンテキスト
struct EmitCtx {
    val_map: HashMap<IrVar, clif::Value>,
    named_vars: HashMap<String, clif::Value>,
    func_refs: HashMap<String, clif::FuncRef>,
    str_globals: HashMap<IrVar, clif::GlobalValue>,
    /// TCO: ループ先頭ブロック（末尾再帰用）
    tco_loop_block: Option<clif::Block>,
    /// TCO: パラメータ Variable のリスト（引数の再代入用）
    tco_param_vars: Vec<cranelift_frontend::Variable>,
}

#[derive(Debug)]
pub struct EmitError {
    pub message: String,
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Emit error: {}", self.message)
    }
}

impl Emitter {
    pub fn new() -> Result<Self, EmitError> {
        let shared_builder = settings::builder();
        let shared_flags = settings::Flags::new(shared_builder);

        let triple = Triple::host();
        let isa = cranelift_codegen::isa::lookup(triple.clone())
            .map_err(|e| EmitError {
                message: format!("ISA lookup failed: {}", e),
            })?
            .finish(shared_flags)
            .map_err(|e| EmitError {
                message: format!("ISA finish failed: {}", e),
            })?;

        let obj_builder = ObjectBuilder::new(
            isa,
            "taida_module",
            cranelift_module::default_libcall_names(),
        )
        .map_err(|e| EmitError {
            message: format!("ObjectBuilder failed: {}", e),
        })?;

        let module = ObjectModule::new(obj_builder);

        Ok(Self {
            module,
            builder_ctx: FunctionBuilderContext::new(),
            ctx: cranelift_codegen::Context::new(),
            declared_funcs: HashMap::new(),
            string_constants: Vec::new(),
            user_func_sigs: HashMap::new(),
        })
    }

    fn declare_runtime_func(
        &mut self,
        name: &str,
        params: &[clif::Type],
        returns: &[clif::Type],
    ) -> Result<FuncId, EmitError> {
        if let Some(&id) = self.declared_funcs.get(name) {
            return Ok(id);
        }

        let mut sig = self.module.make_signature();
        for &p in params {
            sig.params.push(AbiParam::new(p));
        }
        for &r in returns {
            sig.returns.push(AbiParam::new(r));
        }
        sig.call_conv = CallConv::SystemV;

        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| EmitError {
                message: format!("declare_function failed: {}", e),
            })?;

        self.declared_funcs.insert(name.to_string(), id);
        Ok(id)
    }

    fn declare_user_func(&mut self, name: &str, param_count: usize) -> Result<FuncId, EmitError> {
        if let Some(&id) = self.declared_funcs.get(name) {
            return Ok(id);
        }

        let mut sig = self.module.make_signature();
        for _ in 0..param_count {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        sig.call_conv = CallConv::SystemV;

        let params: Vec<clif::Type> = (0..param_count).map(|_| types::I64).collect();
        self.user_func_sigs
            .insert(name.to_string(), (params, vec![types::I64]));

        let id = self
            .module
            .declare_function(name, Linkage::Local, &sig)
            .map_err(|e| EmitError {
                message: format!("declare_function failed: {}", e),
            })?;

        self.declared_funcs.insert(name.to_string(), id);
        Ok(id)
    }

    fn declare_imported_func(
        &mut self,
        name: &str,
        param_count: usize,
    ) -> Result<FuncId, EmitError> {
        if let Some(&id) = self.declared_funcs.get(name) {
            return Ok(id);
        }
        let mut sig = self.module.make_signature();
        for _ in 0..param_count {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        sig.call_conv = CallConv::SystemV;

        let params: Vec<clif::Type> = (0..param_count).map(|_| types::I64).collect();
        self.user_func_sigs
            .insert(name.to_string(), (params, vec![types::I64]));

        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| EmitError {
                message: format!("declare_imported_func failed: {}", e),
            })?;
        self.declared_funcs.insert(name.to_string(), id);
        Ok(id)
    }

    fn declare_exported_func(
        &mut self,
        name: &str,
        param_count: usize,
    ) -> Result<FuncId, EmitError> {
        if let Some(&id) = self.declared_funcs.get(name) {
            return Ok(id);
        }
        let mut sig = self.module.make_signature();
        for _ in 0..param_count {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        sig.call_conv = CallConv::SystemV;

        let params: Vec<clif::Type> = (0..param_count).map(|_| types::I64).collect();
        self.user_func_sigs
            .insert(name.to_string(), (params, vec![types::I64]));

        let id = self
            .module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| EmitError {
                message: format!("declare_exported_func failed: {}", e),
            })?;
        self.declared_funcs.insert(name.to_string(), id);
        Ok(id)
    }

    /// 全 IR 関数からランタイム関数呼び出しを収集して宣言
    fn predeclare_all_runtime_funcs(&mut self, ir_module: &IrModule) -> Result<(), EmitError> {
        for ir_func in &ir_module.functions {
            self.predeclare_runtime_funcs_recursive(&ir_func.body)?;
        }
        Ok(())
    }

    fn predeclare_runtime_funcs_recursive(&mut self, insts: &[IrInst]) -> Result<(), EmitError> {
        for inst in insts {
            match inst {
                IrInst::Call(_, func_name, _) => {
                    if !self.declared_funcs.contains_key(func_name) {
                        let (params, returns) = runtime_func_signature(func_name);
                        self.declare_runtime_func(func_name, &params, &returns)?;
                    }
                }
                IrInst::PackNew(_, _) => {
                    self.ensure_runtime_func("taida_pack_new")?;
                }
                IrInst::PackSet(_, _, _) => {
                    self.ensure_runtime_func("taida_pack_set")?;
                }
                IrInst::PackSetTag(_, _, _) => {
                    self.ensure_runtime_func("taida_pack_set_tag")?;
                }
                IrInst::PackGet(_, _, _) => {
                    self.ensure_runtime_func("taida_pack_get_idx")?;
                }
                IrInst::MakeClosure(_, _, _) => {
                    self.ensure_runtime_func("taida_pack_new")?;
                    self.ensure_runtime_func("taida_pack_set")?;
                    self.ensure_runtime_func("taida_pack_set_hash")?;
                    self.ensure_runtime_func("taida_closure_new")?;
                }
                IrInst::CallIndirect(_, _, _) => {
                    self.ensure_runtime_func("taida_is_closure_value")?;
                    self.ensure_runtime_func("taida_closure_get_fn")?;
                    self.ensure_runtime_func("taida_closure_get_env")?;
                }
                IrInst::Retain(_) => {
                    self.ensure_runtime_func("taida_retain")?;
                }
                IrInst::Release(_) => {
                    self.ensure_runtime_func("taida_release")?;
                }
                IrInst::GlobalSet(_, _) => {
                    self.ensure_runtime_func("taida_global_set")?;
                }
                IrInst::GlobalGet(_, _) => {
                    self.ensure_runtime_func("taida_global_get")?;
                }
                IrInst::CondBranch(_, arms) => {
                    for arm in arms {
                        self.predeclare_runtime_funcs_recursive(&arm.body)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// IR 命令列から CallUser の引数数を収集
    /// IR 命令列から _taida_init_* 関数呼び出しを収集する
    fn collect_init_calls(insts: &[IrInst]) -> Vec<String> {
        let mut result = Vec::new();
        for inst in insts {
            match inst {
                IrInst::CallUser(_, name, _) if name.starts_with("_taida_init_") => {
                    result.push(name.clone());
                }
                IrInst::CondBranch(_, arms) => {
                    for arm in arms {
                        result.extend(Self::collect_init_calls(&arm.body));
                    }
                }
                _ => {}
            }
        }
        result
    }

    fn collect_call_arities(
        insts: &[IrInst],
        targets: &std::collections::HashSet<String>,
        arities: &mut HashMap<String, usize>,
    ) {
        for inst in insts {
            match inst {
                IrInst::CallUser(_, name, args) => {
                    if targets.contains(name) {
                        arities.insert(name.clone(), args.len());
                    }
                }
                IrInst::CondBranch(_, arms) => {
                    for arm in arms {
                        Self::collect_call_arities(&arm.body, targets, arities);
                    }
                }
                _ => {}
            }
        }
    }

    fn ensure_runtime_func(&mut self, name: &str) -> Result<(), EmitError> {
        if !self.declared_funcs.contains_key(name) {
            let (params, returns) = runtime_func_signature(name);
            self.declare_runtime_func(name, &params, &returns)?;
        }
        Ok(())
    }

    pub fn emit_module(&mut self, ir_module: &IrModule) -> Result<(), EmitError> {
        // インポートされた関数名の収集
        let imported_funcs: std::collections::HashSet<String> = ir_module
            .imports
            .iter()
            .flat_map(|(_, syms)| syms.iter().cloned())
            .collect();

        // IR から各インポート関数の引数数を推定
        let mut import_arities: HashMap<String, usize> = HashMap::new();
        for ir_func in &ir_module.functions {
            Self::collect_call_arities(&ir_func.body, &imported_funcs, &mut import_arities);
        }

        // インポートされた関数を宣言
        for mangled in &imported_funcs {
            let arity = import_arities.get(mangled).copied().unwrap_or(0);
            self.declare_imported_func(mangled, arity)?;
        }

        // モジュール init 関数の宣言（_taida_init_<module_key> は他モジュールからインポート）
        let init_funcs: std::collections::HashSet<String> = ir_module
            .functions
            .iter()
            .flat_map(|f| Self::collect_init_calls(&f.body))
            .filter(|name| {
                // 自モジュールで定義されている init 関数は除外
                !ir_module.functions.iter().any(|f| f.name == *name)
            })
            .collect();
        for init_name in &init_funcs {
            // init 関数は引数なし・i64 戻り値
            self.declare_imported_func(init_name, 0)?;
        }

        // エクスポートされる関数名のセット
        let exported_funcs: std::collections::HashSet<String> =
            ir_module.exports.iter().cloned().collect();

        // 1st pass: 全関数を宣言
        for ir_func in &ir_module.functions {
            if ir_func.name == "_taida_main" {
                let mut sig = self.module.make_signature();
                sig.returns.push(AbiParam::new(types::I64));
                sig.call_conv = CallConv::SystemV;

                let id = self
                    .module
                    .declare_function(&ir_func.name, Linkage::Export, &sig)
                    .map_err(|e| EmitError {
                        message: format!("{}", e),
                    })?;
                self.declared_funcs.insert(ir_func.name.clone(), id);
            } else if exported_funcs.contains(&ir_func.name)
                || ir_func.name.starts_with("_taida_init_")
            {
                self.declare_exported_func(&ir_func.name, ir_func.params.len())?;
            } else {
                self.declare_user_func(&ir_func.name, ir_func.params.len())?;
            }
        }

        // ランタイム関数を宣言
        self.predeclare_all_runtime_funcs(ir_module)?;

        // 2nd pass: 各関数を定義
        for ir_func in &ir_module.functions {
            self.emit_function(ir_func)?;
        }

        // 文字列定数をデータセクションに書き込み
        for (data_id, bytes) in &self.string_constants {
            let mut data_desc = cranelift_module::DataDescription::new();
            data_desc.define(bytes.clone().into_boxed_slice());
            self.module
                .define_data(*data_id, &data_desc)
                .map_err(|e| EmitError {
                    message: format!("define_data failed: {}", e),
                })?;
        }

        Ok(())
    }

    fn collect_str_constants(
        &mut self,
        insts: &[IrInst],
        func_name: &str,
    ) -> Result<HashMap<IrVar, clif::GlobalValue>, EmitError> {
        let mut str_globals = HashMap::new();
        self.collect_str_constants_recursive(insts, func_name, &mut str_globals)?;
        Ok(str_globals)
    }

    fn collect_str_constants_recursive(
        &mut self,
        insts: &[IrInst],
        func_name: &str,
        str_globals: &mut HashMap<IrVar, clif::GlobalValue>,
    ) -> Result<(), EmitError> {
        for inst in insts {
            match inst {
                IrInst::ConstStr(dst, string) => {
                    let mut bytes = string.as_bytes().to_vec();
                    bytes.push(0);

                    let data_id = self
                        .module
                        .declare_data(
                            &format!("{}__str_{}", func_name, dst),
                            Linkage::Local,
                            false,
                            false,
                        )
                        .map_err(|e| EmitError {
                            message: format!("declare_data failed: {}", e),
                        })?;

                    self.string_constants.push((data_id, bytes));

                    let global = self
                        .module
                        .declare_data_in_func(data_id, &mut self.ctx.func);
                    str_globals.insert(*dst, global);
                }
                IrInst::CondBranch(_, arms) => {
                    for arm in arms {
                        self.collect_str_constants_recursive(&arm.body, func_name, str_globals)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// IR 命令列に TailCall が含まれるかチェック（再帰的）
    fn contains_tail_call(insts: &[IrInst]) -> bool {
        for inst in insts {
            match inst {
                IrInst::TailCall(_) => return true,
                IrInst::CondBranch(_, arms) => {
                    for arm in arms {
                        if Self::contains_tail_call(&arm.body) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn emit_function(&mut self, ir_func: &IrFunction) -> Result<(), EmitError> {
        let func_id = self.declared_funcs[&ir_func.name];

        let mut sig = self.module.make_signature();
        for _ in &ir_func.params {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        sig.call_conv = CallConv::SystemV;

        self.ctx.func.signature = sig;
        self.ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, func_id.as_u32());

        // FuncRef を先に取得
        let mut func_refs: HashMap<String, clif::FuncRef> = HashMap::new();
        for (name, &fid) in &self.declared_funcs {
            let fref = self.module.declare_func_in_func(fid, &mut self.ctx.func);
            func_refs.insert(name.clone(), fref);
        }

        // 文字列定数の GlobalValue を収集
        let str_globals = self.collect_str_constants(&ir_func.body, &ir_func.name)?;

        let has_tail_call = Self::contains_tail_call(&ir_func.body);

        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);

            let mut ectx = EmitCtx {
                val_map: HashMap::new(),
                named_vars: HashMap::new(),
                func_refs,
                str_globals,
                tco_loop_block: None,
                tco_param_vars: Vec::new(),
            };

            if has_tail_call {
                // TCO: パラメータを Variable に格納し、ループブロックを作成
                let mut param_vars = Vec::new();
                let block_params = builder.block_params(entry_block).to_vec();

                for (i, param_name) in ir_func.params.iter().enumerate() {
                    let var = builder.declare_var(types::I64);
                    builder.def_var(var, block_params[i]);
                    param_vars.push(var);
                    // IrVar のマッピングも設定
                    ectx.val_map.insert(i as IrVar, block_params[i]);
                    ectx.named_vars.insert(param_name.clone(), block_params[i]);
                }

                // ループブロック（エントリから無条件ジャンプ）
                let loop_block = builder.create_block();
                builder.ins().jump(loop_block, &[]);
                builder.seal_block(entry_block);

                builder.switch_to_block(loop_block);
                // loop_block は seal しない（TailCall から戻ってくるため）

                // ループブロックでは Variable から値を読み出す
                for (i, param_name) in ir_func.params.iter().enumerate() {
                    let val = builder.use_var(param_vars[i]);
                    ectx.val_map.insert(i as IrVar, val);
                    ectx.named_vars.insert(param_name.clone(), val);
                }

                ectx.tco_loop_block = Some(loop_block);
                ectx.tco_param_vars = param_vars;

                Self::emit_instructions(&mut builder, &mut ectx, &ir_func.body);

                // ループブロックを seal（全ての predecessor が確定した後）
                builder.seal_block(loop_block);
            } else {
                builder.seal_block(entry_block);

                // パラメータをマッピング
                let block_params = builder.block_params(entry_block).to_vec();
                for (i, param_name) in ir_func.params.iter().enumerate() {
                    let param_val = block_params[i];
                    ectx.val_map.insert(i as IrVar, param_val);
                    ectx.named_vars.insert(param_name.clone(), param_val);
                }

                Self::emit_instructions(&mut builder, &mut ectx, &ir_func.body);
            }

            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut self.ctx)
            .map_err(|e| EmitError {
                message: format!("define_function failed: {}", e),
            })?;

        self.module.clear_context(&mut self.ctx);
        Ok(())
    }

    /// 命令列を現在のブロックに emit する
    fn emit_instructions(builder: &mut FunctionBuilder, ectx: &mut EmitCtx, insts: &[IrInst]) {
        for inst in insts {
            Self::emit_inst(builder, ectx, inst);
        }
    }

    fn emit_inst(builder: &mut FunctionBuilder, ectx: &mut EmitCtx, inst: &IrInst) {
        match inst {
            IrInst::ConstInt(dst, value) => {
                let val = builder.ins().iconst(types::I64, *value);
                ectx.val_map.insert(*dst, val);
            }
            IrInst::ConstFloat(dst, value) => {
                let val = builder.ins().f64const(*value);
                ectx.val_map.insert(*dst, val);
            }
            IrInst::ConstStr(dst, _) => {
                let global = ectx.str_globals[dst];
                let ptr = builder.ins().global_value(types::I64, global);
                ectx.val_map.insert(*dst, ptr);
            }
            IrInst::ConstBool(dst, value) => {
                let val = builder.ins().iconst(types::I64, if *value { 1 } else { 0 });
                ectx.val_map.insert(*dst, val);
            }
            IrInst::DefVar(name, src) => {
                if let Some(&val) = ectx.val_map.get(src) {
                    ectx.named_vars.insert(name.clone(), val);
                }
            }
            IrInst::UseVar(dst, name) => {
                if let Some(&val) = ectx.named_vars.get(name) {
                    ectx.val_map.insert(*dst, val);
                } else {
                    let val = builder.ins().iconst(types::I64, 0);
                    ectx.val_map.insert(*dst, val);
                }
            }
            IrInst::PackNew(dst, field_count) => {
                // taida_pack_new(field_count) -> ptr
                let func_ref = ectx.func_refs["taida_pack_new"];
                let count_val = builder.ins().iconst(types::I64, *field_count as i64);
                let call = builder.ins().call(func_ref, &[count_val]);
                let results = builder.inst_results(call);
                ectx.val_map.insert(*dst, results[0]);
            }
            IrInst::PackSet(pack_var, index, value_var) => {
                // taida_pack_set(ptr, index, value) -> ptr
                let func_ref = ectx.func_refs["taida_pack_set"];
                let pack_val = ectx.val_map[pack_var];
                let idx_val = builder.ins().iconst(types::I64, *index as i64);
                let value_val = ectx.val_map[value_var];
                builder
                    .ins()
                    .call(func_ref, &[pack_val, idx_val, value_val]);
            }
            IrInst::PackSetTag(pack_var, index, tag) => {
                // taida_pack_set_tag(ptr, index, tag) -> ptr
                let func_ref = ectx.func_refs["taida_pack_set_tag"];
                let pack_val = ectx.val_map[pack_var];
                let idx_val = builder.ins().iconst(types::I64, *index as i64);
                let tag_val = builder.ins().iconst(types::I64, *tag);
                builder.ins().call(func_ref, &[pack_val, idx_val, tag_val]);
            }
            IrInst::PackGet(dst, pack_var, index) => {
                // taida_pack_get_idx(ptr, index) -> value
                let func_ref = ectx.func_refs["taida_pack_get_idx"];
                let pack_val = ectx.val_map[pack_var];
                let idx_val = builder.ins().iconst(types::I64, *index as i64);
                let call = builder.ins().call(func_ref, &[pack_val, idx_val]);
                let results = builder.inst_results(call);
                ectx.val_map.insert(*dst, results[0]);
            }
            IrInst::Call(dst, func_name, args) => {
                // ── Integer/Bool intrinsics: emit native instructions instead of call ──
                if let Some(result) = Self::try_emit_intrinsic(builder, ectx, func_name, args) {
                    ectx.val_map.insert(*dst, result);
                } else {
                    let func_ref = ectx.func_refs[func_name];
                    let (param_types, return_types) = runtime_func_signature(func_name);

                    let arg_vals: Vec<clif::Value> = args
                        .iter()
                        .enumerate()
                        .map(|(i, &arg)| {
                            let val = ectx.val_map[&arg];
                            let val_type = builder.func.dfg.value_type(val);
                            let expected_type = param_types.get(i).copied().unwrap_or(types::I64);

                            if val_type == expected_type {
                                val
                            } else if val_type == types::F64 && expected_type == types::I64 {
                                builder
                                    .ins()
                                    .bitcast(types::I64, clif::MemFlags::new(), val)
                            } else if val_type == types::I64 && expected_type == types::F64 {
                                builder
                                    .ins()
                                    .bitcast(types::F64, clif::MemFlags::new(), val)
                            } else {
                                val
                            }
                        })
                        .collect();

                    let call = builder.ins().call(func_ref, &arg_vals);
                    let results = builder.inst_results(call);
                    if !results.is_empty() {
                        ectx.val_map.insert(*dst, results[0]);
                    } else if return_types.is_empty() {
                        // void 関数: trap の代わりに 0 を返す
                        // (trap はブロック終端命令なので後続命令を追加できない)
                        let dummy = builder.ins().iconst(types::I64, 0);
                        ectx.val_map.insert(*dst, dummy);
                    }
                }
            }
            IrInst::CallUser(dst, func_name, args) => {
                let func_ref = ectx.func_refs[func_name];
                let arg_vals: Vec<clif::Value> =
                    args.iter().map(|&arg| ectx.val_map[&arg]).collect();

                let call = builder.ins().call(func_ref, &arg_vals);
                let results = builder.inst_results(call);
                if !results.is_empty() {
                    ectx.val_map.insert(*dst, results[0]);
                }
            }
            IrInst::FuncAddr(dst, func_name) => {
                let fn_ref = ectx.func_refs[func_name];
                let fn_addr = builder.ins().func_addr(types::I64, fn_ref);
                ectx.val_map.insert(*dst, fn_addr);
            }
            IrInst::MakeClosure(dst, func_name, captures) => {
                // 1. 環境パックを作成（キャプチャ変数を格納）
                let pack_new_ref = ectx.func_refs["taida_pack_new"];
                let count_val = builder.ins().iconst(types::I64, captures.len() as i64);
                let pack_call = builder.ins().call(pack_new_ref, &[count_val]);
                let env_ptr = builder.inst_results(pack_call)[0];

                // キャプチャ変数を環境に格納
                let pack_set_ref = ectx.func_refs["taida_pack_set"];
                for (i, cap_name) in captures.iter().enumerate() {
                    if let Some(&cap_val) = ectx.named_vars.get(cap_name) {
                        let idx_val = builder.ins().iconst(types::I64, i as i64);
                        builder
                            .ins()
                            .call(pack_set_ref, &[env_ptr, idx_val, cap_val]);
                    }
                }

                // 2. 関数アドレスを取得
                let fn_ref = ectx.func_refs[func_name];
                let fn_addr = builder.ins().func_addr(types::I64, fn_ref);

                // 3. クロージャ構造体を作成
                let closure_new_ref = ectx.func_refs["taida_closure_new"];
                let closure_call = builder.ins().call(closure_new_ref, &[fn_addr, env_ptr]);
                let closure_ptr = builder.inst_results(closure_call)[0];

                ectx.val_map.insert(*dst, closure_ptr);
            }
            IrInst::CallIndirect(dst, fn_var, args) => {
                let callable = ectx.val_map[fn_var];
                let result_var = builder.declare_var(types::I64);
                let closure_block = builder.create_block();
                let plain_block = builder.create_block();
                let merge_block = builder.create_block();

                let is_closure_ref = ectx.func_refs["taida_is_closure_value"];
                let is_closure_call = builder.ins().call(is_closure_ref, &[callable]);
                let is_closure = builder.inst_results(is_closure_call)[0];
                builder
                    .ins()
                    .brif(is_closure, closure_block, &[], plain_block, &[]);

                builder.switch_to_block(closure_block);
                builder.seal_block(closure_block);
                let get_fn_ref = ectx.func_refs["taida_closure_get_fn"];
                let fn_call = builder.ins().call(get_fn_ref, &[callable]);
                let fn_ptr = builder.inst_results(fn_call)[0];

                let get_env_ref = ectx.func_refs["taida_closure_get_env"];
                let env_call = builder.ins().call(get_env_ref, &[callable]);
                let env_ptr = builder.inst_results(env_call)[0];

                let mut closure_args = vec![env_ptr];
                for &arg in args {
                    closure_args.push(ectx.val_map[&arg]);
                }
                let mut closure_sig = clif::Signature::new(CallConv::SystemV);
                for _ in &closure_args {
                    closure_sig.params.push(AbiParam::new(types::I64));
                }
                closure_sig.returns.push(AbiParam::new(types::I64));
                let closure_sig_ref = builder.import_signature(closure_sig);
                let closure_call =
                    builder
                        .ins()
                        .call_indirect(closure_sig_ref, fn_ptr, &closure_args);
                let closure_result = builder.inst_results(closure_call)[0];
                builder.def_var(result_var, closure_result);
                builder.ins().jump(merge_block, &[]);

                builder.switch_to_block(plain_block);
                builder.seal_block(plain_block);
                let plain_args: Vec<clif::Value> =
                    args.iter().map(|&arg| ectx.val_map[&arg]).collect();
                let mut plain_sig = clif::Signature::new(CallConv::SystemV);
                for _ in &plain_args {
                    plain_sig.params.push(AbiParam::new(types::I64));
                }
                plain_sig.returns.push(AbiParam::new(types::I64));
                let plain_sig_ref = builder.import_signature(plain_sig);
                let plain_call = builder
                    .ins()
                    .call_indirect(plain_sig_ref, callable, &plain_args);
                let plain_result = builder.inst_results(plain_call)[0];
                builder.def_var(result_var, plain_result);
                builder.ins().jump(merge_block, &[]);

                builder.switch_to_block(merge_block);
                builder.seal_block(merge_block);
                let result = builder.use_var(result_var);
                ectx.val_map.insert(*dst, result);
            }
            IrInst::Retain(var) => {
                let val = ectx.val_map[var];
                if let Some(&func_ref) = ectx.func_refs.get("taida_retain") {
                    builder.ins().call(func_ref, &[val]);
                }
            }
            IrInst::Release(var) => {
                let val = ectx.val_map[var];
                if let Some(&func_ref) = ectx.func_refs.get("taida_release") {
                    builder.ins().call(func_ref, &[val]);
                }
            }
            IrInst::TailCall(args) => {
                // 末尾再帰: 引数を Variable に再代入してループブロックにジャンプ
                if let Some(loop_block) = ectx.tco_loop_block {
                    // 新しい引数値を全て先に評価してから代入
                    // （引数間に依存がある場合の正しさを保証）
                    let arg_vals: Vec<clif::Value> =
                        args.iter().map(|&arg| ectx.val_map[&arg]).collect();

                    for (i, val) in arg_vals.iter().enumerate() {
                        if i < ectx.tco_param_vars.len() {
                            builder.def_var(ectx.tco_param_vars[i], *val);
                        }
                    }

                    builder.ins().jump(loop_block, &[]);

                    // TailCall の後にコードが続く場合のために新しいブロック
                    let next_block = builder.create_block();
                    builder.switch_to_block(next_block);
                    builder.seal_block(next_block);
                }
            }
            IrInst::CondBranch(dst, arms) => {
                Self::emit_cond_branch(builder, ectx, *dst, arms);
            }
            IrInst::Return(var) => {
                let val = ectx.val_map[var];
                builder.ins().return_(&[val]);
                let next_block = builder.create_block();
                builder.switch_to_block(next_block);
                builder.seal_block(next_block);
            }
            IrInst::GlobalSet(name_hash, value_var) => {
                let hash_val = builder.ins().iconst(types::I64, *name_hash);
                let val = ectx.val_map[value_var];
                let func_ref = ectx.func_refs["taida_global_set"];
                builder.ins().call(func_ref, &[hash_val, val]);
            }
            IrInst::GlobalGet(dst, name_hash) => {
                let hash_val = builder.ins().iconst(types::I64, *name_hash);
                let func_ref = ectx.func_refs["taida_global_get"];
                let call = builder.ins().call(func_ref, &[hash_val]);
                let result = builder.inst_results(call)[0];
                ectx.val_map.insert(*dst, result);
            }
        }
    }

    /// 条件分岐を CLIF ブロックに変換
    /// Variable を使って結果を受け渡す（BlockArg を回避）
    fn emit_cond_branch(
        builder: &mut FunctionBuilder,
        ectx: &mut EmitCtx,
        result_dst: IrVar,
        arms: &[CondArm],
    ) {
        // 結果を格納する Variable
        let result_var = builder.declare_var(types::I64);

        // マージブロック
        let merge_block = builder.create_block();

        // 各アームを処理
        for (i, arm) in arms.iter().enumerate() {
            match &arm.condition {
                Some(cond_var) => {
                    let then_block = builder.create_block();
                    let else_block = if i + 1 < arms.len() {
                        builder.create_block()
                    } else {
                        merge_block
                    };

                    let cond_val = ectx.val_map[cond_var];
                    builder
                        .ins()
                        .brif(cond_val, then_block, &[], else_block, &[]);

                    // then ブロック
                    builder.switch_to_block(then_block);
                    builder.seal_block(then_block);
                    Self::emit_instructions(builder, ectx, &arm.body);
                    let result_val = ectx.val_map[&arm.result];
                    builder.def_var(result_var, result_val);
                    builder.ins().jump(merge_block, &[]);

                    // else ブロック（次のアームへ）
                    if else_block != merge_block {
                        builder.switch_to_block(else_block);
                        builder.seal_block(else_block);
                    }
                }
                None => {
                    // デフォルトケース (| _ |>)
                    Self::emit_instructions(builder, ectx, &arm.body);
                    let result_val = ectx.val_map[&arm.result];
                    builder.def_var(result_var, result_val);
                    builder.ins().jump(merge_block, &[]);
                }
            }
        }

        // デフォルトケースがない場合のフォールバック
        let has_default = arms.iter().any(|a| a.condition.is_none());
        if !has_default {
            let default_val = builder.ins().iconst(types::I64, 0);
            builder.def_var(result_var, default_val);
            builder.ins().jump(merge_block, &[]);
        }

        builder.switch_to_block(merge_block);
        builder.seal_block(merge_block);

        let result = builder.use_var(result_var);
        ectx.val_map.insert(result_dst, result);
    }

    /// Simple integer/bool operations → native Cranelift instructions.
    /// Returns Some(result_value) if the function was inlined, None otherwise.
    fn try_emit_intrinsic(
        builder: &mut FunctionBuilder,
        ectx: &mut EmitCtx,
        func_name: &str,
        args: &[IrVar],
    ) -> Option<clif::Value> {
        use cranelift_codegen::ir::condcodes::IntCC;

        match func_name {
            // ── Binary integer arithmetic ──
            "taida_int_add" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                Some(builder.ins().iadd(a, b))
            }
            "taida_int_sub" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                Some(builder.ins().isub(a, b))
            }
            "taida_int_mul" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                Some(builder.ins().imul(a, b))
            }

            // ── Integer comparisons (return i64: 0 or 1) ──
            "taida_int_eq" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                let cmp = builder.ins().icmp(IntCC::Equal, a, b);
                Some(builder.ins().uextend(types::I64, cmp))
            }
            "taida_int_neq" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                let cmp = builder.ins().icmp(IntCC::NotEqual, a, b);
                Some(builder.ins().uextend(types::I64, cmp))
            }
            "taida_int_lt" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
                Some(builder.ins().uextend(types::I64, cmp))
            }
            "taida_int_gt" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                let cmp = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
                Some(builder.ins().uextend(types::I64, cmp))
            }
            "taida_int_gte" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                let cmp = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, b);
                Some(builder.ins().uextend(types::I64, cmp))
            }

            // ── Unary integer ──
            "taida_int_neg" if args.len() == 1 => {
                let a = ectx.val_map[&args[0]];
                Some(builder.ins().ineg(a))
            }

            // ── Boolean operations ──
            "taida_bool_not" if args.len() == 1 => {
                let a = ectx.val_map[&args[0]];
                let one = builder.ins().iconst(types::I64, 1);
                Some(builder.ins().bxor(a, one))
            }
            "taida_bool_and" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                Some(builder.ins().band(a, b))
            }
            "taida_bool_or" if args.len() == 2 => {
                let a = ectx.val_map[&args[0]];
                let b = ectx.val_map[&args[1]];
                Some(builder.ins().bor(a, b))
            }

            _ => None,
        }
    }
}
