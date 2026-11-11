// Decides what the page's main button may offer. A download is offered only when releases.json says the release is published
// AND carries a verification record AND the asset's address is this repository's release download for this exact version.
// Anything else (no release yet, a missing build, a phone, an unsupported CPU) falls back to the install section: no dead button.
export const REPOSITORY = 'gustavonline/openrecorder';
export function detectPlatform(platform = '', ua = '', touch = 0) {
  if (/iPhone|iPad|iPod|Android/i.test(ua) || (/Mac/i.test(platform) && touch > 1)) return 'mobile';
  if (/CrOS/i.test(ua) || (/Linux|Win/i.test(platform) && /aarch64|arm(?:64|v[5-9])/i.test(platform + ' ' + ua))) return 'unknown';
  if (/Win/i.test(platform)) return 'windows';
  if (/Mac/i.test(platform)) return 'macos';
  if (/Linux/i.test(platform)) return 'linux';
  return 'unknown';
}
export function allowedDownload(url, manifest) {
  if (typeof url !== 'string' || typeof manifest?.version !== 'string') return false;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' && u.hostname === 'github.com' && !u.username && !u.password && !u.search && !u.hash
      && u.pathname.startsWith(`/${REPOSITORY}/releases/download/v${manifest.version}/`) && !u.pathname.slice(1).split('/').slice(5).join('/').includes('/');
  } catch { return false; }
}
export function published(manifest) {
  return manifest?.status === 'published' && typeof manifest.verifiedAt === 'string' && manifest.verifiedAt.length >= 10 && manifest.repository === REPOSITORY;
}
export function selectAsset(manifest, platform) {
  if (!published(manifest) || !['macos', 'windows', 'linux'].includes(platform)) return null;
  const a = manifest.platforms?.[platform];
  if (platform === 'macos' && !/^\d+(\.\d+)*$/.test(a?.minimumOS || '')) return null;   // never offer the Mac app without stating the macOS it needs
  return a && allowedDownload(a.url, manifest) && /^[0-9a-f]{64}$/.test(a.sha256 || '') && Number.isFinite(a.bytes) && a.bytes > 0 && typeof a.architecture === 'string' && a.name?.includes(manifest.version) ? a : null;
}
export function downloadChoice(manifest, platform) {
  // Linux is never the page's one-click download: the experimental build needs glibc 2.43 or newer and has never run on a desktop, so Linux visitors are pointed at the source build (the archive stays on the release page).
  const asset = platform === 'linux' ? null : selectAsset(manifest, platform);
  const names = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };
  if (asset) return { asset, label: `Download for ${names[platform]} (${asset.architecture}) ↓`, href: asset.url, meta: `${manifest.version} · ${(asset.bytes / 1048576).toFixed(1)} MB · SHA-256 ${asset.sha256.slice(0, 12)}…${platform === 'macos' ? ` · macOS ${asset.minimumOS} or later · not notarized` : ''} ${platform === 'macos' ? '' : '· ' + asset.status}`.trim() };
  const pending = !published(manifest);
  return { asset: null, label: platform === 'mobile' ? 'See how to get it on a computer ↓' : 'How to get it ↓', href: '#install',
    meta: pending ? 'The first public download is being prepared. Until its files are verified there is nothing to download here: build from source, or see the install steps.'
      : platform === 'linux' ? 'Build from source. An experimental Linux archive is on the release page, but it needs glibc 2.43 or newer and has never been run on a desktop.'
      : platform === 'windows' ? 'There is no build for this system in this release: build it from source.' : 'See the install steps.' };
}
