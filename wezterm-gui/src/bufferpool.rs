// Buffer pooling to reduce GPU allocation overhead
//
// This module implements buffer pooling for vertex buffers to avoid
// expensive GPU memory allocations during window resizes and other
// dynamic operations. Instead of allocating fresh buffers every time,
// we reuse buffers from a pool, similar to Zed's approach.

use crate::renderstate::{RenderContext, VertexBuffer};
use std::cell::RefCell;

/// A pool of vertex buffers that can be reused to avoid allocations
pub struct VertexBufferPool {
    context: RenderContext,
    /// Available buffers, sorted by capacity (largest first)
    available: RefCell<Vec<(usize, VertexBuffer)>>,
    /// Statistics
    allocations: RefCell<usize>,
    reuses: RefCell<usize>,
}

impl VertexBufferPool {
    /// Create a new buffer pool
    pub fn new(context: &RenderContext) -> Self {
        Self {
            context: context.clone(),
            available: RefCell::new(Vec::new()),
            allocations: RefCell::new(0),
            reuses: RefCell::new(0),
        }
    }

    /// Acquire a buffer with at least the specified capacity
    /// 
    /// This will try to reuse an existing buffer from the pool if one is available
    /// with sufficient capacity. If not, it will allocate a new buffer with capacity
    /// rounded up to the next power of two for better reuse.
    pub fn acquire(&self, min_quads: usize) -> anyhow::Result<(usize, VertexBuffer)> {
        let mut available = self.available.borrow_mut();

        // Try to find a buffer with sufficient capacity
        if let Some(pos) = available.iter().position(|(cap, _)| *cap >= min_quads) {
            let (capacity, buffer) = available.swap_remove(pos);
            *self.reuses.borrow_mut() += 1;
            
            log::trace!(
                "Buffer pool: reused buffer with capacity {} for request {}",
                capacity,
                min_quads
            );
            
            return Ok((capacity, buffer));
        }

        // No suitable buffer found - allocate a new one
        // Round up to next power of two for better reuse
        let capacity = min_quads.next_power_of_two().max(32);
        
        let initializer = self.context.allocate_vertex_buffer_initializer(capacity);
        let buffer = self.context.allocate_vertex_buffer(capacity, &initializer)?;
        
        *self.allocations.borrow_mut() += 1;
        
        log::debug!(
            "Buffer pool: allocated new buffer with capacity {} for request {} (allocations: {}, reuses: {})",
            capacity,
            min_quads,
            self.allocations.borrow(),
            self.reuses.borrow()
        );
        
        Ok((capacity, buffer))
    }

    /// Release a buffer back to the pool for reuse
    /// 
    /// Buffers are kept in the pool up to a maximum count to avoid
    /// holding onto too much memory.
    pub fn release(&self, capacity: usize, buffer: VertexBuffer) {
        const MAX_POOLED_BUFFERS: usize = 8;
        
        let mut available = self.available.borrow_mut();
        
        if available.len() < MAX_POOLED_BUFFERS {
            // Insert sorted by capacity (largest first) for better reuse
            let pos = available.partition_point(|(cap, _)| *cap >= capacity);
            available.insert(pos, (capacity, buffer));
            
            log::trace!(
                "Buffer pool: released buffer with capacity {} (pool size: {})",
                capacity,
                available.len()
            );
        } else {
            log::trace!(
                "Buffer pool: discarded buffer with capacity {} (pool full at {})",
                capacity,
                available.len()
            );
        }
    }

    /// Get statistics about buffer pool usage
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            *self.allocations.borrow(),
            *self.reuses.borrow(),
            self.available.borrow().len(),
        )
    }

    /// Clear all buffers from the pool
    pub fn clear(&self) {
        self.available.borrow_mut().clear();
        log::debug!("Buffer pool: cleared all buffers");
    }
}

#[cfg(test)]
mod tests {
    // Note: These tests would require a real RenderContext which needs OpenGL/WebGPU
    // For now, we'll document the expected behavior

    #[test]
    fn test_buffer_pool_stats() {
        // This test would verify that:
        // 1. First acquire() increments allocations
        // 2. release() adds buffer to pool
        // 3. Second acquire() increments reuses
        // 4. stats() returns correct counts
    }

    #[test]
    fn test_buffer_pool_capacity_rounding() {
        // This test would verify that:
        // 1. Requesting 100 quads allocates 128 (next power of two)
        // 2. Requesting 33 quads reuses the 128 buffer
        // 3. Requesting 200 quads allocates 256
    }

    #[test]
    fn test_buffer_pool_max_size() {
        // This test would verify that:
        // 1. Pool keeps at most MAX_POOLED_BUFFERS buffers
        // 2. Additional buffers are discarded
    }
}

