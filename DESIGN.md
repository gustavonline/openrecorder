# OpenRecorder design

A small, calm, native utility. Neutral surfaces, system text, one black disc, restrained borders, a clear focus ring. No orange, no purple, no accounts, no editor, no extra engine or webview. These rules are what the code implements; the full history of how they were reached stays with the project's private notes.

## Tokens (Home and guest pages)

Light: window `#f5f5f7`, surface `#fff`, sidebar `#f7f7f9`, control `#f0f0f2`, selected `#e5eefd`, separator `#e5e5ea`, hairline `#d8d8de`, text `#1d1d1f` / secondary `#6e6e73` / tertiary `#9a9aa0`, accent `#0a6cdf`, live `#1f8a4c`, record `#e0393e`, primary button `#1d1d1f` on `#fff`.
Dark: window `#161618`, surface `#1e1e21`, control `#2a2a2e`, hairline `#3a3a40`, text `#f2f2f4`, accent `#4c9aff`, record `#e5484d`, primary `#f2f2f4` on `#111113`.
Type: Inter where present, otherwise the system font. The mark is a plain black disc (`assets/icon.svg`; `assets/icon-dark.svg` adds a `#4a4a50` hairline for dark surfaces).

## The one status icon and the menu

One icon in the menu bar or tray: a solid disc, and a square only while frames are being captured; the elapsed time is separate text. Everything about a take (square, time, Stop, Saving…) disappears in the same update the take ends.
Menu: Record / Stop, Source, Camera, Microphone, Open Home, Settings…, Quit. Source, Camera and Microphone show the real names the system reported with one vocabulary: **Off** · **On** (starts when you record; nothing is open, no device light) · **Starting…** · **Live** · **Live · bubble hidden** · **Not connected** · **Access off**. "Remembered" and "Main display" are never shown; an unresolved device has no name rather than a made-up one. Choosing a device, shape, size or source never opens hardware. During a take devices and sources are locked with the reason; shape, size, mirror and Show camera stay live. The menu closes after a choice (the supported tray libraries cannot keep it open); Home's popovers stay open. Settings… opens Home's Settings; there is no separate native Settings window.

## Camera bubble

Circle, Rounded square, Wide (base×4/3 by base×3/4, 16:9) and Portrait (the opposite, 9:16), sizes S 120 / M 168 / L 224 (legacy sizes and a stored square are kept as stored). Hover controls sit at the top edge away from the face (inset 17 % on a circle, 8 pt otherwise): four shape glyphs, × where there is room and never during a take. Right-click offers shapes, sizes, Mirror, Hide Camera, Turn Camera Off and Stop. The bubble is a cover crop, never stretched; `camera.mp4` is the full unmirrored frame. Show camera off hides the bubble on screen and in the combined recording; the camera original continues.

## Home

Header: title, search and a native sort `<select>`, then the capture group: Source chip (190 pt), Camera and Microphone chips (180 pt) with name and status mark, and the red **Record** (96 × 30). The header wraps instead of overflowing. Record is one human press: it starts the countdown directly (launching the app if needed), never "open, then press Record again"; during a take it becomes Stop with the elapsed time; the countdown shows Cancel. Popovers (300 pt) hold the status line, on/off, device list, Show camera, the four shapes, size and Mirror. A recording that stopped early shows "Recording stopped early / Available files were kept." with **Open recording**; the recording page has a neutral note, a collapsed **Technical details**, "Duration unavailable" unless a trustworthy duration exists, and a player only with proof the files were finished. A frame rate picked there applies to next recordings only.
Cards show the take's own poster or an honest state: "Loading preview…", "Preview can't load here", "No preview image", "Can't play / Not finalized". No invented picture or duration. In an agent window a ••• menu holds Open in Browser (secondary), Reload and Settings.

## Sharing and guest pages

Link lasts 15 min · 1 hour · 8 hours · **Never** ("No end time": no deadline while OpenRecorder and the tunnel run; ends on Revoke, Archive, Quit, host exit or a dropped tunnel). Copy link, Copy code, **Copy invitation** (a short message with link and code, no title; only copied). The unlock, wrong-code, limited, access-ended, link-ended and player pages use the tokens above: one narrow column that reflows below 480 pt, a `!` badge for messages, a visible focus ring, light and dark, no script, no inline style, no external resource. The code field accepts pasted, spaced or dashed codes.

## Honest limits

Not notarized; the Windows and Linux builds are experimental and unverified on hardware; 60 fps is a request until measured on your computer; the real public-tunnel path and the embedded agent window have not been exercised in this build by anyone other than the author. Display switching during a take is not offered.
