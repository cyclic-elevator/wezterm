# WezTerm Resize/Rendering Improvements — Task Plan

Prioritized milestones: Wayland → macOS → X11 → Windows.
Each task is scoped to be buildable and testable, with explicit acceptance criteria.
Feature flags are used to enable safe rollout without behavior changes by default.

## Milestone 1 — Wayland

1. Wayland: Opaque Region for Opaque Windows
- Goal: Reduce compositor blending by setting the `wl_surface` opaque region when the window is visually opaque.
- Code
  - `window/src/os/wayland/window.rs`: after size/config changes in `dispatch_pending_event` and at initial show, create a `wl_region`, `add(0,0,w,h)` in surface coords, and `surface.set_opaque_region(Some(&region))` when background opacity == 1.0 and no overlays; clear (`None`) otherwise.
  - Add config gate: `experimental_wayland_opaque_region` (default: off).
- Tests/Validation
  - Unit: helper function maps pixel size → surface size correctly (scale aware).
  - Manual: run with opaque background, enable flag, capture trace logs for `set_opaque_region` and visually verify reduced CPU with wlroots/mutter.
- Acceptance
  - Builds on Linux Wayland; when flag is on and window is opaque, opaque region is set and cleared on transparency toggle or resize.

2. Wayland: Live‑Resize Paint Throttling
- Goal: Avoid overproducing frames during interactive resize; improve smoothness and CPU.
- Code
  - `window/src/os/wayland/window.rs`: extend `WaylandWindowInner` with `paint_throttled: bool` and `next_paint_allowed_at: Instant`.
  - In `do_paint`: if `frame_callback.is_some()` or `Instant::now() < next_paint_allowed_at`, set `invalidated = true` and return; otherwise request frame and set `next_paint_allowed_at = now + 8–16ms`.
  - Config gate: `experimental_wayland_resize_throttle` (default: on).
- Tests/Validation
  - Unit: simple time‑based throttle test for helper calculating next allowed instant.
  - Manual: resize window rapidly; verify fewer paints (trace logs) and smoother interaction.
- Acceptance
  - Builds and functions with visible CPU reduction while resizing; no missed repaint after throttle interval (due to `invalidated` + frame callback).

3. Wayland: Damage‑Based Present via EGL
- Goal: Present only changed regions using `EGL_KHR_swap_buffers_with_damage`.
- Code
  - `wezterm-gui/src/termwindow/render/paint.rs`: accumulate coarse damage rects per frame (tab bar row, cursor/IME rect, active pane scroll; full surface on resize/shape cache clear).
  - `window/src/egl.rs`: expose `swap_buffers_with_damage(display, surface, &[Rect])` calling `eglSwapBuffersWithDamageEXT` when extension is present; fall back to full swap.
  - Integrate with glium present path used on Wayland; guard by new flag `experimental_partial_present`.
- Tests/Validation
  - Unit: damage union/clip helpers (rect math) with scale conversions.
  - Manual: run with flag on; verify trace shows `swap_buffers_with_damage` path and lower GPU/CPU in steady typing/scroll.
- Acceptance
  - Builds on Wayland; clean fallback without extension; visible damage path taken when enabled.

4. Wayland: Minimize CSD Frame Redraws
- Goal: Avoid SHM re‑uploads when decorations didn’t change.
- Code
  - `window/src/os/wayland/window.rs`: only set `refresh_decorations = true` when `window_frame.resize(..)` changes size or frame state changed; compare prior.
  - `window/src/os/wayland/frame.rs`: ensure `is_dirty()` gates drawing and only commit sub‑surfaces when actually redrawn.
- Tests/Validation
  - Manual: toggle maximize/restore and content scroll; ensure no decoration redraw spam unless size/state changes (trace logs).
- Acceptance
  - Builds; fewer decoration commits during content‑only updates.

5. Wayland: Docs + Feature Flags
- Code
  - `docs/`: short notes for `experimental_wayland_opaque_region`, `experimental_wayland_resize_throttle`, `experimental_partial_present` and how to enable.
- Acceptance
  - CI/docs build passes; flags documented.

## Milestone 2 — macOS

6. Tab Bar: Async Formatting, Caching, and Debounce (Cross‑Platform)
- Goal: Remove synchronous Lua from hot paths (hover/resize), reduce recompute frequency.
- Code
  - `wezterm-gui/src/tabbar.rs`: introduce `CachedTabTitle { items, len, hover_variant }` keyed by `(tab_id, max_width)`; fast path uses cache; background task batches Lua formatting for tabs; precompute hover/normal variants without Lua.
  - `wezterm-gui/src/termwindow/mod.rs`: add `pending_title_update` + debounce (16–32ms) in `update_title_impl`; build from cache; schedule async compute on miss; invalidate once results arrive.
  - Flag: `experimental_async_tabbar` (default: on).
- Tests/Validation
  - Unit: cache hit/miss behavior; hover variant switching; debounce timing (logic only).
  - Manual (macOS): verify reduced hitches on hover/resize, no missed updates.
- Acceptance
  - Builds on macOS; unit tests pass; behavior flag‑gated; UI remains correct under rapid tab/pane updates.

7. macOS: Smoke Validation of Existing Throttle
- Goal: Ensure existing `paint_throttled` behavior continues to coalesce correctly with the new tab bar cadence.
- Code
  - Add targeted trace points around invalidate/NeedRepaint and paint throttle in `window/src/os/macos/window.rs` under debug feature.
- Acceptance
  - Builds; manual test shows no repaint storms with new debounce.

## Milestone 3 — X11

8. X11: Damage‑Based Present via EGL
- Goal: Reuse partial present path on X11.
- Code
  - Share damage accumulation from Task 3.
  - `window/src/egl.rs`: ensure X11 surfaces call damage present when extension is available; fallback safe.
- Tests/Validation
  - Unit: reuse rect math tests; ensure X11 code path compiles.
  - Manual: test on picom/mutter/kwin; verify damage path via trace.
- Acceptance
  - Builds on X11; damage‑present used when enabled; fallback works.

9. X11: Set _NET_WM_OPAQUE_REGION When Opaque
- Goal: Hint compositors to skip blending.
- Code
  - `window/src/os/x11/window.rs` and `connection.rs`: add helpers to compute and set `_NET_WM_OPAQUE_REGION` using XFixes region on size/config changes; clear when translucent.
  - Flag: `experimental_x11_opaque_region` (default: off).
- Tests/Validation
  - Manual: with opaque window and flag, verify property present via `xprop`; monitor CPU improvements.
- Acceptance
  - Builds; property set/cleared correctly; no crashes on compositors lacking support.

10. X11: Optional _NET_WM_BYPASS_COMPOSITOR
- Goal: Allow users to disable compositing for fullscreen/opaque windows.
- Code
  - `window/src/os/x11/window.rs`: set `_NET_WM_BYPASS_COMPOSITOR` on fullscreen or via config flag; clear otherwise.
- Tests/Validation
  - Manual: verify property present via `xprop`; ensure no regressions when compositor ignores it.
- Acceptance
  - Builds; property lifecycle correct.

## Milestone 4 — Windows

11. Windows: Damage‑Based Present via EGL/ANGLE
- Goal: Reuse partial present path under ANGLE.
- Code
  - Share damage accumulation from Task 3; implement `eglSwapBuffersWithDamageEXT` usage when ANGLE advertises the extension.
- Tests/Validation
  - Unit: reuse rect tests.
  - Manual: verify trace path on Windows; ensure fallback on unsupported ANGLE builds.
- Acceptance
  - Builds on Windows; no regressions when extension absent.

12. Windows: Prefer Opaque EGL Configs When Fully Opaque
- Goal: Reduce blending by using alpha‑less EGL configs when possible.
- Code
  - `window/src/egl.rs`: add `want_opaque_surface` parameter; prefer `alpha_size == 0` configs; fall back to alpha if transparency needed (window background opacity < 1.0 or overlays).
  - Thread decision from GUI config down to EGL surface creation.
- Tests/Validation
  - Unit: config selection helper filters as intended.
  - Manual: verify chosen config via EGL logging; compare performance in opaque vs translucent modes.
- Acceptance
  - Builds; opaque config chosen only when safe; no visual regressions.

## Cross‑Cutting — CI, Flags, and Docs

13. Add Feature Flags and Docs
- Code
  - Introduce config entries and/or env flags:
    - `experimental_partial_present`
    - `experimental_wayland_opaque_region`
    - `experimental_wayland_resize_throttle`
    - `experimental_async_tabbar`
    - `experimental_x11_opaque_region`
  - Document in `docs/` how to enable, expected effects, and troubleshooting.
- Acceptance
  - Config parsing round‑trips; docs render; defaults preserve current behavior.

14. Metrics and Tracing Hooks
- Code
  - Add `log::trace!`/metrics counters around present path choice (full vs damage), tab bar format counts, decoration redraw counts.
- Acceptance
  - Visible in debug logs; does not spam in release unless enabled.

15. Build/Smoke Matrix
- Code/Infra
  - Ensure the project builds on all targets in CI with the new flags; add a simple headless unit test suite for rect math and caching.
- Acceptance
  - CI green across Linux (Wayland/X11), macOS, and Windows; unit tests pass.

---

Notes
- Tasks 3, 8, and 11 share the same damage‑present plumbing; implement once in `window/src/egl.rs` and route platform surfaces accordingly.
- Task 6 (tab bar) is cross‑platform but validated primarily under macOS milestone; benefits all platforms.
- All changes are flag‑gated to reduce risk; defaults keep current behavior intact.

