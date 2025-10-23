# Prompt
On wayland, how do smooth-looking UI apps (Zed, VS Code, Chrome etc) handle compositor synchronization to achieve high frame rates and low GPU/CPU impact during resize events?

## Response
On Wayland, apps like **Zed**, **VS Code**, and **Chrome** achieve smooth resizing and animation by tightly synchronizing their rendering loops with the compositor’s frame schedule. The Wayland protocol gives them direct control over *when* to render, which — when used correctly — avoids jank, tearing, and CPU/GPU spikes.

Let’s go through how that works step-by-step.

---

## 🧠 1. The problem: Wayland decouples the app and display

Unlike X11, Wayland doesn’t let the client draw directly into the screen.
Apps render into GPU buffers and then **commit** them to the compositor, which composites and presents them at v-sync intervals.

If a client renders too early or too often (e.g. on every mouse-move during resize), the compositor drops those frames.
If it renders too late, you get visible lag or stutter.

So the challenge is: **Render only once per display frame, in sync with compositor timing.**

---

## ⚙️ 2. The solution: Frame pacing via `wl_surface.frame`

Wayland provides the `wl_surface.frame` callback, which notifies the client when it’s safe to start rendering a new frame.

Smooth clients do this:

1. **Wait** for the previous frame’s `frame_done` signal (from the compositor).
2. **Render** the new frame (reflecting any resize or content updates).
3. **Submit** via `wl_surface.commit`.
4. **Request** the next `frame` callback.

That means they never render faster than the monitor refresh rate (typically 60 or 120 Hz), and never drop frames due to misaligned commits.

---

## 🪞 3. Resizing synchronization

During a resize, the compositor sends new window dimensions. Smooth apps:

* **Throttle** redraws: only paint on the next `frame_done`, not every geometry event.
* **Use double/triple buffering** to avoid blocking on GPU sync (so one buffer is being displayed while another is being rendered).
* **Render the new size progressively** so the visual edge tracks the user’s cursor smoothly.

Many older Wayland clients skip frame throttling and try to repaint for every size change → resulting in visible lag or “rubber-band” effects.

---

## 🧩 4. Toolkit and engine implementations

| Application        | Rendering backend                        | Synchronization strategy                                                                                          |
| ------------------ | ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| **Zed**            | Rust + `wgpu` + `smithay-client-toolkit` | Uses `wl_surface.frame` for frame pacing, `wp_presentation_time` for timing correction, and async resize handling |
| **VS Code**        | Electron → Chromium (Ozone/Wayland)      | Reuses Chrome’s compositor; triple-buffered GPU swap chain; presentation feedback used for vsync alignment        |
| **Chrome / Edge**  | Chromium compositor                      | Full frame scheduling via `wl_surface.frame`, `wp_presentation_feedback`; uses EGL fences for GPU sync            |
| **GTK4 / GDK**     | GSK renderer                             | Integrates `wl_surface.frame` with GDK’s “frame clock”, automatically throttling redraw                           |
| **Qt6 / KDE apps** | QtWayland                                | Uses presentation-time protocol for adaptive frame pacing                                                         |

---

## 🧮 5. Efficiency tricks for low CPU/GPU use

* **Partial redraw (damage tracking):** repaint only changed regions.
* **Idle frame suppression:** skip frame generation when content is static.
* **GPU fences / `dma-buf` sharing:** reuse buffers efficiently, no reallocations.
* **Compositor-driven throttling:** client waits on frame callbacks instead of busy-rendering.

Together these ensure smooth 60–144 Hz animation even during resize, with minimal overhead.

---

## ⚡ 6. Advanced: Presentation-time protocol

The optional `wp_presentation` Wayland extension provides timestamps for when each frame *actually* hit the screen.
Apps use this for latency reduction and adaptive frame pacing:

* Predict next v-blank time.
* Adjust animation interpolation.
* Avoid frame drift at high refresh rates.

Chrome, KWin, GNOME, and Zed’s renderer all use this for perfect compositor sync.

---

## 🧭 Summary

| Mechanism               | Purpose                                     |
| ----------------------- | ------------------------------------------- |
| `wl_surface.frame`      | Throttle rendering to compositor frame rate |
| Double/triple buffering | Prevent stalls during GPU swaps             |
| `wp_presentation`       | Feedback for precise vsync alignment        |
| Damage regions          | Avoid unnecessary redraw                    |
| Idle suppression        | Cut power use during static scenes          |
| GPU fences              | Prevent over-submission and stutter         |

---

So in essence, smooth Wayland apps **delegate frame pacing to the compositor**, producing frames only when the compositor is ready, and relying on GPU-level fences and presentation feedback to stay precisely in phase with vsync — that’s what makes Zed, Chrome, and VS Code so fluid even under continuous resize or animation.
