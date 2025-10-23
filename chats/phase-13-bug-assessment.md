# Phase 13: Critical Bug Assessment - Infinite Loop in Texture Growth

## Date
2025-10-23

## Status
🔴 **CRITICAL BUG** - Infinite loop causing UI hang!

---

## Executive Summary

The deferred texture atlas growth implementation has a **critical infinite loop bug** that causes the UI to hang completely. The application enters an infinite retry loop when texture space is exhausted, logging thousands of messages per second and never breaking out.

**Impact**: **UI is completely unusable!** Application hangs on startup. ❌

---

## Bug Analysis

### Symptoms

From `chats/frame-logs.12`:
- **246,162 log lines** in a short period!
- Repeated messages every ~1ms:
  ```
  02:42:17.405  WARN   Texture atlas out of space (need 256, current 128). Deferring growth to next frame (deferred 1 times).
  02:42:17.405  INFO   Not enough texture space; rendering with degraded quality Scale(2) this frame
  02:42:17.406  INFO   Not enough texture space; rendering with degraded quality Scale(4) this frame
  02:42:17.407  INFO   Not enough texture space; rendering with degraded quality Scale(8) this frame
  02:42:17.408  INFO   Not enough texture space; rendering with degraded quality No this frame
  02:42:17.409  WARN   Already at maximum image scaling, skipping images this frame
  02:42:17.409  INFO   Not enough texture space; rendering with degraded quality No this frame
  ... repeats thousands of times per second ...
  ```

### Root Cause

**The infinite loop happens because**:

1. `paint_pass()` returns `OutOfTextureSpace` error on pass 1+ (not pass 0)
2. Our code queues texture growth and sets `allow_images` to degraded quality
3. **BUT**: We **DON'T BREAK THE LOOP!**
4. The code continues, retrying `paint_pass()` immediately
5. `paint_pass()` STILL fails with `OutOfTextureSpace` (because atlas hasn't grown yet!)
6. Loop continues forever...

### The Broken Code

In `wezterm-gui/src/termwindow/render/paint.rs` (line 103):

```rust
} else {
    // Subsequent passes: defer growth to avoid blocking current frame
    if self.pending_texture_growth.borrow().is_none() {
        *self.pending_texture_growth.borrow_mut() = Some(size);
        *self.texture_growth_deferred_count.borrow_mut() += 1;
        
        log::warn!(...);
    }
    
    // Use current atlas with degraded quality for this frame
    self.allow_images = match self.allow_images {
        AllowImage::Yes => AllowImage::Scale(2),
        AllowImage::Scale(2) => AllowImage::Scale(4),
        AllowImage::Scale(4) => AllowImage::Scale(8),
        AllowImage::Scale(8) => AllowImage::No,
        AllowImage::No | _ => {
            log::warn!("Already at maximum image scaling, skipping images this frame");
            AllowImage::No
        }
    };

    log::info!(...);
    
    self.invalidate_fancy_tab_bar();
    self.invalidate_modal();
    // Don't break - continue rendering with degraded quality  ← BUG!
}
```

**The problem**: **"Don't break"** comment and no `break 'pass;` statement!

This causes the loop to **continue forever**, retrying the same failed operation!

---

## Why This Happens

### The Paint Loop

```rust
'pass: for pass in 0.. {
    match self.paint_pass() {
        Ok(_) => { /* success, check if more quads needed */ }
        Err(err) => {
            if /* OutOfTextureSpace */ {
                if pass == 0 {
                    // First pass: recreate at current size (BLOCKS)
                    // This works fine
                } else {
                    // Subsequent passes: defer growth
                    // Queue growth for next frame
                    // Degrade quality
                    // ??? What now ???
                    // NO BREAK! Loop continues!
                }
            }
        }
    }
}
```

### What Should Happen

**Expected flow**:
1. Pass 0: Fails with OutOfTextureSpace
2. Recreate atlas at current size (blocks, but works)
3. Retry pass 0: Might succeed or fail again
4. If fails again on pass 1+: **Queue growth, BREAK loop, render what we have**
5. Next frame: Apply queued growth at frame start
6. Continue normally

**Actual flow**:
1. Pass 0: Fails with OutOfTextureSpace
2. Recreate atlas at current size
3. Retry pass 0: Fails again (still not enough space)
4. Pass 1: Fails with OutOfTextureSpace
5. Queue growth, degrade quality, **DON'T BREAK** ❌
6. Pass 2: Fails AGAIN (atlas hasn't grown!)
7. Queue growth (no-op, already queued), degrade quality more, **DON'T BREAK** ❌
8. Pass 3: Fails AGAIN...
9. **Infinite loop!** 💀

---

## Additional Issues

### Issue 2: `allow_images` Degrades Every Pass

Each pass through the loop degrades `allow_images` further:
- Pass 1: `Yes` → `Scale(2)`
- Pass 2: `Scale(2)` → `Scale(4)`
- Pass 3: `Scale(4)` → `Scale(8)`
- Pass 4: `Scale(8)` → `No`
- Pass 5+: `No` → `No` (logs "Already at maximum...")

**This creates thousands of log messages per second!**

### Issue 3: Growth Never Applied

Because the loop never breaks, `paint_impl()` never completes, so:
- `call_draw()` never happens
- Frame never finishes
- Next frame never starts
- Queued texture growth never applies
- **System is deadlocked!** 💀

---

## The Fix

### Required Changes

**In `paint.rs`, line 103+**:

```rust
} else {
    // Subsequent passes: defer growth to avoid blocking current frame
    if self.pending_texture_growth.borrow().is_none() {
        *self.pending_texture_growth.borrow_mut() = Some(size);
        *self.texture_growth_deferred_count.borrow_mut() += 1;
        
        log::warn!(
            "Texture atlas out of space (need {}, current {}). Deferring growth to next frame (deferred {} times).",
            size,
            current_size,
            self.texture_growth_deferred_count.borrow()
        );
        
        // Use current atlas with degraded quality for this frame
        self.allow_images = match self.allow_images {
            AllowImage::Yes => AllowImage::Scale(2),
            AllowImage::Scale(2) => AllowImage::Scale(4),
            AllowImage::Scale(4) => AllowImage::Scale(8),
            AllowImage::Scale(8) => AllowImage::No,
            AllowImage::No | _ => AllowImage::No,
        };

        log::info!(
            "Not enough texture space; rendering current frame with degraded quality {:?}, will grow atlas next frame",
            self.allow_images
        );
    }
    
    self.invalidate_fancy_tab_bar();
    self.invalidate_modal();
    
    // CRITICAL: Break the loop to allow frame to complete!
    // The queued texture growth will be applied at the start of next frame.
    break 'pass;  // ← FIX!
}
```

**Key changes**:
1. ✅ **Add `break 'pass;`** after queuing growth
2. ✅ **Move quality degradation inside** `if pending_texture_growth.is_none()`
3. ✅ **Remove redundant logging** after degradation
4. ✅ **Simplify `AllowImage::No` case** (no warning)
5. ✅ **Single clear log message** about deferring to next frame

---

## Why Original Logic Was Wrong

### Misunderstanding of "Don't Break"

The comment said:
```rust
// Don't break - continue rendering with degraded quality
```

**What this SHOULD mean**:
- "Don't break the FRAME - complete rendering with what we have"
- "Break the LOOP - don't retry forever"

**What it ACTUALLY did**:
- "Don't break the LOOP - keep retrying"
- Result: Infinite loop, frame never completes

### Correct Understanding

**Deferred texture growth means**:
1. **Accept that current frame won't be perfect**
2. **Render with degraded quality NOW**
3. **Break the retry loop**
4. **Complete the frame**
5. **Grow atlas at START of next frame**
6. **Next frame will be perfect**

**NOT**:
- ❌ Keep retrying until it works
- ❌ Block current frame forever
- ❌ Never complete the frame

---

## Impact Analysis

### Before Fix

**Symptoms**:
- Application hangs on startup
- 246k log messages in seconds
- CPU at 100% (busy loop)
- UI completely frozen
- **Unusable!** ❌

**Why**:
- Infinite loop in paint loop
- Never completes a frame
- Never applies queued growth
- System deadlocked

### After Fix

**Expected behavior**:
- Texture growth queued ✅
- Current frame completes with degraded quality ✅
- Next frame: Growth applied at start ✅
- Rendering continues normally ✅
- **Usable!** ✅

**Potential visual glitch**:
- One frame with scaled/missing images (barely noticeable)
- **Acceptable tradeoff** vs. infinite hang!

---

## Testing Plan

### After Fix

1. **Rebuild**:
   ```bash
   cargo build --release --package wezterm-gui
   ```

2. **Run with logging**:
   ```bash
   RUST_LOG=wezterm_gui=info ./target/release/wezterm-gui 2>&1 | tee frame-logs.13
   ```

3. **Observe startup**:
   - Should see normal startup logs
   - May see 1-2 "Deferring growth" messages
   - Should NOT see infinite loop
   - Application should be responsive

4. **Check log count**:
   ```bash
   wc -l frame-logs.13
   # Should be < 1000 lines (not 246k!)
   ```

5. **Test resize**:
   - Rapidly resize window
   - Should remain responsive
   - May see occasional "Deferring growth" on large resizes
   - Should recover immediately

### Success Criteria

✅ **Application starts successfully**  
✅ **UI is responsive**  
✅ **Log lines < 1000** (not 246k!)  
✅ **No infinite loops**  
✅ **Texture growth happens when needed**  
✅ **Rendering continues normally**  

---

## Lessons Learned

### 1. Always Have Loop Exit Conditions

**Problem**: Infinite loop with no exit  
**Solution**: **Always add explicit `break` statements** when deferring work!

**Rule**: If you queue something "for later", you MUST break the current loop!

### 2. Test Edge Cases Immediately

**Problem**: Code worked fine when texture was sufficient  
**Solution**: Should have tested with **minimal texture size** immediately!

**Test case**: Set initial texture size to 64x64 to force frequent growth

### 3. Logs Are Not Tests

**Problem**: Assumed degraded rendering would work  
**Solution**: **Visual inspection** required, not just log review!

### 4. Comments Can Mislead

**Problem**: "Don't break" comment was ambiguous  
**Solution**: Be explicit: "Break the LOOP to complete the FRAME"

---

## Recommended Fix Priority

**Priority**: 🔴 **CRITICAL** - Must fix immediately!

**Complexity**: ⭐ **TRIVIAL** - One line change!

**Risk**: ⭐ **NONE** - Fix is obvious and safe

**Impact**: ✅ **Fixes complete UI hang**

---

## Summary

### The Bug

- ❌ **Infinite loop** in paint pass error handling
- ❌ **No break statement** after queueing texture growth
- ❌ **System deadlock** - frame never completes
- ❌ **246k log messages** in seconds
- ❌ **UI completely unusable**

### The Fix

- ✅ **Add `break 'pass;`** after queuing growth
- ✅ **One line change!**
- ✅ **Fixes infinite loop**
- ✅ **Allows frame to complete**
- ✅ **System becomes usable**

### Next Steps

1. **Apply fix** (add `break 'pass;`)
2. **Rebuild**
3. **Test startup** (should work now!)
4. **Test resize** (should be smooth!)
5. **Verify no infinite loops** (log count < 1000)

---

**Status**: 🔴 **FIX READY** - One line change to fix critical bug!

