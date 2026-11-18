//! An emergency memory reserve: one contiguous block set aside while memory allows. When an
//! allocation fails, the allocator frees it and tries again, so a short burst (the renderer's
//! scene list doubling while box art fills the heap, a notification, ...) succeeds instead of
//! aborting the firmware. `rearm` (from the UI loop) sets it aside again once there's room.
//!
//! Only Rust allocations go through this; ESP-IDF's own (FatFs, Wi-Fi) don't.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// Size of the reserve: enough for the renderer's scene list to double once more.
pub const SIZE: usize = 16 * 1024;
/// Only set the reserve aside when the largest free block is at least this big.
const REARM_LARGEST: usize = SIZE + 16 * 1024;

static RESERVE: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
/// How many times the reserve was used (for the log).
static USED: AtomicUsize = AtomicUsize::new(0);

const LAYOUT: Layout = match Layout::from_size_align(SIZE, 4) {
    Ok(l) => l,
    Err(_) => panic!(),
};

pub struct ReserveAlloc;

#[global_allocator]
static ALLOCATOR: ReserveAlloc = ReserveAlloc;

impl ReserveAlloc {
    /// Free the reserve, if it's set aside. Returns whether it was.
    fn release() -> bool {
        let p = RESERVE.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if p.is_null() {
            return false;
        }
        unsafe { System.dealloc(p, LAYOUT) };
        USED.fetch_add(1, Ordering::Relaxed);
        true
    }
}

unsafe impl GlobalAlloc for ReserveAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if p.is_null() && Self::release() {
            return unsafe { System.alloc(layout) };
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if p.is_null() && Self::release() {
            return unsafe { System.alloc_zeroed(layout) };
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if p.is_null() && Self::release() {
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        p
    }
}

/// Whether the reserve is set aside (an allocation burst of up to SIZE will succeed).
pub fn armed() -> bool {
    !RESERVE.load(Ordering::Acquire).is_null()
}

/// Set the reserve aside if it isn't and there's room; log when it was used since.
pub fn rearm() {
    if armed() {
        return;
    }
    let largest = unsafe { esp_idf_svc::sys::heap_caps_get_largest_free_block(esp_idf_svc::sys::MALLOC_CAP_8BIT) };
    if largest < REARM_LARGEST {
        return;
    }
    let p = unsafe { System.alloc(LAYOUT) };
    if p.is_null() {
        return;
    }
    if RESERVE.compare_exchange(std::ptr::null_mut(), p, Ordering::AcqRel, Ordering::Acquire).is_err() {
        unsafe { System.dealloc(p, LAYOUT) };
        return;
    }
    let used = USED.swap(0, Ordering::Relaxed);
    if used > 0 {
        log::warn!("Memory reserve was used {used} time(s); set aside again");
    }
}
