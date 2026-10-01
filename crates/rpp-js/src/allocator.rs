use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use deno_core::v8;

struct Budget {
    limit: usize,
    used: AtomicUsize,
}

/// V8 retains the allocator with its backing stores, including stores freed by GC. A refused
/// allocation is not final: V8 collects garbage and retries, then throws a `RangeError`.
pub(crate) fn bounded(limit: usize) -> v8::UniqueRef<v8::Allocator> {
    const VTABLE: v8::RustAllocatorVtable<Budget> = v8::RustAllocatorVtable {
        allocate,
        allocate_uninitialized: allocate,
        free,
        drop: drop_budget,
    };
    let budget = Box::new(Budget {
        limit,
        used: AtomicUsize::new(0),
    });
    // SAFETY: VTABLE receives this exact Budget until its drop callback reclaims
    // the Box. Atomic accounting supports V8's concurrent backing-store frees.
    unsafe { v8::new_rust_allocator(Box::into_raw(budget), &VTABLE) }
}

unsafe extern "C" fn allocate(budget: &Budget, len: usize) -> *mut c_void {
    let bytes = len.max(1);
    let Ok(layout) = Layout::from_size_align(bytes, 8) else {
        return ptr::null_mut();
    };
    if budget
        .used
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(bytes).filter(|next| *next <= budget.limit)
        })
        .is_err()
    {
        return ptr::null_mut();
    }
    // SAFETY: Layout is valid and nonempty. Eight-byte alignment covers every
    // supported typed-array element. Zeroing also satisfies AllocateUninitialized.
    let data = unsafe { alloc_zeroed(layout) };
    if data.is_null() {
        budget.used.fetch_sub(bytes, Ordering::AcqRel);
    }
    data.cast()
}

unsafe extern "C" fn free(budget: &Budget, data: *mut c_void, len: usize) {
    if data.is_null() {
        return;
    }
    let bytes = len.max(1);
    let Ok(layout) = Layout::from_size_align(bytes, 8) else {
        return;
    };
    // SAFETY: V8 supplies each pointer from allocate exactly once with its original
    // length. The layout matches that allocation, which is no longer in use.
    unsafe { dealloc(data.cast(), layout) };
    budget.used.fetch_sub(bytes, Ordering::AcqRel);
}

unsafe extern "C" fn drop_budget(budget: *const Budget) {
    // SAFETY: new_rust_allocator invokes this once, after its backing stores release
    // their allocator references. This pointer came from Box::into_raw above.
    drop(unsafe { Box::from_raw(budget.cast_mut()) });
}
