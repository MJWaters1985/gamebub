use std::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

/// A buffer for bulk transfers (FPGA programming, file loading and saving), used by one task
/// at a time.
///
/// It's allocated when taken rather than reserved permanently: internal RAM is scarce. If
/// there's no free block of N bytes, a smaller buffer is used (down to 1 KiB): users work in
/// chunks of the buffer's length, so this only makes them slower.
pub struct ScratchBuffer<const N: usize> {
    used: AtomicBool,
}

impl<const N: usize> ScratchBuffer<N> {
    pub const fn new() -> Self {
        ScratchBuffer {
            used: AtomicBool::new(false),
        }
    }

    pub fn take<'a>(&'a self) -> Option<ScratchBufferHandle<'a>> {
        if self.used.swap(true, Ordering::SeqCst) {
            return None;
        }
        let mut size = N;
        let data = loop {
            let mut data = Vec::new();
            if data.try_reserve_exact(size).is_ok() || size <= 1024 {
                data.resize(size, 0);
                break data;
            }
            size /= 2;
        };
        if size < N {
            log::warn!("Scratch buffer: only {size} bytes available");
        }
        Some(ScratchBufferHandle {
            data,
            used: &self.used,
        })
    }
}

pub struct ScratchBufferHandle<'a> {
    data: Vec<u8>,
    used: &'a AtomicBool,
}

impl Deref for ScratchBufferHandle<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for ScratchBufferHandle<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<'a> Drop for ScratchBufferHandle<'a> {
    fn drop(&mut self) {
        self.used.store(false, Ordering::SeqCst);
    }
}
