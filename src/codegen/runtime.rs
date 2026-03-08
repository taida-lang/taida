/// Taida ランタイム関数 — ネイティブコードから呼ばれる extern "C" 関数群。
///
/// Phase N1: debug() のみ。
use std::ffi::CStr;
use std::os::raw::c_char;

/// debug(Int) — 整数値を標準出力に表示
#[unsafe(no_mangle)]
pub extern "C" fn taida_debug_int(value: i64) -> i64 {
    println!("{}", value);
    0 // Unit に相当する戻り値
}

/// debug(Float) — 浮動小数点値を標準出力に表示
#[unsafe(no_mangle)]
pub extern "C" fn taida_debug_float(value: f64) -> i64 {
    println!("{}", value);
    0
}

/// debug(Bool) — 真偽値を標準出力に表示
#[unsafe(no_mangle)]
pub extern "C" fn taida_debug_bool(value: i64) -> i64 {
    if value != 0 {
        println!("true");
    } else {
        println!("false");
    }
    0
}

/// debug(Str) — 文字列を標準出力に表示
/// 文字列は C 文字列ポインタとして渡される
///
/// # Safety
/// `ptr` must be either null or a valid NUL-terminated C string for the duration
/// of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn taida_debug_str(ptr: *const c_char) -> i64 {
    if ptr.is_null() {
        println!();
    } else {
        let cstr = unsafe { CStr::from_ptr(ptr) };
        match cstr.to_str() {
            Ok(s) => println!("{}", s),
            Err(_) => println!("<invalid utf-8>"),
        }
    }
    0
}

/// プロセス終了（Gorilla ><）
#[unsafe(no_mangle)]
pub extern "C" fn taida_gorilla() -> ! {
    std::process::exit(1);
}
