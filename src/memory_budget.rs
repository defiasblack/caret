//! Track live Rust allocations so the isolated Office parser can fail before
//! allocating beyond its budget. The editor itself has no allocation limit.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

pub struct BudgetAllocator {
    used: AtomicUsize,
    limit: AtomicUsize,
}

impl BudgetAllocator {
    pub const fn new() -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit: AtomicUsize::new(usize::MAX),
        }
    }

    pub fn set_limit(&self, bytes: usize) {
        self.limit.store(bytes, Ordering::SeqCst);
    }

    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= self.limit.load(Ordering::Relaxed))
            })
            .is_ok()
    }
}

// Every pointer/layout pair is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for BudgetAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !self.reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(pointer, layout);
        }
        self.used.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let growth = size.saturating_sub(layout.size());
        if !self.reserve(growth) {
            return std::ptr::null_mut();
        }
        let result = unsafe { System.realloc(pointer, layout, size) };
        if result.is_null() {
            self.used.fetch_sub(growth, Ordering::Relaxed);
        } else {
            self.used
                .fetch_sub(layout.size().saturating_sub(size), Ordering::Relaxed);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_rejects_allocations_before_system_allocation() {
        let allocator = BudgetAllocator::new();
        allocator.set_limit(16);
        assert!(allocator.reserve(16));
        assert!(!allocator.reserve(1));
        assert!(!allocator.reserve(usize::MAX));
        allocator.used.fetch_sub(8, Ordering::Relaxed);
        assert!(allocator.reserve(8));
    }
}
