//! The rules for the renderer, as a WebAssembly module with a C ABI (no generated glue):
//! the host copies UTF-8 into memory from `kc_alloc`, calls `kc_invoke`, reads the result and
//! frees both. `crates/kchess-wasm/js/engine.ts` (`wasmBinding`) is the host side.

use kchess_domain::{api, review};

/// First byte of a result buffer: the rest is a JSON payload.
const STATUS_OK: u8 = 0;
/// First byte of a result buffer: the rest is a UTF-8 error message.
const STATUS_ERR: u8 = 1;
/// `fused` flag value that selects fused multiply-adds in `kc_exp`/`kc_use_fused_exp`.
const FUSED_ON: u32 = 1;
/// Largest single `kc_alloc` a host may request; larger sizes return null (OOM signal).
const MAX_ALLOC: usize = 16 * 1024 * 1024;
/// Largest `method`/`args` range `kc_invoke` accepts; larger inputs become error buffers.
const MAX_ARGS: usize = 8 * 1024 * 1024;

/// Pack a `(status, body)` result into the `(ptr << 32) | len` buffer the host reads.
///
/// Packing assumes 32-bit pointers on `wasm32`: the host recovers the pointer with
/// `Number(packed >> 32n)` and the length with `Number(packed & 0xffff_ffffn)`. The same
/// packing is reused on 64-bit test hosts, where a heap pointer above 4 GiB would
/// truncate, so debug builds fail loudly instead of handing the host a bad pointer.
fn packed(status: u8, body: &str) -> u64 {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(status);
    out.extend_from_slice(body.as_bytes());
    let len = out.len();
    let ptr: *mut u8 = Box::into_raw(out.into_boxed_slice()).cast();
    debug_assert!(
        size_of::<*mut u8>() <= size_of::<u64>(),
        "packed result must hold the pointer"
    );
    debug_assert!(
        (ptr as u64) < (1u64 << 32),
        "packed pointer does not fit in 32 bits"
    );
    ((ptr as u64) << 32) | len as u64
}

/// Build an error result buffer without duplicating the packing logic.
fn error_packed(msg: &str) -> u64 {
    packed(STATUS_ERR, msg)
}

/// `exp` as the rules compute it, with (`fused = 1`) or without fused multiply-adds; the host
/// compares both with its own `Math.exp` and keeps the one that matches (`kc_use_fused_exp`).
#[unsafe(no_mangle)]
pub extern "C" fn kc_exp(x: f64, fused: u32) -> f64 {
    review::set_fused_exp(fused == FUSED_ON);
    review::js_exp(x)
}

/// Choose how `exp` rounds for the rest of the session.
#[unsafe(no_mangle)]
pub extern "C" fn kc_use_fused_exp(fused: u32) {
    review::set_fused_exp(fused == FUSED_ON);
}

/// A buffer of `len` bytes the host fills, then passes back or frees with `kc_free`.
///
/// A request larger than `MAX_ALLOC` returns null so the host can report out-of-memory
/// instead of trapping on an oversized allocation. A zero-length request returns the
/// (possibly dangling but non-null) empty boxed slice the allocator produces.
#[unsafe(no_mangle)]
pub extern "C" fn kc_alloc(len: usize) -> *mut u8 {
    if len > MAX_ALLOC {
        return std::ptr::null_mut();
    }
    // A boxed slice's allocation is exactly `len` bytes, so `kc_free` can rebuild it.
    Box::into_raw(vec![0u8; len].into_boxed_slice()).cast()
}

/// Free a buffer from `kc_alloc` or `kc_invoke`. A null `ptr` is a no-op, so hosts can
/// unconditionally free nullable results.
///
/// # Safety
/// `ptr` must come from `kc_alloc(len)` or be a result of `kc_invoke` with its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kc_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the host returns exactly what this module allocated, with its length, so
    // rebuilding the boxed slice here frees the one allocation exactly once.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// `api::call(method, args)`. Returns the result buffer as `(ptr << 32) | len`; its first byte
/// is `STATUS_OK` for a JSON result and `STATUS_ERR` for an error message.
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
    if method.is_null() || args.is_null() {
        return error_packed("null argument");
    }
    if method_len > MAX_ARGS || args_len > MAX_ARGS {
        return error_packed("arguments too large");
    }
    // SAFETY: the host passes ranges inside buffers it filled (null and oversize inputs
    // were rejected above), so both ranges are readable for their lengths for this call.
    let (method, args) = unsafe {
        (
            std::slice::from_raw_parts(method, method_len),
            std::slice::from_raw_parts(args, args_len),
        )
    };
    // `profile.wasm` sets `panic = "abort"`, so on wasm32 a panic traps (the host reports
    // it) and `catch_unwind` cannot intercept it; that abort is intended. This guard only
    // applies to native/test builds with unwinding, where it keeps a panic in `api::call`
    // from unwinding into C (undefined behavior) and returns an error buffer instead.
    let result = match (std::str::from_utf8(method), std::str::from_utf8(args)) {
        (Ok(method), Ok(args)) => {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| api::call(method, args)))
                .unwrap_or_else(|_| Err("internal error".into()))
        }
        _ => Err("arguments must be UTF-8".into()),
    };
    let (status, body) = match result {
        Ok(json) => (STATUS_OK, json),
        Err(message) => (STATUS_ERR, message),
    };
    packed(status, &body)
}
