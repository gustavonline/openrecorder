#!/usr/bin/env python3
"""Turn the product page's download state from `pending` to `published`, only after the published files verify.

  python3 scripts/update-site-release.py DIST          # DIST: the directory made by make-release.py (manifest.json, SHA256SUMS)

Steps: (1) require the published manifest.json and SHA256SUMS to be byte-identical to the reviewed ones in DIST, then download every
asset from this repository's GitHub release URL and verify it against that manifest (scripts/verify-release.py --base-url DIST --expect);
(2) only if that passes, rewrite site/releases.json from the reviewed manifest and a verification time. If the release is not online,
differs from the reviewed one in any way, or any file differs, nothing is written and the page keeps offering no download.
Review and commit the changed releases.json, then deploy the site (docs/release.md).
"""
import datetime, json, pathlib, subprocess, sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import release_tools as rt


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    dist = pathlib.Path(sys.argv[1]).resolve()
    manifest = json.loads((dist / 'manifest.json').read_text())
    version, tag = manifest['version'], manifest['tag']
    assert tag == f'v{version}' and manifest['repository'] == rt.REPO
    base = f'https://github.com/{rt.REPO}/releases/download/{tag}'
    run = subprocess.run([sys.executable, str(rt.ROOT / 'scripts/verify-release.py'), '--base-url', base, '--expect', str(dist)])
    if run.returncode != 0:
        sys.exit('The published release did not verify or is not the reviewed one; site/releases.json was not changed.')
    platforms = {}
    for a in manifest['assets']:
        if a['kind'] == 'app':
            entry = {'architecture': a['architecture'], 'name': a['name'], 'url': f"{base}/{a['name']}", 'sha256': a['sha256'], 'bytes': a['bytes'],
                     'status': a.get('qualification') or 'Preview, not notarized'}
            if a.get('minimumOS'):
                entry['minimumOS'] = a['minimumOS']
            platforms[a['platform']] = entry
    out = {'schema': 1, 'repository': rt.REPO, 'version': version, 'build': manifest['build'], 'tag': tag, 'status': 'published',
           'verifiedAt': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'), 'platforms': platforms,
           'checksums': f'{base}/SHA256SUMS', 'sources': f"{base}/{manifest['correspondingSources']}",
           'notice': 'Verified against the release manifest by scripts/update-site-release.py. Builds for other platforms exist only where listed above.'}
    (rt.ROOT / 'site/releases.json').write_text(json.dumps(out, indent=2) + '\n')
    print(json.dumps(out, indent=2))


if __name__ == '__main__':
    main()
