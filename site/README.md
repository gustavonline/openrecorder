# Product page

Static HTML, CSS and ES modules with locally served Inter. No build step, framework, analytics or third-party request. Domain: **https://openrecorder.gustavonline.com/** (Cloudflare Pages, on the owner's existing account and zone).

```sh
python3 -m http.server 4390 --bind 127.0.0.1 --directory site     # look at it
python3 scripts/check-site.py                                      # structure, assets, policy, release guard
```

`releases.json` decides what the main button offers. It says `pending` until a release is published and verified; then `scripts/update-site-release.py` rewrites it from the release manifest after downloading and checking every asset. `scripts/check-site.py` refuses a `published` file without a verification record, a link to a release asset while pending, and (with `--network`) any asset that does not match.

Deploy exactly the files in this folder (the repository root is never uploaded); see `docs/release.md`. A source edit is not a deployment.
