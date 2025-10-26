• Here’s what I found and what to do next.

  Observations

  - Client looks fixed: logs show redundant resizes are short-circuited and UI frames are OK
      - File: chats/frame-logs.19.2
      - Multiple “🔴 RESIZE STORM: Redundant resize …” lines confirm the early-return guard is working across
        panes
  - Server 100% CPU post-resize points to heavy memory copying
      - Perf shows 86% time in libc __memmove_avx512_unaligned_erms; other visible symbols are string/grapheme
        ops and wezterm_surface::line::Line::set_cell_grapheme
      - File: chats/perf-report.19.2
  - Code changes since 13b6310 implement “size-only TabResized” and avoid client resync for size-only
      - Server emits richer TabResized
          - mux/src/lib.rs:86
          - mux/src/tab.rs:1197
      - Client handles size-only without resync
          - wezterm-client/src/client.rs:300

  Likely cause of server CPU peg

  - After a resize (width changes), many lines rewrap and the terminal model dirties a large range. Server then
    repeatedly constructs/sends changes, copying a lot of memory.
  - Potential multipliers:
      - Pushes are scheduled on every PaneOutput without backpressure
          - wezterm-mux-server-impl/src/dispatch.rs:112
          - wezterm-mux-server-impl/src/sessionhandler.rs:233
      - Per-push work can scan and assemble a lot of data
          - compute_changes compares dimensions, cursor, title, working dir and composes bonus_lines plus all
            dirty lines
          - wezterm-mux-server-impl/src/sessionhandler.rs:52
      - With no coalescing and a flood of PaneOutput, the server spins copying buffers for minutes

  Gaps still present

  - Client “debounce” isn’t a real debounce (spawns a new delayed send each time). Multiple resizes still reach
    the server
      - wezterm-client/src/pane/clientpane.rs:438
  - Server-side push is “immediate per-notification”; no per-pane throttle or “push in progress” flag
      - wezterm-mux-server-impl/src/sessionhandler.rs:233

  High‑impact next steps

  1. Fix client debounce for resize

  - Implement a true debounce per tab (or window), not per event:
      - Keep a shared “pending size”
      - Reset/restart a single timer on new events
      - Only send the latest size when the timer fires
  - Where: wezterm-client/src/pane/clientpane.rs:389
  - Expected effect: drastic reduction in Pdu::Resize and downstream server churn

  2. Coalesce server pushes per pane

  - Add a simple per‑pane “push pending” flag + short coalescing window (e.g., 16–33ms)
      - On PaneOutput, if pending, skip; otherwise mark pending and schedule one maybe_push_pane_changes after
        the window
      - Clear pending when the push completes
  - Where:
      - Define pending map: wezterm-mux-server-impl/src/sessionhandler.rs:210
      - Use in schedule_pane_push: wezterm-mux-server-impl/src/sessionhandler.rs:233
  - Expected effect: collapse hundreds/thousands of pushes to a handful; reduce memmoves

  3. Limit compute scope immediately after resize

  - For N ms after a resize per tab, restrict compute_changes to viewport + small margin and avoid scanning
    large dirty ranges outside viewport
      - Option 1: add a “resize_quiet_until: Instant” per tab, or per pane
      - Option 2: cap dirty_lines sent per push (send the rest later)
  - Where:
      - Track last resize: in tab/pane, propagate a flag reachable from sessionhandler’s compute_changes
      - Bound work in: wezterm-mux-server-impl/src/sessionhandler.rs:52
  - Expected effect: less memory copying per response, faster quiescence

  4. Add backpressure/batching to PaneOutput → push pipeline

  - Don’t schedule a new push while one is running for the same pane
  - If more PaneOutput arrive, mark “needs another pass” and run one more push when the current finishes; avoid
    unbounded per-event work
  - Where:
      - State + logic in SessionHandler: wezterm-mux-server-impl/src/sessionhandler.rs:210

  5. Attribute memmove to call sites with better profiling

  - Re‑run perf with call graph DWARF: perf record -g --call-graph dwarf …
  - Build with frame pointers if needed to get meaningful stacks: -C force-frame-pointers=yes
  - Target: identify exact hot loops (rewrap, serialization, line building) that are copying bulk memory

  6. Instrumentation to prove improvement

  - Server-side metrics and logs:
      - Count/summarize schedule_pane_push per pane, and “push duration” and “bytes sent”
          - wezterm-mux-server-impl/src/sessionhandler.rs:233
      - In compute_changes, log:
          - dirty_lines count, bonus_lines count
          - viewport size and seqno advance
          - elapsed time
          - wezterm-mux-server-impl/src/sessionhandler.rs:52
      - Log when a resize is received and when rewrap completes per pane/tab; measure rewrap time
          - wezterm-mux-server-impl/src/sessionhandler.rs:633
          - mux/src/termwiztermtab.rs:202 (pane.resize) and/or terminal rewrap locations
  - Client-side metrics:
      - Count resize sends per drag burst; confirm true debounce
          - wezterm-client/src/pane/clientpane.rs:438
      - Count TabResized handling by topology flag (sanity)
          - wezterm-client/src/client.rs:300

  Validation path

  - Before changes
      - Observe server pegged CPU post-drag; collect perf with call graph
      - Capture logs for push counts and compute_changes statistics
  - After client true debounce
      - Expect dramatic reduction in Pdu::Resize count during drag
      - Verify fewer TabResized and reduced server churn
  - After server push coalescing + compute limits
      - Expect push rate to stabilize (<= 30–60/sec per pane)
      - compute_changes time and dirty_lines counts drop; total bytes per second down
      - CPU clears within a second or two after drag ends

  File references

  - Client resize debounce: wezterm-client/src/pane/clientpane.rs:438
  - Client TabResized handling: wezterm-client/src/client.rs:300
  - Mux TabResized struct: mux/src/lib.rs:86
  - Mux Tab::resize notification: mux/src/tab.rs:1197
  - Server dispatch notifications: wezterm-mux-server-impl/src/dispatch.rs:120
  - Server schedule_pane_push: wezterm-mux-server-impl/src/sessionhandler.rs:233
  - Server compute_changes: wezterm-mux-server-impl/src/sessionhandler.rs:52
  - Server handle Resize PDU: wezterm-mux-server-impl/src/sessionhandler.rs:633

  Summary next actions

  - Implement real client debounce for resize RPCs.
  - Add per‑pane push coalescing and “one in flight” guard on the server.
  - Temporarily bound compute_changes scope and/or cap work per push immediately after resize.
  - Add targeted server/client instrumentation and re‑profile with call graphs to verify the source of memmove
    and the reductions.


› Review analyses from a third party:
  -         chats/phase-19.2-assessment-summary.md
  -         chats/phase-19.2-server-hang-analysis.md
  -         chats/phase-19.3-action-plan.md

  and comment on their validity, viability, and effectiveness. And propose next steps.


• Explored
  └ Read phase-19.2-assessment-summary.md, phase-19.2-server-hang-analysis.md, phase-19.3-action-plan.md

─ Worked for 55s ──────────────────────────────────────────────────────────────────────────────────────────────

• Here’s a concise review of the three third‑party analyses and what to do next.

  Validity

  - Phase 19.2 assessment summary (chats/phase-19.2-assessment-summary.md)
      - Largely correct:
          - Client responsiveness improved by skipping resync() on size-only TabResized is validated. See
            wezterm-client/src/client.rs:300.
          - Server hang is due to resize RPC flood; “debounce” is not real (spawns per-event tasks without
            cancellation). See wezterm-client/src/pane/clientpane.rs:438.
      - Nuance:
          - Redundant-resize blocking helps but is insufficient in multi-pane layouts because many panes
            legitimately change size. The conclusion that broken debounce must be fixed is correct.
  - Server hang analysis (chats/phase-19.2-server-hang-analysis.md)
      - Valid cause: perf shows 86% time in memmove during the hang, consistent with repeated buffer rewraps/
        copies after many resizes. See chats/perf-report.19.2.
      - Correct identification of client task flood post-drag.
      - One caution: the document implies “thousands of RPCs” as a certainty; that’s plausible given the client
        logs, but confirm via server-side counters to be precise.
  - Phase 19.3 action plan (chats/phase-19.3-action-plan.md)
      - Valid prescription:
          - Implement true client-side debounce (shared pending state + generation token).
          - Add server-side protection (coalescing/dedup/rate limiting).
      - Caution on server “rate limiting”:
          - Dropping resizes by time alone risks leaving an incorrect final size under real drags. Prefer
            last-writer-wins coalescing (apply the latest size after a short quiet period) over outright rate
            limiting.

  Viability

  - Client true debounce (high)
      - Straightforward to add a pending_resize field with generation and only send when the timer fires if
        generation matches. Implement in wezterm-client/src/pane/clientpane.rs:389.
  - Server resize coalescing (high)
      - Two viable places:
          - In mux server impl at the PDU handling site: coalesce in wezterm-mux-server-impl/src/
            sessionhandler.rs:633 by storing per-tab “pending resize” and scheduling a single apply after 50–
            100ms, overwriting with newer sizes.
          - Or in mux/src/tab.rs by adding a short coalescing window, but sessionhandler is simpler to control
            per-session traffic.
  - Push coalescing (high)
      - Prevent unbounded “push on every PaneOutput”: add per-pane “push pending”/“in flight” state and a small
        coalescing window in wezterm-mux-server-impl/src/sessionhandler.rs:233.
  - compute_changes scoping (medium)
      - During immediate post-resize, limit work to viewport + small margin to avoid scanning/copying large
        regions. Implement logic in wezterm-mux-server-impl/src/sessionhandler.rs:52 (e.g., cap dirty_lines
        processed/sent per push while a per-tab “recent resize” window is active).
  - Instrumentation (high)
      - Add counters/timing for resize RPCs, pane pushes, bytes, and compute time. See “Next steps” for exact
        points.

  Effectiveness

  - Client true debounce
      - Expected: burst of resizes → 1 RPC per pane/tab after quiet period. Eliminates the resize flood root
        cause; server CPU returns to normal.
  - Server resize coalescing
      - Belt-and-suspenders. Even if a client regresses, server applies at most one resize per coalescing
        window (last-writer-wins). Prevents prolonged hang.
  - Push coalescing
      - Reduces repeated serialization and copying work triggered by rapid notifications; shrinks the long tail
        after resize.
  - compute_changes scoping (post-resize)
      - Limits per-push copying; accelerates quiescence. Combine with coalesced pushes for best results.

  Proposed next steps

  - Implement client true debounce for resize
      - Where: wezterm-client/src/pane/clientpane.rs:389 and :438.
      - Add pending_resize: Mutex { generation: usize, size: Option<TerminalSize> }.
      - On each resize: update pending, bump generation, start/restart a single timer; on fire, send only if
        generation matches.
  - Add server-side resize coalescing (last-writer-wins)
      - Where: wezterm-mux-server-impl/src/sessionhandler.rs:633 (Pdu::Resize).
      - Maintain pending_resizes: HashMap<TabId, { size, generation }>; schedule one delayed apply (50–100ms).
        Later requests overwrite; when timer fires, apply the latest.
  - Coalesce pane pushes and avoid overlapping pushes
      - Where: wezterm-mux-server-impl/src/sessionhandler.rs:233 schedule_pane_push.
      - Add per-pane “push_pending” and “push_in_flight”. If another PaneOutput arrives while pending or in
        flight, set a dirty bit and run only one more push after the current finishes or at next coalescing
        tick.
  - Bound compute_changes during immediate post-resize (optional, quick win)
      - Where: wezterm-mux-server-impl/src/sessionhandler.rs:52.
      - Track last-resize instant per pane/tab; if within N ms, cap dirty scope to viewport + margin (and/or
        cap number of dirty lines processed per push).
  - Instrumentation to validate
      - Client:
          - Count debounced resize scheduled/cancelled/sent (clientpane.rs: resize).
      - Server:
          - Count Pdu::Resize received and applied (sessionhandler.rs:633).
          - Count/summarize schedule_pane_push executions and push durations/bytes (sessionhandler.rs:233 and
            compute_changes end).
          - Log dirty_lines counts and elapsed time in compute_changes (sessionhandler.rs:52).
      - Re-run perf with call graph to attribute memmove properly (build with frame pointers if needed).
  - Validate with a 2-second drag test
      - Expect:
          - Client: many GUI events; only one “debounced resize sent” per pane/tab.
          - Server: 1 applied resize per pane/tab per drag burst; a handful of pane pushes; CPU clears quickly
            (<1–2s) after drag ends.

  File references

  - Client debounce: wezterm-client/src/pane/clientpane.rs:389, wezterm-client/src/pane/clientpane.rs:438
  - Client TabResized handling: wezterm-client/src/client.rs:300
  - Mux notification type: mux/src/lib.rs:86
  - Mux Tab::resize notify: mux/src/tab.rs:1197
  - Server Resize handling: wezterm-mux-server-impl/src/sessionhandler.rs:633
  - Server push scheduling: wezterm-mux-server-impl/src/sessionhandler.rs:233
  - Server compute_changes: wezterm-mux-server-impl/src/sessionhandler.rs:52
  - Server dispatch of notifications: wezterm-mux-server-impl/src/dispatch.rs:120

  Summary

  - The third-party analyses are broadly correct: client true debounce is critical, and server protections/
    coalescing should be added to prevent hangs.
  - Minor correction: don’t rely on time-based rate limiting alone; prefer last-writer-wins coalescing to
    ensure the final size is applied.
  - Execute the above stepwise plan (client → server coalescing → push coalescing → compute scoping +
    instrumentation), then re-profile to confirm the long tail is gone.

