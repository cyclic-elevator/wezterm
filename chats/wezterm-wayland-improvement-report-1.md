# WezTerm macOS/Wayland Resize & Rendering Improvements

This report reviews WezTerm’s current resize/repaint/update/buffering strategies on macOS and Wayland, identifies bottlenecks (notably tab bar updates and Wayland CPU usage), contrasts with strategies referenced in gpui (see: `~/git/zed/gpui-compare-report-1.md`), and proposes actionable improvements with concrete code touch points, maintenance estimates, and expected gains.

## Summary
- Tab bar updates synchronously call Lua on frequent events, causing UI stalls during resize/hover and unnecessary recomputation.
- Wayland rendering schedules correctly via frame callbacks, but repaints and surface commits can be more selective. There is no explicit opaque region on the main surface and no EGL damage-based present for the main content, both of which increase compositor work/CPU.
- Bringing gpui-style strategies (triple-buffering, partial damage, opaque regions, aggressive coalescing/throttling of UI updates) to WezTerm’s Wayland path and tab bar pipeline can materially improve smoothness and reduce CPU.

## Current Behavior (as implemented)

### macOS
- Rendering layer
  - Creates a CALayer and lets ANGLE/CAMetalLayer sit underneath it; the layer is set non-opaque so compositing can respect transparency (`window/src/os/macos/window.rs`:174–206). Vsync is disabled at the GL layer and painting is throttled at the app level (293–298, 1541+, 3063–3081).
- Resize/scale
  - Queries backing scale via `NSView::convertRectToBacking` and resizes the drawable (`window/src/os/macos/window.rs`:618–624, 941–946, 957–962). Synthesizes a resize event immediately (650–669) so terminal content tracks window rapidly.
- Repaint
  - Uses `invalidate` → `NeedRepaint` with a `paint_throttled` flag to coalesce; avoids redundant work when resize/live-resize is active (772–776, 1541+, 3063–3081).

### Wayland
- Window and frame callbacks
  - Uses `wl_surface.frame` to schedule the next repaint; `do_paint` requests the callback, then dispatches `WindowEvent::NeedRepaint` (`window/src/os/wayland/window.rs`:1076–1110). `CompositorHandler::frame` clears `frame_callback` and triggers a repaint if something invalidated in the meantime (1329–1371, 1122–1126).
- Configure and DPI handling
  - Coalesces `xdg_toplevel::configure` and DPI changes via `PendingEvent`, then emits a `Resized` event and resizes EGL/WL‑EGL surface (800–953). Uses a small “fake” buffer on scale-factor changes to keep wlroots compositors happy (934–949).
- CSD frame (client decorations)
  - Drawn via tiny‑skia into SHM buffers on sub‑surfaces (`window/src/os/wayland/frame.rs`). Uses `damage_buffer` when possible (936, 970, 1001, 1032, 1060).
- Notable gaps
  - No explicit `wl_surface.set_opaque_region` when content is fully opaque; `WlRegion` is imported but not used for the main content.
  - No EGL “swap buffers with damage” for the main content; full-surface presentation likely occurs even for small changes.

### Tab Bar and Title/Status Updates
- Tab bar builds use Lua callbacks synchronously on hot paths:
  - `format-tab-title` is emitted synchronously per tab via `emit_sync_callback` (`wezterm-gui/src/tabbar.rs`:45–86, 133–209), including hover cases.
  - `update_title_impl` is invoked often (resize/key events/mouse events) and rebuilds the tab bar (`wezterm-gui/src/termwindow/mod.rs`:1961–2058, 1983–2007). Hover detection toggles recompute under the mouse.
- Fancy tab bar is cached as a computed box‑model element but invalidated often (render/paint logs and many calls to `invalidate_fancy_tab_bar`).

## Problems Observed
- Synchronous Lua in the title/tab bar path blocks the UI thread and piles onto repaint work during resize or when hovering the tab bar. This directly impacts responsiveness.
- Wayland uses frame callbacks, but still repaints broadly; lack of opaque region and present‑with‑damage increases compositor work and memory bandwidth, contributing to higher CPU.

## Improvements Inspired by gpui Strategies

### 1) Decouple Tab Bar Formatting From Synchronous Events
Goals
- Avoid blocking on Lua when hovering/resizing; recompute titles only when underlying data changes.
- Coalesce multiple invalidations into a single recompute per frame.

Proposed changes
- Introduce a cached, incrementally updated tab bar model:
  - Add a struct `CachedTabTitle { items: Vec<FormatItem>, len: usize, hover_variant: Option<Vec<FormatItem>> }` keyed by `(tab_id, max_width)`.
  - Recompute only on: tab activation change, pane title/progress change, config changes, added/removed tabs, and changes to `max_width`. Hover-only transitions should switch between precomputed normal/hover variants without calling Lua.
- Make Lua formatting async and coalesced:
  - Add an async job queue for tab title formatting. Batch all tabs, run on the config/Lua thread, and apply atomically.
  - During mouse move/hover, do not call Lua; simply select hover palette/variant.
- Rate-limit title rebuilds and UI invalidations:
  - Add a short debounce (e.g., 16–32 ms) for `update_title_impl` to collapse bursts.

Where to change
- `wezterm-gui/src/termwindow/mod.rs`:1961–2007 (update_title_impl), 1896–1903 (update_title):
  - Add a coalescing timer and a `pending_title_update` flag.
  - Build tab bar from cached titles; if cache miss, enqueue async formatting request, but show last known titles immediately.
  - On events that currently call `update_title`, set the flag and arm the debounce instead of calling immediately.
- `wezterm-gui/src/tabbar.rs`:45–209, 329–680:
  - Split `compute_tab_title` into:
    - a fast path that uses cached `Vec<FormatItem>`/`len`, and
    - a background task that resolves Lua and updates the cache.
  - Precompute hover/normal variants without Lua (styling only).

Alignment with code style
- Follows existing separation: tab bar building in Rust, Lua hooks via `config::lua`.
- Uses existing “assign to window then invalidate” event flow.

Maintenance effort
- Medium: 2–4 days for cache + debounce + async batch formatting; 1–2 days to audit update call sites and adjust hover logic; 1 day docs/tests.

Expected gains
- 2–5× reduction in Lua calls during interaction; noticeable reduction in micro‑stutters on resize and hover. Window content repaint no longer blocked by Lua.

### 2) Add Opaque Region for Main Content on Wayland
Goals
- Let the compositor skip blending when the window is visually opaque.

Proposed changes
- When the terminal window background is fully opaque and there are no transparent overlays, set the surface’s opaque region to the full content rectangle.

Where to change
- `window/src/os/wayland/window.rs`:
  - After computing pixel size in `dispatch_pending_event` (around 880–953) and also after initial show:
    - Create a `wl_region` via `compositor.create_region()`.
    - Call `region.add(0, 0, surface_width, surface_height)` in surface coords.
    - Call `surface.set_opaque_region(Some(&region))` and commit.
  - Update/clear region whenever background opacity or window transparency changes.

Alignment with code style
- Mirrors how frame sub‑surfaces use `damage_buffer`; stays within Wayland module.

Maintenance effort
- Low: 1 day including config gating and testing with transparent backgrounds disabled/enabled.

Expected gains
- Reduced compositor work for opaque windows; typical 5–20% CPU reduction during scrolling/typing on wlroots‑ and mutter‑based compositors for opaque themes.

### 3) Implement EGL Swap Buffers With Damage for Main Content
Goals
- Present only the regions that changed each frame; reduce GPU and compositor work.

Proposed changes
- Track damage rectangles during painting and plumb them to the GL/EGL present path.
  - Start with coarse regions: tab bar rect when it changes; cursor/IME rect; active pane region during typing; full‑surface fallback on resize or cache flush.
- Use `EGL_KHR_swap_buffers_with_damage` when available (`window/build.rs` already checks the extension). Fall back to full swap otherwise.

Where to change
- Rendering path used by Wayland (glium backend):
  - `wezterm-gui/src/termwindow/render/paint.rs` and related files: accumulate damage rects per frame (e.g., `self.damaged.push(rect)`).
  - GL present layer (EGL/glium integration): add a `swap_buffers_with_damage(&[Rect])` path that calls `eglSwapBuffersWithDamageEXT` when supported.

Alignment with code style
- Coarse damage tracking is already natural: code knows tab bar rect, cursor rect, and pane bounds.
- Keep initial implementation conservative; refine as confidence grows.

Maintenance effort
- Medium: 3–5 days for damage tracking plumbing and EGL integration; 1–2 days for fallbacks and testing across compositors.

Expected gains
- 10–30% reduction in GPU/compositor overhead in steady‑state typing/scrolling; larger savings for small UI updates.

Notes
- This applies to the GL path. WGPU currently lacks explicit partial present APIs; keep full present there.

### 4) Throttle Live‑Resize Rendering on Wayland
Goals
- Avoid over‑producing frames while the compositor is still delivering `configure` events and the frame callback is pending.

Proposed changes
- Mirror macOS `paint_throttled` pattern on Wayland:
  - Add `paint_throttled: bool` and `next_paint_allowed_at: Instant` to Wayland window inner.
  - During interactive resize (when multiple configures are coalescing), only schedule at most one paint per X ms (e.g., 8–16 ms).
  - If `frame_callback` is present, set `invalidated = true` and rely on `next_frame_is_ready` to coalesce.

Where to change
- `window/src/os/wayland/window.rs`:
  - Extend `WaylandWindowInner` with throttle fields.
  - In `do_paint`, early‑return if before `next_paint_allowed_at`, setting `invalidated = true`.
  - Advance `next_paint_allowed_at` to `Instant::now() + Duration::from_millis(8)`.

Alignment with code style
- Parallels existing invalidation logic; isolated to Wayland backend.

Maintenance effort
- Low: 1–2 days including tuning.

Expected gains
- Smoother interactive resize; 10–20% CPU reduction during window drags on systems with busy compositors.

### 5) Reduce CSD Frame Work During Resize
Goals
- Ensure client‑drawn decorations redraw only when necessary.

Proposed changes
- The frame code already checks `is_dirty()`; ensure `refresh_decorations` is only set when state or size changed (not for pure content changes).
- Skip drawing if hidden or undecorated; already present but audit calls.

Where to change
- `window/src/os/wayland/window.rs` and `window/src/os/wayland/frame.rs`:
  - Only set `refresh_decorations` = true when `window_frame.resize` actually changes size; guard with prior size check.

Maintenance effort
- Low: <1 day.

Expected gains
- Avoids redundant SHM uploads/damage on every repaint during content‑only changes.

## Concrete Code Touch Points
- Tab bar decoupling and caching
  - `wezterm-gui/src/termwindow/mod.rs`:1961–2007 (update_title_impl), 1896–1903 (update_title): add debounce and cache use.
  - `wezterm-gui/src/tabbar.rs`:45–209, 329–680: split Lua path into async batch; add cache and hover variant selection.
  - `wezterm-gui/src/termwindow/render/tab_bar.rs`: compute tab bar rect; expose it to damage tracking.
- Wayland opaque region
  - `window/src/os/wayland/window.rs`: in `dispatch_pending_event` after computing `surface_width/height` and when background opacity toggles; use `WlRegion` to set `opaque_region`.
- EGL damage present
  - `wezterm-gui/src/termwindow/render/paint.rs` and GL/EGL glue (glium/egl path): add damage accumulation and call into `eglSwapBuffersWithDamageEXT` when available.
- Resize throttle on Wayland
  - `window/src/os/wayland/window.rs`: extend `WaylandWindowInner` with throttle fields; adjust `do_paint` to honor them.
- Frame redraw minimization
  - `window/src/os/wayland/window.rs`: only set `refresh_decorations` if frame size/state actually changed.

## Maintenance and Risk Assessment
- All changes are local to GUI and platform‑specific backends; no mux/protocol changes.
- Tab bar caching/async formatting is the largest refactor but aligns with existing architecture (Lua on a dedicated thread, Rust‑side rendering). Risk is manageable with a feature flag (`experimental_async_tabbar`) and gradual rollout.
- Wayland opaque region and damage‑based present are standard practice; guard by capability checks and background/transparency config.
- Resize throttling parallels macOS logic and reuses the existing invalidation/coalescing pattern.

Estimated effort
- Tab bar async caching/coalescing: 1–2 weeks including polish.
- Wayland opaque region: 1 day.
- EGL damage present: 1 week (cross‑compositor validation).
- Resize throttle + frame redraw minimization: 2 days.

## Expected Improvements
- Tab bar responsiveness and overall resize smoothness: noticeable reduction in jank; 2–5× fewer Lua invocations, cutting stalls during hover/resize.
- Wayland CPU usage: 10–30% lower in steady typing/scroll, 5–20% during opaque backgrounds due to compositor optimizations; additional savings during live resize via throttling.

## Alignment With Existing Coding Patterns
- Matches event‑driven `WindowEventSender` model and per‑platform backends.
- Uses existing invalidation semantics (`NeedRepaint`, `invalidated`, `frame_callback`).
- Respects configuration‑based behavior gates and runtime capability checks (e.g., EGL extensions).

## Appendix: gpui Strategies Referenced
- Vsync‑aligned pacing: Wayland uses `frame` callbacks; macOS uses a display link equivalent. WezTerm already aligns broadly; proposals focus on coalescing and damage.
- Triple buffering and non‑blocking presentation: Ensure glium/EGL path does not block; damage‑based present reduces work while leveraging compositor buffering.
- Opaque region and partial damage: Standard compositor hints that reduce blending and work; add explicitly on Wayland.
- Async UI formatting and coalescing: Apply to WezTerm’s Lua tab bar to remove synchrony from hot paths.

---

If you’d like, I can start with a small PR implementing the Wayland opaque region and resize‑throttle, then move on to the tab bar async cache behind a feature flag.


## Platform‑Specific Proposals: Windows

Goals
- Reduce present bandwidth and CPU by leveraging damage‑based present where supported through ANGLE/EGL.
- Prefer opaque swapchain/configs when the window is fully opaque to reduce compositor work.
- Reuse tab bar debounce/caching to avoid WM_MOUSEMOVE‑driven churn.

Proposed changes
- Damage‑based present via EGL on Windows (ANGLE backend):
  - Track per‑frame damage rects in the renderer (cursor, tab bar row, IME, selection changes, scroll deltas; full surface on resize).
  - If `EGL_KHR_swap_buffers_with_damage` or `EGL_EXT_swap_buffers_with_damage` is advertised by ANGLE, call `eglSwapBuffersWithDamageEXT` with those rects; else fall back to full swap.
  - Files: `wezterm-gui/src/termwindow/render/paint.rs` (accumulate damage), GL/EGL glue used on Windows (through `window/src/egl.rs` and the glium backend) to add `swap_buffers_with_damage(&[Rect])`.
- Prefer opaque EGL configs when possible:
  - When the configuration is fully opaque (no transparency, window background opacity == 1.0, no overlays), select an EGL config with `alpha_size == 0` and request an opaque surface. This allows DWM/ANGLE to treat the surface as opaque (lower blending cost).
  - Files: `window/src/egl.rs` (config selection paths `choose_config`, `create_surface`); thread through a “want_opaque_surface” flag from the window backend based on config.
- Coalesce tab bar updates during hover:
  - Reuse the report’s tab bar debounce/caching to reduce `InvalidateRect` storms and WM_PAINT triggers from hover on Windows.
  - Files: `wezterm-gui/src/termwindow/mod.rs` (debounce `update_title_impl`), `wezterm-gui/src/tabbar.rs` (cache computed titles).

Alignment with code style
- Uses existing invalidation model (`NeedRepaint`) and EGL plumbing already present in `window/src/egl.rs`.

Maintenance effort (Windows)
- Damage‑present plumbing (shared with Wayland/X11): 3–5 days, verification on ANGLE/D3D11 path.
- Opaque config selection flagging and threading: 1–2 days.
- Tab bar debounce/caching (shared): included in earlier estimate.

Expected gains (Windows)
- 10–25% reduction in GPU/compositor work in steady typing/scroll, more for small UI updates. Reduced stalls from hover‑driven tab bar updates.


## Platform‑Specific Proposals: X11

Goals
- Leverage existing X11 expose/damage tracking to perform damage‑based presents.
- Hint compositors about opaque regions to reduce blending cost.
- Keep live‑resize smooth with existing throttling/coalescing.

Proposed changes
- Damage‑based present via EGL on X11:
  - Similar to Wayland/Windows: accumulate damage rects during paint and call `eglSwapBuffersWithDamageEXT` if available.
  - Files: `wezterm-gui/src/termwindow/render/paint.rs` (collect rects), GL/EGL glue in `window/src/egl.rs` to expose `swap_buffers_with_damage` for X11 surfaces.
- Set _NET_WM_OPAQUE_REGION when window is fully opaque:
  - Compute region in window coordinates (exclude tab bar if translucent, overlays, etc.).
  - Set EWMH `_NET_WM_OPAQUE_REGION` property using XFixes region when background opacity == 1.0 and no transparency effects are enabled.
  - Files: `window/src/os/x11/window.rs` and/or `window/src/os/x11/connection.rs`: utility to build and set the property on size changes and config changes.
- Optional (behind a config flag): `_NET_WM_BYPASS_COMPOSITOR` hint to reduce compositing for opaque, full‑screen windows when users prefer performance over effects.
  - Files: `window/src/os/x11/window.rs` (set/clear property on state changes).

Where this ties into current code
- X11 backend already records exposed/dirty regions (`window/src/os/x11/window.rs`:194–209). Wire these into the damage list for present and use scissor/partial renders as appropriate.
- Coalescing/throttling already exists (`paint_throttled` and coalesced `Resized` events). Keep semantics and extend only present path.

Maintenance effort (X11)
- Damage‑present (shared): 3–5 days; validating across compositors (picom, mutter, kwin) 1–2 days.
- Opaque region property: 1–2 days.
- Optional bypass compositor hint: <1 day behind feature flag.

Expected gains (X11)
- 10–30% reduction in compositor/GPU work for steady‑state updates; further gains when the window is fully opaque and the compositor honors `_NET_WM_OPAQUE_REGION`.
