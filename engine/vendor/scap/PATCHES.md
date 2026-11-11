# OpenRecorder patches

Based on scap 0.0.8 from crates.io, MIT.

- Nonblocking frame channel capped at two entries; full queues drop incoming frames.
- Timed frame retrieval allows cancellation without waiting for another screen change.
- PipeWire copies respect plane offset and row stride; negotiate supported 32-bit formats.
- Windows capture settings updated for windows-capture 1.5; strip row padding.
- All control channels retain their original semantics.

The application samples the latest screen/camera frames on one fixed output clock, bounds encoder writes and preserves recoverable MP4 fragments on failure.
- Candidate B: macOS window targets carry `owner_pid` (ScreenCaptureKit owning application; Windows via `GetWindowThreadProcessId`; None where the platform gives no ownership). Ownership is a process fact, never a title prefix.
- Candidate B: macOS `get_scale_factor`/`get_target_dimensions` for a window use ScreenCaptureKit's own window frame (points) and the backing scale of the display holding most of it, instead of `NSApp().windowWithWindowNumber`, which only knows this application's windows. A missing/closed window gives `Err(CapturerBuildError::TargetUnavailable)` from `Capturer::build` rather than a panic or a 0 x 0 stream. Sizes are points, scale is pixels per point; nothing is padded or stretched.
