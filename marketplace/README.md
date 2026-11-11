# Marketplace readiness (preparation only)

Nothing in this folder is submitted, uploaded, verified or approved. It follows the plugin-submission preparation guidance (listing fields, icon rules, five positive and three negative review cases) and keeps every unknown as an explicit gap.

| File | What it is |
|---|---|
| `plugin.submission-draft.json` | Listing fields, prompts, review cases and release notes to merge into a **separate copy** of `plugin.json` for a submission ZIP. The public plugin keeps its own `plugin.json`. |
| `assets/icon-256.png`, `icon-48.png` (+ `icon-dark-*`) | The existing black disc rendered to square PNGs (256 and 48 px) by `scripts/render-icons.py` from the same geometry as `assets/icon.svg`: not a new brand. |

Category: **Productivity** (an exact supported title). Subtitle `Record. Save. Share.` is 20 characters (limit 30).

## Cases come from the real tool surface

The five positive cases use only the agent allowlist (`state`, `open-library` through `openrecorder_home`, `update`, `share`, `stop`); the three negative cases are things the plugin must refuse to do (start a recording, turn devices on or choose a source, edit video). They match `PUBLIC_OPS` and the tests that prove no plugin tool can record (this is about the plugin’s tools, not a claim about agents with shell or computer-control access). **None of them has been run in a host**; mark them Not run until someone does.

## Gaps that need a decision or a person

1. **Verified publisher identity**, supported countries, any translations, and the commerce declaration (there is none: no payments).
2. **Terms of service URL.** No terms page exists and none was invented. The privacy and support pages are drafts of facts about the software (`site/privacy.html`, `site/support.html`), not legal documents; the owner should read them before relying on them.
3. **URLs are prepared, not verified.** `https://openrecorder.gustavonline.com/…` must be deployed, then fetched and inspected, before any field is called complete.
4. **Demo recording.** A real recording of the plugin working in a host (for example Codex's app window) is required for review and does not exist.
5. **MCP endpoint.** The plugin runs a local stdio server inside the bundled binary. The submission guidance asks for "verified MCP endpoints in portable `mcp.json`"; whether a local stdio server is acceptable for the public marketplace is not established here and must be asked of the portal before upload.
6. **Tool annotations.** `openrecorder_home` declares only `readOnlyHint`; the checks need `readOnlyHint`, `openWorldHint` and `destructiveHint` booleans on every tool. The fix is tiny but changes `engine/src/library_host.rs`, so it was left for the release owner to approve.
7. **Reviewer access and environment.** Review needs a library with recordings (made on a real Mac by the reviewer or the owner) and, for the share case, `cloudflared`. No credentials are involved and none belong in this repository.
8. **Platforms.** macOS 26.0 or later on Apple silicon is the primary build (the bundled helper is built for 26.0; state this in the listing). Linux and Windows runtimes exist as experimental, never-run builds; the listing says so and should not claim them as supported.

Submission itself (upload, attestations, review) is a separate, authorised step and has not been started.
