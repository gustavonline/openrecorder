OpenRecorder 0.6.0-preview.13 (build 18)

Fixes recordings stopping when small timing delays add up, and adds a choice of where Home opens.

- Video keeps the time at which frames reach the encoder. A busy encoder can deliver fewer frames without making the video run ahead of live sound or triggering the old cumulative 0.75-second stop. Requested resolution and frame rate remain as selected; a genuinely unresponsive encoder still reports failure.
- Microphone start timing is aligned to video, and finalized interrupted takes report their measured duration.
- Settings → General → Open Home in: default browser, Safari, Chrome, Firefox, Edge, Arc, Brave or the installed ChatGPT/Codex OpenRecorder view (specific applications on macOS). Repeated Home/Settings requests reuse a responding page. A browser's inactive Home tab may still need selecting; unsupported or missing applications report an error.
- Redundant source-text/style checks and an obsolete benchmark were removed. Verification covers scheduler behavior, navigation, and short synthetic hardware encoding with gaps. A 30-second 1440p60 workload with camera, audio and originals completed with matching video/audio length. No long physical-device recording is claimed.

Preview for Apple silicon, **macOS 26.0 or later**. Signed with the existing local development identity; **not notarized**. Experimental Linux/Windows builds, if listed among the assets, are compiled from this source and remain unverified on real desktop capture hardware.

The media helper is unchanged. The release includes the app, plugin, corresponding FFmpeg/x264 sources, OpenRecorder source, manifest and SHA256SUMS. Earlier releases remain unchanged. Temporary sharing still needs the computer and its optional cloudflared tunnel online; this change does not qualify a new public-tunnel test.
