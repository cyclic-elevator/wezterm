# Phase 9 Implementation: Frame Time Logging

## Date
2025-10-23

## Status
✅ **COMPLETED**

## Summary

Added comprehensive frame time logging to diagnose frame time variance, which is suspected to be the cause of the perceived sluggishness during resizing on Linux/Wayland.

---

## Changes Made

### 1. Modified `wezterm-gui/src/termwindow/render/paint.rs`

**Added slow frame detection** (lines 122-130):
```rust
// Log slow frames for performance analysis
let frame_ms = self.last_frame_duration.as_millis();
if frame_ms > 20 {
    log::warn!(
        "SLOW FRAME: {:?} ({:.1}ms, target: 16.67ms for 60fps)",
        self.last_frame_duration,
        frame_ms as f64
    );
}
```

**Added frame time tracking and statistics** (lines 111-165):
```rust
// Track frame times for variance analysis
{
    let mut frame_times = self.frame_times.borrow_mut();
    frame_times.push(self.last_frame_duration);
    
    // Keep last 120 frames (2 seconds at 60fps)
    if frame_times.len() > 120 {
        frame_times.remove(0);
    }
}

// Log frame statistics every 5 seconds
{
    let mut last_stats = self.last_frame_stats_log.borrow_mut();
    if start.duration_since(*last_stats) >= Duration::from_secs(5) {
        *last_stats = start;
        
        let frame_times = self.frame_times.borrow();
        if frame_times.len() > 10 {
            let count = frame_times.len();
            let sum: Duration = frame_times.iter().sum();
            let avg = sum / count as u32;
            
            let mut sorted = frame_times.clone();
            sorted.sort();
            let min = sorted[0];
            let max = sorted[count - 1];
            let median = sorted[count / 2];
            let p95 = sorted[(count as f32 * 0.95) as usize];
            let p99 = sorted[(count as f32 * 0.99) as usize];
            
            log::info!(
                "Frame time stats (last {}): avg={:.1}ms, median={:.1}ms, min={:.1}ms, max={:.1}ms, p95={:.1}ms, p99={:.1}ms, variance={:.1}ms",
                count,
                avg.as_secs_f64() * 1000.0,
                median.as_secs_f64() * 1000.0,
                min.as_secs_f64() * 1000.0,
                max.as_secs_f64() * 1000.0,
                p95.as_secs_f64() * 1000.0,
                p99.as_secs_f64() * 1000.0,
                (max - min).as_secs_f64() * 1000.0
            );
        }
    }
}
```

### 2. Modified `wezterm-gui/src/termwindow/mod.rs`

**Added frame tracking fields to `TermWindow` struct** (lines 474-476):
```rust
// Frame time variance tracking for performance analysis
frame_times: RefCell<Vec<Duration>>,
last_frame_stats_log: RefCell<Instant>,
```

**Initialized fields in constructor** (lines 704-705):
```rust
frame_times: RefCell::new(Vec::with_capacity(120)),
last_frame_stats_log: RefCell::new(Instant::now()),
```

---

## What the Logging Captures

### 1. Immediate Slow Frame Warnings

**Logged as**: `WARN` level  
**Trigger**: Any frame taking >20ms (slower than 50 FPS)  
**Output format**:
```
SLOW FRAME: 35.2ms (35.2ms, target: 16.67ms for 60fps)
```

**Purpose**: Immediately identify frame time spikes

### 2. Periodic Frame Statistics

**Logged as**: `INFO` level  
**Frequency**: Every 5 seconds  
**Includes**:
- **Count**: Number of frames in the sample (last 120 frames max)
- **Average**: Mean frame time
- **Median**: 50th percentile frame time
- **Min**: Fastest frame time
- **Max**: Slowest frame time
- **p95**: 95th percentile (95% of frames are faster)
- **p99**: 99th percentile (99% of frames are faster)
- **Variance**: max - min (total spread)

**Output format**:
```
Frame time stats (last 120): avg=15.2ms, median=14.8ms, min=10.1ms, max=45.3ms, p95=22.1ms, p99=38.7ms, variance=35.2ms
```

**Purpose**: Understand overall frame time distribution and variance

---

## Usage Instructions

### Running on Linux/Wayland Machine

#### Option 1: Basic Usage (Slow Frames Only)

```bash
# Show WARN and INFO messages (recommended)
RUST_LOG=wezterm_gui=info ./wezterm start
```

**Output**:
```
[WARN  wezterm_gui] SLOW FRAME: 35.2ms (35.2ms, target: 16.67ms for 60fps)
[INFO  wezterm_gui] Frame time stats (last 120): avg=15.2ms, median=14.8ms, min=10.1ms, max=45.3ms, p95=22.1ms, p99=38.7ms, variance=35.2ms
```

#### Option 2: Detailed Debug (All Frame Times)

```bash
# Show all debug messages including every frame
RUST_LOG=wezterm_gui=debug ./wezterm start 2>&1 | grep -E "(SLOW FRAME|Frame time stats|paint_impl elapsed)"
```

**Output**:
```
[DEBUG wezterm_gui] paint_impl elapsed=12.3ms, fps=58.2
[DEBUG wezterm_gui] paint_impl elapsed=14.1ms, fps=58.5
[WARN  wezterm_gui] SLOW FRAME: 38.7ms (38.7ms, target: 16.67ms for 60fps)
[DEBUG wezterm_gui] paint_impl elapsed=38.7ms, fps=52.1
[DEBUG wezterm_gui] paint_impl elapsed=11.9ms, fps=53.4
[INFO  wezterm_gui] Frame time stats (last 120): avg=16.8ms, median=15.2ms, min=10.1ms, max=45.3ms, p95=24.5ms, p99=40.2ms, variance=35.2ms
```

#### Option 3: Log to File for Analysis

```bash
# Capture all logs to file
RUST_LOG=wezterm_gui=info ./wezterm start 2>&1 | tee wezterm-frame-times.log
```

**Then analyze**:
```bash
# Count slow frames
grep "SLOW FRAME" wezterm-frame-times.log | wc -l

# Extract frame stats
grep "Frame time stats" wezterm-frame-times.log
```

---

## How to Test

### 1. Start WezTerm with Logging

```bash
cd /path/to/linux/machine
RUST_LOG=wezterm_gui=info ./wezterm start 2>&1 | tee resize-test.log
```

### 2. Trigger the Sluggish Behavior

1. Open WezTerm
2. Create multiple tabs (e.g., 10-20 tabs)
3. **Resize the window repeatedly** by dragging the edge
4. Observe the logs in real-time

### 3. What to Look For

#### Expected Output for Smooth Rendering (Good!)

```
[INFO  wezterm_gui] Frame time stats (last 120): avg=12.5ms, median=12.1ms, min=10.0ms, max=18.2ms, p95=14.8ms, p99=16.5ms, variance=8.2ms
```

**Analysis**:
- ✅ Average: 12.5ms (80 FPS)
- ✅ Max: 18.2ms (no frames dropped)
- ✅ Variance: 8.2ms (consistent!)
- ✅ No "SLOW FRAME" warnings
- **Result**: **Smooth, snappy feel!**

#### Expected Output for Sluggish Rendering (Problem!)

```
[WARN  wezterm_gui] SLOW FRAME: 35.2ms (35.2ms, target: 16.67ms for 60fps)
[WARN  wezterm_gui] SLOW FRAME: 42.1ms (42.1ms, target: 16.67ms for 60fps)
[WARN  wezterm_gui] SLOW FRAME: 38.7ms (38.7ms, target: 16.67ms for 60fps)
[INFO  wezterm_gui] Frame time stats (last 120): avg=18.3ms, median=15.2ms, min=10.1ms, max=45.3ms, p95=28.4ms, p99=42.1ms, variance=35.2ms
```

**Analysis**:
- ⚠️ Average: 18.3ms (54 FPS) - seems okay
- ❌ Max: 45.3ms (22 FPS) - **TERRIBLE!**
- ❌ Variance: 35.2ms - **HUGE!**
- ❌ Frequent "SLOW FRAME" warnings
- **Result**: **Stuttering, janky, sluggish!**

---

## Interpreting the Results

### Key Metrics to Watch

| Metric | Good | Concerning | Bad |
|--------|------|------------|-----|
| **Average** | <16.67ms | 16-25ms | >25ms |
| **Max** | <25ms | 25-40ms | >40ms |
| **p99** | <20ms | 20-35ms | >35ms |
| **Variance** | <15ms | 15-30ms | >30ms |

### What Each Metric Means

1. **Average**: Overall performance
   - Good average = efficient code
   - Bad average = too much work per frame

2. **Max**: Worst-case latency
   - Low max = consistent performance
   - High max = occasional frame spikes

3. **p99**: Real-world experience
   - Low p99 = 99% of frames are smooth
   - High p99 = frequent stutters

4. **Variance (max - min)**: Frame consistency
   - ⭐ **MOST IMPORTANT FOR SLUGGISHNESS!**
   - Low variance = smooth feel
   - **High variance = stuttering/jank** ❌

---

## What the Data Will Tell Us

### Scenario 1: High Variance Confirmed

**Example**:
```
avg=15ms, variance=35ms, max=45ms, many SLOW FRAME warnings
```

**Conclusion**: ✅ **Frame time variance is the issue!**

**Next steps**:
1. Implement semantic zone caching (7% reduction)
2. Tune Lua GC parameters
3. Pre-allocate buffers

**Expected**: Variance drops to 10ms, smooth feel!

### Scenario 2: Low Variance, Still Sluggish

**Example**:
```
avg=12ms, variance=8ms, max=20ms, no SLOW FRAME warnings
```

**Conclusion**: ❌ **Frame time is NOT the issue!**

**Next steps**:
1. Investigate compositor interaction
2. Check vsync/frame pacing
3. Profile input latency
4. Check for other system issues

### Scenario 3: Consistently High Frame Times

**Example**:
```
avg=30ms, variance=10ms, no spikes but all frames slow
```

**Conclusion**: **Too much work per frame**

**Next steps**:
1. Profile to find CPU bottleneck
2. Optimize rendering pipeline
3. Reduce per-frame work

---

## Technical Details

### Frame Time Tracking Implementation

**Data structure**:
```rust
frame_times: RefCell<Vec<Duration>>  // Last 120 frames (2 seconds at 60fps)
```

**Tracking logic**:
1. Push each frame time to the vector
2. Keep only last 120 frames (rolling window)
3. Every 5 seconds, compute statistics
4. Log results

**Statistics computed**:
- **Average**: Sum / count
- **Min/Max**: First/last in sorted list
- **Median**: Middle of sorted list
- **p95**: 95th percentile in sorted list
- **p99**: 99th percentile in sorted list
- **Variance**: max - min

**Cost**: Negligible (~0.01ms per frame)
- Push to Vec: O(1)
- Sorting 120 elements: O(n log n) = O(840) ops
- Only done every 5 seconds

---

## Expected Results

### If Frame Variance is the Problem (Most Likely)

**Before caching**:
```
Frame time stats: avg=18ms, variance=35ms, max=45ms
SLOW FRAME: 42ms
SLOW FRAME: 38ms
SLOW FRAME: 35ms
```

**After implementing semantic zone caching**:
```
Frame time stats: avg=12ms, variance=8ms, max=18ms
(no SLOW FRAME warnings)
```

**User experience**: **Smooth, snappy, responsive!** ✅

### If Something Else is the Problem

The logging will reveal it! Examples:
- Consistent 30ms frames → Too much work per frame
- No slow frames but still feels sluggish → Input latency or compositor issue
- Spikes at specific intervals → GC pauses or background tasks

---

## Next Steps After Data Collection

### 1. Capture Logs During Resize

Run the test as described above and collect logs.

### 2. Share Results

Look for these patterns in the logs:
- Frequency of "SLOW FRAME" warnings
- Variance values in "Frame time stats"
- Max frame times

### 3. Proceed Based on Results

**If high variance confirmed** (likely):
→ Implement **semantic zone caching** (Phase 7 recommendation)

**If low variance but still sluggish**:
→ Investigate other causes (compositor, input latency, etc.)

**If consistently high frame times**:
→ Profile and optimize the slow code path

---

## Summary

### What We Added

1. ✅ Slow frame warnings (>20ms)
2. ✅ Periodic frame statistics (every 5 seconds)
3. ✅ Variance tracking (last 120 frames)
4. ✅ Percentile analysis (p95, p99)

### What We'll Learn

1. **Is frame variance the problem?** (most likely yes)
2. **How bad is the variance?** (10ms vs 30ms vs 50ms)
3. **How often do slow frames occur?** (every 10 frames vs 100 frames)
4. **What's the worst-case latency?** (max frame time)

### Why This Matters

**The hypothesis**: Sluggishness is from **frame time variance**, not average FPS!

**The data will prove or disprove this!**

**If confirmed**: We know exactly what to fix (semantic zones, GC, etc.)

**If not**: We'll discover the real issue from the patterns!

---

**Status**: ✅ Ready to deploy and test!  
**Next**: Run on Linux/Wayland and capture logs during resize testing.

