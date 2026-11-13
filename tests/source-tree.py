"""The public source tree and archive never contain a repository's own metadata, in particular a worktree's `.git` pointer file.

Usage: python3 tests/source-tree.py
A `git worktree` or submodule checkout has a `.git` FILE ("gitdir: /path/on/the/owner's/machine/...") instead of a directory. It must
not be enumerated as source, hashed into the tree, or packed into the release source archive, while every real hidden file stays.
Pure file checks in a scratch directory; nothing is built, signed or published.
"""
import importlib.util, pathlib, shutil, subprocess, sys, tarfile, tempfile

root = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'scripts'))
import release_tools as rt

owner_path = '/Users/' + 'someone' + '/Projects/repo/.git/worktrees/w'   # a stand-in; the real marker is checked by scan_private
spec = importlib.util.spec_from_file_location('make_release', root / 'scripts/make-release.py')
make_release = importlib.util.module_from_spec(spec); spec.loader.exec_module(make_release)

with tempfile.TemporaryDirectory(prefix='source-tree-') as t:
    scratch = pathlib.Path(t) / 'checkout'
    layout = {
        '.git': f'gitdir: {owner_path}\n',                     # the worktree pointer FILE
        'vendor/lib/.git': f'gitdir: {owner_path}2\n',          # a submodule / nested pointer file
        'engine/vendor/dep/.git': f'gitdir: {owner_path}3\n',   # one inside the vendored tree too (it has its own exceptions)
        '.github/workflows/ci.yml': 'name: CI\n', '.agents/plugins/marketplace.json': '{}', '.claude-plugin/plugin.json': '{}',
        '.gitignore': 'target\n', '.gitattributes': '* text=auto\n', 'README.md': '# x\n', 'engine/vendor/dep/src/lib.rs': '// vendored\n',
    }
    for name, text in layout.items():
        path = scratch / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    (scratch / 'dirrepo/.git').mkdir(parents=True)
    (scratch / 'dirrepo/.git/config').write_text('[core]\n')    # an ordinary repository's .git directory
    (scratch / 'dirrepo/keep.txt').write_text('kept\n')
    found = sorted(p.relative_to(scratch).as_posix() for p in rt.source_files(scratch))
    assert found == sorted(['.github/workflows/ci.yml', '.agents/plugins/marketplace.json', '.claude-plugin/plugin.json', '.gitignore', '.gitattributes', 'README.md', 'engine/vendor/dep/src/lib.rs', 'dirrepo/keep.txt']), found
    assert not any('.git' in pathlib.PurePosixPath(f).parts for f in found), 'no repository metadata or pointer is source'
    entries, tree = rt.tree_manifest(scratch)
    assert all('.git' not in pathlib.PurePosixPath(e['path']).parts for e in entries)

    # The archive the release tool really writes, from that enumeration: no pointer, no owner path anywhere in it.
    out = pathlib.Path(t) / 'source.tar.gz'
    make_release.deterministic_tar([(p.relative_to(scratch).as_posix(), p) for p in rt.source_files(scratch)], out, 'openrecorder-test')
    with tarfile.open(out) as tar:
        names = tar.getnames()
        blob = b''.join(tar.extractfile(m).read() for m in tar.getmembers() if m.isfile())
    assert not any('.git' in pathlib.PurePosixPath(n).parts for n in names), names
    assert owner_path.encode() not in blob and b'gitdir:' not in blob, 'the pointer text reached the archive'
    assert rt.tar_problems(out) == []

    # If one ever did get in, the verifier names it (an extensionless file is otherwise never read by the marker scan).
    bad = pathlib.Path(t) / 'bad.tar.gz'
    with tarfile.open(bad, 'w:gz') as tar:
        tar.add(scratch / '.git', arcname='openrecorder-test/.git')
    assert any('worktree pointer' in p for p in rt.tar_problems(bad)), rt.tar_problems(bad)
    import zipfile
    bad_zip = pathlib.Path(t) / 'bad.zip'
    with zipfile.ZipFile(bad_zip, 'w') as z:
        z.write(scratch / '.git', 'openrecorder/.git')
    assert any('worktree pointer' in p for p in rt.zip_problems(bad_zip))

# The real checkout: what the tools enumerate is what git would publish (when git is there), and the hidden metadata is all present.
real = rt.source_files()
rels = [p.relative_to(rt.ROOT).as_posix() for p in real]
assert not any('.git' in pathlib.PurePosixPath(r).parts for r in rels), 'the real tree enumerates repository metadata'
for must in ('.github/workflows/ci.yml', '.agents/plugins/marketplace.json', '.claude-plugin/plugin.json', '.gitignore'):
    assert must in rels, must
try:
    listed = subprocess.run(['git', '-C', str(rt.ROOT), 'ls-files', '--cached', '--others', '--exclude-standard'], capture_output=True, text=True, check=True).stdout.split('\n')
except (OSError, subprocess.CalledProcessError):
    listed = None
if listed:
    wanted = sorted(f for f in listed if f and (rt.ROOT / f).is_file() and not (rt.ROOT / f).is_symlink())
    assert sorted(rels) == wanted, sorted(set(rels) ^ set(wanted))[:10]
print(f'source tree: ok ({len(rels)} files)')
