OpenRecorder 0.6.0-preview.14 (build 19)

Fixes another cause of interrupted recordings, including 1080p at 30 fps: on macOS, the accepted microphone socket inherited non-blocking mode. When FFmpeg briefly stopped reading and its loopback buffers filled, that was incorrectly treated as a fatal audio error.

The connection is now explicitly blocking, transient backpressure is retried without duplicating partial writes, and queued samples are drained at Stop. Memory remains bounded. A genuinely disconnected or stuck encoder still fails clearly. If the microphone fails but video files are finalized, new interrupted takes keep their measured duration and playback, with the error retained.

The short real-socket regression reproduces the exact failure on the shipped implementation and passes with the correction. This is not a long physical-device recording claim. Browser destination persistence and actual opening inside ChatGPT/Codex were separately confirmed; no browser preference code changed.

Preview: Apple silicon, **macOS 26.0 or later**, existing local development signature, **not notarized**. Linux and Windows builds remain experimental and unverified on desktop capture hardware. The media helper is unchanged. App/plugin, source, corresponding FFmpeg/x264 sources, manifest and SHA256SUMS are included. Earlier releases remain unchanged.
