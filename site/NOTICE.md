# Notices for the website

The product page is written for OpenRecorder. Its **structure, style rules and small scripts** (header and mobile navigation, theme toggle, download-choice logic, copy buttons) are adapted from the product page of the author's own [OpenPaperDesign](https://github.com/gustavonline/openpaperdesign). That page's code was itself moved from an MIT-licensed source whose licence is kept in `LEGACY-LICENSE.txt` (MIT, Michael Shimeles). OpenPaperDesign's product features, brand mark, testimonials, counters and screenshots are **not** copied.

Assets and where they come from are listed with their SHA-256 in `assets-sources.json`.

- **Inter** (variable, Latin subset): SIL Open Font License 1.1, `assets/fonts/Inter-LICENSE.txt`, source in `assets/fonts/Inter-SOURCE.txt`. Served from this site; no font service.
- **OpenRecorder black disc** (`assets/logo*.svg`): the project's own mark, the same files as `assets/icon*.svg` in the repository.
- **Codex and Claude Code marks**: from the Lobe Icons static SVG collection (MIT, `assets/licenses/Lobe-Icons-LICENSE`). The marks belong to their owners and appear only to show compatibility, not endorsement.
- **Family links** (`arcitai.svg`, `onlinesourdough.svg`, `gustavonline.jpg`): the author's own marks and portrait, reused from the author’s other sites, unchanged.
- **Screenshots** (`guest-unlock.png`, `guest-player.png`): actual captures of OpenRecorder's guest pages served by the real Rust handler to Firefox 159 from a disposable local test server with a synthetic generated video. They show no real recording, person, library or account. Provenance and limits are in `assets-sources.json`.
- The menu and folder pictures are labelled illustrations drawn in HTML, not screenshots.

OpenRecorder itself is MIT licensed; the media helper is FFmpeg with x264 (GPL-2.0-or-later) run as a separate process, with sources published beside each release. Not affiliated with Apple, OpenAI or Anthropic.
