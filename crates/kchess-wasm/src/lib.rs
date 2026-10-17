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

/// The C ABI as the host drives it, under Miri (`cargo miri test -p kchess-wasm`), which checks
/// every allocation, copy and free for undefined behaviour. Native test hosts are left out:
/// their heap pointers do not fit the 32-bit packing (see `packed`).
#[cfg(all(test, miri))]
mod tests {
    use super::*;

    /// Copy `bytes` into a fresh `kc_alloc` buffer, as the host does.
    fn host_buffer(bytes: &[u8]) -> *mut u8 {
        let ptr = kc_alloc(bytes.len());
        assert!(!ptr.is_null());
        // SAFETY: `ptr` holds `bytes.len()` writable bytes from `kc_alloc`.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
        ptr
    }

    /// Read and free a `kc_invoke` result, as the host does.
    fn take_result(packed: u64) -> (u8, String) {
        let ptr = std::ptr::with_exposed_provenance_mut::<u8>((packed >> 32) as usize);
        let len = (packed & 0xffff_ffff) as usize;
        // SAFETY: `kc_invoke` returned a live buffer of `len` bytes at `ptr`.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
        // SAFETY: the result buffer with its own length, freed once.
        unsafe { kc_free(ptr, len) };
        let (status, body) = bytes.split_first().unwrap();
        (*status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn invoke(method: &[u8], args: &[u8]) -> (u8, String) {
        let (m, a) = (host_buffer(method), host_buffer(args));
        // SAFETY: both ranges were just filled from `kc_alloc` buffers of these lengths.
        let result = take_result(unsafe { kc_invoke(m, method.len(), a, args.len()) });
        // SAFETY: the host frees its argument buffers with their lengths.
        unsafe {
            kc_free(m, method.len());
            kc_free(a, args.len());
        }
        result
    }

    #[test]
    fn allocates_and_frees() {
        let empty = kc_alloc(0);
        assert!(!empty.is_null());
        // SAFETY: a zero-length buffer from `kc_alloc(0)`; null is a no-op.
        unsafe {
            kc_free(empty, 0);
            kc_free(std::ptr::null_mut(), 0);
        }
        assert!(kc_alloc(MAX_ALLOC + 1).is_null());
    }

    #[test]
    fn answers_results_and_errors() {
        assert_eq!(
            invoke(b"pathOf", br#"[["e2e4","e7e5"]]"#),
            (STATUS_OK, r#""e2e4 e7e5""#.into())
        );
        let (status, _) = invoke(b"nope", b"[]");
        assert_eq!(status, STATUS_ERR);
        assert_eq!(
            invoke(b"pathOf", b"\xff"),
            (STATUS_ERR, "arguments must be UTF-8".into())
        );
    }

    #[test]
    fn rejects_null_and_oversized_ranges_without_reading_them() {
        let one = host_buffer(b"x");
        // SAFETY: null and oversized ranges are rejected before any read.
        let (null, oversized) = unsafe {
            (
                kc_invoke(std::ptr::null(), 0, one, 1),
                kc_invoke(one, MAX_ARGS + 1, one, 1),
            )
        };
        assert_eq!(take_result(null), (STATUS_ERR, "null argument".into()));
        assert_eq!(
            take_result(oversized),
            (STATUS_ERR, "arguments too large".into())
        );
        // SAFETY: the host's own buffer, freed once.
        unsafe { kc_free(one, 1) };
    }
}
