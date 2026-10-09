//! The rules for the renderer, as a WebAssembly module with a C ABI (no generated glue):
//! the host copies UTF-8 into memory from `kc_alloc`, calls `kc_invoke`, reads the result and
//! frees both. `core/src/domain/engine.ts` (`wasmBinding`) is the host side.

use kchess_domain::{api, review};

/// `exp` as the rules compute it, with (`fused = 1`) or without fused multiply-adds; the host
/// compares both with its own `Math.exp` and keeps the one that matches (`kc_use_fused_exp`).
#[unsafe(no_mangle)]
pub extern "C" fn kc_exp(x: f64, fused: u32) -> f64 {
    review::set_fused_exp(fused == 1);
    review::js_exp(x)
}

/// Choose how `exp` rounds for the rest of the session.
#[unsafe(no_mangle)]
pub extern "C" fn kc_use_fused_exp(fused: u32) {
    review::set_fused_exp(fused == 1);
}

/// A buffer of `len` bytes the host fills, then passes back or frees with `kc_free`.
#[unsafe(no_mangle)]
pub extern "C" fn kc_alloc(len: usize) -> *mut u8 {
    // A boxed slice's allocation is exactly `len` bytes, so `kc_free` can rebuild it.
    Box::into_raw(vec![0u8; len].into_boxed_slice()).cast()
}

/// Free a buffer from `kc_alloc` or `kc_invoke`.
///
/// # Safety
/// `ptr` must come from `kc_alloc(len)` or be a result of `kc_invoke` with its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kc_free(ptr: *mut u8, len: usize) {
    // SAFETY: the host returns exactly what this module allocated, with its length.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// `api::call(method, args)`. Returns the result buffer as `(ptr << 32) | len`; its first byte
/// is `0` for a JSON result and `1` for an error message.
///
/// # Safety
/// Both input ranges must be valid UTF-8 the host wrote into buffers from `kc_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kc_invoke(
    method: *const u8,
    method_len: usize,
    args: *const u8,
    args_len: usize,
) -> u64 {
    // SAFETY: the host passes ranges inside buffers it filled.
    let (method, args) = unsafe {
        (
            std::slice::from_raw_parts(method, method_len),
            std::slice::from_raw_parts(args, args_len),
        )
    };
    let result = match (std::str::from_utf8(method), std::str::from_utf8(args)) {
        (Ok(method), Ok(args)) => api::call(method, args),
        _ => Err("arguments must be UTF-8".into()),
    };
    let (status, body) = match result {
        Ok(json) => (0u8, json),
        Err(message) => (1u8, message),
    };
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(status);
    out.extend_from_slice(body.as_bytes());
    let len = out.len();
    let ptr: *mut u8 = Box::into_raw(out.into_boxed_slice()).cast();
    ((ptr as u64) << 32) | len as u64
}
