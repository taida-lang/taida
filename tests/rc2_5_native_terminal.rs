//! RC2.5 — Rust ↔ C ABI struct layout parity test.
//!
//! This is the only RC2.5 test that lives in `taida/taida`. The C
//! `_Static_assert`s in `native_runtime.c` lock the C-side sizes at
//! compile time (Phase 1). Here we assert that Rust's
//! `std::mem::size_of::<TaidaAddon...V1>()` agrees with those same
//! literal numbers. Together the two checks form a bidirectional drift
//! detector: if either the Rust `#[repr(C)]` definition in
//! `crates/addon-rs/src/abi.rs` or the C mirror in `native_runtime.c`
//! changes without the other, one of the two sides fails.
//!
//! Runtime integration tests (full v1 surface, interpreter↔native
//! parity, dlopen hard-fail, cdylib-moved diagnostic, Status::Error
//! → catchable AddonError) previously lived here and in
//! `tests/rc2_5_native_terminal_phase3.rs`. Those tests required the
//! workspace-built `libtaida_addon_terminal_sample.so` and were
//! architecturally misplaced: `taida-lang/terminal` is an external
//! package, not bundled with the compiler, so end-to-end tests of
//! it belong in the terminal package's own repository. They have
//! been removed here and are tracked as tech debt to be re-homed
//! (see `.dev/RC2_5_BLOCKERS.md::RC2.5B-009`).

#![cfg(feature = "native")]

#[test]
fn abi_struct_layout_parity_matches_c_static_assert_sizes() {
    use std::mem::size_of;
    use taida_addon::{
        TaidaAddonBytesPayload, TaidaAddonDescriptorV1, TaidaAddonErrorV1, TaidaAddonFloatPayload,
        TaidaAddonFunctionV1, TaidaAddonIntPayload, TaidaAddonPackEntryV1, TaidaAddonPackPayload,
        TaidaAddonValueV1, TaidaHostV1,
    };

    // These numbers must match exactly the `_Static_assert(sizeof(...)
    // == N, ...)` lines in `native_runtime.c`. If you change one,
    // change the other in the same commit.
    assert_eq!(
        size_of::<TaidaAddonValueV1>(),
        16,
        "TaidaAddonValueV1 layout drift (Rust vs expected C sizeof)"
    );
    assert_eq!(
        size_of::<TaidaAddonErrorV1>(),
        16,
        "TaidaAddonErrorV1 layout drift (Rust vs expected C sizeof)"
    );
    assert_eq!(
        size_of::<TaidaAddonIntPayload>(),
        8,
        "TaidaAddonIntPayload layout drift"
    );
    assert_eq!(
        size_of::<TaidaAddonFloatPayload>(),
        8,
        "TaidaAddonFloatPayload layout drift"
    );
    assert_eq!(
        size_of::<TaidaAddonBytesPayload>(),
        16,
        "TaidaAddonBytesPayload layout drift"
    );
    assert_eq!(
        size_of::<TaidaAddonFunctionV1>(),
        24,
        "TaidaAddonFunctionV1 layout drift"
    );
    assert_eq!(
        size_of::<TaidaAddonDescriptorV1>(),
        40,
        "TaidaAddonDescriptorV1 layout drift"
    );
    assert_eq!(size_of::<TaidaHostV1>(), 96, "TaidaHostV1 layout drift");
    assert_eq!(
        size_of::<TaidaAddonPackEntryV1>(),
        16,
        "TaidaAddonPackEntryV1 layout drift"
    );
    assert_eq!(
        size_of::<TaidaAddonPackPayload>(),
        16,
        "TaidaAddonPackPayload layout drift"
    );
}
