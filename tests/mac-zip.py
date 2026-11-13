"""The distributable Mac ZIP carries the app's files, modes and symlinks and NOTHING the build machine attached to them.

Usage: python3 tests/mac-zip.py      (macOS only; skipped elsewhere)
A staged bundle carries extended attributes (`com.apple.provenance`, quarantine, ...). Plain `ditto -c -k` serializes them as AppleDouble
`._name` entries, which are not part of the app: they once put `Resources/._OpenRecorder.icns` into a release candidate. The packager's
`zip_folder` must leave them out of the ARCHIVE while leaving the staged files, and the attributes on them, untouched, and the archive
checks must name any such entry. Own scratch bundles only; nothing is signed or launched.
"""
import importlib.util, os, pathlib, subprocess, sys, tempfile

if sys.platform != 'darwin':
    print('mac zip: skipped (not macOS)'); sys.exit(0)
root = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'scripts'))
import release_tools as rt
spec = importlib.util.spec_from_file_location('make_release', root / 'scripts/make-release.py')
mr = importlib.util.module_from_spec(spec); spec.loader.exec_module(mr)


def xattrs(path):
    return sorted(subprocess.run(['/usr/bin/xattr', str(path)], capture_output=True, text=True).stdout.split())


with tempfile.TemporaryDirectory(prefix='mac-zip-') as t:
    t = pathlib.Path(t)
    stage = t / 'OpenRecorder-test-mac-arm64'
    app = stage / 'OpenRecorder.app/Contents'
    for d in ('MacOS', 'Helpers', 'Resources'):
        (app / d).mkdir(parents=True)
    (app / 'MacOS/OpenRecorder').write_bytes(b'#!/bin/sh\nexit 0\n'); (app / 'MacOS/OpenRecorder').chmod(0o755)
    (app / 'Helpers/ffmpeg').write_bytes(b'helper bytes'); (app / 'Helpers/ffmpeg').chmod(0o755)
    (app / 'Resources/OpenRecorder.icns').write_bytes(b'icns data'); (app / 'Info.plist').write_bytes(b'<plist/>')
    os.symlink('Resources/OpenRecorder.icns', app / 'icon-link')
    (stage / 'LICENSE').write_text('MIT\n')
    for target in (app / 'Resources/OpenRecorder.icns', app / 'MacOS/OpenRecorder', app / 'Helpers/ffmpeg', app / 'Info.plist', stage / 'LICENSE'):
        subprocess.run(['/usr/bin/xattr', '-w', 'com.apple.provenance', 'build-machine', str(target)], check=True)
    subprocess.run(['/usr/bin/xattr', '-w', 'com.apple.quarantine', '0081;5f;Safari;', str(app / 'MacOS/OpenRecorder')], check=True)
    before = {p: xattrs(p) for p in stage.rglob('*') if not p.is_symlink()}
    assert before[app / 'Resources/OpenRecorder.icns'] == ['com.apple.provenance'], 'the fixture must carry the attribute that caused the failure'

    # The control: what plain ditto does, which the checks must name (so this test would have caught the original failure).
    plain = t / 'plain.zip'
    subprocess.run(['/usr/bin/ditto', '-c', '-k', '--keepParent', str(stage), str(plain)], check=True)
    problems = rt.zip_problems(plain)
    assert any('._OpenRecorder.icns' in p and 'filesystem metadata' in p for p in problems), problems

    # The packager.
    out = t / 'out.zip'
    mr.zip_folder(stage, out)
    assert rt.zip_problems(out) == [], rt.zip_problems(out)
    import zipfile
    with zipfile.ZipFile(out) as z:
        names = z.namelist()
    assert not any(n.split('/')[-1].startswith('._') or '__MACOSX' in n for n in names), names
    expected = {'OpenRecorder-test-mac-arm64/OpenRecorder.app/Contents/' + n for n in ('MacOS/OpenRecorder', 'Helpers/ffmpeg', 'Resources/OpenRecorder.icns', 'Info.plist', 'icon-link')} | {'OpenRecorder-test-mac-arm64/LICENSE'}
    assert expected <= set(names), sorted(expected - set(names))
    extracted = t / 'x'; extracted.mkdir()
    subprocess.run(['/usr/bin/ditto', '-x', '-k', str(out), str(extracted)], check=True)
    top = extracted / 'OpenRecorder-test-mac-arm64'
    for rel in ('OpenRecorder.app/Contents/MacOS/OpenRecorder', 'OpenRecorder.app/Contents/Helpers/ffmpeg', 'OpenRecorder.app/Contents/Resources/OpenRecorder.icns', 'OpenRecorder.app/Contents/Info.plist', 'LICENSE'):
        assert (top / rel).read_bytes() == (stage / rel).read_bytes(), rel
        assert os.stat(top / rel).st_mode & 0o777 == os.stat(stage / rel).st_mode & 0o777, f'{rel}: mode'
    assert os.path.islink(top / 'OpenRecorder.app/Contents/icon-link') and os.readlink(top / 'OpenRecorder.app/Contents/icon-link') == 'Resources/OpenRecorder.icns', 'the symlink is kept as a symlink'
    # The recipient's disk gets no build-machine attributes, and the staged files (the lead's build) keep every attribute they had.
    assert all(xattrs(p) in ([], ['com.apple.provenance']) and 'com.apple.quarantine' not in xattrs(p) for p in top.rglob('*') if not p.is_symlink()), 'no build-machine attribute was serialized'
    after = {p: xattrs(p) for p in stage.rglob('*') if not p.is_symlink()}
    assert after == before, 'packaging must not remove or change the staged files\' attributes'
    # The same checks apply to tar archives.
    import tarfile
    bad = t / 'bad.tar.gz'
    with tarfile.open(bad, 'w:gz') as tar:
        tar.add(stage / 'LICENSE', arcname='x/._LICENSE'); tar.add(stage / 'LICENSE', arcname='__MACOSX/x/LICENSE')
    assert len([p for p in rt.tar_problems(bad) if 'filesystem metadata' in p]) == 2
print('mac zip: ok')
