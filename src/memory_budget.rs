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
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = used
                .checked_add(bytes)
                .filter(|next| *next <= self.limit.load(Ordering::Relaxed))
            else {
                return false;
            };
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return true,
                Err(current) => used = current,
            }
        }
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
        // A system realloc may allocate the new block before freeing the old
        // one. Reserve that peak before calling it, even if it can grow in place.
        if !self.reserve(size) {
            return std::ptr::null_mut();
        }
        let result = unsafe { System.realloc(pointer, layout, size) };
        self.used.fetch_sub(
            if result.is_null() {
                size
            } else {
                layout.size()
            },
            Ordering::Relaxed,
        );
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
    #[test]
    fn realloc_reserves_the_peak_and_preserves_the_old_block_on_failure() {
        let allocator = BudgetAllocator::new();
        allocator.set_limit(16);
        let layout = Layout::from_size_align(8, 8).unwrap();
        // SAFETY: all operations use the same allocator and original layout;
        // the failed realloc leaves the original allocation owned by this test.
        unsafe {
            let pointer = allocator.alloc(layout);
            assert!(!pointer.is_null());
            assert!(allocator.realloc(pointer, layout, 12).is_null());
            assert_eq!(allocator.used.load(Ordering::Relaxed), 8);
            allocator.dealloc(pointer, layout);
        }
        assert_eq!(allocator.used.load(Ordering::Relaxed), 0);
    }
}
