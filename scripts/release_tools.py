"""Shared helpers for the release scripts (hashing, tree manifests, archive inspection). Standard library only."""
import hashlib, json, pathlib, re, subprocess, tarfile, zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
REPO = 'gustavonline/openrecorder'
# What a build or a checkout leaves behind that is never part of the source tree (mirrors .gitignore).
IGNORED_PARTS = {'target', 'runtime', 'dist', '.git', '__pycache__', 'node_modules'}
IGNORED_SUFFIXES = ('.zip', '.log', '.pem', '.key', '.p12', '.DS_Store', '.pyc')
# Strings that must never appear in anything we publish.
# (Written in pieces so that this file does not contain the strings it looks for.)
PRIVATE_MARKERS = [b'gustav' + b'anderson', b'/Users/' + b'gustav', b'BEGIN ' + b'PRIVATE KEY', b'BEGIN RSA ' + b'PRIVATE KEY', b'BEGIN EC ' + b'PRIVATE KEY', b'SIGN_' + b'KEYCHAIN=']


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def is_source(path, root=ROOT):
    rel = path.relative_to(root)
    # A repository's own metadata is never source, wherever it sits and whatever it is: the `.git` directory of a checkout, and the
    # `.git` FILE of a worktree or submodule, which names the owner's path on this machine. Hidden files that are source (.github,
    # .agents, .claude-plugin, .gitignore, ...) have other names and stay.
    if '.git' in rel.parts:
        return False
    if any(part in IGNORED_PARTS for part in rel.parts[:-1]) and not (rel.parts[0] == 'engine' and rel.parts[1] == 'vendor'):
        return False
    if rel.parts[0] in ('engine',) and len(rel.parts) > 1 and rel.parts[1] in ('target', 'runtime'):
        return False
    return not rel.name.endswith(IGNORED_SUFFIXES) and rel.name != '.DS_Store'


def source_files(root=ROOT):
    return sorted(p for p in root.rglob('*') if p.is_file() and not p.is_symlink() and is_source(p, root))


def tree_manifest(root=ROOT):
    """Sorted (path, bytes, sha256) of the clean source tree, and one hash over that listing."""
    entries = [{'path': p.relative_to(root).as_posix(), 'bytes': p.stat().st_size, 'sha256': sha256(p)} for p in source_files(root)]
    listing = '\n'.join(f"{e['sha256']}  {e['path']}" for e in entries) + '\n'
    return entries, hashlib.sha256(listing.encode()).hexdigest()


def scan_private(blob, where):
    return [f'{where}: contains {m.decode(errors="replace")!r}' for m in PRIVATE_MARKERS if m in blob]


def _filesystem_metadata(name):
    """AppleDouble `._x` companions and `__MACOSX` folders: a build machine's extended attributes serialized into an archive."""
    parts = pathlib.PurePosixPath(name).parts
    return '__MACOSX' in parts or any(part.startswith('._') for part in parts)


def zip_problems(path):
    """Structural problems of a published ZIP: unsafe names, private markers in text members, keys."""
    problems = []
    with zipfile.ZipFile(path) as z:
        for info in z.infolist():
            name = info.filename
            if name.startswith('/') or '..' in pathlib.PurePosixPath(name).parts:
                problems.append(f'{path.name}: unsafe member name {name!r}')
            if name.lower().endswith(('.pem', '.key', '.p12')):
                problems.append(f'{path.name}: key material {name!r}')
            if '.git' in pathlib.PurePosixPath(name).parts:
                problems.append(f'{path.name}: repository metadata or worktree pointer {name!r}')
            if _filesystem_metadata(name):
                problems.append(f'{path.name}: build-machine filesystem metadata {name!r}')
            if not info.is_dir() and info.file_size < 2_000_000 and name.endswith(('.json', '.md', '.txt', '.svg', '.html', '.js', '.py', '.toml', '.plist')):
                problems += scan_private(z.read(name), f'{path.name}:{name}')
    return problems


def tar_problems(path):
    problems = []
    with tarfile.open(path) as t:
        for m in t.getmembers():
            if m.name.startswith('/') or '..' in pathlib.PurePosixPath(m.name).parts:
                problems.append(f'{path.name}: unsafe member name {m.name!r}')
            if m.issym() or m.islnk():
                problems.append(f'{path.name}: link member {m.name!r}')
            if '.git' in pathlib.PurePosixPath(m.name).parts:
                problems.append(f'{path.name}: repository metadata or worktree pointer {m.name!r}')
            if _filesystem_metadata(m.name):
                problems.append(f'{path.name}: build-machine filesystem metadata {m.name!r}')
            if m.isfile() and m.size < 2_000_000 and m.name.endswith(('.json', '.md', '.txt', '.sh', '.rs', '.toml', '.py', '.js', '.html', '.css')):
                problems += scan_private(t.extractfile(m).read(), f'{path.name}:{m.name}')
    return problems


def semver(version):
    return re.fullmatch(r'\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?', version) is not None


def _version_tuple(v):
    return tuple(int(x) for x in v.split('.'))


def macho_minos(path):
    """The deployment target (LC_BUILD_VERSION minos) of a Mach-O file, read with `otool -l` (read-only)."""
    out = subprocess.run(['/usr/bin/otool', '-l', str(path)], capture_output=True, text=True, check=True).stdout
    block = out.split('LC_BUILD_VERSION', 1)[1]
    return re.search(r'minos (\S+)', block).group(1), re.search(r'sdk (\S+)', block).group(1)


def macos_minimum(app, info):
    """What the shipped macOS bundle really requires: the highest minimum of its executables, plus the evidence."""
    app = pathlib.Path(app)
    exe_minos, sdk = macho_minos(app / 'Contents/MacOS/OpenRecorder')
    helper_minos, _ = macho_minos(app / 'Contents/Helpers/ffmpeg')
    effective = max(exe_minos, helper_minos, key=_version_tuple)
    return {'minimumOS': effective, 'architecture': 'arm64', 'basis': 'highest LC_BUILD_VERSION minos among the shipped Mach-O executables (otool -l)',
            'executableMinos': exe_minos, 'mediaHelperMinos': helper_minos, 'sdk': sdk,
            'infoPlistLSMinimumSystemVersion': info.get('LSMinimumSystemVersion'),
            'note': ('The Info.plist value equals the effective minimum: the bundled media helper sets it and the plist says the same.' if info.get('LSMinimumSystemVersion') == effective else
                     'The Info.plist value differs from the effective minimum, which the bundled media helper sets. The signed app is not altered.')}
