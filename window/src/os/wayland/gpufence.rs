// Phase 17.2: GPU Fence Implementation for Wayland
// 
// This module implements EGL sync fences to prevent GPU queue overflow
// and reduce GPU stalls during high-frequency operations like resize.
//
// References:
// - EGL_KHR_fence_sync extension
// - Chrome: ui/ozone/platform/wayland/gpu/wayland_surface_gpu.cc
// - https://www.khronos.org/registry/EGL/extensions/KHR/EGL_KHR_fence_sync.txt

use crate::egl::ffi;
use anyhow::{anyhow, Result};
use std::ptr;
use std::time::{Duration, Instant};

/// GPU fence for synchronizing CPU and GPU work
/// 
/// This prevents over-submission of GPU commands by allowing the CPU
/// to wait for the GPU to finish processing a frame before submitting
/// the next one.
pub struct GpuFence {
    sync: ffi::types::EGLSync,
    display: ffi::types::EGLDisplay,
    egl: *const ffi::Egl,
    created_at: Instant,
}

impl GpuFence {
    /// Create a new GPU fence
    /// 
    /// This should be called after submitting a frame to the GPU.
    /// The fence will be signaled when the GPU completes all commands
    /// submitted before the fence.
    pub fn create(
        egl: *const ffi::Egl,
        display: ffi::types::EGLDisplay,
    ) -> Result<Self> {
        unsafe {
            let egl_ref = &*egl;
            
            // Create EGL sync fence
            // EGL_SYNC_FENCE_KHR = 0x30F9
            let sync = egl_ref.CreateSync(
                display,
                0x30F9, // EGL_SYNC_FENCE_KHR
                ptr::null(),
            );
            
            if sync == ffi::NO_SYNC {
                return Err(anyhow!(
                    "Failed to create EGL sync fence (error: 0x{:x})",
                    egl_ref.GetError()
                ));
            }
            
            Ok(Self {
                sync,
                display,
                egl,
                created_at: Instant::now(),
            })
        }
    }
    
    /// Wait for the fence to be signaled (GPU work complete)
    /// 
    /// Returns true if the fence was signaled within the timeout,
    /// false if the timeout expired.
    /// 
    /// A timeout of 0 means check status without blocking.
    /// A timeout of u64::MAX means wait indefinitely.
    pub fn wait(&self, timeout: Duration) -> bool {
        unsafe {
            let egl_ref = &*self.egl;
            let timeout_ns = timeout.as_nanos() as u64;
            
            // EGL_SYNC_FLUSH_COMMANDS_BIT_KHR = 0x0001
            let result = egl_ref.ClientWaitSync(
                self.display,
                self.sync,
                0x0001, // EGL_SYNC_FLUSH_COMMANDS_BIT_KHR
                timeout_ns,
            );
            
            // EGL_CONDITION_SATISFIED_KHR = 0x30F6
            // EGL_TIMEOUT_EXPIRED_KHR = 0x30F5
            match result {
                0x30F6 => true,  // Signaled
                0x30F5 => false, // Timeout
                _ => {
                    log::warn!(
                        "EGL sync wait returned unexpected status: 0x{:x} (error: 0x{:x})",
                        result,
                        egl_ref.GetError()
                    );
                    false
                }
            }
        }
    }
    
    /// Check if the fence is signaled (non-blocking)
    pub fn is_signaled(&self) -> bool {
        self.wait(Duration::from_nanos(0))
    }
    
    /// Get the age of this fence
    pub fn age(&self) -> Duration {
        Instant::now().duration_since(self.created_at)
    }
}

impl Drop for GpuFence {
    fn drop(&mut self) {
        unsafe {
            let egl_ref = &*self.egl;
            if egl_ref.DestroySync(self.display, self.sync) == ffi::FALSE {
                log::warn!(
                    "Failed to destroy EGL sync fence (error: 0x{:x})",
                    egl_ref.GetError()
                );
            }
        }
    }
}

/// Manager for GPU fences with rate limiting and diagnostics
/// 
/// This tracks pending fences and provides statistics on GPU sync behavior.
pub struct GpuFenceManager {
    /// The most recent fence (if any)
    pending_fence: Option<GpuFence>,
    
    /// Statistics
    total_fences_created: usize,
    total_waits: usize,
    total_timeouts: usize,
    total_wait_time: Duration,
    max_wait_time: Duration,
    
    /// For rate-limited logging
    last_timeout_log: Instant,
    last_stats_log: Instant,
}

impl GpuFenceManager {
    pub fn new() -> Self {
        Self {
            pending_fence: None,
            total_fences_created: 0,
            total_waits: 0,
            total_timeouts: 0,
            total_wait_time: Duration::ZERO,
            max_wait_time: Duration::ZERO,
            last_timeout_log: Instant::now(),
            last_stats_log: Instant::now(),
        }
    }
    
    /// Create a new fence, replacing any pending fence
    /// 
    /// If there's already a pending fence, it will be dropped (and its
    /// destructor will clean up the EGL resources).
    pub fn create_fence(
        &mut self,
        egl: *const ffi::Egl,
        display: ffi::types::EGLDisplay,
    ) -> Result<()> {
        match GpuFence::create(egl, display) {
            Ok(fence) => {
                self.pending_fence = Some(fence);
                self.total_fences_created += 1;
                Ok(())
            }
            Err(e) => {
                log::warn!("Failed to create GPU fence: {}", e);
                Err(e)
            }
        }
    }
    
    /// Wait for the pending fence (if any) with a timeout
    /// 
    /// Returns:
    /// - None if there's no pending fence
    /// - Some(true) if the fence was signaled
    /// - Some(false) if the timeout expired
    pub fn wait_for_fence(&mut self, timeout: Duration) -> Option<bool> {
        let fence = self.pending_fence.take()?;
        
        let start = Instant::now();
        let signaled = fence.wait(timeout);
        let wait_time = start.elapsed();
        
        // Update statistics
        self.total_waits += 1;
        self.total_wait_time += wait_time;
        if wait_time > self.max_wait_time {
            self.max_wait_time = wait_time;
        }
        
        if !signaled {
            self.total_timeouts += 1;
            
            // Rate-limited warning
            if self.last_timeout_log.elapsed() > Duration::from_secs(5) {
                log::warn!(
                    "GPU fence timeout after {:?} (fence age: {:?})",
                    timeout,
                    fence.age()
                );
                self.last_timeout_log = Instant::now();
            }
        }
        
        // Periodic statistics logging
        if self.last_stats_log.elapsed() > Duration::from_secs(60) {
            self.log_stats();
            self.last_stats_log = Instant::now();
        }
        
        Some(signaled)
    }
    
    /// Check if there's a pending fence and if it's signaled
    pub fn is_fence_signaled(&self) -> Option<bool> {
        self.pending_fence.as_ref().map(|f| f.is_signaled())
    }
    
    /// Log statistics about GPU fence usage
    pub fn log_stats(&self) {
        if self.total_waits == 0 {
            return;
        }
        
        let avg_wait = self.total_wait_time / self.total_waits as u32;
        let timeout_rate = (self.total_timeouts as f64 / self.total_waits as f64) * 100.0;
        
        log::info!(
            "GPU Fence Stats: {} fences, {} waits, avg wait: {:?}, max wait: {:?}, timeout rate: {:.1}%",
            self.total_fences_created,
            self.total_waits,
            avg_wait,
            self.max_wait_time,
            timeout_rate
        );
    }
}

impl Default for GpuFenceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fence_manager_creation() {
        let manager = GpuFenceManager::new();
        assert_eq!(manager.total_fences_created, 0);
        assert_eq!(manager.total_waits, 0);
    }
}

