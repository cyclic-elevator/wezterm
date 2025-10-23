// Phase 17.3: wp_presentation_time Protocol Support
//
// This module implements the Wayland presentation-time protocol for precise
// vsync alignment and timing feedback.
//
// References:
// - https://wayland.app/protocols/presentation-time
// - https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/presentation-time/presentation-time.xml
// - Chrome: ui/ozone/platform/wayland/host/wayland_frame_manager.cc

use std::time::{Duration, Instant};

/// Presentation timing feedback from the compositor
/// 
/// This tracks when frames actually hit the screen and helps predict
/// the next vsync for optimal frame timing.
#[derive(Debug, Clone)]
pub struct PresentationFeedback {
    /// When this frame was presented to the display
    pub present_time: Instant,
    
    /// The refresh interval of the display (e.g., 16.67ms for 60Hz)
    pub refresh_interval: Duration,
    
    /// Presentation flags
    pub flags: PresentationFlags,
}

bitflags::bitflags! {
    /// Flags from wp_presentation_feedback
    pub struct PresentationFlags: u32 {
        /// The presentation was synchronized to the "vertical retrace" by the display hardware
        const VSYNC = 0x1;

        /// The display hardware provided measurements that the hardware driver converted into a presentation timestamp
        const HW_CLOCK = 0x2;

        /// The display hardware signaled that it started using the new image content
        const HW_COMPLETION = 0x4;

        /// The presentation of this update was done zero-copy
        const ZERO_COPY = 0x8;
    }
}

impl PresentationFeedback {
    /// Predict the next vsync time based on this feedback
    pub fn predict_next_vsync(&self) -> Instant {
        let elapsed = Instant::now().duration_since(self.present_time);
        let intervals_passed = (elapsed.as_nanos() / self.refresh_interval.as_nanos()) as u32;
        self.present_time + self.refresh_interval * (intervals_passed + 1)
    }
    
    /// Get the optimal time to start rendering the next frame
    ///
    /// This is typically a few milliseconds before the next vsync to account for
    /// rendering time and compositor latency.
    pub fn optimal_render_start(&self, render_time_budget: Duration) -> Instant {
        let next_vsync = self.predict_next_vsync();
        if next_vsync > Instant::now() + render_time_budget {
            next_vsync - render_time_budget
        } else {
            next_vsync
        }
    }
}

/// Manager for presentation timing
/// 
/// This tracks feedback from the compositor and provides timing predictions
/// for optimal frame pacing.
pub struct PresentationManager {
    /// Most recent presentation feedback
    last_feedback: Option<PresentationFeedback>,
    
    /// Estimated refresh rate (fallback if no feedback received)
    estimated_refresh_interval: Duration,
    
    /// Statistics
    total_feedbacks: usize,
    vsync_hits: usize,
    zero_copy_frames: usize,
    
    /// For rate-limited logging
    last_stats_log: Instant,
}

impl PresentationManager {
    /// Create a new presentation manager
    /// 
    /// The default refresh interval is used as a fallback if no feedback is received.
    /// Typically 16.67ms for 60Hz displays.
    pub fn new(default_refresh_hz: u32) -> Self {
        let estimated_refresh_interval = Duration::from_nanos(1_000_000_000 / default_refresh_hz as u64);
        
        Self {
            last_feedback: None,
            estimated_refresh_interval,
            total_feedbacks: 0,
            vsync_hits: 0,
            zero_copy_frames: 0,
            last_stats_log: Instant::now(),
        }
    }
    
    /// Record new presentation feedback from the compositor
    pub fn record_feedback(&mut self, feedback: PresentationFeedback) {
        self.total_feedbacks += 1;
        
        if feedback.flags.contains(PresentationFlags::VSYNC) {
            self.vsync_hits += 1;
        }
        
        if feedback.flags.contains(PresentationFlags::ZERO_COPY) {
            self.zero_copy_frames += 1;
        }
        
        // Update refresh interval estimate
        if let Some(last) = &self.last_feedback {
            let interval = feedback.present_time.duration_since(last.present_time);
            // Use exponential moving average to smooth out jitter
            self.estimated_refresh_interval = self.estimated_refresh_interval.mul_f64(0.9)
                + interval.mul_f64(0.1);
        }
        
        self.last_feedback = Some(feedback);
        
        // Periodic logging
        if self.last_stats_log.elapsed() > Duration::from_secs(60) {
            self.log_stats();
            self.last_stats_log = Instant::now();
        }
    }
    
    /// Predict the next vsync time
    /// 
    /// If we have recent feedback, use that. Otherwise, use the estimated refresh interval.
    pub fn predict_next_vsync(&self) -> Instant {
        if let Some(feedback) = &self.last_feedback {
            feedback.predict_next_vsync()
        } else {
            // No feedback yet - use estimated interval
            Instant::now() + self.estimated_refresh_interval
        }
    }
    
    /// Get the optimal time to start rendering the next frame
    /// 
    /// This accounts for expected rendering time and compositor latency.
    pub fn optimal_render_start(&self, render_time_budget: Duration) -> Instant {
        if let Some(feedback) = &self.last_feedback {
            feedback.optimal_render_start(render_time_budget)
        } else {
            // No feedback - start rendering immediately
            Instant::now()
        }
    }
    
    /// Check if we should start rendering the next frame
    /// 
    /// Returns true if we're at or past the optimal render start time.
    pub fn should_render_now(&self, render_time_budget: Duration) -> bool {
        Instant::now() >= self.optimal_render_start(render_time_budget)
    }
    
    /// Get the current refresh interval estimate
    pub fn refresh_interval(&self) -> Duration {
        self.estimated_refresh_interval
    }
    
    /// Get the estimated refresh rate in Hz
    pub fn refresh_rate_hz(&self) -> f64 {
        1_000_000_000.0 / self.estimated_refresh_interval.as_nanos() as f64
    }
    
    /// Log statistics about presentation timing
    pub fn log_stats(&self) {
        if self.total_feedbacks == 0 {
            log::info!("Presentation Stats: No feedback received yet");
            return;
        }
        
        let vsync_rate = (self.vsync_hits as f64 / self.total_feedbacks as f64) * 100.0;
        let zero_copy_rate = (self.zero_copy_frames as f64 / self.total_feedbacks as f64) * 100.0;
        
        log::info!(
            "Presentation Stats: {} feedbacks, refresh: {:.1} Hz, vsync: {:.1}%, zero-copy: {:.1}%",
            self.total_feedbacks,
            self.refresh_rate_hz(),
            vsync_rate,
            zero_copy_rate
        );
    }
}

impl Default for PresentationManager {
    fn default() -> Self {
        Self::new(60) // Assume 60Hz by default
    }
}

// TODO: Integration steps for wp_presentation_time:
// 
// 1. Add to WaylandState in state.rs:
//    ```rust
//    pub(super) presentation: Option<PresentationState>,
//    ```
//
// 2. Bind the global in connection.rs:
//    ```rust
//    use wayland_protocols::wp::presentation_time::client::*;
//    
//    // In global handler:
//    if interface == "wp_presentation" {
//        let presentation = registry.bind::<WpPresentation, _, _>(
//            name,
//            version.min(1),
//            &qh,
//            (),
//        );
//        state.presentation = Some(presentation);
//    }
//    ```
//
// 3. Add to WaylandWindowInner in window.rs:
//    ```rust
//    presentation_manager: RefCell<PresentationManager>,
//    ```
//
// 4. Request feedback in do_paint():
//    ```rust
//    if let Some(presentation) = &conn.wayland_state.borrow().presentation {
//        let feedback = presentation.feedback(&qh, self.surface());
//        // Store feedback callback for handling
//    }
//    ```
//
// 5. Implement wp_presentation_feedback handler:
//    ```rust
//    impl Dispatch<WpPresentationFeedback, ()> for WaylandState {
//        fn event(
//            state: &mut Self,
//            feedback: &WpPresentationFeedback,
//            event: <WpPresentationFeedback as Proxy>::Event,
//            data: &(),
//            conn: &Connection,
//            qh: &QueueHandle<Self>,
//        ) {
//            match event {
//                wp_presentation_feedback::Event::Presented { ... } => {
//                    // Create PresentationFeedback and record it
//                    let feedback = PresentationFeedback { ... };
//                    window.presentation_manager.borrow_mut().record_feedback(feedback);
//                }
//                _ => {}
//            }
//        }
//    }
//    ```
//
// 6. Use timing predictions in do_paint():
//    ```rust
//    let manager = self.presentation_manager.borrow();
//    if !manager.should_render_now(Duration::from_millis(8)) {
//        // Too early - defer the paint
//        self.invalidated = true;
//        return Ok(());
//    }
//    ```

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_presentation_manager_creation() {
        let manager = PresentationManager::new(60);
        assert_eq!(manager.total_feedbacks, 0);
        assert!((manager.refresh_rate_hz() - 60.0).abs() < 0.1);
    }
    
    #[test]
    fn test_vsync_prediction() {
        let mut manager = PresentationManager::new(60);
        
        let feedback = PresentationFeedback {
            present_time: Instant::now(),
            refresh_interval: Duration::from_millis(16),
            flags: PresentationFlags::VSYNC,
        };
        
        manager.record_feedback(feedback.clone());
        
        let next_vsync = manager.predict_next_vsync();
        let elapsed = next_vsync.duration_since(feedback.present_time);
        
        // Should predict the next frame (within 1-2 intervals)
        assert!(elapsed > Duration::from_millis(1));
        assert!(elapsed < Duration::from_millis(50));
    }
}

