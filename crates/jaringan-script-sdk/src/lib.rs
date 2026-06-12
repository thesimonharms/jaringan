#![no_std]
extern crate alloc;

pub use serde_json;

use alloc::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use core::panic::PanicInfo;

// ── Panic handler for WASM targets ──────────────────────────────────

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

// ── Bump allocator for WASM ─────────────────────────────────────────

/// A minimal bump allocator for use on `wasm32-unknown-unknown`.
/// Allocates from a static 64 KB heap region.
struct BumpAllocator {
    heap: AtomicUsize,
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1);
        let start = self.heap.fetch_add(size, Ordering::SeqCst);
        if start + size > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        HEAP.as_ptr().add(start) as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator never frees.
    }
}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator {
    heap: AtomicUsize::new(0),
};

const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

// ── Host function declarations ──────────────────────────────────────

#[link(wasm_import_module = "jaringan")]
extern "C" {
    fn fetch(ptr: i32, len: i32) -> i32;
    fn log(level_ptr: i32, level_len: i32, msg_ptr: i32, msg_len: i32);
    fn navigate(ptr: i32, len: i32) -> i32;
}

// ── Memory constants ────────────────────────────────────────────────

/// Scratch area for temporary data (URLs, intermediate strings).
const SCRATCH: usize = 16384;
/// Standard output buffer location.
const OUTPUT: usize = 65536;

// ── Entry point macro ───────────────────────────────────────────────

/// Generate the `process(i32, i32) -> i32` WASM entry point.
///
/// Your handler receives the input JSON string and must return the output
/// JSON string.
///
/// ```ignore
/// fn my_handler(input: &str) -> String {
///     // ... parse, transform, return ...
/// }
/// jaring_script_sdk::export_process!(my_handler);
/// ```
#[macro_export]
macro_rules! export_process {
    ($handler:path) => {
        #[no_mangle]
        pub extern "C" fn process(input_ptr: i32, input_len: i32) -> i32 {
            let input = $crate::read_input(input_ptr, input_len);
            let output = $handler(&input);
            $crate::write_output(&output);
            65536i32
        }
    };
}

// ── Input / output helpers ──────────────────────────────────────────

/// Read a string from WASM linear memory at the given offset + length.
pub fn read_input(ptr: i32, len: i32) -> String {
    if len <= 0 {
        return String::new();
    }
    let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    alloc::str::from_utf8(slice).unwrap_or_default().to_owned()
}

/// Write a string to the standard output buffer (offset 65536).
/// Format: 4‑byte LE length + UTF‑8 body.
pub fn write_output(json: &str) {
    let bytes = json.as_bytes();
    let len = bytes.len() as u32;
    unsafe {
        core::ptr::write_unaligned(OUTPUT as *mut u32, len);
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), (OUTPUT + 4) as *mut u8, bytes.len());
    }
}

// ── Host function wrappers ──────────────────────────────────────────

/// Fetch a URL via `jaringan.fetch`.
///
/// The returned string is the JSON body written by the host (containing
/// `status`, `content_type`, `body`, and `error` fields).
pub fn js_fetch(url: &str) -> Result<String, String> {
    let url_bytes = url.as_bytes();
    // Write URL to scratch zone
    unsafe {
        core::ptr::copy_nonoverlapping(url_bytes.as_ptr(), SCRATCH as *mut u8, url_bytes.len());
    }
    // Call host — reads from SCRATCH, writes result to OUTPUT
    let _result_ptr = unsafe { fetch(SCRATCH as i32, url_bytes.len() as i32) };
    // Read result from OUTPUT (4‑byte LE length + body)
    let result = unsafe {
        let len = core::ptr::read_unaligned(OUTPUT as *const u32);
        let slice = core::slice::from_raw_parts((OUTPUT + 4) as *const u8, len as usize);
        let v: Vec<u8> = slice.to_vec();
        String::from_utf8(v).unwrap_or_default()
    };
    Ok(result)
}

/// Log a message via `jaringan.log`.
pub fn js_log(level: &str, msg: &str) {
    unsafe {
        log(
            level.as_ptr() as i32,
            level.len() as i32,
            msg.as_ptr() as i32,
            msg.len() as i32,
        );
    }
}

/// Navigate to a URL via `jaringan.navigate`.
pub fn js_navigate(url: &str) -> Result<String, String> {
    let url_bytes = url.as_bytes();
    unsafe {
        core::ptr::copy_nonoverlapping(url_bytes.as_ptr(), SCRATCH as *mut u8, url_bytes.len());
    }
    let _result_ptr = unsafe { navigate(SCRATCH as i32, url_bytes.len() as i32) };
    let result = unsafe {
        let len = core::ptr::read_unaligned(OUTPUT as *const u32);
        let slice = core::slice::from_raw_parts((OUTPUT + 4) as *const u8, len as usize);
        let v: Vec<u8> = slice.to_vec();
        String::from_utf8(v).unwrap_or_default()
    };
    Ok(result)
}
