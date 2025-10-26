# Comprehensive Change Review (since 64f2907c6)

Scope: Analyze code changes and test/perf reports from commit 64f2907c635b7bab407ac300b2261c77a85c1c8e to HEAD. Summaries focus on GUI rendering (buffer pooling, tab bar/title caching, Wayland presentation/triple buffering), Wayland windowing (resizing, fences), and mux/client (remote resize, coalescing).

Sources reviewed:
- Commits: 41 commits from a7dcc90c..641ca0379.
- Key code: `wezterm-gui/src/*` (renderstate, bufferpool, termwindow, tabbar/title cache), `window/src/os/wayland/*` (window.rs, presentation.rs, triplebuffer.rs, gpufence.rs), `wezterm-client/src/pane/clientpane.rs`, `wezterm-mux-server-impl/src/sessionhandler.rs`, `mux/src/*`.
- Test/Perf artifacts: `chats/frame-logs.*`, `chats/perf-report.*`, `chats/build-logs.*`, phase assessment summaries.

High‑level themes:
- Phases 0–15: incremental GUI/cache/render infra changes with perf logging; minimal behavioral risk.
- Phases 16–18: Wayland timing/triple buffering infrastructure added; wiring is partial; guarded by new modules; moderate integration risk if enabled.
- Phase 19.x: client/mux resize storm mitigation and coalescing; first iterations incomplete; 19.3 adds true debounce with cancellation; 19.4 fixes in draw path; evidence shows 19.2 reduced some storms but server still hit 100% on rewrap from excessive RPCs until 19.3 changes.

-----

Categorization by invasiveness, effectiveness, secondary benefit/risk

Low invasiveness, high effectiveness
- Tab/window title and status caching (new caches, limited call sites)
  - Files: `wezterm-gui/src/callback_cache.rs`, `tab_title_cache.rs`, `tabbar.rs`, usage in `main.rs`, `termwindow/mod.rs`.
  - Effectiveness: Reduces repeated Lua/formatting work; faster tab bar updates; minimal surface area.
  - Secondary: Slight staleness windows bounded by generation/time buckets; well contained.
- Vertex buffer pooling infrastructure
  - Files: `wezterm-gui/src/bufferpool.rs`, integrated in `renderstate.rs` and layer buffers.
  - Effectiveness: Cuts GPU buffer alloc/memmove under resize/frame churn; supports reuse; logs stats.
  - Secondary: Pool size bound; reuse by capacity; minimal shared mut via RefCell; low risk.

Medium invasiveness, medium/high effectiveness
- Selective viewport invalidation on resize (client)
  - Files: `wezterm-client/src/pane/clientpane.rs` selective `make_viewport_stale(100)`.
  - Effectiveness: Avoids full scrollback invalidation, reduces fetch and draw cost substantially.
  - Secondary: Risk of stale lines outside viewport in unusual scrollback manipulations; mitigated by margin.
- True debounce with cancellation (client) and coalesced server pushes
  - Files: `wezterm-client/src/pane/clientpane.rs` (PendingResize gen token); `wezterm-mux-server-impl/src/sessionhandler.rs` (push coalescing flags, change computation).
  - Effectiveness: Neutralizes resize‑storm RPC spam; major win against server rewrap hang reported in perf‑report.19.2.
  - Secondary: Requires careful generation updates; race risk is low given single mutex, but mis‑wiring would drop a resize.
- Wayland coordinate conversions and resize handling refinements
  - Files: `window/src/os/wayland/window.rs` (DPI conversions, input repeat, resize sequencing).
  - Effectiveness: Better pixel/surface rounding, fewer off‑by‑one terminal row losses; smoother repeat handling.
  - Secondary: Behavior depends on compositor; conservative rounding avoids truncation; low risk.

Higher invasiveness, incremental effectiveness (infrastructure ready; partial wiring)
- Wayland presentation/triple buffering/fences
  - Files: `window/src/os/wayland/presentation.rs`, `triplebuffer.rs`, `gpufence.rs`, `mod.rs`
  - Effectiveness: Provides building blocks to avoid GPU stalls, improve pacing; logs/structs present; build logs note unused warnings → partial integration.
  - Secondary: Incorrect activation could interact poorly with EGL/wgpu present mode and compositor timing; keep behind internal gates until fully verified.

-----

Categorization by chance of acceptance by maintainers

Very likely to accept (small, targeted wins)
- Title/tabbar/status caching utilities and generation‑based invalidation
- VertexBufferPool and RenderContext plumbing (non‑intrusive, guarded; improves perf)
- Selective viewport invalidation on resize (clear win; scoped to client)
- Minor Wayland pixel/surface rounding fixes; key repeat scheduling cleanup

Likely to accept with review notes
- Client true debounce and resize generation coalescing
  - Strong motivation from logs; clear correctness; ensure logging tone/no emojis is adjusted before merge.
- Server coalesced push path in SessionHandler
  - Ensure no starvation; guard against missing initial palette; code already accounts for palette/config signals.

Needs explicit discussion/design sign‑off
- Wayland triple buffering/presentation activation
  - The modules exist but are not fully wired; enabling changes pacing; will need feature flag and measurement on multiple compositors.

-----

Categorization by potential impact to other areas

Low impact (localized)
- Caching modules in GUI (title/status), buffer pooling
  - Limited to render path; no protocol changes.

Medium impact (crosses client⇄server boundary or window system)
- Resize debounce and selective invalidation
  - Impacts RPC rate and server rewrap frequency; improves stability; needs alignment with mux resize semantics and Tab topology.
- Wayland rounding and repeat timing changes
  - Could change perceived sizes by ±1 pixel/row on some DPIs; better than truncation; verify on HiDPI/XWayland.

High impact (system‑level behavior, timing)
- Presentation/triple buffering/fence introduction
  - Affects frame pacing and GPU/CPU overlap; interacts with compositor vsync; ensure opt‑in and guarded.

-----

Evidence from test/perf reports
- perf-report.10: shows reduced GPU buffer alloc hotspots after pooling; Lua and alloc hot paths visible but lower.
- frame-logs.17 / perf-report.17: baseline prior to 19; deserialization cost noted (~11–12%).
- perf-report.19 and phase-19 failure assessments: initial 19 code paths not executing; follow‑ups added logging and fixes.
- perf-report.19.2: server dominated by `__memmove_avx512_unaligned_erms` (~86%) consistent with terminal rewrap storm after many resizes.
- frame-logs.19.2: thousands of debounced tasks firing post‑drag; corroborates need for generation‑based cancellation (implemented in 19.3).
- frame-logs.19.3/19.4: shows “RESIZE STORM” redundant detections and debounced send logging; indicates paths are now active.

-----

Notable code changes by area (representative, not exhaustive)
- GUI render pipeline
  - `bufferpool.rs`: VertexBufferPool with capacity‑sorted reuse, pow2 growth, stats/logs.
  - `renderstate.rs`: Abstractions for GPU buffers; allocators; integration points for pooling and WebGPU.
  - `termwindow/render/{draw,paint}.rs`: webgpu/glium draw paths; layer iteration and resource setup.
  - `tabbar.rs`, `tab_title_cache.rs`, `callback_cache.rs`: caching and invalidation.
- Wayland
  - `window.rs`: DPI conversions with ceil to avoid losing final row; key repeat scheduling tuned; numerous input/resize paths touched.
  - `presentation.rs`: Presentation feedback modeling, EMA for refresh estimate, optimal render start calculation.
  - `triplebuffer.rs`: Buffer state machine and starvation handling; logging for diagnostics.
  - `gpufence.rs`: Fences integration scaffolding (per build logs); partial usage warnings.
- Mux/Client/Server
  - `wezterm-client/src/pane/clientpane.rs`: resize redundancy detection; viewport-only invalidation; true debounce with generation token; emergency logging.
  - `wezterm-mux-server-impl/src/sessionhandler.rs`: coalesced push logic; palette/config change propagation; dirty line computation constrained to viewport plus cursor line.
  - `mux/src/tab.rs`, `codec/src/lib.rs`, `wezterm-client/src/client.rs`: topology‑aware resize and event propagation (phase 19.2 follow‑ups).

-----

Recommendations before proposing upstream merge
- Normalize logging levels/content
  - Replace emoji/urgent logs with structured trace/debug; keep a temporary feature flag for diagnostics when needed.
- Keep Wayland presentation/triple buffering behind a feature flag
  - Add config/compile‑time flag; document compositor coverage and fallbacks; ensure no regressions when disabled.
- Add targeted tests/integration checks
  - Client resize debounce: unit test the generation token logic; simulate out‑of‑order futures.
  - SessionHandler change computation: verify no duplicate palette sends; ensure cursor line inclusion always works.
  - Wayland rounding conversions: property-based tests for round‑trip pixel↔surface on typical DPIs.
- Validate on diverse environments
  - X11, Wayland (GNOME, KDE, Sway), mixed DPI; confirm no regressions in resize behavior and input repeat.

-----

Acceptance likelihood summary
- Merge now: caching, buffer pooling, client selective invalidation + debounce, server coalesced push, Wayland rounding fixes.
- Stage/flag: presentation/triple buffer/fences until fully wired and measured.

-----

Appendix: Commit/topics map (abridged)
- Phases 0–6: GUI caching, Wayland window tweaks, perf reports.
- Phases 9–12: frame logging, buffer pool, GPU stall diagnostics, cache/invalidation refinement.
- Phases 14–15: render pipeline cleanups; consistent perf deltas.
- Phases 16–18: Wayland presentation/triple buffering/fence scaffolding; window.rs updates; quick‑fixes.
- Phase 19.x: mux/client resize storm mitigation, server push coalescing, remote mux fixes, test logs and follow‑ups; final minor draw.rs fix.

