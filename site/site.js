import {detectPlatform, downloadChoice} from '/downloads.js';
const root = document.documentElement;
const themeButton = document.querySelector('#theme-toggle');
const system = matchMedia('(prefers-color-scheme: dark)');
let preference;
try { preference = localStorage.getItem('openrecorder-site-theme'); } catch {}
if (!['dark', 'light'].includes(preference)) preference = null;
function applyTheme() { root.dataset.theme = preference || (system.matches ? 'dark' : 'light'); const next = root.dataset.theme === 'dark' ? 'light' : 'dark'; themeButton.setAttribute('aria-label', `Switch to ${next} mode`); themeButton.title = `Switch to ${next} theme`; }
applyTheme();
themeButton.addEventListener('click', () => { preference = root.dataset.theme === 'dark' ? 'light' : 'dark'; applyTheme(); try { localStorage.setItem('openrecorder-site-theme', preference); } catch {} });
system.addEventListener('change', () => { if (!preference) applyTheme(); });
const dialog = document.querySelector('#mobile-navigation');
const menuButton = document.querySelector('#menu-toggle');
let previousOverflow = '', navigationTarget = null;
menuButton.addEventListener('click', () => { previousOverflow = document.body.style.overflow; dialog.showModal(); document.body.style.overflow = 'hidden'; menuButton.setAttribute('aria-expanded', 'true'); });
function closeMenu() { if (dialog.open) dialog.close(); }
document.querySelector('#menu-close').addEventListener('click', closeMenu);
dialog.addEventListener('close', () => { document.body.style.overflow = previousOverflow; menuButton.setAttribute('aria-expanded', 'false'); if (navigationTarget) { navigationTarget.setAttribute('tabindex', '-1'); navigationTarget.focus({preventScroll: true}); navigationTarget = null; } else if (menuButton.getClientRects().length) { menuButton.focus(); } });
dialog.querySelectorAll('a').forEach(a => a.addEventListener('click', () => { navigationTarget = a.origin === location.origin && a.pathname === location.pathname && a.hash ? document.getElementById(decodeURIComponent(a.hash.slice(1))) : null; closeMenu(); }));
matchMedia('(min-width:800.02px)').addEventListener('change', e => { if (e.matches) closeMenu(); });
const offers = document.querySelector('.offers');
document.addEventListener('click', e => { if (!offers.contains(e.target)) offers.open = false; });
document.addEventListener('keydown', e => { if (e.key === 'Escape' && offers.open) { offers.open = false; offers.querySelector('summary').focus(); } });
offers.addEventListener('focusout', e => { if (!offers.contains(e.relatedTarget)) offers.open = false; });
// The two guest-page screenshots. Both are real captures; the captions say what they are.
const previews = {
  unlock: ['/assets/guest-unlock.png', 'The page a guest opens: it asks for the access code. Actual screenshot of OpenRecorder’s guest page in Firefox (a test server, no real recording).', 'The guest page asking for an access code, with the OpenRecorder black disc above it.'],
  player: ['/assets/guest-player.png', 'After the right code: the player and a download button. Actual screenshot in Firefox with a synthetic test video, not a real recording.', 'The guest player showing a synthetic gradient test video with its controls and a Download recording button.']
};
document.querySelectorAll('[data-preview]').forEach(button => button.addEventListener('click', () => {
  const [src, caption, alt] = previews[button.dataset.preview] || []; if (!src) return;
  document.querySelectorAll('[data-preview]').forEach(x => x.setAttribute('aria-pressed', String(x === button)));
  const image = document.querySelector('#product-screenshot'); image.src = src; image.alt = alt;
  document.querySelector('#preview-open').href = src; document.querySelector('#preview-caption').textContent = caption;
}));
const copyStatus = document.querySelector('#copy-status');
for (const button of document.querySelectorAll('[data-copy]')) button.addEventListener('click', async () => {
  const code = document.getElementById(button.dataset.copy);
  try { await navigator.clipboard.writeText(code.textContent); copyStatus.textContent = 'Commands copied.'; }
  catch { const selection = getSelection(); const range = document.createRange(); range.selectNodeContents(code); selection.removeAllRanges(); selection.addRange(range); copyStatus.textContent = 'Copy is unavailable. The text is selected; press Command+C or Ctrl+C to copy.'; code.tabIndex = -1; code.focus({preventScroll: true}); }
});
// The main button: a real, verified download, or an honest pointer to the install steps.
(async () => {
  const primary = document.querySelector('#primary-install'), note = document.querySelector('#release-note'), meta = document.querySelector('#download-meta');
  if (!primary) return;
  let manifest = null;
  try { const r = await fetch('/releases.json', {cache: 'no-store'}); if (r.ok) manifest = await r.json(); } catch {}
  const choice = downloadChoice(manifest, detectPlatform(navigator.platform, navigator.userAgent, navigator.maxTouchPoints));
  primary.textContent = choice.label; primary.href = choice.href;
  if (choice.asset) { primary.setAttribute('download', ''); } else { primary.removeAttribute('download'); }
  if (note) note.textContent = choice.meta;
  if (meta && choice.asset) { meta.hidden = false; meta.querySelector('a').href = choice.asset.url.replace(/[^/]+$/, 'SHA256SUMS'); }
})();
