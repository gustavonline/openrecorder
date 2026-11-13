#!/usr/bin/env python3
"""Check the static product page in site/: structure, assets, policy and the release guard. Standard library only.

  python3 scripts/check-site.py             # local checks (what CI runs)
  python3 scripts/check-site.py --network   # also, if releases.json says "published", download and verify every release asset

This is not a visual check: layout, themes, the mobile menu and the screenshot tabs need a person (or a browser run) to look.
"""
import argparse, colorsys, hashlib, html.parser, json, pathlib, re, subprocess, sys, urllib.parse
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import release_tools as rt

SITE = rt.ROOT / 'site'
HOST = 'openrecorder.gustavonline.com'
EXTERNAL_OK = {'github.com': ('/gustavonline/',), 'gustavonline.com': ('/',), 'arcitai.com': ('/',), 'onlinesourdough.com': ('/',), 'openpaperdesign.gustavonline.com': ('/',),
               'developers.openai.com': ('/codex',), 'www.anthropic.com': ('/claude-code',), 'support.apple.com': ('/guide/mac-help/',)}
FORBIDDEN_CLAIMS = ['almost zero', 'zero ram', 'zero-ram', 'fastest', 'notarized by apple', 'apple-notarized', 'fully verified', 'works on windows', 'works on linux', 'best-in-class', 'trusted by']
problems = []


def fail(msg):
    problems.append(msg); print('FAIL', msg)


def ok(msg):
    print('ok  ', msg)


VOID = {'meta', 'link', 'img', 'br', 'hr', 'input', 'source', 'path', 'area', 'base', 'col', 'embed', 'track', 'wbr'}


class Page(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(); self.stack = []; self.unbalanced = []; self.refs = []; self.h1 = 0; self.inline_script = 0; self.inline_style = 0; self.style_attr = 0; self.on_attr = 0; self.title = ''; self._in_title = False; self._script = False; self.meta = {}; self.canonical = None; self.lang = None; self.ids = set()
    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag not in VOID: self.stack.append(tag)
        if tag == 'html': self.lang = a.get('lang')
        if tag == 'h1': self.h1 += 1
        if tag == 'title': self._in_title = True
        if tag == 'style': self.inline_style += 1
        if tag == 'script':
            self._script = True
            if 'src' not in a: self.inline_script += 1
        if 'style' in a: self.style_attr += 1
        if any(k.startswith('on') for k in a): self.on_attr += 1
        if 'id' in a: self.ids.add(a['id'])
        if tag == 'meta' and a.get('name'): self.meta[a['name']] = a.get('content', '')
        if tag == 'link' and a.get('rel') == 'canonical': self.canonical = a.get('href')
        for key in ('href', 'src'):
            if key in a: self.refs.append((tag, key, a[key]))
    def handle_endtag(self, tag):
        if tag not in VOID:
            if self.stack and self.stack[-1] == tag: self.stack.pop()
            else: self.unbalanced.append((tag, self.getpos()))
        if tag == 'title': self._in_title = False
        if tag == 'script': self._script = False
    def handle_data(self, data):
        if self._in_title: self.title += data


def hue_bad(hexcolor):
    h = hexcolor.lstrip('#')
    if len(h) == 3: h = ''.join(c * 2 for c in h)
    r, g, b = (int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))
    hh, l, s = colorsys.rgb_to_hls(r, g, b)
    deg = hh * 360
    return s > .25 and .12 < l < .92 and (15 <= deg <= 60 or 255 <= deg <= 335)


def local_target(ref, base):
    path = urllib.parse.urlsplit(ref).path
    target = (SITE / path.lstrip('/')) if path.startswith('/') else (base.parent / path)
    return SITE / 'index.html' if path in ('', '/') else target


def check_pages():
    pages = sorted(SITE.glob('*.html'))
    ok(f'{len(pages)} pages: ' + ', '.join(p.name for p in pages))
    canon = set()
    for page in pages:
        p = Page(); p.feed(page.read_text())
        name = page.name
        for cond, what in [(p.h1 == 1, 'exactly one h1'), (p.lang == 'en', 'lang=en'), (bool(p.title.strip()), 'a title'), (len(p.meta.get('description', '')) > 50, 'a description'),
                           (p.canonical == f'https://{HOST}' + ('/' if name == 'index.html' else '/' + name), 'its canonical URL'), (p.inline_script == 0, 'no inline script'),
                           (p.inline_style == 0 and p.style_attr == 0, 'no inline style'), (p.on_attr == 0, 'no event-handler attributes'), (not p.stack and not p.unbalanced, f'balanced tags {p.stack or p.unbalanced}')]:
            (ok if cond else fail)(f'{name}: {what}')
        canon.add(p.canonical)
        for tag, key, ref in p.refs:
            u = urllib.parse.urlsplit(ref)
            if u.scheme or u.netloc:
                if u.scheme != 'https': fail(f'{name}: non-https reference {ref}'); continue
                if u.netloc == HOST: continue
                if tag in ('script', 'img', 'link') and key == 'src' or (tag == 'link' and key == 'href' and 'canonical' not in ref and u.netloc != HOST and not ref.startswith('https://' + HOST)):
                    fail(f'{name}: external resource {ref}'); continue
                allowed = EXTERNAL_OK.get(u.netloc)
                if not allowed or not u.path.startswith(allowed): fail(f'{name}: link to an unlisted destination {ref}')
                if u.netloc == 'github.com' and not u.path.startswith('/gustavonline/'): fail(f'{name}: GitHub link outside the owner: {ref}')
            elif ref.startswith('#'):
                if len(ref) > 1 and ref[1:] not in p.ids: fail(f'{name}: dangling anchor {ref}')
            elif ref.startswith('data:'):
                continue
            else:
                t = local_target(ref, page)
                if not t.is_file(): fail(f'{name}: {ref} does not exist')
        text = re.sub(r'<[^>]+>', ' ', page.read_text()).lower()
        for claim in FORBIDDEN_CLAIMS:
            if claim in text: fail(f'{name}: forbidden claim {claim!r}')
        if '/releases/download/' in page.read_text(): fail(f'{name}: a page may not hard-code a release asset link; releases.json decides')
    sitemap = pathlib.Path(SITE / 'sitemap.xml').read_text()
    (ok if all(c in sitemap for c in canon) else fail)('sitemap lists every page')
    (ok if f'https://{HOST}/sitemap.xml' in (SITE / 'robots.txt').read_text() else fail)('robots.txt points to the sitemap')


def check_assets():
    listed = {a['path']: a for a in json.loads((SITE / 'assets-sources.json').read_text())['assets']}
    for path, a in listed.items():
        f = rt.ROOT / path
        (ok if f.is_file() and rt.sha256(f) == a['sha256'] else fail)(f'{path}: present and matches its recorded sha256')
    on_disk = {'site/' + p.relative_to(SITE).as_posix() for p in (SITE / 'assets').rglob('*') if p.is_file()}
    missing = sorted(on_disk - set(listed))
    (ok if not missing else fail)('every file under site/assets has provenance' + (f' (missing: {missing})' if missing else ''))
    for name in ('guest-unlock.png', 'guest-player.png'):
        a = listed[f'site/assets/{name}']
        (ok if a.get('capture', {}).get('media', '').startswith('a 4 second generated') else fail)(f'{name}: provenance says it is synthetic')
    for name in ('logo.svg', 'logo-dark.svg'):
        src = rt.ROOT / 'assets' / name.replace('logo', 'icon')
        (ok if (SITE / 'assets' / name).read_bytes() == src.read_bytes() else fail)(f'{name} is the project black disc, unchanged')


def check_policy():
    headers = (SITE / '_headers').read_text()
    csp = re.search(r'Content-Security-Policy: (.+)', headers).group(1)
    good = all(d in csp for d in ("default-src 'self'", "script-src 'self'", "style-src 'self'", "frame-ancestors 'none'", "base-uri 'none'")) and not any(x in csp for x in ('unsafe', '*', 'http:'))
    (ok if good else fail)('_headers: strict content policy, no unsafe-inline, no wildcard')
    css = (SITE / 'styles.css').read_text()
    bad = [c for c in set(re.findall(r'#[0-9a-fA-F]{3,6}\b', css)) if len(c) in (4, 7) and hue_bad(c)]
    (ok if not bad else fail)(f'styles.css: no orange or purple{" " + str(bad) if bad else ""}')
    if re.search(r'url\((?!["\']?/assets/|["\']?data:)', css) or '@import' in css: fail('styles.css: a resource that is not local')
    for f in SITE.rglob('*'):
        if f.is_file() and f.suffix in ('.html', '.js', '.css', '.json', '.md', '.txt', '.xml'):
            for line in rt.scan_private(f.read_bytes(), str(f.relative_to(rt.ROOT))): fail(line)
    ok('no private paths or key material in site/')


def check_release(network):
    r = json.loads((SITE / 'releases.json').read_text())
    plugin = json.loads((rt.ROOT / 'plugin.json').read_text())
    order = lambda v: tuple(int(n) for n in re.findall(r'\d+', v))
    # A pending file describes the version being prepared. A published one describes a release that is out; the source may already
    # be ahead of it (the next version is unreleased), but never behind it.
    same = r['version'] == plugin['version']
    (ok if r['tag'] == 'v' + r['version'] and r['repository'] == rt.REPO and (same if r['status'] == 'pending' else order(r['version']) <= order(plugin['version'])) else fail)('releases.json: tag and repository match, and its version is this source\'s (pending) or this source\'s or an earlier release (published)')
    if r['status'] == 'pending':
        (ok if not r['platforms'] and r['verifiedAt'] is None else fail)('releases.json is pending: no platform asset, no verification record, so no download button')
        return
    if r['status'] != 'published': fail('releases.json: status must be pending or published'); return
    base = f"https://github.com/{rt.REPO}/releases/download/{r['tag']}/"
    if not r.get('verifiedAt') or not r['platforms']: fail('releases.json: published without a verification record or any asset'); return
    for platform, a in r['platforms'].items():
        good = a.get('url', '').startswith(base) and re.fullmatch(r'[0-9a-f]{64}', a.get('sha256', '')) and a.get('bytes', 0) > 0 and r['version'] in a.get('name', '') and a['url'].endswith('/' + a['name'])
        (ok if good else fail)(f'releases.json: {platform} asset is complete and points at this release')
    if network:
        run = subprocess.run([sys.executable, str(rt.ROOT / 'scripts/verify-release.py'), '--base-url', base.rstrip('/')])
        (ok if run.returncode == 0 else fail)('every published asset downloaded and verified')
    else:
        print('skip network verification (use --network)')


def check_platform_claim():
    """The macOS minimum on the public pages must be the one the shipped helper really needs (recorded in the provenance file)."""
    minimum = json.loads((rt.ROOT / 'licenses/media-provenance.json').read_text())['helpers']['mac-arm64']['machOMinos']
    wrong = re.compile(r'macOS 12\.3|12\.3\+|12\.3 or later')
    for name in ('README.md', 'docs/install.md', 'docs/release-notes-0.6.0-preview.11.md', 'docs/release-notes-0.6.0-preview.12.md', 'site/index.html'):
        text = (rt.ROOT / name).read_text()
        (ok if f'macOS {minimum} or later' in text else fail)(f'{name}: states macOS {minimum} or later')
        (ok if not wrong.search(text) else fail)(f'{name}: no older macOS minimum is claimed')


def main():
    ap = argparse.ArgumentParser(); ap.add_argument('--network', action='store_true'); a = ap.parse_args()
    check_pages(); check_assets(); check_policy(); check_platform_claim(); check_release(a.network)
    print(json.dumps({'passed': not problems, 'problems': problems}, indent=1))
    sys.exit(1 if problems else 0)


if __name__ == '__main__':
    main()
