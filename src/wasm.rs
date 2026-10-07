//! The WebAssembly entry point, which powers the playground on the
//! documentation site.
//!
//! This is a plain C ABI rather than `wasm-bindgen`, so the whole toolchain is
//! `cargo build --target wasm32-unknown-unknown` and about forty lines of
//! JavaScript. There is nothing to install.
//!
//! **The calling convention.** Strings cross the boundary as UTF-8 in linear
//! memory, length-prefixed by a little-endian `u32`:
//!
//! ```text
//!   let p = alloc(bytes.length)        JS writes the source at p
//!   let r = run(p, bytes.length)       Rust answers at r
//!   let n = u32 at r                   the answer's length
//!   let json = bytes at r+4 .. r+4+n   UTF-8 JSON
//!   dealloc(r, n + 4)                  JS gives the memory back
//! ```
//!
//! `run` answers with `{"transcript": …, "passed": n, "total": n, "ok": bool}`.
//!
//! Only scripted mode exists here: wasm has no sockets, no threads and no
//! clock, so `live` isn't compiled in (`lib.rs`). That is exactly what the
//! playground wants — every example is deterministic, free, and needs no API
//! key.

use crate::driver::Interp;
use std::path::Path;

/// Hand JavaScript `len` bytes of our linear memory to write into.
///
/// # Safety
/// The caller must eventually pass the pointer back to `dealloc` with the same
/// length, and must not write beyond it.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Take back a buffer from `alloc`, or a result from `run`.
///
/// # Safety
/// `ptr` must have come from `alloc` or `run`, with `len` its exact length,
/// and must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
    }
}

/// Load a µNorman program and run its unit tests, as `norman FILE.nrm` does.
///
/// # Safety
/// `ptr` must point to `len` bytes of UTF-8 that `alloc` returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn run(ptr: *mut u8, len: usize) -> *mut u8 {
    let source = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(source).into_owned();
    unsafe { dealloc(ptr, len) };
    reply(&evaluate(&source))
}

/// The result of one program, as JSON.
fn evaluate(source: &str) -> serde_json::Value {
    let mut interp = Interp::new();
    // `Interp::new` loads the prelude, whose own (absent) tests would otherwise
    // head the transcript.
    interp.clear_transcript();

    match interp.load_str(source, "playground.nrm", Path::new(".")) {
        Ok(summary) => serde_json::json!({
            "transcript": interp.transcript(),
            "passed": summary.passed,
            "total": summary.total,
            "ok": summary.all_passed(),
        }),
        // A file that can't be read at all: the lexer failed, so nothing ran.
        Err(msg) => serde_json::json!({
            "transcript": format!("{}\n{}", interp.transcript(), msg),
            "passed": 0,
            "total": 0,
            "ok": false,
        }),
    }
}

/// Lay a JSON value out as `[u32 length][UTF-8 bytes]` and hand it to JS.
fn reply(value: &serde_json::Value) -> *mut u8 {
    let body = value.to_string().into_bytes();
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    let ptr = out.as_mut_ptr();
    std::mem::forget(out);
    ptr
}
