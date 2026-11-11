#!/usr/bin/env python3
"""Assemble the release assets for THIS version from artifacts that really exist. Local only: nothing is uploaded.

Usage:
  python3 scripts/make-release.py --app /path/to/OpenRecorder.app --media-sources /path/to/media-sources.tar.gz --out DIST

  --app            the signed macOS bundle to publish. It is only read (copied with ditto); never patched or re-signed.
  --media-sources  the corresponding-source archive of the FFmpeg/x264 helper (see licenses/media-provenance.json).
  --out            a new directory for the assets.
  --linux-core / --linux-helper                       an experimental Linux x64 core and its media helper (both or neither)
  --windows-core / --windows-helper / --windows-dll   an experimental Windows x64 core, helper and libwinpthread-1.dll (all or none)
  --built-from     the sha256 of the source archive those cores were built from (recorded in the manifest)

Linux and Windows assets are produced only when you pass real builds of THIS version. They are labelled as built but never run on
hardware; the helper and DLL hashes must equal licenses/media-provenance.json or the script refuses.

Only platforms with a real, verified build for this version are produced. There is no Linux or Windows asset unless you
build one (scripts/build-native.py) and extend this script: an old build is never relabelled with a new version.
Writes: the macOS app ZIP, the plugin ZIP, the media-sources archive, the source archive, SHA256SUMS and manifest.json.
"""
import argparse, gzip, hashlib, io, json, os, pathlib, plistlib, shutil, subprocess, sys, tarfile, tempfile, zipfile
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import release_tools as rt

ROOT = rt.ROOT


def run(*cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def deterministic_tar(members, out, prefix):
    """members: list of (relative posix path, absolute file path). Sorted, zero owner/time, reproducible."""
    with open(out, 'wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, compresslevel=9, mtime=0) as gz, tarfile.open(fileobj=gz, mode='w') as tar:
        for rel, path in sorted(members):
            info = tar.gettarinfo(str(path), arcname=f'{prefix}/{rel}')
            info.uid = info.gid = 0; info.uname = info.gname = ''; info.mtime = 0
            info.mode = 0o755 if os.access(path, os.X_OK) else 0o644
            with open(path, 'rb') as f:
                tar.addfile(info, f)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--app', required=True); ap.add_argument('--media-sources', required=True); ap.add_argument('--out', required=True)
    for flag in ('linux-core', 'linux-helper', 'windows-core', 'windows-helper', 'windows-dll', 'built-from'):
        ap.add_argument('--' + flag)
    a = ap.parse_args()
    linux = (a.linux_core, a.linux_helper); windows = (a.windows_core, a.windows_helper, a.windows_dll)
    assert all(linux) or not any(linux), 'give both Linux files or neither'
    assert all(windows) or not any(windows), 'give all three Windows files or none'
    assert (all(linux) or all(windows)) <= bool(a.built_from), '--built-from is required with Linux or Windows builds'
    plugin = json.loads((ROOT / 'plugin.json').read_text()); version = plugin['version']
    assert rt.semver(version), version
    app = pathlib.Path(a.app).resolve(); out = pathlib.Path(a.out).resolve()
    assert app.name == 'OpenRecorder.app' and app.is_dir(), 'pass the OpenRecorder.app bundle'
    assert not out.exists(), 'choose a new output directory'
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    assert info['CFBundleIdentifier'] == 'online.openrecorder.desktop' and info['LSUIElement'] is True, 'unexpected bundle identity'
    build = info['CFBundleVersion']
    build_script = (ROOT / 'scripts/build-native.py').read_text()
    assert f'"CFBundleVersion":"{build}"' in build_script, 'the bundle build number is not the one build-native.py writes'
    run('/usr/bin/codesign', '--verify', '--deep', '--strict', str(app))
    exe = app / 'Contents/MacOS/OpenRecorder'; helper = app / 'Contents/Helpers/ffmpeg'
    # The effective minimum macOS is the highest Mach-O minimum of what is shipped; read it from the files, never assume it.
    macos = rt.macos_minimum(app, info)
    assert macos['mediaHelperMinos'] == json.loads((ROOT / 'licenses/media-provenance.json').read_text())['helpers']['mac-arm64']['machOMinos'], 'the helper minimum differs from licenses/media-provenance.json'
    assert not any(p.is_symlink() for p in app.rglob('*')), 'the bundle is expected to hold no symlinks'
    out.mkdir(parents=True)
    names = {
        'app': f'OpenRecorder-{version}-mac-arm64.zip',
        'plugin': f'openrecorder-plugin-{version}-darwin-arm64.zip',
        'media': f'openrecorder-media-sources-{version}.tar.gz',
        'source': f'openrecorder-{version}-source.tar.gz',
    }
    with tempfile.TemporaryDirectory(prefix='openrecorder-release-') as t:
        t = pathlib.Path(t)
        # 1. the standalone app: ditto keeps the bundle byte for byte, with its signature seal. The licence texts and notices that
        #    must travel with the GPL helper are placed NEXT TO the .app inside the ZIP's one folder; the bundle itself is not touched.
        mac_top = names['app'][:-len('.zip')]
        mac_min = macos['minimumOS']; exe_minos = macos['executableMinos']; helper_minos = macos['mediaHelperMinos']
        stage = t / mac_top; stage.mkdir()
        copy = stage / 'OpenRecorder.app'; run('/usr/bin/ditto', str(app), str(copy))
        run('/usr/bin/codesign', '--verify', '--deep', '--strict', str(copy))
        for name in ('LICENSE', 'THIRD_PARTY.md'):
            shutil.copyfile(ROOT / name, stage / name)
        shutil.copytree(ROOT / 'licenses', stage / 'licenses')
        shutil.copyfile(ROOT / 'docs/install.md', stage / 'INSTALL.md')
        (stage / 'README.txt').write_text(
            f'OpenRecorder {version} for macOS (Apple silicon). Requires macOS {mac_min} or later.\n\n'
            f'The app executable is built for macOS {exe_minos}, but the bundled media helper (Contents/Helpers/ffmpeg) is built for macOS {helper_minos},\n'
            'so the app as shipped needs macOS ' + mac_min + '. Info.plist still says 12.3 (the capture interface the source uses); the signed app is unchanged.\n\n'
            'Move OpenRecorder.app to ~/Applications and open it. It is signed with a local development identity, not an Apple Developer ID,\n'
            'and is not notarized, so macOS warns the first time; INSTALL.md links Apple\'s steps for opening it. OpenRecorder.app is the accepted\n'
            'bundle, unchanged (its executable and media-helper SHA-256 are recorded in manifest.json); the notices beside it are not part of the signed bundle.\n\n'
            'Contents/Helpers/ffmpeg inside the app is FFmpeg with x264 (GPL-2.0-or-later) run as a separate program. Its licence texts are in\n'
            'licenses/, the credits are in THIRD_PARTY.md, and its corresponding sources are published beside this download as\n'
            f'openrecorder-media-sources-{version}.tar.gz. OpenRecorder itself is MIT (LICENSE).\n')
        stamp0 = int(os.environ.get('SOURCE_DATE_EPOCH', '1767225600'))
        for item in [stage / n for n in ('LICENSE', 'THIRD_PARTY.md', 'INSTALL.md', 'README.txt')] + list((stage / 'licenses').rglob('*')) + [stage / 'licenses']:
            os.utime(item, (stamp0, stamp0))
        run('/usr/bin/ditto', '-c', '-k', '--keepParent', str(stage), str(out / names['app']))
        # 2. the plugin: the same bundle as its runtime.
        runtime = t / 'darwin-arm64.tar.gz'
        with tarfile.open(runtime, 'w:gz') as tar:
            tar.add(copy, arcname='OpenRecorder.app')
        run(sys.executable, str(ROOT / 'scripts/package-plugin.py'), 'darwin-arm64', str(out / names['plugin']), str(runtime), stdout=subprocess.DEVNULL)
        (out / names['plugin']).with_suffix('.json').unlink()   # the receipt is folded into manifest.json
        # 3. the corresponding sources: the archive as given plus a note that says what it is.
        src = pathlib.Path(a.media_sources).resolve(); work = t / 'media'; work.mkdir()
        with tarfile.open(src) as tar:
            tar.extractall(work, filter='data')
        (top,) = [p for p in work.iterdir() if p.is_dir()]
        prov = json.loads((ROOT / 'licenses/media-provenance.json').read_text())
        # The archive's build-windows.sh pointed at ../ffmpeg-n8.0.tar.gz (wrong case and one level up) and at a private toolchain path.
        # Ship a corrected portable copy; recorded-build-windows.sh stays byte for byte as the executed history.
        portable = top / 'build-windows.sh'; text = portable.read_text()
        fixes = (('tar -xzf ../ffmpeg-n8.0.tar.gz -C windows-src', 'tar -xzf FFmpeg-n8.0.tar.gz -C windows-src'),
                 ('cross=/home/gustav/Projects/openrecorder-preview/work/mingw-root/usr/bin/x86_64-w64-mingw32-\n',
                  'cross=${CROSS_PREFIX:?set CROSS_PREFIX to your mingw-w64 prefix, for example /usr/bin/x86_64-w64-mingw32-}\n'),
                 ('set -eu\n', '# Portable Windows (mingw-w64 cross) build of the media helper. Run it from anywhere: every path is relative to this script, which\n'
                  '# sits next to FFmpeg-n8.0.tar.gz in the extracted archive. Needs a mingw-w64 toolchain (set CROSS_PREFIX) and x264 built statically for\n'
                  '# Windows in ./x264-windows (see BUILD.txt). recorded-build-windows.sh is the unedited history of what was actually run.\n'
                  'set -eu\n'))
        for was, now in fixes:
            assert text.count(was) == 1, f'build-windows.sh no longer has the expected line: {was[:40]}'
            text = text.replace(was, now)
        portable.write_text(text)
        assert 'ffmpeg-n8.0' not in text and '/home/' not in text and (top / 'FFmpeg-n8.0.tar.gz').is_file()
        (top / 'README-OPENRECORDER.txt').write_text(
            f"Corresponding sources for the media helper shipped with OpenRecorder {version} (FFmpeg 8.0 + x264, GPL-2.0-or-later).\n\n"
            "Contents are those of the original sources archive (FFmpeg 8.0 and the AVFoundation patch for macOS, FFmpeg n8.0 for Linux and\n"
            "Windows, x264 at the recorded commit, and the build scripts; recorded-build-*.sh keep the exact configure flags that were run).\n"
            f"The macOS helper inside the {version} app has sha256 {prov['helpers']['mac-arm64']['sha256']} and the same configure flags as\n"
            "recorded-build-mac.sh. A read-only comparison found its code and data sections identical to the helper (sha256 e6c2345a...) in the\n"
            "earlier preview.4 macOS package (a locally distributed test ZIP, never publicly released): every byte before the code signature is identical,\n"
            "with the same length and Mach-O UUID, and only the signature differs. That does not show that these sources reproduce the binary,\n"
            "and a bit-identical rebuild from them has not been verified. The experimental Linux and Windows archives of this\n"
            "release ship the helpers listed in licenses/media-provenance.json, built from the unmodified FFmpeg n8.0 sources recorded here.\n"
            "BUILD.txt in this archive was written for an earlier packaging and says 'owner-fixes'.\n\n"
            "build-windows.sh was corrected for this release: the original looked for ../ffmpeg-n8.0.tar.gz, but the archive holds FFmpeg-n8.0.tar.gz next to\n"
            "the script, and it named a private toolchain directory; it now extracts the sibling archive and takes CROSS_PREFIX from the environment. Its\n"
            "configure flags are unchanged. recorded-build-*.sh are the unedited executed history and keep their original paths (/home/..., /tmp/...), which\n"
            "must be adapted to your machine. Neither script builds x264: build it statically for each target from x264-source.tar.gz (commit in provenance.json)\n"
            "into x264-windows / x264-linux; the exact x264 cross-build command that produced the shipped helpers was not recorded.\n")
        deterministic_tar([(p.relative_to(top).as_posix(), p) for p in top.rglob('*') if p.is_file()], out / names['media'], top.name)
        # 4. the source tree.
        files = rt.source_files()
        deterministic_tar([(p.relative_to(ROOT).as_posix(), p) for p in files], out / names['source'], f'openrecorder-{version}')
    extra = []
    prov = json.loads((ROOT / 'licenses/media-provenance.json').read_text())
    with tempfile.TemporaryDirectory(prefix='openrecorder-platforms-') as t2:
        t2 = pathlib.Path(t2)
        for plat, files, exe_name, helper_name in (('linux-x64', linux, 'openrecorder-core', 'ffmpeg'), ('win32-x64', windows, 'openrecorder-core.exe', 'ffmpeg.exe')):
            if not all(files):
                continue
            key = 'linux-x64' if plat == 'linux-x64' else 'windows-x64'
            core, plat_helper = pathlib.Path(files[0]).resolve(), pathlib.Path(files[1]).resolve()
            assert rt.sha256(plat_helper) == prov['helpers'][key]['sha256'], f'{plat} helper does not match licenses/media-provenance.json'
            magic = core.read_bytes()[:4]
            assert magic == (b'\x7fELF' if plat == 'linux-x64' else b'MZ\x90\x00'), f'{plat} core is not the expected executable format'
            runtime = t2 / f'{plat}.tar.gz'; staging = t2 / f'stage-{plat}'; staging.mkdir()
            members = [(exe_name, core), (helper_name, plat_helper)]
            if plat == 'win32-x64':
                dll = pathlib.Path(windows[2]).resolve()
                assert rt.sha256(dll) == prov['helpers']['windows-x64']['runtimeDll']['sha256'], 'libwinpthread-1.dll does not match licenses/media-provenance.json'
                members.append(('libwinpthread-1.dll', dll))
            for name, src in members:
                shutil.copyfile(src, staging / name); os.chmod(staging / name, 0o755)
            with tarfile.open(runtime, 'w:gz') as tar:
                for name, _ in members:
                    tar.add(staging / name, arcname=name)
            plugin_name = f'openrecorder-plugin-{version}-{plat}.zip'
            run(sys.executable, str(ROOT / 'scripts/package-plugin.py'), plat, str(out / plugin_name), str(runtime), stdout=subprocess.DEVNULL)
            (out / plugin_name).with_suffix('.json').unlink()
            # the standalone archive: the same files plus the notices that must travel with the GPL helper
            label = 'linux-x64' if plat == 'linux-x64' else 'windows-x64'
            top = f'OpenRecorder-{version}-{label}'
            std = t2 / top; std.mkdir()
            for name, _ in members:
                shutil.copyfile(staging / name, std / name); os.chmod(std / name, 0o755)
            for name in ('LICENSE', 'THIRD_PARTY.md'):
                shutil.copyfile(ROOT / name, std / name)
            shutil.copytree(ROOT / 'licenses', std / 'licenses')
            shutil.copyfile(ROOT / 'docs/install.md', std / 'INSTALL.md')
            (std / 'README.txt').write_text(
                f'OpenRecorder {version} for {label}: EXPERIMENTAL build.\n\n'
                'This build was compiled from the same source as the macOS release but has not been run on a real desktop by its author:\n'
                'tray, screen capture and permissions are unverified. Run the program ('+exe_name+') to start the menu/tray recorder; '
                f'`{exe_name} --doctor` checks the media helper next to it and touches no device. See INSTALL.md.\n\n'
                'ffmpeg'+('.exe' if plat == 'win32-x64' else '')+' is FFmpeg with x264 (GPL-2.0-or-later) run as a separate program; its corresponding sources are published beside the release.\n')
            stamp = int(os.environ.get('SOURCE_DATE_EPOCH', '1767225600'))
            for item in std.rglob('*'):
                os.utime(item, (stamp, stamp))
            std_files = [(p.relative_to(std).as_posix(), p) for p in std.rglob('*') if p.is_file()]
            if plat == 'linux-x64':
                std_name = f'{top}.tar.gz'; deterministic_tar(std_files, out / std_name, top)
            else:
                std_name = f'{top}.zip'
                with zipfile.ZipFile(out / std_name, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
                    for rel, path in sorted(std_files):
                        z.write(path, f'{top}/{rel}')
            extra.append((plat, label, std_name, plugin_name, core, plat_helper))
    entries, tree = rt.tree_manifest()
    assets = []
    kinds = {'app': ('macos', 'arm64', 'Standalone macOS app'), 'plugin': ('macos', 'arm64', 'Codex / Claude Code plugin (local marketplace) with the macOS runtime'), 'media': (None, None, 'Corresponding FFmpeg/x264 sources'), 'source': (None, None, 'Source tree')}
    for key, name in names.items():
        p = out / name
        platform, arch, label = kinds[key]
        assets.append({'name': name, 'kind': key, 'platform': platform, 'architecture': arch, 'description': label, 'bytes': p.stat().st_size, 'sha256': rt.sha256(p),
                       **({'minimumOS': macos['minimumOS']} if platform == 'macos' else {})})
    quals = {'linux-x64': ('Experimental build for Linux x64 (needs glibc 2.43 or newer, libdbus, libpipewire and libasound); compiled, never run on a real desktop by its author.', 'linux'),
             'win32-x64': ('Experimental build for Windows x64 (cross-built, MinGW runtime included); never run on real Windows hardware.', 'windows')}
    built_from = a.built_from
    for plat, label, std_name, plugin_name, core, plat_helper in extra:
        text, osname = quals[plat]
        for name, kind, description in ((std_name, 'app', f'Standalone {osname} build (experimental)'), (plugin_name, 'plugin', f'Codex / Claude Code plugin with the {osname} runtime (experimental)')):
            p = out / name
            assets.append({'name': name, 'kind': kind, 'platform': osname, 'architecture': 'x64', 'description': description, 'bytes': p.stat().st_size, 'sha256': rt.sha256(p), 'qualification': text, 'coreSha256': rt.sha256(core), 'mediaHelperSha256': rt.sha256(plat_helper), 'builtFromSourceArchiveSha256': built_from})
    (out / 'SHA256SUMS').write_text(''.join(f"{x['sha256']}  {x['name']}\n" for x in assets))
    manifest = {
        'product': 'OpenRecorder', 'version': version, 'build': int(build), 'tag': f'v{version}', 'repository': rt.REPO,
        'sourceTreeSha256': tree, 'sourceFiles': len(entries),
        'notarized': False, 'signing': 'Local development identity (not an Apple Developer ID); macOS shows its unknown-developer warning on first open.',
        'macos': macos,
        'executableSha256': rt.sha256(exe), 'mediaHelperSha256': rt.sha256(app / 'Contents/Helpers/ffmpeg'), 'bundleBuildId': info['OpenRecorderBuild'],
        'assets': assets,
        'notPublished': {k: 'no build for this version' for k, ok_ in (('linux-x64', bool(a.linux_core)), ('win32-x64', bool(a.windows_core))) if not ok_},
        'correspondingSources': names['media'],
    }
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({'out': str(out), 'version': version, 'build': build, 'sourceTreeSha256': tree, 'assets': [(x['name'], x['bytes'], x['sha256']) for x in assets]}, indent=1))


if __name__ == '__main__':
    main()
