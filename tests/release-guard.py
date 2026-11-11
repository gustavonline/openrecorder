"""The published-release guard: a release that is online, internally consistent and still not the reviewed one must be refused.

Usage: python3 tests/release-guard.py      (no network: the download function is replaced by a copy from a local directory)
"""
import hashlib, importlib.util, json, pathlib, sys, tempfile

root = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'scripts'))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, root / 'scripts' / file)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


verify, site = load('verify_release', 'verify-release.py'), load('update_site_release', 'update-site-release.py')


def release(directory, payload):
    directory.mkdir(parents=True)
    (directory / 'asset.bin').write_bytes(payload)
    digest = hashlib.sha256(payload).hexdigest()
    manifest = {'version': '0.0.0-test', 'assets': [{'name': 'asset.bin', 'bytes': len(payload), 'sha256': digest}]}
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=1))
    (directory / 'SHA256SUMS').write_text(f'{digest}  asset.bin\n')
    return directory


with tempfile.TemporaryDirectory() as t:
    t = pathlib.Path(t)
    reviewed = release(t / 'reviewed', b'the reviewed asset')
    same = release(t / 'same', b'the reviewed asset')
    other = release(t / 'other', b'a different but self-consistent asset')   # coherent: its own manifest, sums and file agree

    assert verify.reviewed_identity_problems(same, reviewed) == []
    assert len(verify.reviewed_identity_problems(other, reviewed)) == 2, 'a coherent release with other files must differ in both lists'
    (t / 'sums').mkdir(); (t / 'sums' / 'manifest.json').write_bytes((reviewed / 'manifest.json').read_bytes()); (t / 'sums' / 'SHA256SUMS').write_text('0' * 64 + '  asset.bin\n')
    assert verify.reviewed_identity_problems(t / 'sums', reviewed) == ['the published SHA256SUMS differs from the reviewed local SHA256SUMS']
    (t / 'trailing').mkdir(); (t / 'trailing' / 'manifest.json').write_bytes((reviewed / 'manifest.json').read_bytes() + b'\n'); (t / 'trailing' / 'SHA256SUMS').write_bytes((reviewed / 'SHA256SUMS').read_bytes())
    assert len(verify.reviewed_identity_problems(t / 'trailing', reviewed)) == 1, 'byte identity, not semantic equality'
    assert verify.reviewed_identity_problems(t / 'nothing-published', reviewed)
    assert verify.reviewed_identity_problems(same, t / 'nothing-reviewed')

    # verify_remote: the remote is a local directory; the full check is replaced by a counter.
    for remote, passes in ((same, True), (other, False)):
        verify.problems.clear(); fetched, checked = [], []
        verify.fetch = lambda base, name, dest, remote=remote: (fetched.append(name), dest.write_bytes((remote / name).read_bytes()))
        verify.check = lambda dist: checked.append(dist)
        verify.verify_remote('https://example.invalid/release', reviewed)
        assert bool(verify.problems) is (not passes), (remote.name, verify.problems)
        assert bool(checked) is passes, 'a release that is not the reviewed one must not be downloaded or checked'
        assert ('asset.bin' in fetched) is passes, fetched
    verify.problems.clear()

    # update-site-release must ask for that comparison and must write nothing when it fails.
    released = site.rt.ROOT / 'site/releases.json'
    before = released.read_bytes()
    reviewed_real = release(t / 'reviewed-real', b'x'); manifest = json.loads((reviewed_real / 'manifest.json').read_text())
    manifest.update({'tag': 'v0.0.0-test', 'repository': site.rt.REPO}); (reviewed_real / 'manifest.json').write_text(json.dumps(manifest))
    seen = []
    site.subprocess.run = lambda cmd, **k: (seen.append(cmd), type('R', (), {'returncode': 1})())[1]
    sys.argv = ['update-site-release.py', str(reviewed_real)]
    try:
        site.main(); raise AssertionError('a failed verification must stop the script')
    except SystemExit as e:
        assert e.code and 'not the reviewed one' in str(e.code), e.code
    assert '--expect' in seen[0] and str(reviewed_real.resolve()) == seen[0][seen[0].index('--expect') + 1], seen
    assert released.read_bytes() == before, 'site/releases.json changed although the verification failed'
print('release guard: ok')
