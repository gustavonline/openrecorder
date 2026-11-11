#!/usr/bin/env python3
"""Verify a set of release assets against their manifest. Read-only.

  python3 scripts/verify-release.py DIST                       # a local directory made by make-release.py
  python3 scripts/verify-release.py --base-url https://github.com/gustavonline/openrecorder/releases/download/v0.6.0-preview.11
                                                                 # download every asset and check it (what the release workflow does)
  python3 scripts/verify-release.py --base-url URL --expect DIST  # the same, and the published manifest.json and SHA256SUMS must be
                                                                 # byte-identical to the reviewed local ones in DIST, or nothing is trusted

Checks: SHA256SUMS and manifest.json agree with each file's size and hash; only the platforms listed as built are present; the
version agrees across plugin.json, the Claude manifest, the app's Info.plist and the file names; the plugin ZIP is a complete local
plugin (contained MCP command, both marketplace files, the runtime, no Node, no private markers); ZIPs and tarballs have no unsafe
names, links, keys or private paths; the corresponding-sources archive is present; the standalone macOS ZIP carries the licence texts and
notices next to the app; the macOS app verifies with codesign (on macOS).
Exit status 1 on any problem; every problem is printed.
"""
import argparse, json, pathlib, plistlib, re, subprocess, sys, tempfile, urllib.request, zipfile
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import release_tools as rt

problems = []


def fail(msg):
    problems.append(msg); print('FAIL', msg)


def ok(msg):
    print('ok  ', msg)


def fetch(base, name, dest):
    req = urllib.request.Request(f'{base.rstrip("/")}/{name}', headers={'User-Agent': 'openrecorder-verify-release'})
    with urllib.request.urlopen(req, timeout=120) as r, open(dest, 'wb') as f:
        while block := r.read(1 << 20):
            f.write(block)


def reviewed_identity_problems(published, reviewed):
    """A published release may only be trusted if its manifest.json and SHA256SUMS are byte-identical to the reviewed local ones.
    Self-consistency of a remote release is not enough: a coherent release with other files would pass every other check."""
    found = []
    for name in ('manifest.json', 'SHA256SUMS'):
        a, b = pathlib.Path(published) / name, pathlib.Path(reviewed) / name
        if not b.is_file():
            found.append(f'the reviewed {name} is missing, so the published release cannot be compared with it')
        elif not a.is_file() or a.read_bytes() != b.read_bytes():
            found.append(f'the published {name} differs from the reviewed local {name}')
    return found


def check_media_archive(path):
    """The portable Windows build script must find its source archive where the archive really keeps it, and the executed
    history must still be there. The script is run from a scratch copy only as far as the extraction (it then stops at configure,
    which has no cross toolchain here); nothing outside the scratch directory is touched."""
    import tarfile
    with tempfile.TemporaryDirectory() as t:
        with tarfile.open(path) as tar:
            tar.extractall(t, filter='data')
        (top,) = [p for p in pathlib.Path(t).iterdir() if p.is_dir()]
        script = (top / 'build-windows.sh').read_text()
        names = {p.name for p in top.iterdir()}
        wanted = re.findall(r'tar -x\w+ (\S+) -C windows-src', script)
        good = bool(wanted) and all(w in names for w in wanted) and 'ffmpeg-n8.0' not in script and '/home/' not in script
        (ok if good else fail)(f'{path.name}: build-windows.sh extracts {wanted or "nothing"}, which is in the archive next to the script, and names no private path')
        (ok if 'recorded-build-windows.sh' in names and 'recorded-build-linux.sh' in names and 'recorded-build-mac.sh' in names else fail)(f'{path.name}: the executed build history is kept')
        subprocess.run(['/bin/sh', str(top / 'build-windows.sh')], cwd=t, env={'PATH': '/usr/bin:/bin', 'CROSS_PREFIX': 'no-such-toolchain-'}, capture_output=True)
        (ok if (top / 'windows-src/configure').is_file() else fail)(f'{path.name}: running build-windows.sh extracts FFmpeg to windows-src before configure (the layout matches the script)')


def check(dist):
    manifest = json.loads((dist / 'manifest.json').read_text())
    version = manifest['version']
    plugin = json.loads((rt.ROOT / 'plugin.json').read_text()); claude = json.loads((rt.ROOT / '.claude-plugin/plugin.json').read_text())
    (ok if plugin['version'] == claude['version'] == version else fail)(f'version {version} agrees with plugin.json and the Claude manifest')
    if manifest.get('tag') != f'v{version}':
        fail('tag does not match the version')
    sums = {l.split('  ', 1)[1]: l.split('  ', 1)[0] for l in (dist / 'SHA256SUMS').read_text().splitlines() if l.strip()}
    names = {a['name'] for a in manifest['assets']}
    (ok if names == set(sums) else fail)('SHA256SUMS lists exactly the manifest assets')
    for a in manifest['assets']:
        p = dist / a['name']
        if not p.is_file():
            fail(f"missing {a['name']}"); continue
        good = p.stat().st_size == a['bytes'] and rt.sha256(p) == a['sha256'] == sums.get(a['name'])
        (ok if good else fail)(f"{a['name']}: size and sha256 match the manifest and SHA256SUMS")
        if version not in a['name']:
            fail(f"{a['name']} does not carry the version {version}")
    extra = {p.name for p in dist.iterdir()} - names - {'SHA256SUMS', 'manifest.json'}
    (ok if not extra else fail)(f'no unlisted files{": " + ", ".join(sorted(extra)) if extra else ""}')
    by = lambda kind, platform: next((a for a in manifest['assets'] if a['kind'] == kind and a['platform'] == platform), None)
    for kind in ('media', 'source'):
        (ok if any(a['kind'] == kind for a in manifest['assets']) else fail)(f'{kind} asset present')
    published = {a['platform'] for a in manifest['assets'] if a['platform']}
    (ok if 'macos' in published else fail)('the macOS build is present')
    for platform, arch in (('linux-x64', 'linux'), ('win32-x64', 'windows')):
        listed_missing = platform in manifest.get('notPublished', {})
        (ok if listed_missing != (arch in published) else fail)(f'{platform}: exactly one of "published" and "listed as not published"')
        if arch in published:
            for a in manifest['assets']:
                if a['platform'] == arch and not a.get('qualification'):
                    fail(f"{a['name']}: a non-macOS build must say plainly what was and was not tested")
                if a['platform'] == arch and not a.get('builtFromSourceArchiveSha256'):
                    fail(f"{a['name']}: a non-macOS build must record the source archive it was built from")
    if manifest.get('notarized') is not False:
        fail('manifest must say plainly that the build is not notarized')
    if manifest.get('correspondingSources') not in names:
        fail('corresponding-sources archive is not among the assets')
    exe_of = {'macos': 'OpenRecorder.app/Contents/MacOS/OpenRecorder', 'linux': 'openrecorder-core', 'windows': 'openrecorder-core.exe'}
    for plugin_asset in (a for a in manifest['assets'] if a['kind'] == 'plugin'):
        exe = 'openrecorder/.runtime/' + exe_of[plugin_asset['platform']]
        with zipfile.ZipFile(dist / plugin_asset['name']) as z:
            members = z.namelist()
            portable = json.loads(z.read('openrecorder/mcp.json'))['mcpServers']['openrecorder']
            claude_mcp = json.loads(z.read('openrecorder/.mcp.json'))['mcpServers']['openrecorder']
            good = (portable['command'] == './.runtime/' + exe_of[plugin_asset['platform']] and portable['args'] == ['--stdio'] and exe in members
                    and claude_mcp['command'] == '${CLAUDE_PLUGIN_ROOT}/.runtime/' + exe_of[plugin_asset['platform']]
                    and 'openrecorder/.agents/plugins/marketplace.json' in members and 'openrecorder/.claude-plugin/marketplace.json' in members
                    and json.loads(z.read('openrecorder/plugin.json'))['version'] == version and 'node' not in json.dumps([portable, claude_mcp]).lower())
            (ok if good else fail)(f"{plugin_asset['name']}: contained MCP command, runtime present, both marketplace files, version, no Node")
            (ok if (z.getinfo(exe).external_attr >> 16) & 0o111 else fail)(f"{plugin_asset['name']}: the executable keeps its execute bit")
            if plugin_asset['platform'] != 'macos':
                helper = 'openrecorder/.runtime/ffmpeg' + ('.exe' if plugin_asset['platform'] == 'windows' else '')
                magic = z.read(exe)[:4]
                (ok if helper in members and magic[:2] in (b'\x7fE', b'MZ') else fail)(f"{plugin_asset['name']}: media helper next to the core, and the core is an ELF or PE executable")
                if plugin_asset['platform'] == 'windows':
                    (ok if 'openrecorder/.runtime/libwinpthread-1.dll' in members else fail)(f"{plugin_asset['name']}: MinGW runtime DLL included")
    import tarfile
    for std in (a for a in manifest['assets'] if a['kind'] == 'app' and a['platform'] in ('linux', 'windows')):
        path = dist / std['name']
        names_in = zipfile.ZipFile(path).namelist() if path.suffix == '.zip' else tarfile.open(path).getnames()
        top = std['name'].rsplit('.zip', 1)[0].removesuffix('.tar.gz')
        core = 'openrecorder-core' + ('.exe' if std['platform'] == 'windows' else '')
        need = [f'{top}/{n}' for n in (core, 'ffmpeg' + ('.exe' if std['platform'] == 'windows' else ''), 'LICENSE', 'THIRD_PARTY.md', 'README.txt', 'INSTALL.md', 'licenses/ffmpeg-COPYING.GPLv2', 'licenses/media-provenance.json')]
        (ok if all(n in names_in for n in need) else fail)(f"{std['name']}: the core, the helper, the licences and the notice travel together")
    check_media_archive(dist / next(a['name'] for a in manifest['assets'] if a['kind'] == 'media'))
    for a in manifest['assets']:
        p = dist / a['name']
        if not p.is_file():
            continue
        found = rt.zip_problems(p) if p.suffix == '.zip' else rt.tar_problems(p)
        for line in found:
            fail(line)
        if not found:
            ok(f"{a['name']}: safe names, no links, keys or private paths")
    mac = by('app', 'macos')
    stated = manifest.get('macos') or {}
    required = max(stated.get('executableMinos', '0'), stated.get('mediaHelperMinos', '0'), key=lambda v: tuple(int(x) for x in v.split('.')))
    (ok if stated.get('minimumOS') == required and tuple(int(x) for x in required.split('.')) > (0,) else fail)(f"manifest: the stated macOS minimum {stated.get('minimumOS')} is the highest of the executable ({stated.get('executableMinos')}) and the media helper ({stated.get('mediaHelperMinos')})")
    (ok if all(a.get('minimumOS') == stated.get('minimumOS') for a in manifest['assets'] if a['platform'] == 'macos') and not any('minimumOS' in a for a in manifest['assets'] if a['platform'] != 'macos') else fail)('manifest: every macOS asset, and only those, carries the macOS minimum')
    prov_min = json.loads((rt.ROOT / 'licenses/media-provenance.json').read_text())['helpers']['mac-arm64']['machOMinos']
    (ok if prov_min == stated.get('mediaHelperMinos') else fail)('manifest: the helper minimum equals licenses/media-provenance.json')
    mac_top = mac['name'][:-len('.zip')]
    with zipfile.ZipFile(dist / mac['name']) as z:
        inside = set(z.namelist())
    need = [f'{mac_top}/{n}' for n in ('OpenRecorder.app/Contents/MacOS/OpenRecorder', 'LICENSE', 'THIRD_PARTY.md', 'README.txt', 'INSTALL.md', 'licenses/ffmpeg-COPYING.GPLv2', 'licenses/x264-COPYING', 'licenses/media-provenance.json')]
    (ok if all(n in inside or any(i.startswith(n) for i in inside) for n in need) else fail)(f"{mac['name']}: the app, the licence texts and the notices travel together")
    with zipfile.ZipFile(dist / mac['name']) as z:
        said = all(f"macOS {stated.get('minimumOS')} or later" in z.read(f'{mac_top}/{n}').decode() for n in ('README.txt', 'INSTALL.md'))
    (ok if said else fail)(f"{mac['name']}: its README.txt and INSTALL.md state macOS {stated.get('minimumOS')} or later")
    (ok if not any(i.startswith(f'{mac_top}/OpenRecorder.app/Contents/Resources/') for i in inside) else fail)(f"{mac['name']}: nothing was added inside the signed bundle")
    if sys.platform == 'darwin':
        with tempfile.TemporaryDirectory() as t:
            subprocess.run(['/usr/bin/ditto', '-x', '-k', str(dist / mac['name']), t], check=True)
            app = pathlib.Path(t) / mac_top / 'OpenRecorder.app'
            r = subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], capture_output=True, text=True)
            (ok if r.returncode == 0 else fail)('app ZIP: the extracted bundle verifies with codesign')
            info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
            good = info['CFBundleIdentifier'] == 'online.openrecorder.desktop' and info['LSUIElement'] is True and str(manifest['build']) == info['CFBundleVersion'] and rt.sha256(app / 'Contents/MacOS/OpenRecorder') == manifest['executableSha256'] and rt.sha256(app / 'Contents/Helpers/ffmpeg') == manifest['mediaHelperSha256']
            seen = rt.macos_minimum(app, info)
            (ok if all(seen[k] == stated.get(k) for k in ('minimumOS', 'executableMinos', 'mediaHelperMinos', 'sdk')) else fail)(f"app ZIP: otool shows executable {seen['executableMinos']}, helper {seen['mediaHelperMinos']}, so macOS {seen['minimumOS']}, as the manifest states")
            (ok if good else fail)('app ZIP: identity, one menu-bar icon (LSUIElement), build number and executable and media-helper hashes match the manifest')
    else:
        print('skip codesign check (not macOS)')


def verify_remote(base_url, expect=None):
    with tempfile.TemporaryDirectory() as t:
        dist = pathlib.Path(t)
        fetch(base_url, 'manifest.json', dist / 'manifest.json'); fetch(base_url, 'SHA256SUMS', dist / 'SHA256SUMS')
        if expect:
            mismatch = reviewed_identity_problems(dist, expect)
            for line in mismatch:
                fail(line)
            if mismatch:
                return   # do not download or trust the rest of a release that is not the reviewed one
            ok('the published manifest.json and SHA256SUMS are byte-identical to the reviewed local ones')
        for asset in json.loads((dist / 'manifest.json').read_text())['assets']:
            fetch(base_url, asset['name'], dist / asset['name'])
        check(dist)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('dist', nargs='?'); ap.add_argument('--base-url'); ap.add_argument('--expect')
    a = ap.parse_args()
    if bool(a.dist) == bool(a.base_url):
        ap.error('give a directory or --base-url')
    if a.expect and not a.base_url:
        ap.error('--expect only applies with --base-url')
    if a.base_url:
        if not a.base_url.startswith(f'https://github.com/{rt.REPO}/releases/download/v'):
            ap.error('--base-url must be this repository\'s release download URL')
        verify_remote(a.base_url, pathlib.Path(a.expect).resolve() if a.expect else None)
    else:
        check(pathlib.Path(a.dist).resolve())
    print(json.dumps({'passed': not problems, 'problems': problems}, indent=1))
    sys.exit(1 if problems else 0)


if __name__ == '__main__':
    main()
