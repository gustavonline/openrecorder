# OpenRecorder product page

Job: tell a visitor in one screen what OpenRecorder is, give an honest way to get it, and show real things. Primary action: a verified download when one exists, otherwise a clear pointer to the install steps; never a dead button.

Structure: quiet header (wordmark with the black disc, the author's Online menu, Features, Install, GitHub, theme toggle; an icon-only full-screen menu at 800 px and below) → large headline and one paragraph → status line → actual guest-page screenshots → three feature rows (menu illustration, folder illustration, sharing with a real screenshot) → agents band (can / cannot) → local-first and open-source → install cards and a plain "what is and is not verified" list → footer with the author's other sites, project links and credits.

Look: the app's neutral tokens (window `#f5f5f7` / `#161618`, surface `#fff` / `#1e1e21`, ink `#1d1d1f` / `#f2f2f4`, hairline `#d8d8de` / `#3a3a40`, focus `#0a6cdf` / `#4c9aff`), Inter served locally, one black disc with a hairline so it survives dark mode, restrained borders, a visible focus ring, no orange or purple, light and dark by system preference with a toggle.

Truth rules: a screenshot is an actual capture and says what it is; an illustration says it is one; no invented camera feed, demo recording, user count, benchmark, "almost zero RAM" or "notarized" claim; Windows and Linux are stated as experimental builds, never run on hardware (or source-only while none is published); the download state comes only from `releases.json`, which `scripts/update-site-release.py` writes after the published files are verified.

Constraints: static HTML, CSS and modules; no framework, bundler, analytics, third-party request or cookie; a strict policy (`_headers`: no inline script or style).

Acceptance (done by a person in a browser, not by this repository's checks): wide, intermediate (about 900 px), mobile (390 px) and narrow (320 px); both themes; keyboard and the mobile menu; the two screenshot tabs; Copy buttons including clipboard failure; no horizontal scroll; the main button before and after a release is published.
