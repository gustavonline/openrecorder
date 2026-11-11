// The product page's main button may offer a download only for a published, verified release of this repository and version.
// Run: node --test tests/site-downloads.mjs   (Node is a test tool only; the site itself is static.)
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {detectPlatform, downloadChoice, selectAsset, allowedDownload} from '../site/downloads.js';

const committed = JSON.parse(await readFile(new URL('../site/releases.json', import.meta.url), 'utf8'));
const plugin = JSON.parse(await readFile(new URL('../plugin.json', import.meta.url), 'utf8'));
const v = committed.version, base = `https://github.com/gustavonline/openrecorder/releases/download/v${v}/`;
// An explicit pending release, independent of what the committed releases.json says (it changes when a release is published).
const pending = {schema: 1, repository: 'gustavonline/openrecorder', version: v, build: committed.build, tag: `v${v}`, status: 'pending', verifiedAt: null, platforms: {}};
const published = {...pending, status: 'published', verifiedAt: '2026-11-11T12:00:00Z', platforms: {macos: {architecture: 'arm64', name: `OpenRecorder-${v}-mac-arm64.zip`, url: `${base}OpenRecorder-${v}-mac-arm64.zip`, sha256: 'a'.repeat(64), bytes: 5564621, minimumOS: '26.0'}}};

test('a pending release offers no download on any system', () => {
  assert.equal(pending.status, 'pending'); assert.deepEqual(pending.platforms, {});
  for (const p of ['macos', 'windows', 'linux', 'mobile', 'unknown']) { const c = downloadChoice(pending, p); assert.equal(c.asset, null); assert.equal(c.href, '#install'); }
});
test('the committed releases.json is valid for the phase it is in: pending offers nothing, published offers only verified assets of this release', () => {
  assert.equal(committed.repository, 'gustavonline/openrecorder');
  assert.equal(committed.version, plugin.version); assert.equal(committed.tag, `v${plugin.version}`);
  assert.ok(['pending', 'published'].includes(committed.status), `unknown status ${committed.status}`);
  if (committed.status === 'pending') {
    assert.deepEqual(committed.platforms, {}); assert.equal(committed.verifiedAt, null);
    for (const p of ['macos', 'windows', 'linux', 'mobile', 'unknown']) { const c = downloadChoice(committed, p); assert.equal(c.asset, null, p); assert.equal(c.href, '#install'); }
    return;
  }
  assert.match(committed.verifiedAt, /^\d{4}-\d{2}-\d{2}T/, 'a published release carries its verification time');
  assert.ok(Object.keys(committed.platforms).length > 0);
  assert.equal(committed.checksums, `${base}SHA256SUMS`);
  assert.ok(allowedDownload(committed.sources, committed), 'the corresponding sources are this release');
  for (const [platform, a] of Object.entries(committed.platforms)) {
    assert.ok(['macos', 'windows', 'linux'].includes(platform), platform);
    assert.ok(allowedDownload(a.url, committed), `${platform}: the address is this repository's download for this exact version`);
    assert.equal(a.url, base + a.name); assert.ok(a.name.includes(v)); assert.match(a.sha256, /^[0-9a-f]{64}$/); assert.ok(a.bytes > 0);
    assert.equal(typeof a.architecture, 'string'); assert.equal(typeof a.status, 'string');
  }
  assert.match(committed.platforms.macos?.minimumOS ?? '', /^\d+(\.\d+)*$/, 'the Mac app states the macOS it needs');
  const mac = downloadChoice(committed, 'macos');
  assert.equal(mac.asset, committed.platforms.macos); assert.equal(mac.href, committed.platforms.macos.url);
  assert.match(mac.meta, new RegExp(`macOS ${committed.platforms.macos.minimumOS.replace(/\./g, '\\.')} or later`));
  assert.equal(downloadChoice(committed, 'linux').asset, null, 'Linux is never the one-click download');
  const win = downloadChoice(committed, 'windows');
  assert.equal(win.asset, committed.platforms.windows ?? null);
  if (win.asset) assert.match(win.meta, /never run on real Windows hardware/i, 'an experimental build says so beside the button');
  for (const p of ['mobile', 'unknown']) assert.equal(downloadChoice(committed, p).asset, null, p);
});
test('a published, verified macOS asset is the one real download', () => {
  assert.match(downloadChoice(published, 'macos').label, /^Download for macOS/);
  assert.equal(downloadChoice(published, 'macos').href, published.platforms.macos.url);
  for (const p of ['windows', 'linux', 'mobile', 'unknown']) assert.equal(downloadChoice(published, p).asset, null, p);
  assert.match(downloadChoice(published, 'windows').meta, /no build for this system/i);
});
test('an experimental Windows asset is offered with its status stated', () => {
  const win = {...published, platforms: {...published.platforms, windows: {architecture: 'x64', name: `OpenRecorder-${v}-windows-x64.zip`, url: `${base}OpenRecorder-${v}-windows-x64.zip`, sha256: 'b'.repeat(64), bytes: 3000000, status: 'Experimental build, never run on real Windows hardware.'}}};
  const c = downloadChoice(win, 'windows');
  assert.match(c.label, /^Download for Windows/); assert.match(c.meta, /never run on real Windows hardware/);
  assert.equal(downloadChoice(win, 'linux').asset, null);
});
test('the Mac download states the macOS it needs, and is withheld without it', () => {
  assert.match(downloadChoice(published, 'macos').meta, /macOS 26\.0 or later/);
  const bare = {...published, platforms: {macos: {...published.platforms.macos, minimumOS: undefined}}};
  assert.equal(downloadChoice(bare, 'macos').asset, null);
});
test('Linux is never the one-click download, even when an experimental archive is published', () => {
  const lin = {...published, platforms: {...published.platforms, linux: {architecture: 'x64', name: `OpenRecorder-${v}-linux-x64.tar.gz`, url: `${base}OpenRecorder-${v}-linux-x64.tar.gz`, sha256: 'c'.repeat(64), bytes: 8000000, status: 'Experimental'}}};
  const c = downloadChoice(lin, 'linux');
  assert.equal(c.asset, null); assert.equal(c.href, '#install'); assert.match(c.meta, /build from source/i); assert.match(c.meta, /glibc 2\.43/);
  assert.match(downloadChoice(lin, 'macos').label, /^Download for macOS/);
});
test('anything unverified, foreign, old or malformed is refused', () => {
  const with_ = patch => selectAsset({...published, ...patch.top, platforms: {macos: {...published.platforms.macos, ...patch.asset}}}, 'macos');
  assert.equal(with_({asset: {url: 'https://evil.example/x.zip'}}), null);
  assert.equal(with_({asset: {url: `${base.replace(v, '0.6.0-preview.9')}x.zip`}}), null, 'another version');
  assert.equal(with_({top: {verifiedAt: null}}), null, 'no verification record');
  assert.equal(with_({top: {status: 'pending'}}), null);
  assert.equal(with_({top: {repository: 'someone/else'}}), null);
  assert.equal(with_({asset: {sha256: 'zz'}}), null);
  assert.equal(with_({asset: {bytes: 0}}), null);
  assert.equal(with_({asset: {name: 'OpenRecorder.zip', url: `${base}OpenRecorder.zip`}}), null, 'a name without the version');
  assert.equal(allowedDownload(`${base}a.zip?x=1`, published), false);
  assert.equal(allowedDownload(`https://user:pw@github.com/gustavonline/openrecorder/releases/download/v${v}/a.zip`, published), false);
  assert.equal(allowedDownload(`http://github.com/gustavonline/openrecorder/releases/download/v${v}/a.zip`, published), false);
});
test('platform detection keeps phones and unsupported CPUs away from a desktop download', () => {
  assert.equal(detectPlatform('MacIntel', 'Mozilla/5.0 (Macintosh)', 0), 'macos');
  assert.equal(detectPlatform('MacIntel', '', 5), 'mobile');
  assert.equal(detectPlatform('Linux armv8l', 'Android', 5), 'mobile');
  assert.equal(detectPlatform('Win32', 'Windows', 0), 'windows');
  assert.equal(detectPlatform('Linux aarch64', '', 0), 'unknown');
});
