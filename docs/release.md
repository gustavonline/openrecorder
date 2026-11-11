# Releasing

For the person publishing (not needed to use or build OpenRecorder). Every step that talks to GitHub or Cloudflare is deliberate and manual; nothing in CI publishes.

## What a release contains

Only builds that exist for this version, never an older build relabelled. The macOS app is the primary build. Linux and Windows cores are added only when you pass real builds of this version to `make-release.py` (`--linux-core/--linux-helper`, `--windows-core/--windows-helper/--windows-dll`, `--built-from`); they are labelled experimental in the manifest and on the page because they have not been run on hardware:

| Asset | Contents |
|---|---|
| `OpenRecorder-<version>-mac-arm64.zip` | One folder: the signed macOS app, byte for byte, and beside it the licence texts and notices (`LICENSE`, `THIRD_PARTY.md`, `licenses/`, `README.txt`, `INSTALL.md`); nothing is added inside the bundle |
| `openrecorder-plugin-<version>-darwin-arm64.zip` | The local plugin (Codex and Claude Code), carrying the same app as `.runtime/` |
| `openrecorder-media-sources-<version>.tar.gz` | Corresponding FFmpeg/x264 sources and build scripts (GPL obligation for the bundled helper) |
| `openrecorder-<version>-source.tar.gz` | The source tree |
| `OpenRecorder-<version>-linux-x64.tar.gz`, `OpenRecorder-<version>-windows-x64.zip`, `openrecorder-plugin-<version>-linux-x64.zip`, `…-win32-x64.zip` | Experimental: core + media helper (+ MinGW DLL) with licences, only if built |
| `SHA256SUMS`, `manifest.json` | Sizes and SHA-256 of every asset, the source tree hash, the executable and helper hashes, `notarized: false`, the qualification and source archive of each non-macOS build, and the platforms that are not published |

A build of another platform must come from the same source as the macOS app; say in `manifest.json` (the `qualification` field) what was and was not tested.

## Steps

1. **Clean checkout of the commit to release**; `plugin.json`, `.claude-plugin/plugin.json`, `engine/Cargo.toml`, the page's `ui/initialize` version and `CFBundleVersion` in `scripts/build-native.py` agree (the Rust tests and `scripts/verify-release.py` check this).
2. **Build the app** with `scripts/build-native.py /path/to/ffmpeg` on a Mac, signed with your own identity (`OPENRECORDER_SIGN_IDENTITY`, optionally `OPENRECORDER_SIGN_KEYCHAIN`; the default is an ad-hoc signature). Keep the same identity across releases or macOS treats each as a new app. Do not paste the identity or the keychain path into any file in this repository.
3. **Assemble**: `python3 scripts/make-release.py --app /path/to/OpenRecorder.app --media-sources /path/to/media-sources.tar.gz --out dist` (the bundle is only read).
4. **Verify locally**: `python3 scripts/verify-release.py dist` and `python3 scripts/check-site.py`. Both must print `"passed": true`.
5. **Check by hand** what automation cannot: open the app from the ZIP on a Mac, see the menu-bar disc, `OpenRecorder --doctor`; add the plugin ZIP to Codex and to Claude Code as in `docs/install.md`; record a short take yourself; open Home; create and open a share with a real tunnel if you can. Write down what you ran.
6. **Publish the release** (a pre-release while the product is a preview):
   ```sh
   git tag -s v<version>   # or an annotated tag
   gh release create v<version> dist/* --prerelease --title "OpenRecorder <version>" --notes-file docs/release-notes-<version>.md
   ```
   The notes are the checked-in file `docs/release-notes-<version>.md` (for 0.6.0-preview.11: `docs/release-notes-0.6.0-preview.11.md`). They must say: preview, macOS 26.0 or later on Apple silicon, not notarized, Linux and Windows builds experimental and never run on hardware, and carry the checksums.
7. **Let the verify workflow run** (or run `python3 scripts/verify-release.py --base-url https://github.com/gustavonline/openrecorder/releases/download/v<version>`): it downloads every asset and checks it.
8. **Open the download on the site** only now: `python3 scripts/update-site-release.py dist` (the reviewed local directory) first requires the published `manifest.json` and `SHA256SUMS` to be byte-identical to the local ones, then re-verifies the published files and rewrites `site/releases.json` from the reviewed manifest; a coherent release that differs from the reviewed one is refused. Review, commit and push it.
9. **Deploy the site** (Cloudflare Pages, existing account; `wrangler login` first):
   ```sh
   npx wrangler pages deploy site --project-name openrecorder --branch main
   ```
   First time: create the Pages project `openrecorder` in the existing account and add the custom domain `openrecorder.gustavonline.com` in the Cloudflare dashboard (the zone is already there); wait for the certificate. Then fetch `https://openrecorder.gustavonline.com/`, `/releases.json`, `/support.html` and `/privacy.html` and read them: a successful status is not enough.
10. **Rollback**: re-deploy the previous site commit; to retract a release, delete it on GitHub and restore `site/releases.json` to `pending`. `scripts/check-site.py` refuses a `published` file without a verification record.

## What the checks do and do not prove

`verify-release.py` (with `--base-url … --expect dist` it also demands byte-identical published manifest and checksums; `tests/release-guard.py` proves that offline): hashes, sizes, the plugin's manifests and executable bit, safe archive names, no key material or private paths, `codesign --verify` of the app (on macOS). `check-site.py`: links and assets resolve, provenance hashes, no inline script or style, a strict policy, no orange/purple, no forbidden claims, the release guard. They do **not** prove the app records correctly, that Codex or Claude Code loads the plugin, that sharing works over a real tunnel, or how the page looks: those are the manual step above.

## Notarization

The app is signed with a local identity and is not notarized, so macOS shows its unknown-developer warning. Notarization needs an Apple Developer ID and Apple's service; it is not part of this process and the documentation never claims it.
