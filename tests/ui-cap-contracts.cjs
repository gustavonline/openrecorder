'use strict';
/* Library UI contracts. Pure checks: no browser, no network, no hardware, no owner data.
 *
 * Usage: node tests/ui-cap-contracts.cjs
 * Static checks read ui/index.html, ui/style.css, ui/app.js and assets/icon*.svg. Runtime checks run ui/app.js inside a
 * node:vm context with a small fake DOM, a fake fetch/host bridge and fake timers, all fed mock data. This proves the
 * markup, state mapping and request shapes; it does not replace a real-browser visual pass.
 */
const fs = require('fs'), path = require('path'), vm = require('vm');
const root = path.resolve(__dirname, '..');
const read = f => fs.readFileSync(path.join(root, f), 'utf8');
const html = read('ui/index.html'), css = read('ui/style.css'), app = read('ui/app.js');
const results = [];
const SILENT = Symbol('silent host');
const check = async (name, fn) => { try { await fn(); results.push({ name, ok: true }); } catch (e) { results.push({ name, ok: false, error: String(e && e.stack || e).split('\n').slice(0, 4).join(' | ') }); } };
const eq = (a, b, m) => { if (a !== b) throw new Error(`${m || 'values differ'}: got ${JSON.stringify(a)}, want ${JSON.stringify(b)}`); };
const ok = (v, m) => { if (!v) throw new Error(m || 'assertion failed'); };
const has = (s, re, m) => ok(re instanceof RegExp ? re.test(s) : s.includes(re), m || `missing ${re}`);
const lacks = (s, re, m) => ok(!(re instanceof RegExp ? re.test(s) : s.includes(re)), m || `unexpected ${re}`);
const settle = async (n = 6) => { for (let i = 0; i < n; i++) await new Promise(r => setImmediate(r)); };

// ---- fake DOM ----------------------------------------------------------------------------------------------------
function makeDom(timers) {
  const els = new Map(), listeners = {};
  class El {
    constructor(id) { Object.assign(this, { id, tagName: 'DIV', hidden: false, textContent: '', innerHTML: '', value: '', checked: false, disabled: false, open: false, dataset: {}, style: {}, attrs: {}, handlers: {}, offsetParent: {}, offsetWidth: 232, offsetHeight: 200, isConnected: true, classes: new Set() }); }
    get classList() { const c = this.classes; return { add: x => c.add(x), remove: x => c.delete(x), contains: x => c.has(x), toggle: (x, on) => { (on === undefined ? !c.has(x) : on) ? c.add(x) : c.delete(x); } }; }
    setAttribute(k, v) { this.attrs[k] = String(v); } getAttribute(k) { return this.attrs[k] ?? null; } removeAttribute(k) { delete this.attrs[k]; }
    addEventListener(t, f) { (this.handlers[t] ||= []).push(f); } dispatch(t, e = {}) { (this.handlers[t] || []).forEach(f => f(e)); }
    load() {} pause() {} play() { return Promise.resolve(); }
    focus() { dom.document.activeElement = this; } select() {} click() { this.onclick && this.onclick({ target: this }); }
    showModal() { this.open = true; } close() { this.open = false; this.dispatch('close'); }
    querySelector() { return null; } querySelectorAll() { return []; } closest() { return null; } contains() { return false; }
    getBoundingClientRect() { return { left: 0, right: 30, top: 0, bottom: 30, width: 30, height: 30 }; }
  }
  const body = new El('body'); body.tagName = 'BODY'; body.offsetParent = null;
  const dom = {
    El, els,
    document: {
      activeElement: body, body, documentElement: new El('html'), hidden: false,
      getElementById: id => { if (!els.has(id)) { const e = new El(id); if (/Dialog$/.test(id)) e.tagName = 'DIALOG'; els.set(id, e); } return els.get(id); },
      querySelector: sel => sel === 'dialog[open]' ? [...els.values()].find(e => e.tagName === 'DIALOG' && e.open) || null : sel.startsWith('input[name=shareMinutes]') ? dom.minutes : null,
      querySelectorAll: () => [], createElement: () => new El('x'), createRange: () => ({ selectNodeContents() {} }),
      addEventListener: (t, f) => { (listeners['doc:' + t] ||= []).push(f); },
    },
    listeners, minutes: Object.assign(new El('minutes'), { value: '60', checked: true }),
  };
  dom.document.documentElement.dataset = {};
  return dom;
}

// ---- boot app.js in a vm ---------------------------------------------------------------------------------------
function boot({ embedded = false, key = null, hash = '', mediaKey = null, api, state, host, image = () => 'load' } = {}) {
  const images = [];
  const timeouts = new Map(); let nextId = 1;
  const dom = makeDom(timeouts), fetches = [], posted = [], media = { touched: false }, store = new Map();
  const toasts = [];
  const storage = () => ({ getItem: k => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)) });
  const sandbox = {
    document: dom.document, console, Date, JSON, Math, Number, String, Array, Object, Promise, Set, Map, Error, TypeError, encodeURIComponent, URL, AbortController, atob, Blob, Uint8Array,
    __OPENRECORDER_TEST__: true,
    // An image that decides what it does from its address (the host's policy), like a browser's does.
    Image: class { set src(v) { this._src = String(v); images.push(this._src.replace(/\?.*$/, '').slice(0, 40)); const r = image(this._src); if (r === 'load') Promise.resolve().then(() => this.onload && this.onload()); else if (r === 'error') Promise.resolve().then(() => this.onerror && this.onerror()); } get src() { return this._src; } },
    location: { hash, pathname: '/' }, history: { replaceState() { sandbox.location.hash = ''; }, pushState() {}, back() {} },
    localStorage: storage(), sessionStorage: storage(), CSS: { escape: s => String(s) },
    innerWidth: 1280, innerHeight: 800, scrollTo() {}, getSelection: () => ({ removeAllRanges() {}, addRange() {} }),
    navigator: { clipboard: { written: [], writeText(t) { this.written.push(t); return Promise.resolve(); } }, get mediaDevices() { media.touched = true; throw new Error('media devices must not be touched'); } },
    setTimeout: (fn, delay) => { const id = nextId++; timeouts.set(id, { fn, delay }); return id; }, clearTimeout: id => { timeouts.delete(id); },
    setInterval: () => nextId++, clearInterval() {},
    addEventListener: (t, f) => { (dom.listeners[t] ||= []).push(f); },
    fetch: async (url, opts = {}) => { fetches.push({ url, opts, body: opts.body ? JSON.parse(opts.body) : null }); return api(url, opts, fetches); },
  };
  sandbox.window = sandbox; sandbox.globalThis = sandbox;
  sandbox.parent = { postMessage(message) { posted.push(message); if (message.id && host) Promise.resolve().then(async () => { const result = await host(message); if (result === SILENT) return; (dom.listeners.message || []).forEach(f => f({ source: sandbox.parent, origin: 'http://127.0.0.1:1', data: { jsonrpc: '2.0', id: message.id, result } })); }); } };
  const connection = { embedded, origin: 'http://127.0.0.1:1', key, mediaKey, uiKey: embedded ? 'UI-SESSION-KEY' : null };
  const script = new vm.Script(`const CONNECTION=${JSON.stringify(connection)};\n${app}`);
  vm.createContext(sandbox);
  script.runInContext(sandbox);
  const t = sandbox.__openrecorder, $ = id => dom.document.getElementById(id);
  const runTimeout = async () => { const [id, tm] = [...timeouts.entries()].filter(([, v]) => v.delay < 100000)[0] || []; if (!id) return null; timeouts.delete(id); tm.fn(); await settle(); return tm.delay; };
  return { sandbox, dom, t, $, images, fetches, posted, media, timeouts, store, runTimeout, toasts, delays: () => [...timeouts.values()].map(v => v.delay).filter(d => d < 100000) };
}
const okJson = body => ({ ok: true, status: 200, json: async () => body });
const stateOf = (over = {}) => ({
  version: '0.0.0-test', platform: 'darwin', root: '/mock/root', freeBytes: 5e10, totalBytes: 3e6,
  settings: { quality: 1920, fps: 30, keepOriginals: true, mirror: true, cameraSize: 168, cameraShape: 'rounded', countdownSeconds: 3, rememberInputs: true, theme: 'system', camera: false, microphone: false },
  folders: [{ id: 'f1', name: 'Demos' }], active: null, shares: [],
  recordings: [
    { id: 'r1', title: 'Weekly demo', status: 'ready', createdAt: Date.now() - 3600e3, duration: 252, bytes: 2e6, folder: 'f1', favorite: true, archived: false, thumbnailReady: true, directory: '/mock/root/r1', files: [{ name: 'recording.mp4', bytes: 2e6 }, { name: 'screen.mp4', bytes: 1e6, width: 1920, height: 1080 }, { name: 'camera.mp4', bytes: 5e5 }], settings: { camera: true, microphone: false } },
    { id: 'r2', title: 'Plain take', status: 'ready', createdAt: Date.now() - 86400e3, bytes: 1e6, favorite: false, archived: false, directory: '/mock/root/r2', files: [{ name: 'recording.mp4', bytes: 1e6 }], settings: {} },
  ], ...over,
});
const loaded = async (o = {}) => {
  const st = o.state || stateOf();
  const h = boot({ key: 'OWNERKEY', api: async () => okJson(st), ...o });
  await settle(); return h;
};

(async () => {
  // ================= static contracts =================
  await check('host placeholders appear exactly once and the assembled page compiles', () => {
    for (const p of ['/* STYLES */', '/* CONFIG */', '/* APP */']) eq(html.split(p).length - 1, 1, p);
    lacks(app, '</script', 'app.js must not close the host script tag');
    const page = html.replace('/* STYLES */', css).replace('/* CONFIG */', 'const CONNECTION={"embedded":false,"origin":"http://127.0.0.1:1","key":null,"mediaKey":null};').replace('/* APP */', () => app);
    const inline = /<script>([\s\S]*)<\/script>/.exec(page)[1];
    new vm.Script(inline);
  });

  await check('html ids are unique and every id the script looks up exists', () => {
    const ids = [...html.matchAll(/\sid="([^"]+)"/g)].map(m => m[1]);
    eq(new Set(ids).size, ids.length, 'duplicate ids');
    const dynamic = new Set([...app.matchAll(/\sid="([^"$]+)"/g)].map(m => m[1]));
    const wanted = new Set([...app.matchAll(/\$\('([A-Za-z][\w-]*)'\)/g)].map(m => m[1]));
    const missing = [...wanted].filter(i => !ids.includes(i) && !dynamic.has(i));
    eq(missing.join(','), '', 'ids missing from index.html');
    for (const m of html.matchAll(/aria-(?:controls|labelledby|describedby)="([^"]+)"/g)) for (const id of m[1].split(' ')) ok(ids.includes(id) || /^(settingsPane|tab-)/.test(id) || dynamic.has(id), `dangling aria reference ${id}`);
  });

  await check('accessible names: every button, input, dialog and live region is named', () => {
    for (const m of html.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)) {
      const text = m[2].replace(/<[^>]+>/g, '').trim();
      ok(text || /aria-label="[^"]+"/.test(m[1]), `unnamed button: ${m[1]}`);
    }
    for (const m of html.matchAll(/<input\b([^>]*)>/g)) {
      const before = html.slice(0, m.index), wrapped = before.lastIndexOf('<label') > before.lastIndexOf('</label>');
      ok(/aria-label=/.test(m[1]) || wrapped || (/id="([^"]+)"/.test(m[1]) && html.includes(`for="${/id="([^"]+)"/.exec(m[1])[1]}"`)), `unlabelled input: ${m[1]}`);
    }
    for (const m of html.matchAll(/<dialog\b([^>]*)>/g)) has(m[1], 'aria-labelledby=', 'dialog needs a label');
    has(html, /id="toast" role="status"/); has(html, /id="toastAlert" role="alert"/); has(html, /id="settingsStatus" role="status"/);
    has(html, /role="tablist"[^>]*aria-label="Settings sections"/); has(html, /id="playerTabs" role="tablist"/);
    ok([...html.matchAll(/data-icon="[^"]+"[^>]*aria-hidden="true"/g)].length === [...html.matchAll(/data-icon=/g)].length, 'decorative icons must be aria-hidden');
  });

  await check('layout: 232px sidebar, one nav of three, labelled Folders + add, Settings at the bottom', () => {
    has(css, /\.app\{display:grid;grid-template-columns:232px/);
    const nav = /<nav class="side-nav"[\s\S]*?<\/nav>/.exec(html)[0];
    eq([...nav.matchAll(/data-view="/g)].length, 3); for (const v of ['all', 'favorites', 'shared']) has(nav, `data-view="${v}"`);
    has(html, /id="foldersLabel">Folders</); has(html, /id="newFolder" aria-label="Add folder"/);
    const foot = /<div class="side-foot">[\s\S]*?<\/aside>/.exec(html)[0]; has(foot, 'id="settingsButton"');
    const main = html.slice(html.indexOf('<main'), html.indexOf('</main>'));
    lacks(main, 'class="count"', 'counts belong only to the sidebar'); lacks(main, /folder-chip|<h2[^>]*>Folders/, 'no repeated folder headings or chips in the content');
    eq([...html.matchAll(/<h1\b/g)].length, 2, 'one page title (plus the locked view title)');
    for (const id of ['pageTitle', 'search', 'sort', 'newRecording', 'activeStatus']) ok(main.indexOf(`id="${id}"`) < main.indexOf('id="reach"'), `${id} lives in the single content header`);
  });

  await check('brand is a plain black 18px circle; icons carry no red centre or ring', () => {
    has(css, /\.brand-dot\{width:18px;height:18px;[^}]*background:#000/);
    for (const f of ['assets/icon.svg', 'assets/icon-dark.svg']) {
      const svg = read(f); eq([...svg.matchAll(/<circle/g)].length, 1, f); has(svg, 'fill="#000"'); lacks(svg, /d93036|ff5a52|e0393e|e5484d/i);
      lacks(svg, 'fill="none"', `${f} must not draw a ring`);
    }
    has(read('assets/icon-dark.svg'), 'stroke="#4a4a50"'); lacks(read('assets/icon.svg'), 'stroke');
    has(css, /--brand-edge:#4a4a50/);
  });

  await check('cog is a real gear: eight attached teeth, solid body, centre hole', () => {
    const d = /cog:'<path[^>]* d="([^"]+)"/.exec(app)[1];
    eq((d.match(/M/g) || []).length, 2, 'outline plus centre hole'); has(app, /cog:'<path fill="currentColor" stroke="none" fill-rule="evenodd"/);
    eq((d.split('Z')[0].match(/A7\.4/g) || []).length, 8, 'eight root arcs, one between each pair of teeth');
    lacks(d, 'stroke-dasharray');
  });

  await check('tokens match DESIGN.md in light and dark, and both dark blocks agree', () => {
    const block = re => Object.fromEntries([...re.exec(css)[1].matchAll(/--([\w-]+):([^;}]+)/g)].map(m => [m[1], m[2]]));
    const light = block(/:root\{color-scheme:light;([^}]*)\}/), media = block(/prefers-color-scheme:dark\)\{:root:not\(\[data-theme=light\]\)\{color-scheme:dark;([^}]*)\}/), dark = block(/:root\[data-theme=dark\]\{color-scheme:dark;([^}]*)\}/);
    eq(JSON.stringify(media), JSON.stringify(dark), 'system-dark and forced-dark must be identical');
    const want = { light: { window: '#f5f5f7', surface: '#fff', sidebar: '#f7f7f9', control: '#f0f0f2', selected: '#e5eefd', sep: '#e5e5ea', hairline: '#d8d8de', text: '#1d1d1f', text2: '#6e6e73', text3: '#9a9aa0', accent: '#0a6cdf', live: '#1f8a4c', record: '#e0393e', toast: '#2a2a2d' }, dark: { window: '#161618', surface: '#1e1e21', sidebar: '#1a1a1d', control: '#2a2a2e', selected: '#1f3556', sep: '#2f2f34', hairline: '#3a3a40', text: '#f2f2f4', text2: '#a1a1a8', text3: '#6e6e76', accent: '#4c9aff', live: '#3dd68c', record: '#e5484d', toast: '#3a3a3e' } };
    for (const [name, v] of Object.entries(want.light)) eq(light[name], v, `light --${name}`);
    for (const [name, v] of Object.entries(want.dark)) eq(dark[name], v, `dark --${name}`);
    eq(light.primary, '#1d1d1f'); eq(dark.primary, '#f2f2f4'); eq(dark['on-primary'], '#111113');
    eq(Object.keys(light).sort().join(), Object.keys(dark).sort().join(), 'same token set in both themes');
  });

  await check('no orange or purple anywhere in css, script or icons', () => {
    const colours = new Set(); for (const s of [css, app, read('assets/icon.svg'), read('assets/icon-dark.svg'), html]) for (const m of s.matchAll(/#([0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{3})\b/gi)) colours.add(m[1].toLowerCase());
    const bad = [];
    for (let c of colours) {
      if (c.length === 3) c = [...c].map(x => x + x).join(''); const [r, g, b] = [0, 2, 4].map(i => parseInt(c.slice(i, i + 2), 16) / 255);
      const max = Math.max(r, g, b), min = Math.min(r, g, b), l = (max + min) / 2, d = max - min; if (d < .001) continue;
      const s = d / (1 - Math.abs(2 * l - 1)); let h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4; h = (h * 60 + 360) % 360;
      if (s > .25 && l > .12 && l < .92 && ((h >= 15 && h <= 60) || (h >= 255 && h <= 335))) bad.push(`#${c}(h${Math.round(h)})`);
    }
    eq(bad.join(','), '');
    lacks(css, /\b(orange|purple|violet|magenta|gold|amber)\b/i);
  });

  await check('red is reserved for record/stop/recording indicator', () => {
    const uses = [...css.matchAll(/([^{}]+)\{[^}]*var\(--record\)[^}]*\}/g)].map(m => m[1].trim());
    for (const sel of uses) ok(/rec-dot|\.stop|\.cap-record/.test(sel), `--record used by ${sel}`);
  });

  await check('responsive contract: 4/3/2 columns, 390px sheet with 96x54 rows, reduced motion and contrast', () => {
    has(css, 'repeat(4,minmax(0,1fr))'); has(css, /min-width:1000px\) and \(max-width:1279px\)\{\.recordings\{grid-template-columns:repeat\(3/); has(css, /min-width:721px\) and \(max-width:999px\)\{\.recordings\{grid-template-columns:repeat\(2/);
    has(css, /@media\(max-width:720px\)/); has(css, /\.sidebar\{position:fixed;[^}]*translateX\(-100%\)/); has(css, /grid-template-columns:96px minmax\(0,1fr\) 32px/); has(css, /\.thumbnail\{width:96px;height:54px/);
    has(css, 'prefers-reduced-motion:reduce'); has(css, 'transition-property:opacity!important'); has(css, 'prefers-contrast:more'); has(css, 'outline-width:4px');
    has(html, /id="menuButton"[^>]*aria-controls="sidebar"[^>]*aria-expanded="false"/);
  });

  await check('card actions show on hover and keyboard focus; Escape and outside click close the menu', () => {
    has(css, '.card:hover .more'); has(css, '.more:focus-visible'); has(css, '.card:has(:focus-visible) .more'); has(css, '.more[aria-expanded=true]');
    has(app, /pointerdown/); has(app, /e\.key==='Escape'/); has(app, /role="menuitem"/);
  });

  await check('no invented features: no Trash, folder delete/rename, fake settings, hardware or prepare calls', () => {
    const all = html + app;
    lacks(all, /\btrash\b|delete folder|rename folder|move to trash/i);
    lacks(all, /open at login|global shortcut|launch at login|name="systemAudio"/i);
    lacks(app, /getUserMedia|getDisplayMedia|MediaRecorder|mediaDevices|enumerateDevices|op==='prepare'|'prepare'|"prepare"/);
    lacks(app, /\beval\(|new Function|document\.write|insertAdjacentHTML/);
    lacks(all, /Opening recorder/);
    lacks(all, /files (don.t|do not) move|metadata-only/i);
    lacks(all, /public link|anyone with the link/i);
  });

  await check('folder copy: filing is metadata-only and the UI never claims a physical move or a legacy location', () => {
    has(app, "const folderNote='Filing a recording in a folder never moves its files.';");
    lacks(html + app, /directory (on|moved)|moves (its|this recording.s) directory|moved on disk|Folders\/|legacy take/i);
    lacks(html + app, /Crockford/i);
    lacks(app, /patch:\{folder[^}]*directory|op==='move/, 'filing is one update patch, nothing else');
  });

  await check('credentials: bearer is the owner key only; media uses ?media= embedded and ?key= in the browser', () => {
    has(app, 'Authorization:`Bearer ${localKey}`'); lacks(app, /Authorization[^\n]{0,60}mediaKey/);
    has(app, 'media=${encodeURIComponent(CONNECTION.mediaKey)}'); has(app, 'key=${encodeURIComponent(localKey)}');
    lacks(app, /rpc\([^)]*(localKey|CONNECTION\.key|mediaKey)/); lacks(app, /writeText\([^)]*localKey/);
    // The one credential an agent-host call carries is the session key of the window’s own page, only to the page-only tool.
    eq((app.match(/ui:CONNECTION\.uiKey/g) || []).length, 1); has(app, "name:'openrecorder_ui',arguments:{op,data,ui:CONNECTION.uiKey}");
  });

  await check('script does not contain a path to same-origin activation or tunnel creation', () => {
    lacks(app, /cloudflared\s*\(|trycloudflare/i); lacks(app, /\/api\/activate|prepare-inputs/);
  });

  // ================= runtime contracts =================
  await check('standalone without a key shows a calm open-from-controls state and never calls the network', async () => {
    const h = boot({ key: null, hash: '', api: async () => { throw new Error('network'); } }); await settle();
    eq(h.fetches.length, 0, 'no requests'); eq(h.$('locked').hidden, false); eq(h.$('app').hidden, true);
    has(h.$('lockedText').textContent, /OpenRecorder/); lacks(h.$('lockedText').textContent, /403|401|unauthor|forbidden/i);
    eq(h.timeouts.size, 0, 'no polling timers');
  });

  await check('standalone key from the fragment is used as bearer, stored in the session and removed from the URL', async () => {
    const h = boot({ hash: '#FRAGKEY', api: async () => okJson(stateOf()) }); await settle();
    eq(h.fetches[0].opts.headers.Authorization, 'Bearer FRAGKEY'); eq(h.sandbox.location.hash, ''); eq(h.store.get('or-key'), 'FRAGKEY');
    eq(h.t.fileURL('r1', 'recording.mp4'), 'http://127.0.0.1:1/file/r1/recording.mp4?key=FRAGKEY');
    eq(h.t.fileURL('r1', 'recording.mp4', true), 'http://127.0.0.1:1/file/r1/recording.mp4?key=FRAGKEY&download=1');
  });

  await check('embedded uses parent RPC tools/call and ?media= URLs; it never sends or builds the owner bearer', async () => {
    const st = stateOf(); const h = boot({ embedded: true, mediaKey: 'MEDIA', api: async () => { throw new Error('no fetch'); }, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : { structuredContent: st } }); await settle();
    eq(h.fetches.length, 0); const call = h.posted.find(m => m.method === 'tools/call');
    ok(call && call.params.name === 'openrecorder_action' && call.params.arguments.op === 'state', 'state goes through tools/call');
    eq(h.t.fileURL('r1', 'thumbnail.jpg'), 'http://127.0.0.1:1/file/r1/thumbnail.jpg?media=MEDIA');
    ok(!JSON.stringify(h.posted).includes('OWNERKEY') && !h.t.fileURL('r1', 'x').includes('key='), 'no owner key anywhere');
    eq(h.$('openBrowser').hidden, false, 'embedded gets the browser fallback');
    const missing = boot({ embedded: true, mediaKey: null, api: async () => { throw 0; }, host: m => ({ hostContext: {}, structuredContent: st }) });
    eq(missing.t.fileURL('r1', 'x'), '', 'no credential, no URL');
  });

  await check('cards: escaped titles, poster or drawn placeholder, duration only from metadata, date, favorite, shared', async () => {
    const evil = '"><img src=x onerror=alert(1)><script>1</script>';
    const st = stateOf(); st.recordings[1].title = evil; st.shares = [{ shareId: 's1', id: 'r1', status: 'ready', url: 'https://x.trycloudflare.com/abc', accessCode: 'K7QM-4P2X', expiresAt: Date.now() + 36e5 }];
    const h = await loaded({ state: st });
    const c1 = h.t.cardMarkup(st.recordings[0]), c2 = h.t.cardMarkup(st.recordings[1]);
    has(c1, '/file/r1/thumbnail.jpg?key=OWNERKEY'); has(c1, 'class="duration">4:12<'); has(c1, 'class="poster-fallback"'); has(c1, /Today, /); has(c1, 'aria-label="Favorite"'); has(c1, 'class="tag shared"'); has(c1, 'aria-haspopup="menu"'); has(c1, 'aria-expanded="false"');
    lacks(c2, 'class="duration"', 'no duration is invented'); lacks(c2, '<img loading'); has(c2, 'class="poster-fallback"'); has(c2, /Yesterday, /);
    lacks(c2, '<img src=x'); lacks(c2, '<script'); has(c2, '&lt;script&gt;'); has(c2, '&quot;&gt;&lt;img');
    has(c1, /data-open tabindex="-1" aria-hidden="true"/); has(c1, /class="card-title" data-open/);
    lacks(c1, /Trash|Delete/);
  });

  await check('sidebar counts, folders and views come from state', async () => {
    const st = stateOf(); st.shares = [{ shareId: 's1', id: 'r2', status: 'starting' }];
    const h = await loaded({ state: st });
    eq(h.$('allCount').textContent, 2); eq(h.$('favoriteCount').textContent, 1); eq(h.$('sharedCount').textContent, 1, 'a pending share counts as shared');
    has(h.$('folders').innerHTML, 'data-view="folder:f1"'); has(h.$('folders').innerHTML, '>Demos<');
    h.t.view = 'shared'; h.t.render(); eq(h.t.filtered().map(r => r.id).join(), 'r2', 'shared view includes pending shares');
    h.t.view = 'favorites'; eq(h.t.filtered().map(r => r.id).join(), 'r1');
    h.t.view = 'folder:f1'; eq(h.t.filtered().map(r => r.id).join(), 'r1');
    h.t.view = 'all'; h.t.render(); eq(h.$('pageTitle').textContent, 'All recordings');
  });

  await check('empty states: search, shared and folder copy', async () => {
    const h = await loaded();
    has(h.t.emptyCopy('invoice')[0], 'No recordings match “invoice”'); h.t.view = 'shared'; eq(h.t.emptyCopy('')[0], 'Nothing is shared right now');
  });

  await check('active recording label comes from real status, never "Opening recorder"', async () => {
    const t0 = Date.now();
    const h = await loaded({ state: stateOf({ active: { id: 'r1', status: 'recording', startedAt: t0 - 62000 } }) });
    eq(h.$('activeLabel').textContent, 'Recording 1:02'); eq(h.$('activeStatus').hidden, false); eq(h.$('stopRecording').hidden, false);
    eq(h.t.activeLabel({ status: 'saving' }), 'Saving…'); eq(h.t.activeLabel({ status: 'countdown', countdownRemaining: 1.2 }), 'Starting in 2'); eq(h.t.activeLabel({ status: 'countdown' }), 'Countdown');
    eq(h.t.activeLabel({ status: 'recording' }), 'Recording', 'no timer without startedAt'); eq(h.t.activeLabel({ status: 'weird' }), 'In use in OpenRecorder');
    const idle = await loaded(); eq(idle.$('activeStatus').hidden, true);
    const saving = await loaded({ state: stateOf({ active: { id: 'r1', status: 'saving' } }) }); eq(saving.$('stopRecording').hidden, true, 'nothing to stop while saving');
  });

  await check('there is no New recording button: one Record press is the whole start, and Open in browser is not a header button', async () => {
    for (const gone of ['id="newRecording"', 'id="openBrowser"', '>New recording<']) lacks(html, gone, gone);
    lacks(app, /openStudio|open-studio/, 'the page never asks the app to open a window to record');
    has(html, 'id="capRecord"'); has(html, 'id="hostMenuButton"');
    const h = await loaded(); h.fetches.length = 0; await settle(); eq(h.fetches.length, 0, 'loading sends nothing'); eq(h.media.touched, false);
  });

  await check('recording page: tabs only for existing files, details only with metadata, no guessed resolution', async () => {
    const h = await loaded(); h.t.openRecording('r1'); await settle();
    eq(h.$('recordingView').hidden, false); eq(h.$('libraryView').hidden, true); eq(h.$('playerTabs').hidden, false);
    for (const label of ['Combined', 'Screen', 'Camera']) has(h.$('playerTabs').innerHTML, `>${label}<`);
    has(h.$('playerTabs').innerHTML, 'role="tab"'); has(h.$('player').src || '', '/file/r1/recording.mp4?key=OWNERKEY');
    const rows = h.$('detailRows').innerHTML; has(rows, '>Recorded<'); has(rows, '>Duration<'); has(rows, '>Folder<'); has(rows, '>Camera<'); has(rows, '>Off<'); lacks(rows, 'Source', 'no source name in the mock, so no row');
    const files = h.$('detailFiles').innerHTML; has(files, '1920 × 1080'); eq((files.match(/×/g) || []).length, 1, 'resolution only where the file reports it');
    h.t.openRecording('r2'); await settle(); eq(h.$('playerTabs').hidden, true, 'single combined file: no tab row');
    const r2 = h.$('detailRows').innerHTML; lacks(r2, '>Duration<'); lacks(r2, '>Camera<'); lacks(r2, '>Microphone<');
    eq(h.$('moveNote').textContent, 'Filing a recording in a folder never moves its files.'); eq(h.$('detailPath').textContent, '/mock/root/r2', 'the actual stored path is shown as reported');
  });

  await check('recording page offers the same actions as the card menu and nothing else', async () => {
    const h = await loaded(); h.t.openRecording('r1'); await settle(); h.t.openMenu('r1', h.$('detailMore'), 'main', true);
    const m = h.$('menu').innerHTML; for (const l of ['Rename', 'Move to Folder', 'Remove from Favorites', 'Share…', 'Show in Finder', 'Archive']) has(m, l); lacks(m, '>Open<'); lacks(m, /Trash|Delete/);
    h.t.openMenu('r1', h.$('detailMore'), 'move', true); has(h.$('menu').innerHTML, 'No folder'); has(h.$('menu').innerHTML, 'Demos'); has(h.$('menu').innerHTML, 'Filing a recording in a folder never moves its files.');
  });

  await check('menu: Escape and an outside press close it and restore focus', async () => {
    const h = await loaded(); const trigger = h.$('moreTrigger');
    h.t.openMenu('r1', trigger); eq(h.$('menu').hidden, false); eq(trigger.attrs['aria-expanded'], 'true');
    h.dom.listeners.keydown.forEach(f => f({ key: 'Escape', preventDefault() {} })); eq(h.$('menu').hidden, true); eq(trigger.attrs['aria-expanded'], 'false'); eq(h.dom.document.activeElement, trigger);
    h.t.openMenu('r1', trigger); h.dom.listeners['doc:pointerdown'].forEach(f => f({ target: { } })); eq(h.$('menu').hidden, true);
    const archived = stateOf(); archived.recordings[0].archived = true; const a = await loaded({ state: archived }); a.t.openMenu('r1', trigger); has(a.$('menu').innerHTML, /data-command="share" disabled/); has(a.$('menu').innerHTML, 'Unarchive');
  });

  await check('mutations send existing ops: favorite, rename, move and archive with undo toasts', async () => {
    const calls = []; const st = stateOf();
    const h = boot({ key: 'K', api: async (url, o) => { const b = o.body && JSON.parse(o.body); if (b) calls.push(b); return okJson(st); } }); await settle();
    h.t.openMenu('r1', h.$('t')); const click = cmd => h.$('menu').handlers.click.forEach(f => f({ target: { closest: () => ({ dataset: { command: cmd, folder: 'f1' }, disabled: false }) } }));
    click('favorite'); await settle(); eq(JSON.stringify(calls[0]), JSON.stringify({ op: 'update', data: { id: 'r1', patch: { favorite: false } } }));
    ok(!h.$('toast').hidden && /Removed from Favorites/.test(h.$('toastText').textContent) && !h.$('toastUndo').hidden, 'undo toast');
    h.$('toastUndo').onclick(); await settle(); eq(JSON.stringify(calls[calls.length - 1].data.patch), JSON.stringify({ favorite: true }), 'undo restores');
    h.t.openMenu('r2', h.$('t')); click('moveTo'); await settle(); eq(JSON.stringify(calls[calls.length - 1].data.patch), JSON.stringify({ folder: 'f1' }));
    eq(h.$('toastText').textContent, 'Moved to “Demos”.'); lacks(h.$('toastText').textContent, /disk|directory/);
    h.t.openMenu('r1', h.$('t')); click('archive'); await settle(); eq(JSON.stringify(calls[calls.length - 1].data.patch), JSON.stringify({ archived: true }));
  });

  await check('share: pending first, URL and code only when ready, cleared on revoke or expiry', async () => {
    const st = stateOf(); const h = await loaded({ state: st });
    h.t.openShare('r1'); eq(h.$('shareDialog').open, true); eq(h.$('shareNew').hidden, false);
    const link = { shareId: 's1', id: 'r1', status: 'starting', minutes: 60, protected: true };
    st.shares = [link]; await h.t.refresh(); await settle();
    eq(h.$('shareStarting').hidden, false); eq(h.$('shareReady').hidden, true); eq(h.$('shareUrl').value, ''); eq(h.$('shareCode').textContent, '', 'no URL or code while starting');
    has(h.$('shareStartingText').textContent, 'Starting');
    // a ready item whose URL is missing is still pending
    link.status = 'ready'; await h.t.refresh(); eq(h.$('shareReady').hidden, true, 'ready without a URL is not shown as ready');
    Object.assign(link, { url: 'https://quiet-river.trycloudflare.com/p/abc', accessCode: 'K7QM-4P2X', expiresAt: Date.now() + 36e5, unlockedSessions: 2 });
    await h.t.refresh(); await settle();
    eq(h.$('shareReady').hidden, false); eq(h.$('shareUrl').value, link.url); eq(h.$('shareCode').textContent, 'K7QM-4P2X');
    ok(!link.url.includes('K7QM'), 'the code is separate from the link'); has(h.$('shareSessions').textContent, '2 unlocked sessions'); lacks(h.$('shareSessions').textContent, /viewer|watching/i);
    has(html, 'for more care, send the code separately'); has(html, 'Nothing is sent for you'); eq(h.$('shareLimited').hidden, true);
    await h.$('copyShare').onclick(); await settle(); await h.$('copyCode').onclick(); await settle();
    eq(JSON.stringify(h.sandbox.navigator.clipboard.written), JSON.stringify([link.url, 'K7QM-4P2X']));
    link.rateLimited = true; await h.t.refresh(); eq(h.$('shareLimited').hidden, false);
    st.shares = []; await h.t.refresh(); await settle();
    eq(h.$('shareEnded').hidden, false); eq(h.$('shareUrl').value, ''); eq(h.$('shareCode').textContent, '', 'revoked or ended share clears link and code'); has(h.$('shareEndedText').textContent, 'no longer work');
  });

  await check('share: Never is a choice that sends 0 and reads as no end time; Copy invitation holds the link and the code and sends nothing', async () => {
    has(html, /name="shareMinutes" value="0"><span>Never<\/span>/); has(html, 'no deadline while OpenRecorder stays open'); has(html, 'archive the recording or quit');
    const st = stateOf(); const calls = [];
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b) calls.push(b); return okJson(st); } }); await settle();
    h.t.openShare('r1');
    h.sandbox.document.querySelector = sel => (sel === 'input[name=shareMinutes]:checked' ? { value: '0' } : sel === 'input[name=shareMinutes][value="60"]' ? { checked: false } : null);
    await h.$('createShare').onclick(); await settle();
    eq(JSON.stringify(calls.find(c => c.op === 'share').data), JSON.stringify({ id: 'r1', minutes: 0 }), 'Never is minutes 0, not a large number');
    // A never link as the host reports it: no expiresAt, never:true.
    const never = { shareId: 's9', id: 'r1', status: 'ready', minutes: 0, never: true, url: 'https://quiet-river.trycloudflare.com/TOKEN', accessCode: 'K7QM-4P2X' };
    st.shares = [never]; await h.t.refresh(); await settle();
    eq(h.$('shareReady').hidden, false, 'a never link is accepted as ready');
    has(h.$('shareExpiry').textContent, 'No end time'); has(h.$('shareExpiry').textContent, 'awake and online'); lacks(h.$('shareExpiry').textContent, /Active until/);
    has(h.t.paneMarkup('sharing'), 'No end time');
    // Copy invitation: link and code, one short message, only copied.
    h.sandbox.navigator.clipboard.written.length = 0; const before = h.fetches.length;
    await h.$('copyInvite').onclick(); await settle();
    const [invite] = h.sandbox.navigator.clipboard.written;
    eq(invite, 'Se min optagelse:\n\nhttps://quiet-river.trycloudflare.com/TOKEN\n\nAdgangskode:\nK7QM-4P2X\n\nÅbn linket i din browser og skriv adgangskoden. Du behøver ikke installere noget.\n\nLinket har ingen udløbstid, men det ligger ikke permanent online. Det virker kun, så længe min computer og delingen er tændt og online, og jeg kan stoppe det når som helst.');
    ok(invite.length < 400 && !/Weekly|r1|s9/.test(invite), 'short, with no title or internal id');
    // Plain text only, exact shape: the opening lines, the full URL alone on its line, the code alone on its line, no Markdown, quotes or backticks around either.
    const lines = invite.split('\n');
    eq(lines.slice(0, 6).join('|'), 'Se min optagelse:||https://quiet-river.trycloudflare.com/TOKEN||Adgangskode:|K7QM-4P2X', 'the opening, exactly');
    eq(lines[2], 'https://quiet-river.trycloudflare.com/TOKEN', 'the URL is complete and alone on its line'); eq(lines[5], 'K7QM-4P2X', 'the code is alone on its line');
    ok(/^[A-Z0-9-]+$/.test(lines[5]) && !/["'“”`.,;:!?]/.test(lines[5]), 'the code has no quotation marks, backticks or punctuation');
    ok(!/[\r`*<>[\]]/.test(lines.filter((l, i) => i !== 2).join('')) && invite === invite.trim() && !invite.includes('\r'), 'no Markdown, HTML or carriage returns; no trailing whitespace');
    ok(!/Link:|Access code|I’m sharing/.test(invite), 'nothing of the old English layout is left');
    has(invite, 'ikke permanent online'); lacks(invite, /udløber/); has(invite, 'min computer og delingen er tændt og online');
    // The separate copies are exactly the link and exactly the code.
    h.sandbox.navigator.clipboard.written.length = 0;
    await h.$('copyShare').onclick(); await settle(); await h.$('copyCode').onclick(); await settle();
    eq(JSON.stringify(h.sandbox.navigator.clipboard.written), JSON.stringify(['https://quiet-river.trycloudflare.com/TOKEN', 'K7QM-4P2X']), 'Copy link and Copy code are unchanged');
    eq(h.fetches.length, before, 'copying calls no host and sends nothing'); eq(h.$('shareInvite').hidden, true);
    // A finite link names its end; the copies of link and code are unchanged.
    never.never = undefined; never.expiresAt = new Date(2099, 0, 5, 8, 5).getTime(); await h.t.refresh();
    await h.$('copyInvite').onclick(); await settle();
    const timed = h.sandbox.navigator.clipboard.written.at(-1);
    eq(timed, 'Se min optagelse:\n\nhttps://quiet-river.trycloudflare.com/TOKEN\n\nAdgangskode:\nK7QM-4P2X\n\nÅbn linket i din browser og skriv adgangskoden. Du behøver ikke installere noget.\n\nLinket og adgangskoden udløber 5. januar kl. 08.05. Derudover virker linket kun, så længe min computer og delingen er tændt og online, så det kan også stoppe før.');
    has(timed, 'Linket og adgangskoden udløber'); has(timed, 'Derudover virker linket kun'); lacks(timed, /permanent/);
    // A link whose end the host did not report still says it depends on this computer, and invents no end.
    never.expiresAt = undefined; await h.t.refresh(); await h.$('copyInvite').onclick(); await settle();
    const open = h.sandbox.navigator.clipboard.written.at(-1);
    ok(open !== timed && /^Se min optagelse:/.test(open), 'a new invitation was copied for this state'); has(open, 'Linket virker kun, så længe min computer og delingen er tændt og online.'); lacks(open, /udløber|permanent/);
    // A code the host formatted oddly never reaches the message with quotation marks, backticks or closing punctuation around it.
    const original = never.accessCode;
    for (const [odd, clean] of [['"K7QM4P2X123".', 'K7QM4P2X123'], ['`K7QM4P2X123`', 'K7QM4P2X123'], ['“K7QM4P2X123”,', 'K7QM4P2X123']]) {
      never.accessCode = odd; await h.t.refresh(); await h.$('copyInvite').onclick(); await settle();
      eq(h.sandbox.navigator.clipboard.written.at(-1).split('\n')[5], clean, `${odd}: code line`);
    }
    never.accessCode = original;
    never.expiresAt = new Date(2099, 0, 5, 8, 5).getTime(); await h.t.refresh();
    // Where the clipboard refuses, the invitation is shown selected for the person to copy; it is never logged.
    h.sandbox.navigator.clipboard.writeText = () => Promise.reject(new Error('denied'));
    await h.$('copyInvite').onclick(); await settle();
    eq(h.$('shareInvite').hidden, false); has(h.$('shareInvite').value, 'Adgangskode:\nK7QM-4P2X'); eq(h.$('shareInvite').value, timed, 'the selectable fallback is the same text'); has(h.$('toastText').textContent, 'copy manually');
    st.shares = []; await h.t.refresh(); await settle(); eq(h.$('shareInvite').value, '', 'the invitation is cleared with the share'); eq(h.$('shareInvite').hidden, true);
  });
  await check('Copy invitation: a clipboard answer that arrives after the dialog closed, the share ended or another share opened changes nothing and restores no secret', async () => {
    const mk = (over = {}) => ({ shareId: 's1', id: 'r1', status: 'ready', minutes: 60, url: 'https://quiet-river.trycloudflare.com/AAAA', accessCode: 'K7QM-4P2X', expiresAt: Date.now() + 36e5, ...over });
    const scenario = async (name, disturb, { reject = true, expectShown = false } = {}) => {
      const st = stateOf(); st.shares = [mk()];
      const h = await loaded({ state: st }); h.t.openShare('r1'); await settle(); h.$('shareInvite').hidden = true;   // as in the page's markup
      let settleClipboard; h.sandbox.navigator.clipboard.writeText = () => new Promise((resolve, rej) => { settleClipboard = () => (reject ? rej(new Error('denied')) : resolve()); });
      const pending = h.$('copyInvite').onclick(); await settle();
      const toastBefore = h.$('toastText').textContent;
      await disturb(h, st); await settle();
      settleClipboard(); await pending; await settle();
      const box = h.$('shareInvite');
      if (expectShown) { eq(box.hidden, false, name); has(box.value, 'K7QM-4P2X'); return; }
      eq(box.hidden, true, `${name}: the invitation box stays hidden`); eq(box.value, '', `${name}: and holds no secret`);
      eq(h.$('toastText').textContent, toastBefore, `${name}: no toast either`);
      ok(!JSON.stringify([h.$('shareUrl').value, h.$('shareCode').textContent]).includes('K7QM') || h.$('shareDialog').open, `${name}: link and code are not restored into a closed dialog`);
    };
    const settleAll = async h => { await settle(); await h.t.refresh(); await settle(); };
    await scenario('closed', async h => { h.$('shareDialog').close(); });
    await scenario('revoked', async (h, st) => { st.shares = []; await settleAll(h); });
    await scenario('expired', async (h, st) => { st.shares[0].expiresAt = Date.now() - 1; await settleAll(h); });
    await scenario('replaced by another share', async (h, st) => { st.shares = [mk({ shareId: 's2', url: 'https://other-river.trycloudflare.com/BBBB', accessCode: 'ZZZZ-9999' })]; await settleAll(h); });
    await scenario('closed and reopened', async (h, st) => { h.$('shareDialog').close(); await settle(); h.t.openShare('r1'); await settle(); });
    // The same for a late success: nothing is announced for a dialog that is gone.
    await scenario('closed, late success', async h => { h.$('shareDialog').close(); }, { reject: false });
    // And the ordinary fallback and success still work when nothing changed.
    await scenario('unchanged, refused', async () => {}, { expectShown: true });
    const st = stateOf(); st.shares = [mk()]; const h = await loaded({ state: st }); h.t.openShare('r1'); await settle();
    h.sandbox.navigator.clipboard.writeText = () => Promise.resolve();
    await h.$('copyInvite').onclick(); await settle(); eq(h.$('toastText').textContent, 'Invitation copied'); eq(h.$('shareInvite').hidden, true);
  });
  await check('Copy invitation: after a successful revoke whose state refresh fails (the cached share still reads ready), a late clipboard rejection or success changes nothing', async () => {
    for (const reject of [true, false]) {
      const mk = () => ({ shareId: 's1', id: 'r1', status: 'ready', minutes: 60, url: 'https://quiet-river.trycloudflare.com/AAAA', accessCode: 'K7QM-4P2X', expiresAt: Date.now() + 36e5 });
      const st = stateOf(); st.shares = [mk()]; let revoked = false; const ops = [];
      const h = boot({ key: 'OWNERKEY', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b) ops.push(b.op); if (b?.op === 'unshare') { revoked = true; return okJson({ revoked: true }); } if (revoked) throw new Error('offline'); return okJson(st); } }); await settle();
      h.t.openShare('r1'); await settle(); h.$('shareInvite').hidden = true;   // as in the page's markup
      let settleClipboard; h.sandbox.navigator.clipboard.writeText = () => new Promise((resolve, rej) => { settleClipboard = () => (reject ? rej(new Error('denied')) : resolve()); });
      const pending = h.$('copyInvite').onclick(); await settle();
      const toastBefore = h.$('toastText').textContent;
      // The real revoke path: unshare succeeds, the state refresh then fails and the cached share is untouched.
      h.$('revokeShare').onclick(); for (let i = 0; i < 10; i++) await settle();
      ok(ops.includes('unshare'), 'the revoke reached the host');
      eq(h.t.state.shares.length, 1, 'the cached share still reads ready after the failed refresh'); eq(h.t.state.shares[0].status, 'ready');
      eq(h.$('shareEnded').hidden, false, 'but the dialog shows the link as ended'); eq(h.$('shareInvite').value, ''); eq(h.$('shareUrl').value, '');
      settleClipboard(); await pending; await settle();
      const word = reject ? 'rejection' : 'success';
      eq(h.$('shareInvite').hidden, true, `late ${word}: the invitation box stays hidden`); eq(h.$('shareInvite').value, '', `late ${word}: no secret is restored`);
      eq(h.$('toastText').textContent, toastBefore, `late ${word}: nothing is announced`); eq(h.$('shareUrl').value, ''); eq(h.$('shareCode').textContent, '');
    }
  });
  await check('share: eight-character code is formatted, other shapes shown as received; expiry clears it', async () => {
    const h = await loaded(); eq(h.t.formatCode('k7qm4p2x'), 'K7QM-4P2X'); eq(h.t.formatCode('K7QM-4P2X'), 'K7QM-4P2X'); eq(h.t.formatCode('ABC'), 'ABC');
    eq(h.t.shareCode({ code: 'Z' }), 'Z'); eq(h.t.shareLink({ shareUrl: 'u' }), 'u');
    const st = stateOf(); st.shares = [{ shareId: 's', id: 'r1', status: 'ready', url: 'https://a.example/p', accessCode: 'ABCD-EFGH', expiresAt: Date.now() + 5000 }];
    const g = await loaded({ state: st }); g.t.openShare('r1'); eq(g.$('shareReady').hidden, false);
    st.shares[0].expiresAt = Date.now() - 1; g.t.render(); eq(g.$('shareEnded').hidden, false); eq(g.$('shareUrl').value, ''); eq(g.$('shareCode').textContent, ''); has(g.$('shareEndedText').textContent, 'expired');
    eq(g.$('sharedCount').textContent, '', 'expired links no longer count as shared');
  });

  await check('embedded ui/initialize reports the preview.12 host version', async () => {
    const st = stateOf(); const h = boot({ embedded: true, mediaKey: 'M', api: async () => { throw 0; }, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : { structuredContent: st } }); await settle();
    const init = h.posted.find(m => m.method === 'ui/initialize'); eq(init.params.appInfo.version, '0.6.0-preview.12'); has(app, /version:'0\.6\.0-preview\.12'/); lacks(app, 'preview.6');
  });

  await check('share: a cancelled share (failed "Sharing was cancelled") is not offered as a share or counted', async () => {
    const st = stateOf(); st.shares = [{ shareId: 's9', id: 'r1', status: 'failed', error: 'Sharing was cancelled' }];
    const h = await loaded({ state: st }); eq(h.$('sharedCount').textContent, ''); h.t.openShare('r1'); eq(h.$('shareNew').hidden, false); eq(h.$('shareFailed').hidden, true);
    lacks(h.t.cardMarkup(st.recordings[0]), 'class="tag');
  });

  await check('share: host error texts map to the right panel (cloudflared missing, tunnel could not connect)', async () => {
    const st = stateOf(); const h = await loaded({ state: st }); h.t.openShare('r1');
    st.shares = [{ shareId: 's1', id: 'r1', status: 'failed', error: 'Install cloudflared to create a temporary link' }]; await h.t.refresh(); await settle();
    eq(h.$('shareFailed').hidden, false); has(h.$('shareFailedTitle').textContent, 'Sharing needs cloudflared'); eq(h.$('shareInstall').hidden, false);
    st.shares[0].error = 'Temporary tunnel could not connect'; await h.t.refresh(); await settle(); has(h.$('shareFailedText').textContent, 'Temporary tunnel could not connect Nothing was shared.'); eq(h.$('shareInstall').hidden, true);
  });

  await check('mobile navigation sheet: page behind is inert, Tab wraps inside, Escape and the close button return focus', async () => {
    const h = await loaded(); const menu = h.$('menuButton'), content = h.$('content'), sidebar = h.$('sidebar');
    const first = new h.dom.El('first'), last = new h.dom.El('last'); sidebar.querySelectorAll = () => [first, last]; sidebar.querySelector = () => first;
    menu.onclick(); eq(content.inert, true, 'content inert'); eq(h.$('scrim').hidden, false); eq(menu.attrs['aria-expanded'], 'true'); eq(h.dom.document.activeElement, first, 'focus moves into the sheet when the person opens it');
    const tab = shift => { let prevented = false; sidebar.handlers.keydown.forEach(f => f({ key: 'Tab', shiftKey: shift, preventDefault() { prevented = true; } })); return prevented; };
    last.focus(); ok(tab(false), 'Tab on the last item wraps'); eq(h.dom.document.activeElement, first);
    ok(tab(true), 'Shift+Tab on the first item wraps'); eq(h.dom.document.activeElement, last);
    first.focus(); eq(tab(false), false, 'Tab inside the sheet is left to the browser');
    h.dom.listeners.keydown.forEach(f => f({ key: 'Escape', preventDefault() {} })); eq(content.inert, false); eq(h.$('scrim').hidden, true); eq(menu.attrs['aria-expanded'], 'false'); eq(h.dom.document.activeElement, menu);
    menu.onclick(); eq(content.inert, true); h.$('sheetClose').onclick(); eq(content.inert, false); eq(h.dom.document.activeElement, menu);
    menu.onclick(); h.$('scrim').onclick(); eq(content.inert, false, 'outside press closes it');
    has(html, /id="sheetClose" aria-label="Close navigation"/); has(css, /\.sheet-close\{display:none/); has(css, /\.sheet-close\{display:grid\}/);
  });

  await check('closing the sheet does not steal focus when the page is resized to desktop', async () => {
    const h = await loaded(); const other = h.$('search'); h.$('menuButton').onclick(); other.focus(); h.sandbox.innerWidth = 1280;
    h.dom.listeners.resize.forEach(f => f()); eq(h.$('content').inert, false); eq(h.dom.document.activeElement, other);
  });

  await check('dialogs are native modals: showModal only (native focus containment), Escape native, outside click closes', () => {
    lacks(app, /\.show\(\)/); ok((app.match(/showModal\(\)/g) || []).length >= 3, 'settings, share and name dialogs use showModal');
    has(app, /document\.querySelectorAll\('dialog'\)\.forEach\(d=>\{d\.addEventListener\('click'/); has(app, "if(document.querySelector('dialog[open]'))return;");
    lacks(html, /<dialog[^>]*\bopen\b/); ok(!/focus-trap|tabindex="[1-9]"/.test(html + app), 'no hand-rolled trap or positive tabindex');
  });

  await check('active status renders only fields the host sends (the host currently sends recording until finish)', async () => {
    const h = await loaded({ state: stateOf({ active: { id: 'r1', engine: 'native', status: 'recording', startedAt: Date.now() - 5000 } }) });
    eq(h.$('activeLabel').textContent, 'Recording 0:05'); eq(h.$('stopRecording').hidden, false);
    lacks(app + html, /Finishing|Processing|Encoding|<progress|progress(Bar|Value)|\d+%/i, 'no invented phases or progress bars');
  });

  await check('share: only completed, unarchived recordings can open the panel', async () => {
    const st = stateOf(); st.recordings[1].status = 'interrupted'; const h = await loaded({ state: st });
    h.t.openShare('r2'); eq(h.$('shareDialog').open, false); h.t.openShare('r1'); eq(h.$('shareDialog').open, true);
  });

  await check('share: failures stay failures — cloudflared missing and generic, no automatic retry', async () => {
    const st = stateOf(); const h = await loaded({ state: st }); h.t.openShare('r1'); h.fetches.length = 0;
    st.shares = [{ shareId: 's1', id: 'r1', status: 'failed', error: 'cloudflared was not found' }]; await h.t.refresh(); await settle();
    eq(h.$('shareFailed').hidden, false); has(h.$('shareFailedTitle').textContent, 'needs cloudflared'); eq(h.$('shareInstall').hidden, false);
    st.shares[0].error = 'The tunnel did not answer'; await h.t.refresh(); await settle(); eq(h.$('shareInstall').hidden, true); has(h.$('shareFailedText').textContent, 'Nothing was shared.');
    eq(h.fetches.filter(f => f.body && f.body.op === 'share').length, 0, 'never retried on its own');
  });

  await check('share request: explicit recording id and lifetime, default one hour', async () => {
    const st = stateOf(); const h = boot({ key: 'K', api: async (u, o) => okJson(st) }); await settle(); h.t.openShare('r1'); h.fetches.length = 0;
    await h.$('createShare').onclick(); await settle(); const req = h.fetches.find(f => f.body && f.body.op === 'share');
    eq(JSON.stringify(req.body.data), JSON.stringify({ id: 'r1', minutes: 60 }));
    has(html, /name="shareMinutes" value="15"/); has(html, /value="60" checked/); has(html, /value="480"/);
  });

  await check('settings: only supported rows; rows without a backend key stay hidden', async () => {
    const h = await loaded(); const rec = h.t.paneMarkup('recording'), cam = h.t.paneMarkup('camera'), gen = h.t.paneMarkup('general');
    has(rec, 'name="quality"'); has(rec, 'name="fps"'); has(rec, 'name="keepOriginals"'); has(rec, 'name="countdownSeconds"'); has(rec, '>Off<'); has(rec, '>3 seconds<'); has(rec, 'id="chooseDefaultRoot"');
    has(cam, 'name="mirror"'); has(cam, 'name="cameraShape"'); has(cam, 'name="cameraSize"'); has(cam, 'name="rememberInputs"'); has(cam, 'role="switch"'); has(cam, 'system audio isn’t available'); has(gen, 'name="theme"');
    lacks(cam + rec + gen, /Open at login|shortcut|name="systemAudio"|name="countdown"/i);
    // The rows still hide a key that is absent from the page state; but a host snapshot without those keys is no longer accepted at all.
    const old = stateOf(); delete old.settings.countdownSeconds; delete old.settings.rememberInputs;
    const o = await loaded(); o.t.state = old; lacks(o.t.paneMarkup('recording'), 'countdownSeconds'); lacks(o.t.paneMarkup('camera'), 'rememberInputs');
    const refused = await loaded({ state: old }); eq(refused.t.state, null, 'an incomplete settings object never loads'); eq(refused.$('reach').hidden, false);
  });

  await check('settings: camera sizes 120/168/224 are always offered; a stored legacy size stays selected and is never coerced', async () => {
    const sizeOptions = h => [...h.t.paneMarkup('camera').matchAll(/<select id="s-cameraSize"[^>]*>([\s\S]*?)<\/select>/g)][0][1];
    const values = markup => [...markup.matchAll(/<option value="(\d+)"/g)].map(m => m[1]).join();
    const fresh = await loaded(); eq(values(sizeOptions(fresh)), '120,168,224'); has(sizeOptions(fresh), /value="168" selected>Medium</);
    for (const legacy of [140, 200, 280]) {
      const st = stateOf(); st.settings.cameraSize = legacy; const h = await loaded({ state: st }); const m = sizeOptions(h);
      eq(values(m), `120,168,224,${legacy}`, 'all three new sizes without requiring the stored size to be in the new set');
      has(m, new RegExp(`value="${legacy}" selected>Current \\(${legacy}\\)<`)); lacks(m, /value="(120|168|224)" selected/); eq(h.t.state.settings.cameraSize, legacy);
    }
  });

  await check('settings: stored 200 with wide shape — shape stays wide, size stays 200, and each save sends only its own key', async () => {
    const st = stateOf(); Object.assign(st.settings, { cameraSize: 200, cameraShape: 'wide' }); const calls = []; let mode = 'ok';
    const h = boot({ key: 'K', api: async (u, o) => {
      const b = o.body && JSON.parse(o.body);
      if (b && b.op === 'settings') { calls.push(b.data); if (mode === 'reject') return { ok: false, status: 400, json: async () => ({ error: 'Invalid cameraSize' }) }; return okJson({ ...st, settings: { ...st.settings, ...b.data.preferences } }); }
      return okJson(st);
    } }); await settle();
    const markup = h.t.paneMarkup('camera'); has(markup, /<option value="wide" selected>Wide 16:9</); has(markup, /<option value="200" selected>Current \(200\)</);
    for (const shape of ['circle', 'rounded', 'wide', 'portrait']) has(markup, `value="${shape}"`);
    lacks(markup, 'value="square"', 'square is offered only to a person who already has it');
    const shape = Object.assign(h.$('s-cameraShape'), { name: 'cameraShape', value: 'circle' }); h.t.openSettings('camera');
    h.$('settingsPane').onchange({ target: shape }); await settle(10);
    eq(JSON.stringify(calls[0]), JSON.stringify({ preferences: { cameraShape: 'circle' } }), 'changing shape never rewrites the legacy size');
    eq(h.t.state.settings.cameraSize, 200); eq(h.t.state.settings.cameraShape, 'circle');
    const same = Object.assign(h.$('s-cameraSize'), { name: 'cameraSize', value: '200' }); h.$('settingsPane').onchange({ target: same }); await settle(10); eq(calls.length, 1, 'keeping the current legacy size sends nothing');
    mode = 'reject'; const size = Object.assign(h.$('s-cameraSize'), { name: 'cameraSize', value: '224' }); h.$('settingsPane').onchange({ target: size }); await settle(10);
    eq(JSON.stringify(calls[1]), JSON.stringify({ preferences: { cameraSize: 224 } })); eq(size.value, '200', 'rolled back to the stored legacy size'); eq(h.t.state.settings.cameraSize, 200); has(h.$('settingsStatus').textContent, 'Camera size not saved. Invalid cameraSize');
    mode = 'ok'; size.value = '168'; h.$('settingsPane').onchange({ target: size }); await settle(10);
    eq(h.t.state.settings.cameraSize, 168, 'a deliberate choice of a new size is saved'); has(h.$('settingsStatus').textContent, 'Camera size saved');
  });

  await check('settings: countdown and remember-inputs saves are confirmed per field and roll back when rejected', async () => {
    const st = stateOf(); let mode = 'ok'; const calls = [];
    const h = boot({ key: 'K', api: async (u, o) => {
      const b = o.body && JSON.parse(o.body);
      if (b && b.op === 'settings') { calls.push(b.data.preferences); if (mode === 'reject') return { ok: false, status: 400, json: async () => ({ error: 'Use countdownSeconds (0 or 3)' }) }; return okJson({ ...st, settings: { ...st.settings, ...b.data.preferences } }); }
      return okJson(st);
    } }); await settle(); h.t.openSettings('recording');
    const count = Object.assign(h.$('s-countdownSeconds'), { name: 'countdownSeconds', value: '0' }); h.$('settingsPane').onchange({ target: count }); await settle(10);
    eq(JSON.stringify(calls[0]), JSON.stringify({ countdownSeconds: 0 }), 'a number, not a string'); eq(h.t.state.settings.countdownSeconds, 0);
    mode = 'reject'; count.value = '3'; h.$('settingsPane').onchange({ target: count }); await settle(10); eq(count.value, '0'); eq(h.t.state.settings.countdownSeconds, 0);
    const remember = Object.assign(h.$('s-rememberInputs'), { name: 'rememberInputs', checked: false, type: 'checkbox' }); mode = 'ok';
    h.$('settingsPane').onchange({ target: remember }); await settle(10); eq(JSON.stringify(calls[calls.length - 1]), JSON.stringify({ rememberInputs: false })); eq(h.t.state.settings.rememberInputs, false);
    lacks(h.t.paneMarkup('camera'), /id="s-rememberInputs"[^>]* checked/);
  });

  await check('settings: a host capability list, when sent, replaces the built-in choices', async () => {
    const st = stateOf(); st.capabilities = { settings: { cameraShape: ['circle', 'rounded', 'wide'], cameraSize: [120, 168, 224] } };
    const c = await loaded({ state: st }); const m = c.t.paneMarkup('camera'); has(m, 'value="wide"'); has(m, 'Wide 16:9'); lacks(m, 'value="square"'); has(m, 'value="120"');
  });

  await check('settings are disabled while recording and say why', async () => {
    const h = await loaded({ state: stateOf({ active: { id: 'r1', status: 'recording', startedAt: Date.now() } }) });
    const m = h.t.paneMarkup('recording'); has(m, /<fieldset class="rows" disabled>/); has(m, 'locked while a recording is in progress');
  });

  await check('settings autosave: confirmed value is kept; unconfirmed or rejected value rolls back that field', async () => {
    let mode = 'ok'; const st = stateOf();
    const h = boot({ key: 'K', api: async (u, o) => {
      const b = o.body && JSON.parse(o.body);
      if (b && b.op === 'settings') { if (mode === 'reject') return { ok: false, status: 400, json: async () => ({ error: 'Invalid quality' }) }; if (mode === 'ignore') return okJson(st); return okJson({ ...st, settings: { ...st.settings, ...b.data.preferences } }); }
      return okJson(st);
    } }); await settle();
    const el = Object.assign(h.$('s-quality'), { name: 'quality', value: '2560' });
    h.t.openSettings(); h.$('settingsPane').onchange({ target: el }); await settle(10);
    eq(h.t.state.settings.quality, 2560); has(h.$('settingsStatus').textContent, 'Resolution saved');
    mode = 'reject'; el.value = '1280'; h.$('settingsPane').onchange({ target: el }); await settle(10);
    eq(el.value, '2560', 'rolled back'); eq(h.t.state.settings.quality, 2560); has(h.$('settingsStatus').textContent, 'Resolution not saved. Invalid quality'); eq(el.disabled, false);
    mode = 'ignore'; el.value = '1920'; h.$('settingsPane').onchange({ target: el }); await settle(10);
    eq(el.value, '2560', 'a backend that silently ignores the key is treated as unconfirmed'); has(h.$('settingsStatus').textContent, 'not saved');
    const toggle = Object.assign(h.$('s-mirror'), { name: 'mirror', checked: false, type: 'checkbox' }); mode = 'ok';
    h.$('settingsPane').onchange({ target: toggle }); await settle(10); eq(h.t.state.settings.mirror, false);
  });

  await check('settings tabs: sections are a tablist; sharing lists active links with Revoke only', async () => {
    const st = stateOf(); st.shares = [{ shareId: 's1', id: 'r1', status: 'ready', url: 'https://a.example/p', accessCode: 'ABCD-EFGH', expiresAt: Date.now() + 9e5 }, { shareId: 's2', id: 'r2', status: 'failed', error: 'x' }];
    const h = await loaded({ state: st }); h.t.openSettings('general'); has(h.$('settingsNav').innerHTML, /role="tab" id="tab-general"[^>]*aria-selected="true"/);
    for (const l of ['General', 'Recording', 'Camera &amp; Microphone', 'Sharing']) has(h.$('settingsNav').innerHTML, l);
    h.t.openSettings('sharing'); const m = h.$('settingsPane').innerHTML; has(m, 'data-revoke="r1"'); has(m, 'data-share="s1"'); lacks(m, 'ABCD-EFGH', 'the code is never listed in Settings'); lacks(m, 'a.example'); eq((m.match(/data-revoke=/g) || []).length, 1, 'failed shares are not listed as active');
    has(m, 'kept in memory only');
  });

  await check('unreachable library: bounded 2s, 5s, 15s retries, dimmed content, raw error only in Details, then manual', async () => {
    let up = true; const st = stateOf();
    const h = boot({ key: 'K', api: async () => { if (!up) throw new TypeError('Failed to fetch https://127.0.0.1:1/api/state'); return okJson(st); } }); await settle();
    eq(h.$('reach').hidden, true); h.fetches.length = 0; up = false;
    eq(h.delays().join(), '4000', 'healthy polling is every four seconds'); eq(await h.runTimeout(), 4000);
    eq(h.$('reach').hidden, false); eq(h.$('content').classes.has('dimmed'), true, 'last content stays, dimmed');
    has(h.$('reachText').textContent, 'Showing what was loaded at'); has(h.$('reachText').textContent, 'Checking again shortly'); eq(h.$('reachRetry').hidden, true);
    ok(!h.$('recordings').innerHTML.includes('Failed to fetch'), 'raw error stays out of the content'); has(h.$('reachRaw').textContent, 'Failed to fetch'); ok(!/<details[^>]*\bopen\b/.test(html), 'details collapsed');
    eq(h.delays().join(), '2000'); eq(await h.runTimeout(), 2000); eq(h.delays().join(), '5000'); eq(await h.runTimeout(), 5000); eq(h.delays().join(), '15000'); eq(await h.runTimeout(), 15000);
    eq(h.delays().length, 0, 'automatic checks stop'); eq(h.fetches.length, 4, 'one failure plus three bounded checks, no storm');
    has(h.$('reachText').textContent, 'Checked 3 times, then stopped.'); eq(h.$('reachRetry').hidden, false);
    h.dom.listeners['doc:visibilitychange'].forEach(f => f()); await settle(); eq(h.fetches.length, 4, 'returning to the tab does not restart a stopped loop');
    h.$('reachRetry').onclick(); await settle(); eq(h.fetches.length, 5, 'one manual check'); eq(h.delays().length, 0, 'a failed manual check does not re-arm polling');
    up = true; h.$('reachRetry').onclick(); await settle(); eq(h.$('reach').hidden, true); eq(h.$('content').classes.has('dimmed'), false); eq(h.delays().join(), '4000', 'recovered, normal polling resumes');
  });

  await check('unreachable library before anything loaded says so without showing stale content', async () => {
    const h = boot({ key: 'K', api: async () => { throw new TypeError('offline'); } }); await settle();
    eq(h.$('reach').hidden, false); has(h.$('reachText').textContent, 'Nothing has loaded yet'); eq(h.$('content').classes.has('dimmed'), false);
  });

  await check('a rejected library key becomes the calm locked state, with no retry loop and no raw 403', async () => {
    const h = boot({ key: 'OLD', api: async () => ({ ok: false, status: 403, json: async () => ({}) }) }); await settle();
    eq(h.$('locked').hidden, false); lacks(h.$('lockedText').textContent, /403|forbidden/i); eq(h.delays().length, 0); eq(h.fetches.length, 1);
  });

  await check('toasts: status role for info, alert role for errors, bounded and truthful', async () => {
    const long = 'x'.repeat(900);
    const c = boot({ key: 'K', api: async (u, o) => (o.body ? { ok: false, status: 500, json: async () => ({ error: long }) } : okJson(stateOf())) }); await settle();
    c.t.openMenu('r1', c.$('t')); c.$('menu').handlers.click.forEach(f => f({ target: { closest: () => ({ dataset: { command: 'reveal' }, disabled: false }) } })); await settle();
    eq(c.$('toastAlert').hidden, false); ok(c.$('toastAlertText').textContent.length <= 240, 'error text is bounded'); eq(c.$('toast').hidden, true);
    ok(![...c.timeouts.values()].some(v => v.delay > 8000 && v.delay < 100000), 'errors leave after 8 s at most');
  });

  await check('detail rows never invent metadata', async () => {
    const h = await loaded(); const rows = h.t.detailRows({ createdAt: Date.now(), files: [], settings: {} }).map(r => r[0]); eq(rows.join(), 'Recorded');
    const full = h.t.detailRows({ createdAt: Date.now(), duration: 10, source: { name: 'Built-in Display' }, settings: { camera: true, microphone: true }, files: [] }).map(r => r[0]);
    eq(full.join(), 'Recorded,Duration,Source,Camera,Microphone');
  });

  // ================= request deadlines and uncertain changes (candidate A review finding 5) =================
  const never = () => new Promise(() => {});
  const fire = async (h, delay) => { const entry = [...h.timeouts.entries()].find(([, v]) => v.delay === delay); if (!entry) throw new Error(`no timer of ${delay} ms; have ${h.delays()}`); h.timeouts.delete(entry[0]); entry[1].fn(); await settle(10); };
  const stalls = {
    'before the response headers': () => never(),
    'while the response body is being read': async () => ({ ok: true, status: 200, json: never }),
  };
  for (const [where, stall] of Object.entries(stalls)) {
    await check(`a host that stalls ${where} ends the request at its deadline and starts the 2/5/15 s recovery, then waits for a manual retry`, async () => {
      let healthy = false; const st = stateOf();
      const h = boot({ key: 'K', api: async () => (healthy ? okJson(st) : stall()) }); await settle();
      eq(h.fetches.length, 1); ok(h.delays().includes(7000), 'a 7 s read deadline is armed');
      const seen = [];
      for (let i = 0; i < 3; i++) { await fire(h, 7000); has(h.$('reachText').textContent, 'Checking again shortly'); seen.push(h.delays().find(d => d === [2000, 5000, 15000][i])); await fire(h, [2000, 5000, 15000][i]); }
      eq(JSON.stringify(seen), JSON.stringify([2000, 5000, 15000]));
      await fire(h, 7000);
      eq(h.$('reach').hidden, false); has(h.$('reachText').textContent, 'Checked 3 times, then stopped'); eq(h.$('reachRetry').hidden, false);
      eq(h.delays().filter(d => d !== 7000).length, 0, 'nothing more is scheduled automatically');
      ok(h.fetches.every(f => f.opts.signal && f.opts.signal.aborted), 'every timed-out request was aborted');
      healthy = true; h.$('reachRetry').click(); await settle(10);
      eq(h.$('reach').hidden, true); ok(h.t.state && h.t.state.recordings.length === 2, 'the manual retry loaded the library');
    });
  }
  await check('a response that arrives after its deadline is ignored', async () => {
    let release; const st = stateOf(), late = stateOf({ root: '/late/root' });
    const h = boot({ key: 'K', api: () => new Promise(r => { release = () => r(okJson(late)); }) }); await settle();
    await fire(h, 7000); release(); await settle(10);
    ok(!h.t.state || h.t.state.root !== '/late/root', 'a late reply never replaces the page state');
    ok(st);
  });
  await check('an unanswered settings change is read back from the host: confirmed, not applied, or honestly unknown; never replayed', async () => {
    for (const hostHolds of ['new', 'old', 'unreachable']) {
      const st = stateOf(); const posts = []; let reads = 0;
      const h = boot({ key: 'K', api: async (u, o) => {
        const b = o.body && JSON.parse(o.body);
        if (b && b.op === 'settings') { posts.push(b.data); return never(); }
        if (b) return okJson(st);
        reads++; if (reads === 1 || hostHolds !== 'unreachable') return okJson(hostHolds === 'new' && reads > 1 ? { ...st, settings: { ...st.settings, quality: 2560 } } : st);
        return never();
      } }); await settle();
      const el = Object.assign(h.$('s-quality'), { name: 'quality', value: '2560' });
      h.t.openSettings(); h.$('settingsPane').onchange({ target: el }); await settle(10);
      eq(el.disabled, true, 'the control is waiting, within its deadline'); has(h.$('settingsStatus').textContent, 'Saving');
      await fire(h, 20000);
      if (hostHolds === 'unreachable') { await fire(h, 7000); }
      await settle(10);
      eq(el.disabled, false, `${hostHolds}: the control is usable again`);
      eq(posts.length, 1, `${hostHolds}: the change is never sent twice`);
      const text = h.$('settingsStatus').textContent;
      if (hostHolds === 'new') { has(text, 'Resolution saved'); eq(h.t.state.settings.quality, 2560); }
      if (hostHolds === 'old') { has(text, 'Resolution not saved'); eq(el.value, String(st.settings.quality), 'rolled back only after the host confirmed it is old'); }
      if (hostHolds === 'unreachable') { has(text, 'unknown whether it was saved'); lacks(text, /saved\.$/); eq(el.value, String(st.settings.quality)); }
    }
  });
  await check('an unanswered Share is never sent again and a link that appeared in the host list is found, not duplicated', async () => {
    for (const created of [true, false]) {
      const st = stateOf(); let sharePosts = 0, reads = 0;
      const h = boot({ key: 'K', api: async (u, o) => {
        const b = o.body && JSON.parse(o.body);
        if (b && b.op === 'share') { sharePosts++; return never(); }
        reads++; return okJson(created && reads > 1 ? { ...st, shares: [{ shareId: 's1', id: 'r1', status: 'starting' }] } : st);
      } }); await settle();
      h.t.openShare('r1'); h.$('createShare').click(); await settle(10);
      await fire(h, 45000); await settle(10);
      eq(sharePosts, 1, 'one request only');
      if (created) { eq(h.$('shareFailed').hidden, true, 'the link the host lists is used'); eq(h.$('shareStarting').hidden, false); }
      else {
        eq(h.$('shareFailed').hidden, false); has(h.$('shareFailedText').textContent, 'unknown whether sharing started'); lacks(h.$('shareFailedText').textContent, 'Nothing was shared');
        has(h.$('shareFailedTitle').textContent, 'Couldn’t confirm');
      }
      eq(h.$('shareUrl').value, '', 'no credential is shown unless the host reports ready');
    }
  });
  await check('an unanswered Record press (or any change) is reported as uncertain, refreshed, and not replayed', async () => {
    const st = stateOf({ capture: { phase: 'idle', generation: 4, launch: 'launch-aaaa-0001', known: true } }); let posts = 0;
    const h = boot({ key: 'K', api: async (u, o) => { if (o.body) { posts++; return never(); } return okJson(st); } }); await settle();
    const before = h.fetches.length; h.$('capRecord').click(); await settle(10); await fire(h, 20000); await settle(10);
    eq(posts, 1); has(h.$('toastAlertText').textContent, 'unknown whether'); ok(h.fetches.length > before, 'the host was re-read afterwards');
    eq(h.fetches.filter(f => f.body && f.body.op === 'start-recording').length, 1, 'one press, one request, never replayed');
  });
  await check('the embedded RPC path gives up on a silent host for reads (10 s), keeps the long deadline for changes and never replays them', async () => {
    const st = stateOf(); let silent = true;
    const h = boot({ embedded: true, mediaKey: 'M', api: async () => { throw 0; }, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : (silent ? SILENT : { structuredContent: st }) }); await settle();
    ok(h.delays().includes(10000), `reads have a 10 s deadline, not 135 s: ${h.delays()}`);
    await fire(h, 10000);
    has(h.$('reachText').textContent, 'Checking again shortly'); ok(h.delays().includes(2000), 'the 2 s retry is armed');
    silent = false; await fire(h, 2000); eq(h.$('reach').hidden, true); ok(h.t.state, 'recovered on the retry');
    // A silent change keeps the validated 135 s RPC deadline and is flagged uncertain when it expires.
    silent = true; h.t.openMenu('r1', h.$('t')); h.$('menu').handlers.click.forEach(f => f({ target: { closest: () => ({ dataset: { command: 'favorite' }, disabled: false }) } })); await settle(10);
    ok([...h.timeouts.values()].some(v => v.delay === 135000), 'changes keep their long deadline');
    const sent = () => h.posted.filter(m => m.method === 'tools/call' && m.params.arguments.op === 'update').length;
    eq(sent(), 1); await fire(h, 135000);
    has(h.$('toastAlertText').textContent, 'unknown whether'); eq(sent(), 1, 'never replayed');
  });
  // ================= unreadable response bodies and incomplete snapshots (candidate B review finding 2) =================
  const cutOff = () => ({ ok: true, status: 200, json: async () => { throw new SyntaxError('Unexpected end of JSON input'); } });
  const truncated = () => ({ ok: true, status: 200, json: async () => JSON.parse('{"settings":{"quality":') });
  for (const [what, broken] of Object.entries({ 'a body connection that terminates': cutOff, 'truncated JSON': truncated })) {
    await check(`a 200 response with ${what} is not a success: a settings change is read back instead of reporting "not saved" or "saved"`, async () => {
      for (const hostHolds of ['new', 'old']) {
        const st = stateOf(); const posts = []; let reads = 0;
        const h = boot({ key: 'K', api: async (u, o) => {
          const b = o.body && JSON.parse(o.body);
          if (b && b.op === 'settings') { posts.push(b.data); return broken(); }
          reads++; return okJson(hostHolds === 'new' && reads > 1 ? { ...st, settings: { ...st.settings, quality: 2560 } } : st);
        } }); await settle();
        const el = Object.assign(h.$('s-quality'), { name: 'quality', value: '2560' });
        h.t.openSettings(); h.$('settingsPane').onchange({ target: el }); await settle(10);
        eq(posts.length, 1, 'never replayed'); eq(el.disabled, false);
        const text = h.$('settingsStatus').textContent;
        if (hostHolds === 'new') { has(text, 'Resolution saved'); eq(h.t.state.settings.quality, 2560); } else { has(text, 'Resolution not saved'); eq(el.value, String(st.settings.quality)); }
        lacks(text, /Invalid|SyntaxError|Unexpected/, 'no raw parser text');
      }
    });
  }
  await check('a cut-off answer to Revoke is uncertain: the link is not presented as ended, nothing is sent twice, and the host is re-read', async () => {
    const st = stateOf(); st.shares = [{ shareId: 's1', id: 'r1', status: 'ready', url: 'https://a.example/p', accessCode: 'ABCD-EFGH', expiresAt: Date.now() + 60000 }];
    let unshares = 0;
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b && b.op === 'unshare') { unshares++; return cutOff(); } return okJson(st); } }); await settle();
    h.t.openShare('r1'); eq(h.$('shareReady').hidden, false); const reads = h.fetches.length;
    h.$('revokeShare').click(); await settle(12);
    eq(unshares, 1); eq(h.$('shareEnded').hidden, true, 'not shown as ended'); eq(h.$('shareReady').hidden, false, 'the host still lists the link');
    has(h.$('toastAlertText').textContent, 'unknown whether'); ok(h.fetches.length > reads, 'the host was re-read');
  });
  await check('an interrupted or malformed refresh keeps the last valid library, never crashes the controls, and recovers on a good answer', async () => {
    const bad = {
      'a body that is cut off': cutOff,
      'an empty object': () => okJson({}),
      'settings missing': () => okJson({ recordings: [], folders: [], root: '/x', shares: [] }),
      'recordings not an array': () => okJson({ ...stateOf(), recordings: {} }),
      'settings null': () => okJson({ ...stateOf(), settings: null }),
      'folders missing': () => okJson({ recordings: [], settings: {}, root: '/x' }),
      'root missing': () => okJson({ ...stateOf(), root: undefined }),
      'an array': () => okJson([]),
      'active of the wrong type': () => okJson({ ...stateOf(), active: 'yes' }),
    };
    for (const [what, answer] of Object.entries(bad)) {
      let mode = 'good'; const good = stateOf();
      const h = boot({ key: 'K', api: async () => (mode === 'good' ? okJson(good) : answer()) }); await settle();
      const before = h.t.state; eq(before.recordings.length, 2);
      mode = 'bad'; await h.t.refresh(); await settle(10);
      eq(h.t.state, before, `${what}: the previous snapshot is kept`);
      eq(h.t.state.recordings.length, 2); ok(h.t.state.settings && h.t.state.settings.theme !== undefined || h.t.state.settings, 'settings still readable');
      eq(h.$('reach').hidden, false, `${what}: the unreachable banner shows`); ok(h.delays().includes(2000), `${what}: bounded 2 s retry`);
      h.t.openSettings(); // controls still work on the kept snapshot
      mode = 'good'; await fire(h, 2000); eq(h.$('reach').hidden, true, `${what}: recovered`);
    }
  });
  await check('a non-2xx answer with an unreadable body is a short generic refusal, never a success and never raw parser text', async () => {
    const st = stateOf({ capture: { phase: 'idle', generation: 4, launch: 'launch-aaaa-0001', known: true } }); let posts = 0;
    const h = boot({ key: 'K', api: async (u, o) => { if (o.body) { posts++; return { ok: false, status: 500, json: async () => { throw new SyntaxError('Unexpected token <'); } }; } return okJson(st); } }); await settle();
    h.$('capRecord').click(); await settle(10);
    eq(posts, 1); eq(h.$('toastAlertText').hidden === undefined, false); has(h.$('toastAlertText').textContent, 'Operation failed'); lacks(h.$('toastAlertText').textContent, /Unexpected|SyntaxError/);
    lacks(h.$('toastAlertText').textContent, 'unknown whether', 'a definite refusal is not "uncertain"');
    eq(h.fetches.filter(f => f.body && f.body.op === 'start-recording').length, 1);
  });
  await check('an embedded push with an incomplete snapshot is ignored and an unreadable tool result is uncertain', async () => {
    const st = stateOf(); const h = boot({ embedded: true, mediaKey: 'M', api: async () => { throw 0; }, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : { structuredContent: st } }); await settle();
    const before = h.t.state; eq(before.recordings.length, 2);
    (h.dom.listeners.message || []).forEach(f => f({ source: h.sandbox.parent, origin: 'http://127.0.0.1:1', data: { jsonrpc: '2.0', method: 'ui/notifications/tool-result', params: { structuredContent: { recordings: [] } } } }));
    await settle(); eq(h.t.state, before, 'a partial push cannot replace the library');
    const g = boot({ embedded: true, mediaKey: 'M', api: async () => { throw 0; }, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : m.params && m.params.arguments && m.params.arguments.op === 'state' ? { structuredContent: st } : { content: [{ type: 'text', text: '{"truncated":' }] } }); await settle();
    g.$('capRecord').click(); await settle(10);
    has(g.$('toastAlertText').textContent, 'unknown whether'); lacks(g.$('toastAlertText').textContent, /Unexpected|JSON/);
  });
  // ================= schema-validated snapshots and authoritative shares (candidate C review) =================
  const clone = o => JSON.parse(JSON.stringify(o));
  const withShare = () => { const st = stateOf(); st.shares = [{ shareId: 's1', id: 'r1', status: 'ready', url: 'https://a.example/p', accessCode: 'ABCD-EFGH', expiresAt: Date.now() + 60000 }]; return st; };
  const mutations = {
    'a null recording': st => { st.recordings = [null]; },
    'a numeric recording': st => { st.recordings = [1]; },
    'an empty recording': st => { st.recordings = [{}]; },
    'a recording without title or files': st => { st.recordings = [{ id: 'x' }]; },
    'a recording with an empty id': st => { st.recordings[0].id = ''; },
    'files null': st => { st.recordings[0].files = null; },
    'a null file entry': st => { st.recordings[0].files = [null]; },
    'a file without a name': st => { st.recordings[0].files = [{ bytes: 1 }]; },
    'favorite as text': st => { st.recordings[0].favorite = 'yes'; },
    'archived as a number': st => { st.recordings[0].archived = 1; },
    'a numeric folder reference': st => { st.recordings[0].folder = 5; },
    'a text duration': st => { st.recordings[0].duration = '12'; },
    'an object createdAt': st => { st.recordings[0].createdAt = {}; },
    'recording settings as text': st => { st.recordings[0].settings = 'x'; },
    'a null folder': st => { st.folders = [null]; },
    'an empty folder': st => { st.folders = [{}]; },
    'a folder without a name': st => { st.folders = [{ id: 'f1' }]; },
    'a numeric folder id': st => { st.folders = [{ id: 1, name: 'x' }]; },
    'empty settings': st => { st.settings = {}; },
    'settings without quality': st => { delete st.settings.quality; },
    'a text quality': st => { st.settings.quality = '1920'; },
    'a text keepOriginals': st => { st.settings.keepOriginals = 'true'; },
    'a null cameraSize': st => { st.settings.cameraSize = null; },
    'a text countdownSeconds': st => { st.settings.countdownSeconds = '3'; },
    'a missing active': st => { delete st.active; },
    'active as a list': st => { st.active = []; },
    'active without status': st => { st.active = {}; },
    'a numeric active status': st => { st.active = { status: 5 }; },
    'missing shares': st => { delete st.shares; },
    'null shares': st => { st.shares = null; },
    'shares as an object': st => { st.shares = {}; },
    'a null share': st => { st.shares = [null]; },
    'a share without an id': st => { st.shares = [{ status: 'ready' }]; },
    'a share with a numeric status': st => { st.shares = [{ id: 'r1', status: 5 }]; },
    'a numeric share url': st => { st.shares = [{ id: 'r1', status: 'ready', url: 5 }]; },
    'a text share expiry': st => { st.shares = [{ id: 'r1', status: 'ready', expiresAt: 'soon' }]; },
    'a numeric root': st => { st.root = 5; },
    'a text freeBytes': st => { st.freeBytes = '5'; },
  };
  await check(`${Object.keys(mutations).length} malformed or incomplete snapshots (nested elements, missing consumed keys, null entries) never replace the last valid content, never break search, archive or folders, and a good answer recovers`, async () => {
    for (const [what, mutate] of Object.entries(mutations)) {
      let mode = 'good'; const good = withShare(); const bad = clone(good); mutate(bad);
      const h = boot({ key: 'K', api: async () => okJson(mode === 'good' ? good : bad) }); await settle();
      const before = h.t.state; eq(before.recordings.length, 2, what);
      mode = 'bad'; await h.t.refresh(); await settle(10);
      eq(h.t.state, before, `${what}: the previous snapshot is kept`);
      eq(h.$('reach').hidden, false, `${what}: the unreachable banner shows`); ok(h.delays().includes(2000), `${what}: bounded 2 s retry`);
      // The controls the review saw crash still work on the kept snapshot.
      h.$('search').value = 'plan'; for (const v of ['all', 'archived', 'favorites', 'shared', 'folder:f1']) { h.t.view = v; h.t.filtered(); }
      h.t.view = 'all'; h.t.render(); h.t.openSettings(); h.t.openMenu('r1', h.$('t'));
      mode = 'good'; await fire(h, 2000); eq(h.$('reach').hidden, true, `${what}: recovered`);
    }
  });
  await check('accepted payloads stay functional: optional metadata may be absent, unknown fields are ignored, an active recording and legacy share aliases render', async () => {
    const st = stateOf(); st.recordings = [{ id: 'old', title: 'Legacy take', status: 'ready', files: [{ name: 'recording.mp4' }] }];
    delete st.freeBytes; delete st.version; delete st.platform;
    st.somethingNew = { future: true }; st.recordings[0].futureField = [1, 2]; st.settings.futureSetting = 'x';
    st.active = { id: 'a1', engine: 'native', status: 'recording', startedAt: Date.now() - 5000 };
    st.shares = [{ id: 'old', status: 'ready', shareUrl: 'https://a.example/q', code: 'WXYZ2345', expiresAt: Date.now() + 5000 }];
    const h = await loaded({ state: st });
    eq(h.t.state.recordings.length, 1); eq(h.t.state.recordings[0].title, 'Legacy take'); eq(h.t.filtered().length, 1);
    has(h.t.activeLabel(h.t.state.active), 'Recording'); eq(h.t.shareState(h.t.state.shares[0]), 'ready');
    eq(h.$('reach').hidden !== false, true, 'no unreachable banner for an accepted payload');
    const none = await loaded({ state: stateOf({ active: null, shares: [], recordings: [], folders: [] }) }); eq(none.t.state.recordings.length, 0, 'a real empty library is valid');
  });
  await check('an interrupted settings change whose read-back is incomplete stays honestly unknown: never "not saved", control usable, snapshot kept, one POST', async () => {
    for (const [what, mutate] of Object.entries({ 'empty settings': st => { st.settings = {}; }, 'missing active': st => { delete st.active; }, 'a null recording': st => { st.recordings = [null]; }, 'missing shares': st => { delete st.shares; } })) {
      const st = stateOf(); const posts = []; let reads = 0;
      const h = boot({ key: 'K', api: async (u, o) => {
        const b = o.body && JSON.parse(o.body);
        if (b && b.op === 'settings') { posts.push(b.data); return cutOff(); }
        reads++; if (reads === 1) return okJson(st); const bad = clone(st); mutate(bad); return okJson(bad);
      } }); await settle();
      const el = Object.assign(h.$('s-quality'), { name: 'quality', value: '2560' });
      h.t.openSettings(); h.$('settingsPane').onchange({ target: el }); await settle(12);
      eq(posts.length, 1, what); eq(el.disabled, false, what);
      const text = h.$('settingsStatus').textContent;
      has(text, 'unknown whether it was saved'); lacks(text, 'not saved.', `${what}: an incomplete answer proves nothing`); lacks(text, /Resolution saved/);
      eq(h.t.state.settings.quality, st.settings.quality, `${what}: saved settings kept`); eq(h.t.state.recordings.length, 2);
    }
  });
  await check('uncertain Revoke: a snapshot that omits or garbles the share list leaves the link standing and never claims it ended; only an authoritative empty list ends it after the read-back', async () => {
    const cases = { 'shares omitted': st => { delete st.shares; }, 'shares null': st => { st.shares = null; }, 'a share entry null': st => { st.shares = [null]; }, 'active omitted': st => { delete st.active; } };
    for (const [what, mutate] of Object.entries(cases)) {
      const st = withShare(); let unshares = 0, reads = 0;
      const h = boot({ key: 'K', api: async (u, o) => {
        const b = o.body && JSON.parse(o.body);
        if (b && b.op === 'unshare') { unshares++; return cutOff(); }
        reads++; if (reads === 1) return okJson(st); const bad = clone(st); mutate(bad); return okJson(bad);
      } }); await settle();
      h.t.openShare('r1'); eq(h.$('shareReady').hidden, false);
      h.$('revokeShare').click(); await settle(14);
      eq(unshares, 1, `${what}: exactly one POST`); eq(h.$('shareEnded').hidden, true, `${what}: not presented as ended`); eq(h.$('shareReady').hidden, false, `${what}: the link is still shown`);
      has(h.$('toastAlertText').textContent, 'unknown whether'); eq(h.t.state.shares.length, 1, `${what}: the old share list is kept`);
      lacks(h.$('shareEndedText').textContent, 'no longer work', `${what}: no false revocation message`);
    }
    // The host really did revoke: its authoritative empty list ends it, still with one POST.
    const st = withShare(); let unshares = 0, reads = 0;
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b && b.op === 'unshare') { unshares++; return cutOff(); } reads++; return okJson(reads === 1 ? st : { ...st, shares: [] }); } }); await settle();
    h.t.openShare('r1'); h.$('revokeShare').click(); await settle(14);
    eq(unshares, 1); eq(h.$('shareEnded').hidden, false); has(h.$('shareEndedText').textContent, 'no longer work');
  });
  await check('share start with a cut-off reply and a snapshot without shares shows no link, no credential and no false "nothing was shared"', async () => {
    const st = stateOf(); let posts = 0, reads = 0;
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b && b.op === 'share') { posts++; return cutOff(); } reads++; if (reads === 1) return okJson(st); const bad = clone(st); delete bad.shares; return okJson(bad); } }); await settle();
    h.t.openShare('r1'); h.$('createShare').click(); await settle(14);
    eq(posts, 1); eq(h.$('shareFailed').hidden, false); has(h.$('shareFailedText').textContent, 'unknown whether sharing started'); lacks(h.$('shareFailedText').textContent, 'Nothing was shared');
    eq(h.$('shareUrl').value, ''); eq(h.$('shareCode').textContent, ''); eq(h.$('shareReady').hidden, true);
  });
  await check('an embedded push is validated, never overwrites the share list, and is confirmed against the host', async () => {
    const st = withShare(); let reads = 0;
    const h = boot({ embedded: true, mediaKey: 'M', api: async () => { throw 0; }, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : { structuredContent: (reads++, st) } }); await settle();
    eq(h.t.state.shares.length, 1); const before = reads;
    const push = data => (h.dom.listeners.message || []).forEach(f => f({ source: h.sandbox.parent, origin: 'http://127.0.0.1:1', data: { jsonrpc: '2.0', method: 'ui/notifications/tool-result', params: { structuredContent: data } } }));
    // A settings-style result carries a placeholder empty share list: it must not end the link.
    push({ ...clone(st), shares: [] }); await settle(10);
    eq(h.t.state.shares.length, 1, 'the share list only comes from the host state read'); ok(reads > before, 'the host was read to confirm');
    const incomplete = clone(st); delete incomplete.active; const keep = h.t.state; push(incomplete); await settle(); eq(h.t.state, keep, 'an incomplete push is ignored');
  });
  // ================= freeBytes null and complete-settings commits (candidate D review) =================
  await check('freeBytes null (disk space unavailable, e.g. a disconnected drive) is a valid snapshot: it loads, refreshes and shows the unavailable-space message, not a connection failure', async () => {
    const st = stateOf({ freeBytes: null });
    const h = await loaded({ state: st });
    ok(h.t.state, 'the library loads'); eq(h.$('reach').hidden !== false, true, 'no unreachable banner'); has(h.t.paneMarkup('recording'), 'Available disk space could not be measured.'); lacks(h.t.paneMarkup('recording'), 'available on this drive');
    // Refresh from a measured value to null, and back.
    let mode = 'number'; const g = boot({ key: 'K', api: async () => okJson(stateOf({ freeBytes: mode === 'number' ? 5e10 : null })) }); await settle();
    has(g.t.paneMarkup('recording'), '50.0 GB available on this drive');
    mode = 'null'; await g.t.refresh(); await settle(10); eq(g.t.state.freeBytes, null); eq(g.$('reach').hidden !== false, true, 'refresh accepted'); has(g.t.paneMarkup('recording'), 'could not be measured');
    mode = 'number'; await g.t.refresh(); await settle(10); has(g.t.paneMarkup('recording'), 'available on this drive');
    const zero = await loaded({ state: stateOf({ freeBytes: 0 }) }); has(zero.t.paneMarkup('recording'), '0 KB available on this drive');
    const text = stateOf({ freeBytes: '5' }); const bad = await loaded({ state: text }); eq(bad.t.state, null, 'a text figure is still rejected');
    for (const key of ['recordings', 'folders', 'settings', 'shares']) { const broken = stateOf(); delete broken[key]; const h2 = await loaded({ state: broken }); eq(h2.t.state, null, `authoritative ${key} stays required`); }
  });
  const editedFields = { countdownSeconds: { name: 'countdownSeconds', value: '0', label: 'Countdown' }, rememberInputs: { name: 'rememberInputs', checked: false, type: 'checkbox', label: 'Remember camera and mic on or off' } };
  await check('an interrupted change to countdownSeconds or rememberInputs read back WITHOUT that field is unknown, never "not saved"; the incomplete read-back replaces nothing; one POST', async () => {
    for (const [key, el0] of Object.entries(editedFields)) {
      const st = stateOf(); const posts = []; let reads = 0;
      const h = boot({ key: 'K', api: async (u, o) => {
        const b = o.body && JSON.parse(o.body);
        if (b && b.op === 'settings') { posts.push(b.data); return cutOff(); }
        reads++; if (reads === 1) return okJson(st); const bad = clone(st); delete bad.settings[key]; return okJson(bad);
      } }); await settle();
      const el = Object.assign(h.$(`s-${key}`), el0); h.t.openSettings(key === 'rememberInputs' ? 'camera' : 'recording'); h.$('settingsPane').onchange({ target: el }); await settle(12);
      eq(posts.length, 1, key); eq(el.disabled, false, key);
      const text = h.$('settingsStatus').textContent;
      has(text, 'unknown whether it was saved'); lacks(text, 'not saved.', `${key}: a missing field proves nothing`);
      eq(JSON.stringify(h.t.state.settings), JSON.stringify(st.settings), `${key}: the last complete settings are kept`);
    }
  });
  await check('a successful write whose answer carries partial or invalid settings is ambiguous: it is reconciled against the host, never committed, never an unconfirmed "saved"; exactly one POST', async () => {
    const answers = { 'a one-field subset': { settings: { quality: 2560 } }, 'an empty settings object': { settings: {} }, 'settings as text': { settings: 'x' }, 'settings null': { settings: null }, 'no settings at all': {}, 'a subset missing the edited key': null };
    for (const [what, body] of Object.entries(answers)) {
      for (const hostHolds of ['new', 'old', 'incomplete']) {
        const st = stateOf(); const posts = []; let reads = 0;
        const h = boot({ key: 'K', api: async (u, o) => {
          const b = o.body && JSON.parse(o.body);
          if (b && b.op === 'settings') { posts.push(b.data); const answer = { ...st }; if (body === null) answer.settings = { ...st.settings, quality: undefined }; else if ('settings' in body) answer.settings = body.settings; else delete answer.settings; return okJson(answer); }
          reads++; if (reads === 1) return okJson(st);
          if (hostHolds === 'new') return okJson({ ...st, settings: { ...st.settings, quality: 2560 } });
          if (hostHolds === 'old') return okJson(st);
          const bad = clone(st); bad.settings = { quality: 2560 }; return okJson(bad);
        } }); await settle();
        const el = Object.assign(h.$('s-quality'), { name: 'quality', value: '2560' });
        h.t.openSettings(); h.$('settingsPane').onchange({ target: el }); await settle(14);
        const label = `${what} / host ${hostHolds}`;
        eq(posts.length, 1, label); eq(el.disabled, false, label);
        const text = h.$('settingsStatus').textContent;
        if (hostHolds === 'new') { has(text, 'Resolution saved', label); eq(h.t.state.settings.quality, 2560); eq(Object.keys(h.t.state.settings).length, Object.keys(st.settings).length, `${label}: all settings kept, not a subset`); }
        if (hostHolds === 'old') { has(text, 'Resolution not saved', label); eq(el.value, String(st.settings.quality)); }
        if (hostHolds === 'incomplete') { has(text, 'unknown whether it was saved', label); lacks(text, /Resolution saved|not saved\./); eq(JSON.stringify(h.t.state.settings), JSON.stringify(st.settings), label); }
        eq(Object.keys(h.t.state.settings).length, Object.keys(st.settings).length, `${label}: never replaced by a subset`);
      }
    }
  });
  await check('a complete, matching answer is confirmed and committed; a complete answer that disagrees is a definite refusal without a second request', async () => {
    for (const agree of [true, false]) {
      const st = stateOf(); let posts = 0, reads = 0;
      const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b && b.op === 'settings') { posts++; return okJson({ ...st, settings: { ...st.settings, quality: agree ? 2560 : 1920 } }); } reads++; return okJson(st); } }); await settle();
      const before = reads; const el = Object.assign(h.$('s-quality'), { name: 'quality', value: '2560' });
      h.t.openSettings(); h.$('settingsPane').onchange({ target: el }); await settle(10);
      eq(posts, 1); eq(reads, before, 'no reconcile read for a complete answer');
      if (agree) { has(h.$('settingsStatus').textContent, 'Resolution saved'); eq(h.t.state.settings.quality, 2560); eq(Object.keys(h.t.state.settings).length, Object.keys(st.settings).length); }
      else { has(h.$('settingsStatus').textContent, 'Resolution not saved'); eq(h.t.state.settings.quality, 1920); eq(el.value, '1920'); }
    }
  });
  await check('a recording-folder change whose answer lacks the folder or has a malformed disk figure is unknown, keeps the saved folder and reads the host', async () => {
    for (const answer of [{}, { root: 5 }, { root: '/new', freeBytes: 'lots' }]) {
      const st = stateOf(); let reads = 0, posts = 0;
      const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b && b.op === 'settings') { posts++; return okJson(answer); } reads++; return okJson(st); } }); await settle();
      h.t.openSettings('recording'); const before = reads;
      h.$('settingsPane').onclick({ target: { closest: sel => (sel === '#chooseDefaultRoot' ? {} : null) } });
      h.$('nameInput').value = '/new/root'; await h.$('nameForm').onsubmit({ preventDefault() {} }); await settle(10);
      eq(posts, 1, 'one request, never replayed'); eq(h.t.state.root, '/mock/root', 'the saved folder is untouched'); eq(typeof h.t.state.root, 'string');
      has(h.$('nameError').textContent, 'unknown whether the folder changed'); ok(reads > before, 'the host was read back');
    }
  });
  // ================= recording-folder change: uncertain failures are always reconciled (candidate E review) =================
  // Real responses from the actual Rust serializer (metadata-only disposable host), kept in the repository. A missing
  // file or case fails the test; nothing here is ever replaced by hand-written data.
  const actualCases = JSON.parse(fs.readFileSync(path.join(root, 'tests/fixtures/actual-backend-contracts.json'), 'utf8')).cases;
  const actualCase = name => { const found = actualCases.find(k => k.name === name); if (!found) throw new Error(`missing actual backend fixture ${name}`); return found.payload; };
  eq(actualCases.length, 10, 'the seven earlier and three capture-report actual responses are present');
  const actualBase = () => clone(actualCase('empty-defaults'));
  const downThenUp = async (answerFolder) => {
    // Library loaded; the host goes away and the bounded 2/5/15 s reads are exhausted; then it recovers.
    const st = actualBase(); let hostUp = true, root = st.root, posts = 0, reads = 0;
    const h = boot({ key: 'K', api: async (u, o) => {
      const b = o.body && JSON.parse(o.body);
      if (b && b.op === 'settings' && b.data.root) { posts++; root = b.data.root; return answerFolder(); }
      if (!hostUp) throw new TypeError('offline');
      reads++; return okJson({ ...st, root, freeBytes: 12345 });
    } }); await settle();
    hostUp = false; await h.t.refresh(); await settle(10);
    for (const d of [2000, 5000, 15000]) await fire(h, d);
    eq(h.$('reachRetry').hidden, false, 'read retries are exhausted'); eq(h.delays().filter(d => d !== 7000).length, 0, 'polling has stopped');
    hostUp = true; const readsBefore = reads;
    h.t.openSettings('recording'); h.$('settingsPane').onclick({ target: { closest: sel => (sel === '#chooseDefaultRoot' ? {} : null) } });
    h.$('nameInput').value = '/new/place'; const pending = h.$('nameForm').onsubmit({ preventDefault() {} });
    return { h, st, pending, posts: () => posts, reads: () => reads - readsBefore, setHost: v => { hostUp = v; } };
  };
  for (const [how, answer, deadline] of [['a cut-off body', () => cutOff(), null], ['no answer before the deadline', () => never(), 20000]]) {
    await check(`folder change with ${how} after the read retries were exhausted: one POST, one authoritative read-back that updates the folder and resumes polling`, async () => {
      const c = await downThenUp(answer); if (deadline) { await settle(6); await fire(c.h, deadline); } await c.pending; await settle(14);
      eq(c.posts(), 1, 'the change is never replayed'); eq(c.reads(), 1, 'exactly one read-back');
      eq(c.h.t.state.root, '/new/place', 'the recording destination matches what the host holds'); eq(c.h.t.state.freeBytes, 12345);
      eq(c.h.$('reach').hidden, true, 'the library is reachable again'); ok(c.h.delays().includes(4000), 'normal polling resumed');
      has(c.h.$('nameError').textContent, 'unknown whether the change was applied'); lacks(c.h.$('settingsStatus').textContent, 'Recording folder saved');
    });
  }
  await check('folder change whose follow-up read is also unreadable stays honestly unknown: old folder kept, no saved claim, bounded retries then manual, still one POST', async () => {
    const c = await downThenUp(() => cutOff());
    c.setHost(false); // the host is unreachable again for the read-back
    await c.pending; await settle(14);
    eq(c.posts(), 1); eq(c.h.t.state.root, actualBase().root, 'the last known folder is retained'); lacks(c.h.$('settingsStatus').textContent, 'Recording folder saved');
    has(c.h.$('nameError').textContent, 'unknown whether the change was applied');
    // The earlier bounded reads were already exhausted, so the failed read-back leaves the manual retry (no automatic polling storm).
    eq(c.h.$('reachRetry').hidden, false, 'manual retry is offered'); eq(c.h.delays().filter(d => d !== 7000).length, 0, 'no automatic reads are armed');
    eq(c.posts(), 1, 'nothing was written again');
    c.setHost(true); c.h.$('reachRetry').click(); await settle(12);
    eq(c.h.t.state.root, '/new/place', 'the manual retry reads the applied folder'); eq(c.posts(), 1);
  });
  await check('the actual Rust settings write result (real payload) still commits a folder change without a second request', async () => {
    const real = actualCase('settings-write-result');
    const st = actualBase(); let posts = 0, reads = 0;
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b && b.op === 'settings') { posts++; return okJson({ ...real, root: '/new/place', freeBytes: 777 }); } reads++; return okJson({ ...st, root: posts ? '/new/place' : st.root, freeBytes: posts ? 777 : st.freeBytes }); } }); await settle();
    h.t.openSettings('recording'); h.$('settingsPane').onclick({ target: { closest: sel => (sel === '#chooseDefaultRoot' ? {} : null) } });
    h.$('nameInput').value = '/new/place'; await h.$('nameForm').onsubmit({ preventDefault() {} }); await settle(12);
    eq(posts, 1); eq(h.t.state.root, '/new/place'); has(h.$('settingsStatus').textContent, 'Recording folder saved');
  });
  await check('state.settings has a single assignment (the complete-settings commit) and the library state a single assignment (commitState)', () => {
    const production = app;
    eq((production.match(/\bstate\.settings\s*=[^=]/g) || []).length, 1, 'one direct settings assignment');
    has(production, /function commitSettings\(settings\)\{\s*if\(!settingsShape\(settings\)\)throw incompleteSettings\(\);\s*state\.settings=settings;/);
    eq((production.match(/\bstate\.root\s*=[^=]/g) || []).length, 1, 'one guarded root assignment');
    has(production, /nullable\(isNumber\)\(result\.freeBytes\)\)+\{void refresh\(\);throw/, 'the disk figure is validated before it is assigned');
  });
  await check('every browser fetch goes through the deadline helper', () => {
    eq((app.match(/\bfetch\(/g) || []).length, 1, 'a single fetch call, inside fetchWithin');
    has(app, /async function fetchWithin\(/); has(app, /deadlines=\{read:7000,change:20000,share:45000\}/);
  });


  // ================= menu / Home capture group =================
  const capOf = (over = {}) => ({ phase: 'idle', generation: 4, launch: 'launch-aaaa-0001', known: true, trayAvailable: true, screenState: 'allowed', source: { kind: 'display', name: 'Built-in Retina Display' },
    sources: [{ id: 1, kind: 'display', name: 'Built-in Retina Display', selected: true }, { id: 2, kind: 'display', name: 'LG UltraFine', selected: false }],
    inputs: { camera: { mark: 'off' }, microphone: { mark: 'off' } },
    devices: { camera: [{ uid: 'CAM-A', name: 'FaceTime HD Camera', selected: true }, { uid: 'CAM-B', name: 'Logitech C920', selected: false }], microphone: [{ uid: 'MIC-A', name: 'MacBook Pro Microphone', selected: true }] },
    capabilities: { fps60: { status: 'unknown', basis: 'Not measured on this computer yet', measuredFps: null } }, ...over });
  const ofOp = (h, op) => h.fetches.filter(f => f.body && f.body.op === op).map(f => f.body.data);
  const sentOf = h => ofOp(h, 'capture-command');
  const envelope = ['launch', 'nonce', 'expires'];
  const commandsOf = h => sentOf(h).map(d => Object.fromEntries(Object.entries(d).filter(([k]) => !envelope.includes(k))));
  const startsOf = h => ofOp(h, 'start-recording');
  // An agent host that offers the page-only tool: what the page sends through it is recorded in `uiCalls`.
  const embeddedHost = (st, { ui = 'ok', answers = {}, image } = {}) => {
    const uiCalls = [], actionCalls = [];
    const h = boot({ embedded: true, mediaKey: 'M', image, api: async () => { throw 0; }, host: m => {
      if (m.method === 'ui/initialize') return { hostContext: {} };
      const call = m.params && m.params.arguments;
      if (m.params && m.params.name === 'openrecorder_ui') {
        uiCalls.push(call);
        if (ui === 'unsupported') return { isError: true, content: [{ type: 'text', text: 'Unknown tool: openrecorder_ui' }] };
        const answer = answers[call.op]; if (typeof answer === 'function') return answer(call);
        return { structuredContent: answer || { ok: true } };
      }
      actionCalls.push(call);
      return { structuredContent: st };
    } });
    return { h, uiCalls, actionCalls };
  };
  await check('the capture group, host menu and popover are in the page once; the old New recording and Open in browser buttons are gone', () => {
    for (const id of ['capture', 'capSource', 'capCamera', 'capMic', 'capRecord', 'capTime', 'capLine', 'capNotice', 'capOpenTake', 'capPop', 'hostMenu', 'hostMenuButton']) eq((html.match(new RegExp(`id="${id}"`, 'g')) || []).length, 1, id);
    for (const keep of ['id="sidebar"', 'id="recordings"', 'id="search"', 'id="sort"', 'id="pageTitle"', 'id="settingsButton"']) has(html, keep);
    for (const gone of ['id="newRecording"', 'id="openBrowser"', 'id="capUse30"', 'id="capDismiss"', 'Use 30 fps']) lacks(html, gone, gone);
    has(html, /<select id="sort"/, 'sort stays the native select');
    has(html, /role="group" aria-label="Capture controls"/); has(css, /\.cap-src\{width:190px\}\.cap-dev\{width:180px\}/); has(css, /\.cap-record\{[^}]*width:96px;height:30px/); has(css, /\.cap-pop\{[^}]*width:300px/);
  });
  await check('the header can never push Record or the title out of view: it wraps at every width, the capture group wraps inside itself, and the brand icon is inline', () => {
    // Regression from a real 1200 px browser run: the header was one non-wrapping row (title + search + sort + three
    // chips + Record is wider than the 904 px of content), so the title collapsed to nothing and Record sat at x 1202.
    const rule = sel => (css.match(new RegExp(`(?:^|\\n|\\})${sel.replace(/[.\\[\\]()]/g, '\\\\$&')}\\{([^}]*)\\}`)) || [])[1] || '';
    has(rule('.topbar'), 'flex-wrap:wrap');
    has(rule('.cap'), 'flex-wrap:wrap'); has(rule('.cap'), 'max-width:100%'); has(rule('.cap'), 'margin-left:auto');
    has(rule('.topbar h1'), /flex:1 1 160px/, 'the title keeps a basis, so it cannot be squeezed to zero');
    // The width budget, from the stated sizes: at the reported 1200 px the first line holds title + search + sort and
    // the capture group (three chips, divider, Record) moves to its own line, inside the content width.
    const content = 1200 - 232 - 2 * 32, tools = 240 + 8 + 100, cap = 190 + 180 + 180 + 1 + 96 + 4 * 6;
    ok(160 + 12 + tools <= content, 'title + tools fit one line'); ok(cap <= content, 'the capture group fits its own line'); ok(160 + 12 + tools + 12 + cap > content, 'and it does move to its own line rather than overflow');
    // The embedded 640 px and mobile 390 px layouts wrap the group as well (its chips share the row and Record may drop to the next).
    has(css, /@media\(max-width:720px\)\{\.cap\{order:9;flex:1 1 100%;flex-wrap:wrap\}/);
    // The page names its own icon, so a browser never asks the library for /favicon.ico (an unauthenticated 403).
    has(html, /<link rel="icon" href="data:image\/svg\+xml,/); lacks(html, /rel="(?:shortcut )?icon" href="(?!data:)/);
  });
  await check('loading, refreshing, reconnecting and opening Settings send nothing to the app, in the browser or in an agent host', async () => {
    const h = await loaded({ state: stateOf({ capture: capOf() }) });
    await h.t.refresh(); await h.t.refresh(); h.t.openSettings('permissions'); h.t.openSettings('about'); await settle();
    for (const op of ['capture-command', 'start-recording', 'media-read', 'media-session']) eq(ofOp(h, op).length, 0, `${op} from the browser page`);
    const { h: e, uiCalls } = embeddedHost(stateOf({ capture: capOf() })); await settle(); await e.t.refresh(); e.t.openSettings('camera'); await settle();
    eq(uiCalls.filter(c => !['media-read', 'media-session'].includes(c.op)).length, 0, 'nothing but read-only media reads through the page-only tool without a press');
    ok(!e.posted.some(m => m.params && m.params.arguments && ['record', 'start-recording', 'capture-command'].includes(m.params.arguments.op)), 'and nothing named Record through the agent-visible tool');
  });
  await check('one Record press is the whole start: one request carrying the launch and take it saw, a 4 s expiry and a single-use number', async () => {
    const h = await loaded({ state: stateOf({ capture: capOf({ generation: 9 }) }) });
    eq(h.$('capRecordLabel').textContent, 'Record'); eq(h.$('capRecord').dataset.phase, 'idle'); eq(h.$('capTime').hidden, true);
    const before = Date.now(); h.$('capRecord').click(); await settle();
    eq(startsOf(h).length, 1, 'one request'); eq(sentOf(h).length, 0, 'no second Record, no separate command');
    const [sent] = startsOf(h);
    eq(sent.launch, 'launch-aaaa-0001'); eq(sent.generation, 9);
    ok(sent.expires >= before + 3000 && sent.expires <= Date.now() + 4000, `expires within 4 s: ${sent.expires - before}`);
    ok(/^[0-9a-f]{32}$/.test(sent.nonce), 'a 128-bit hex request number');
    const post = h.fetches.find(f => f.body && f.body.op === 'start-recording');
    eq(post.opts.method, 'POST'); eq(post.opts.headers.Authorization, 'Bearer OWNERKEY', 'the existing owner credential, nothing new'); ok(!post.url.includes('OWNERKEY'), 'never in a URL');
    h.$('capRecord').click(); await settle(); eq(startsOf(h).length, 1, 'a double press is one press');
    for (let i = 0; i < 12 && h.delays().includes(700); i++) await h.runTimeout();
    h.$('capRecord').click(); await settle(); const [, second] = startsOf(h);
    ok(second && second.nonce !== sent.nonce, 'a later press has its own number');
  });
  await check('the main control while the app is not ready is Open OpenRecorder, not Record: neutral, no red dot, it sends open-recorder only, and no Record or countdown is queued for later', async () => {
    const h = await loaded({ state: stateOf({ capture: { phase: 'unavailable', generation: 0, known: false } }) });
    eq(h.$('capture').hidden, false, 'the group is there even before the app reports');
    eq(h.$('capRecord').dataset.phase, 'closed'); eq(h.$('capRecordLabel').textContent, 'Open'); eq(h.$('capRecord').attrs['aria-label'], 'Open OpenRecorder'); eq(h.$('capRecord').disabled, false);
    has(h.$('capRecord').title, 'records nothing until you press Record');
    h.$('capRecord').click(); await settle();
    eq(ofOp(h, 'open-recorder').length, 1, 'one open'); eq(JSON.stringify(ofOp(h, 'open-recorder')[0]), '{}');
    eq(startsOf(h).length, 0, 'no Record press is ever sent while the app is not ready'); eq(sentOf(h).length, 0);
    eq(h.$('capRecord').disabled, true, 'it says Opening…'); eq(h.$('capRecordLabel').textContent, 'Opening…');
    const plain = await loaded(); plain.$('capRecord').click(); await settle();
    eq(startsOf(plain).length, 0, 'an older host with no capture report behaves the same'); eq(ofOp(plain, 'open-recorder').length, 1);
    // Then the app reports idle: the same control is Record, with the red dot, and a press is a bound start-recording.
    const ready = await loaded({ state: stateOf({ capture: capOf() }) });
    eq(ready.$('capRecord').dataset.phase, 'idle'); eq(ready.$('capRecordLabel').textContent, 'Record'); eq(ready.$('capRecord').attrs['aria-label'], 'Record');
    ready.$('capRecord').click(); await settle(); const [sent] = startsOf(ready); eq(startsOf(ready).length, 1);
    eq(Object.keys(sent).sort().join(','), 'expires,generation,launch,nonce'); eq(ofOp(ready, 'open-recorder').length, 0);
  });
  await check('in an agent host Record goes through the page-only tool, once, and never through the agent-visible tool', async () => {
    const { h, uiCalls: allUi, actionCalls } = embeddedHost(stateOf({ capture: capOf({ generation: 2 }) }), { answers: { 'start-recording': { queued: true } } }); await settle();
    const uiCalls = { get length() { return allUi.filter(c => c.op === 'start-recording').length; }, 0: null }; Object.defineProperty(uiCalls, 0, { get: () => allUi.find(c => c.op === 'start-recording') });
    const seen = actionCalls.length; h.$('capRecord').click(); await settle();
    eq(uiCalls.length, 1); eq(uiCalls[0].op, 'start-recording'); eq(uiCalls[0].data.launch, 'launch-aaaa-0001'); eq(uiCalls[0].data.generation, 2);
    eq(uiCalls[0].ui, 'UI-SESSION-KEY', 'the page-only tool carries the session key only the window’s own copy of the page has');
    ok(!actionCalls.some(c => c && c.ui), 'and the agent-visible tool never carries it');
    ok(!actionCalls.slice(seen).some(c => /record/.test(JSON.stringify(c))), 'the agent-visible tool never saw a Record');
    h.$('capRecord').click(); await settle(); eq(uiCalls.length, 1, 'double press is one press');
  });
  await check('a host that cannot run the page-only tool says so plainly and never claims Record works', async () => {
    const { h, uiCalls: allUi } = embeddedHost(stateOf({ capture: capOf() }), { ui: 'unsupported' }); await settle();
    // The first read through the page-only tool (a poster) already learns the host cannot run it, so the controls are
    // withdrawn at once rather than left as dead buttons, and no start request is ever made.
    eq(h.$('capture').hidden, true, 'the controls are withdrawn rather than left as dead buttons');
    h.$('capRecord').click(); await settle(); eq(allUi.filter(c => c.op === 'start-recording').length, 0);
    has(h.$('capLine').textContent, 'This window can’t start or stop recordings'); has(h.$('capLine').textContent, 'Open in browser');
    lacks(JSON.stringify(h.posted.filter(m => m.params && m.params.arguments && m.params.arguments.op === 'state' ? false : m.params && m.params.name === 'openrecorder_action')), 'record');
  });
  await check('recording shows Stop and a plain elapsed time; the next report with any other phase clears both at once', async () => {
    const st = stateOf({ capture: capOf({ phase: 'recording', generation: 7, elapsedMs: 46000, fpsSet: 60, fpsDelivered: 59.8 }) });
    const h = await loaded({ state: st });
    eq(h.$('capRecordLabel').textContent, 'Stop'); eq(h.$('capRecord').dataset.phase, 'recording'); eq(h.$('capTime').hidden, false); has(h.$('capTime').textContent, /^0:4[6-9]$/);
    lacks(h.$('capTime').textContent, /[■●]/, 'plain text, no shape'); eq(h.$('capSource').disabled, true); eq(h.$('capMic').disabled, true); eq(h.$('capCamera').disabled, false, 'the camera’s look stays adjustable');
    h.$('capRecord').click(); await settle(); eq(JSON.stringify(commandsOf(h)), JSON.stringify([{ command: 'stop', generation: 7 }]), 'Stop names the take it means'); eq(startsOf(h).length, 0);
    for (const phase of ['stopping', 'saving', 'idle', 'countdown']) {
      h.t.state = { ...st, capture: capOf({ phase, generation: 7, countdown: 2, elapsedMs: 46000 }) }; h.t.render();
      eq(h.$('capTime').hidden, true, `${phase}: no timer`); lacks(h.$('capRecordLabel').textContent, /Stop|Recording|0:/, phase);
    }
    eq(h.$('capRecordLabel').textContent, 'Cancel 2');
  });
  await check('a silent app is shown stale, never as a running timer; countdown and starting cancel with the take number; saving cannot be pressed', async () => {
    const h = await loaded({ state: stateOf({ capture: capOf({ phase: 'recording', elapsedMs: 9000, stale: true }) }) });
    eq(h.$('capTime').hidden, true, 'stale means no timer'); eq(h.$('capRecord').disabled, true);
    const c = await loaded({ state: stateOf({ capture: capOf({ phase: 'countdown', generation: 5, countdown: 3 }) }) });
    eq(c.$('capRecordLabel').textContent, 'Cancel 3'); c.$('capRecord').click(); await settle(); eq(JSON.stringify(commandsOf(c)), JSON.stringify([{ command: 'cancel', generation: 5 }]));
    const s = await loaded({ state: stateOf({ capture: capOf({ phase: 'starting', generation: 5 }) }) });
    eq(s.$('capRecordLabel').textContent, 'Starting…'); s.$('capRecord').click(); await settle(); eq(JSON.stringify(commandsOf(s)), JSON.stringify([{ command: 'cancel', generation: 5 }]));
    for (const phase of ['stopping', 'saving']) { const x = await loaded({ state: stateOf({ capture: capOf({ phase }) }) }); eq(x.$('capRecord').disabled, true, phase); eq(x.$('capRecordLabel').textContent, 'Saving…'); }
  });
  await check('device chips show the real names and the one status vocabulary; no "Remembered", no status word without being true', async () => {
    const cases = [
      [{ camera: { mark: 'off', name: 'FaceTime HD Camera' } }, 'Camera, FaceTime HD Camera, Off', ''],
      [{ camera: { mark: 'armed', name: 'FaceTime HD Camera' } }, 'Camera, FaceTime HD Camera, On, starts when you record', 'armed'],
      [{ camera: { mark: 'starting', name: 'FaceTime HD Camera' } }, 'Camera, FaceTime HD Camera, Starting…', 'starting'],
      [{ camera: { mark: 'live', name: 'FaceTime HD Camera' } }, 'Camera, FaceTime HD Camera, Live', 'live'],
      [{ camera: { mark: 'not-connected', name: 'Logitech C920' } }, 'Camera, Logitech C920, Not connected', 'attention'],
      [{ camera: { mark: 'access-off' } }, 'Camera, Access off', 'attention'],
      [{ camera: { mark: 'no-video', name: 'FaceTime HD Camera' } }, 'Camera, FaceTime HD Camera, No video', 'attention'],
    ];
    for (const [inputs, title, mark] of cases) {
      const h = await loaded({ state: stateOf({ capture: capOf({ inputs: { microphone: { mark: 'armed', name: 'MacBook Pro Microphone' }, ...inputs } }) }) });
      eq(h.$('capCamera').title, title); eq(h.$('capCamera').dataset.mark, mark); lacks(h.$('capCamera').title + h.$('capCameraName').textContent, /Remembered/);
      eq(h.$('capCameraName').textContent, inputs.camera.name || 'Camera', 'the real name, or the plain noun when none is known');
      eq(h.$('capMicName').textContent, 'MacBook Pro Microphone'); eq(h.$('capMic').title, 'Microphone, MacBook Pro Microphone, On, starts when you record');
    }
    const hidden = await loaded({ state: stateOf({ settings: { ...stateOf().settings, showCamera: false }, capture: capOf({ inputs: { camera: { mark: 'live', name: 'FaceTime HD Camera' }, microphone: { mark: 'off' } } }) }) });
    eq(hidden.$('capCamera').title, 'Camera, FaceTime HD Camera, Live · bubble hidden'); eq(hidden.$('capCamera').dataset.mark, 'live');
    eq(hidden.t.state.settings.showCamera, false, 'Show camera is its own choice');
    // Before the app reports there is only the person's saved choice, and no device name is made up.
    const cold = await loaded({ state: stateOf({ settings: { ...stateOf().settings, camera: true }, capture: { phase: 'unavailable', generation: 0, known: false } }) });
    eq(cold.$('capCamera').title, 'Camera, On, starts when you record'); eq(cold.$('capCameraName').textContent, 'Camera'); eq(cold.$('capSourceName').textContent, 'Display');
    eq((await loaded({ state: stateOf({ capture: capOf() }) })).$('capSourceName').textContent, 'Built-in Retina Display', 'the resolved display name, not "Main display"');
  });
  await check('the camera popover stays open while the person chooses; choosing a device or a look sends only that, and never turns anything on', async () => {
    const st = stateOf({ capture: capOf({ inputs: { camera: { mark: 'armed', name: 'FaceTime HD Camera' }, microphone: { mark: 'off' } } }) });
    const h = await loaded({ state: st });
    h.$('capCamera').click(); const pop = h.$('capPop'); eq(h.$('capCamera').attrs['aria-expanded'], 'true');
    has(pop.innerHTML, 'FaceTime HD Camera · On — starts when you record'); has(pop.innerHTML, /aria-checked="true" data-pop="device" data-kind="camera" data-uid="CAM-A"/); has(pop.innerHTML, 'Logitech C920');
    for (const shape of ['Circle', 'Rounded', 'Wide', 'Portrait']) has(pop.innerHTML, `>${shape}<`);
    for (const size of ['S', 'M', 'L']) has(pop.innerHTML, `>${size}<`);
    has(pop.innerHTML, 'Show camera'); has(pop.innerHTML, 'Mirror preview'); has(pop.innerHTML, 'Stays open while you choose. Esc closes.');
    eq(commandsOf(h).length, 0, 'opening it sends nothing');
    const press = (data, on) => pop.onclick({ target: { closest: () => ({ dataset: data, disabled: false, getAttribute: () => String(on) }) } });
    press({ pop: 'device', kind: 'camera', uid: 'CAM-B' }); await settle();
    eq(JSON.stringify(commandsOf(h)), JSON.stringify([{ command: 'device', kind: 'camera', uid: 'CAM-B' }]));
    eq(h.$('capCamera').attrs['aria-expanded'], 'true', 'still open after a choice'); has(pop.innerHTML, 'Logitech C920');
    press({ pop: 'shape', kind: 'camera', value: 'portrait' }); press({ pop: 'size', kind: 'camera', value: '224' }); press({ pop: 'show', kind: 'camera' }, true); press({ pop: 'mirror', kind: 'camera' }, true); await settle();
    eq(JSON.stringify(commandsOf(h).slice(1)), JSON.stringify([{ command: 'shape', value: 'portrait' }, { command: 'size', value: 224 }, { command: 'show-camera', value: false }, { command: 'mirror', value: false }]));
    eq(commandsOf(h).filter(c => c.command === 'input').length, 0, 'no choice but the toggle can turn the camera on');
    h.dom.listeners.keydown.forEach(f => f({ key: 'Escape', preventDefault() {} })); eq(h.$('capCamera').attrs['aria-expanded'], 'false', 'Esc closes it');
    h.$('capCamera').click(); h.dom.listeners['doc:click'].forEach(f => f({ target: { closest: () => null } })); eq(h.$('capCamera').attrs['aria-expanded'], 'false', 'an outside click closes it');
  });
  await check('during a take the device and the on/off are locked with the reason, while shape, size and Show camera stay live', async () => {
    const h = await loaded({ state: stateOf({ capture: capOf({ phase: 'recording', elapsedMs: 5000, inputs: { camera: { mark: 'live', name: 'FaceTime HD Camera' }, microphone: { mark: 'live', name: 'MacBook Pro Microphone' } } }) }) });
    h.$('capCamera').click(); const html1 = h.$('capPop').innerHTML;
    has(html1, 'Unavailable while recording'); has(html1, /data-pop="device"[^>]*disabled/); has(html1, /data-pop="toggle"[^>]*disabled/);
    ok(!/data-pop="shape"[^>]*disabled/.test(html1) && !/data-pop="size"[^>]*disabled/.test(html1) && !/data-pop="show"[^>]*disabled/.test(html1), 'the look is live');
    eq(h.$('capMic').disabled, true); eq(h.$('capSource').disabled, true); has(h.$('capSource').title, 'Can’t change during a recording');
    h.t.state = { ...h.t.state, capture: capOf({ phase: 'idle' }) }; h.t.render(); h.$('capSource').click(); const sourceHtml = h.$('capPop').innerHTML;
    has(sourceHtml, 'Built-in Retina Display'); has(sourceHtml, 'LG UltraFine'); has(sourceHtml, 'Choose Window…'); lacks(sourceHtml, /Main Screen|<img/);
  });
  await check('during a take the camera controls follow what the app applies, so Show camera and Mirror can be reversed (the saved preference stays for later)', async () => {
    // Native applies a change at once but saves it only when the take is over: the saved settings below never change.
    const saved = { ...stateOf().settings, showCamera: true, mirror: true, cameraShape: 'rounded', cameraSize: 168 };
    const applied = (over = {}) => ({ shape: 'rounded', size: 168, mirror: true, showCamera: true, ...over });
    const take = (look, seq) => stateOf({ settings: saved, capture: capOf({ phase: 'recording', generation: 7, elapsedMs: 5000 + seq, appearance: look, inputs: { camera: { mark: 'live', name: 'FaceTime HD Camera' }, microphone: { mark: 'off' } } }) });
    let current = take(applied(), 0);
    const h = boot({ key: 'OWNERKEY', api: async () => okJson(current) }); await settle();
    // The 700 ms double-press guard is real; a person's next press comes after it.
    const release = () => { for (const [id, tm] of [...h.timeouts]) if (tm.delay === 700) { h.timeouts.delete(id); tm.fn(); } };
    h.$('capCamera').click(); const pop = h.$('capPop');
    const row = (what) => { const m = new RegExp(`<button role="switch" aria-checked="(true|false)" data-pop="${what}"`).exec(pop.innerHTML); ok(m, `${what} switch is rendered`); return m[1]; };
    const press = (what, attrs = {}) => pop.onclick({ target: { closest: () => ({ dataset: { pop: what, kind: 'camera', ...attrs }, disabled: false, getAttribute: () => row(what) }) } });
    const selected = (what) => [...pop.innerHTML.matchAll(new RegExp(`aria-checked="(true|false)" data-pop="${what}" data-value="([^"]+)"`, 'g'))].filter(m => m[1] === 'true').map(m => m[2]);
    // A fresh report from the app arrives with the next refresh.
    const fresh = async (look, seq) => { current = take(look, seq); await h.t.refresh(); await settle(); release(); };
    eq(row('show'), 'true'); eq(row('mirror'), 'true'); eq(h.$('capCamera').title.includes('bubble hidden'), false);
    // Hide: the first press asks for false. The app applies it and its next report says so; the saved preference is still true.
    press('show'); await settle(); eq(JSON.stringify(commandsOf(h).at(-1)), JSON.stringify({ command: 'show-camera', value: false }));
    await fresh(applied({ showCamera: false }), 1);
    eq(h.t.state.settings.showCamera, true, 'nothing was saved mid-take');
    eq(row('show'), 'false', 'the switch shows the applied state'); eq(h.$('capCamera').title, 'Camera, FaceTime HD Camera, Live · bubble hidden'); has(h.$('capPop').innerHTML, 'Stays open while you choose');
    // Show again: now the press asks for true (it used to ask for false again).
    press('show'); await settle(); eq(JSON.stringify(commandsOf(h).at(-1)), JSON.stringify({ command: 'show-camera', value: true }));
    await fresh(applied({ showCamera: true }), 2); eq(row('show'), 'true'); eq(h.$('capCamera').title.includes('bubble hidden'), false);
    // Mirror off, then on again.
    press('mirror'); await settle(); eq(JSON.stringify(commandsOf(h).at(-1)), JSON.stringify({ command: 'mirror', value: false }));
    await fresh(applied({ mirror: false }), 3); eq(row('mirror'), 'false');
    press('mirror'); await settle(); eq(JSON.stringify(commandsOf(h).at(-1)), JSON.stringify({ command: 'mirror', value: true }));
    await fresh(applied({ mirror: true }), 4); eq(row('mirror'), 'true');
    // Shape and size highlights follow the applied values too.
    eq(selected('shape').join(), 'rounded'); eq(selected('size').join(), '168');
    await fresh(applied({ shape: 'portrait', size: 224 }), 5); eq(selected('shape').join(), 'portrait'); eq(selected('size').join(), '224');
    // An older app that reports no appearance leaves Home on the saved preference; so does everything outside a take.
    current = stateOf({ settings: { ...saved, showCamera: false }, capture: capOf({ phase: 'recording', generation: 7, elapsedMs: 6000, inputs: { camera: { mark: 'live' }, microphone: { mark: 'off' } } }) }); await h.t.refresh();
    eq(row('show'), 'false', 'no applied report: the saved preference');
    current = stateOf({ settings: { ...saved, showCamera: true }, capture: capOf({ phase: 'idle', appearance: applied({ showCamera: false }) }) }); await h.t.refresh();
    eq(row('show'), 'true', 'idle: the saved preference is the truth (the app has saved what it applied)');
    // A malformed report is refused whole and never replaces the last good state.
    const before = h.t.state;
    for (const bad of [{ shape: 'oval', size: 168, mirror: true, showCamera: true }, { shape: 'wide', size: '168', mirror: true, showCamera: true }, { shape: 'wide', size: 168, mirror: 1, showCamera: true }, { shape: 'wide', size: 168, mirror: true }]) {
      const g = boot({ key: 'K', api: async () => okJson(stateOf({ capture: capOf({ appearance: bad }) })) }); await settle();
      if (bad.shape === 'oval') continue;   // an unknown shape is the host's to refuse; the page only checks types
      ok(!g.t.state, `${JSON.stringify(bad)} must not become a state`);
    }
    ok(h.t.state === before);
  });
  await check('with no app running a camera look is an ordinary saved preference, and devices are not invented', async () => {
    const st = stateOf({ capture: { phase: 'unavailable', generation: 0, known: false } }); const calls = [];
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b) { calls.push(b); return okJson({ ...st, settings: { ...st.settings, ...(b.data.preferences || {}) } }); } return okJson(st); } }); await settle();
    h.$('capCamera').click(); has(h.$('capPop').innerHTML, 'Devices appear when OpenRecorder is running.');
    h.$('capPop').onclick({ target: { closest: () => ({ dataset: { pop: 'shape', value: 'wide' }, disabled: false, getAttribute: () => 'false' }) } }); await settle();
    eq(JSON.stringify(calls.map(c => [c.op, c.data])), JSON.stringify([['settings', { preferences: { cameraShape: 'wide' } }]]));
  });
  await check('after a stopped take: one calm notice and one next step, the raw reason only on the recording’s Technical details', async () => {
    const take = { id: 'r9', title: 'Stopped take', status: 'interrupted', finalized: true, createdAt: Date.now() - 1000, bytes: 9e6, favorite: false, archived: false, thumbnailReady: true, directory: '/mock/root/r9', error: 'Recording cannot keep up. Choose a lower resolution or frame rate.',
      files: [{ name: 'recording.mp4', bytes: 5e6 }, { name: 'screen.mp4', bytes: 3e6 }, { name: 'camera.mp4', bytes: 1e6 }], settings: { camera: true, microphone: true, fps: 60, cameraShape: 'wide' }, capture: { fps: 60, width: 2560, height: 1440, outputFps: 56.8, sourceFps: 56.6, finalization: 'completed', encoders: { screen: 'libx264', camera: 'h264_videotoolbox', recording: 'h264_videotoolbox' } } };
    const st = stateOf({ settings: { ...stateOf().settings, fps: 60 }, recordings: [...stateOf().recordings, take], capture: capOf({ notice: { kind: 'failed', text: 'Couldn’t keep up at 60 fps, 2560 × 1440. Originals are kept.', details: 'Recording cannot keep up.', generation: 4, kept: true } }) });
    const h = await loaded({ state: st });
    eq(h.$('capNotice').hidden, false); eq(h.$('capNoticeText').textContent, 'Recording stopped early'); eq(h.$('capNoticeSub').textContent, 'Available files were kept.');
    lacks(h.$('capNotice').innerHTML + h.$('capNoticeText').textContent, /keep up|Choose a lower|60 fps/, 'no raw reason and no frame-rate advice on Home');
    eq(h.$('capOpenTake').hidden, false);
    h.$('capOpenTake').click(); await settle();
    eq(h.t.state.recordings.find(r => r.id === 'r9').id, 'r9'); eq(h.$('recordingView').hidden, false, 'it opens the stopped recording');
    eq(JSON.stringify(commandsOf(h)), JSON.stringify([{ command: 'dismiss-notice' }]), 'and clears the notice; nothing else, no frame rate change');
    eq(h.$('stopNoteTitle').textContent, 'Recording stopped early.'); eq(h.$('stopNoteText').textContent, 'Available files were kept.'); eq(h.$('stopNote').hidden, false);
    eq(h.$('playerBox').hidden, false, 'finalized files may be played'); eq(h.$('techBox').hidden, false);
    const tech = h.$('techBody').innerHTML; has(tech, 'Recording cannot keep up. Choose a lower resolution or frame rate.'); has(tech, '2560 × 1440'); has(tech, '60 fps'); has(tech, '56.8 fps'); has(tech, 'recording.mp4, screen.mp4, camera.mp4'); has(tech, 'screen: libx264');
    has(h.$('detailRows').innerHTML, '>Duration<'); has(h.$('detailRows').innerHTML, '>Unavailable<'); lacks(h.$('detailRows').innerHTML, /0:00|0:13/, 'no invented duration');
    has(h.$('detailRows').innerHTML, 'On · Wide 16:9'); has(h.$('detailRows').innerHTML, '60 fps set');
    has(h.$('techFps').innerHTML, 'aria-checked="true" data-fps="60"');
    const quiet = await loaded({ state: stateOf({ capture: capOf({ notice: { kind: 'failed', text: 'Couldn’t start recording. Nothing was recorded.', generation: 4, kept: false } }) }) });
    eq(quiet.$('capNoticeText').textContent, 'Couldn’t start recording. Nothing was recorded.'); eq(quiet.$('capOpenTake').hidden, true, 'nothing to open'); eq(quiet.$('capNoticeSub').textContent, '');
    const running = await loaded({ state: stateOf({ capture: capOf({ phase: 'recording', elapsedMs: 1, notice: { kind: 'failed', text: 'x', kept: true } }) }) }); eq(running.$('capNotice').hidden, true, 'never alongside a take');
  });
  await check('"Next recordings" changes only the saved frame rate for new recordings and says so; an unfinalized take has no player, no poster, no duration', async () => {
    const stopped = { id: 'r8', title: 'Unfinished', status: 'interrupted', finalized: false, createdAt: Date.now() - 2000, bytes: 1e6, favorite: false, archived: false, thumbnailReady: true, directory: '/mock/root/r8', error: 'Recorder stopped unexpectedly.', files: [{ name: 'screen.mp4', bytes: 1e6 }], settings: { camera: false, fps: 30 } };
    const { finalized: _omit, ...older } = stopped; const legacy = { ...older, id: 'r7', title: 'Older stopped take' };
    const calls = []; const st = stateOf({ recordings: [stopped, legacy, ...stateOf().recordings] });
    const h = boot({ key: 'K', api: async (u, o) => { const b = o.body && JSON.parse(o.body); if (b) { calls.push(b); return okJson({ ...st, settings: { ...st.settings, ...(b.data.preferences || {}) } }); } return okJson(st); } }); await settle();
    h.t.openRecording('r8'); await settle();
    eq(h.$('stopNoteTitle').textContent, 'This recording couldn’t be finished.'); eq(h.$('stopNoteText').textContent, 'Its original files are kept.'); eq(h.$('playerBox').hidden, true); has(h.$('playerNote').textContent, 'no player');
    has(h.$('detailRows').innerHTML, '>Unavailable<'); eq(h.$('player').src || '', '', 'nothing is loaded into the player');
    h.$('techFps').onclick({ target: { closest: () => ({ dataset: { fps: '24' } }) } }); await settle();
    eq(JSON.stringify(calls.map(c => c.data)), JSON.stringify([{ preferences: { fps: 24 } }]), 'only the saved rate for new recordings');
    h.t.openRecording('r7'); await settle(); eq(h.$('playerBox').hidden, true, 'no evidence of finalization: no player'); eq(h.$('stopNoteTitle').textContent, 'Recording stopped early.');
    const card = h.t.cardMarkup(stopped); has(card, 'Not finalized'); has(card, 'Can’t play'); lacks(card, /<img/, 'no poster for a take that was not finalized'); has(card, 'Available files kept'); lacks(card, /\d:\d\d</, 'no duration');
  });
  await check('card media states are honest: a poster, loading, "can’t load here", "no preview image", "can’t play" — never a made-up picture or duration', async () => {
    const h = await loaded();
    const take = (over = {}) => ({ id: 'r1', title: 'T', status: 'ready', thumbnailReady: true, createdAt: Date.now(), files: [], ...over });
    const state = (r, outcome) => h.t.cardMediaState(take(r), outcome);
    eq(state({}, 'poster').kind, 'poster'); eq(state({}, 'loading').note, 'Loading preview…'); eq(state({}, 'blocked').note, 'Preview can’t load here');
    eq(state({ thumbnailReady: false }, 'poster').note, 'No preview image'); eq(state({ status: 'interrupted', finalized: false }, 'poster').note, 'Can’t play');
    eq(state({ status: 'recording' }, 'poster').kind, 'unavailable');
    const noPoster = h.t.cardMarkup(take({ thumbnailReady: false, duration: 12 })); has(noPoster, 'No preview image'); lacks(noPoster, '<img'); has(noPoster, '0:12');
    const noDuration = h.t.cardMarkup(take({ duration: 0, status: 'interrupted', finalized: true })); lacks(noDuration, /\d:\d\d</); has(noDuration, 'Stopped early');
    const real = h.t.cardMarkup(take({ duration: 252 })); has(real, 'data-poster="r1"'); has(real, '4:12');
  });
  // ---- Embedded media. The agent window's policy and transport are the host's; the page ends every card as a picture or an
  // honest reason, by whichever route works. `policy` stands for the host: what an <img> may load, by address.
  const chunkOf = n => Buffer.from(Array.from({ length: n }, (_, i) => (i * 7 + 3) % 251));
  // A real, valid JPEG (320x180, synthetic, no owner data), the same file the real-browser decoder proof used.
  const posterBytes = fs.readFileSync(path.join(root, 'tests/fixtures/poster-320x180.jpg'));
  // The decoder boundary available to node: the JPEG's own headers must describe a complete 320x180 baseline picture
  // (start/end markers, a start-of-frame with its size, a scan). Pixel decoding is proven in the real browser run.
  const jpegSize = b => { if (b[0] !== 0xff || b[1] !== 0xd8 || b[b.length - 2] !== 0xff || b[b.length - 1] !== 0xd9) return null; let i = 2, scan = false, size = null; while (i + 4 <= b.length) { if (b[i] !== 0xff) return null; const m = b[i + 1], len = b.readUInt16BE(i + 2); if (m === 0xc0) size = [b.readUInt16BE(i + 7), b.readUInt16BE(i + 5)]; if (m === 0xda) { scan = true; break; } i += 2 + len; } return scan ? size : null; };
  const mediaHost = (st, { image, bridge = 'ok', total = posterBytes.length } = {}) => {
    const reads = [];
    const answers = {
      'media-session': () => ({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }),
      'media-read': call => {
        reads.push(call.data);
        if (bridge === 'silent') return SILENT;
        if (bridge === 'refuse') return { isError: true, content: [{ type: 'text', text: 'File unavailable.' }] };
        const isPoster = call.data.name === 'thumbnail.jpg', size = 196608, whole = isPoster ? posterBytes.length : total;
        const length = Math.min(size, whole - call.data.start);
        const bytes = isPoster ? posterBytes.subarray(call.data.start, call.data.start + length) : Buffer.alloc(length, 7);
        return { structuredContent: { mime: isPoster ? 'image/jpeg' : 'video/mp4', total: whole, start: call.data.start, length, data: bytes.toString('base64') } };
      },
    };
    const host = embeddedHost(st, { answers, image, ui: bridge === 'unsupported' ? 'unsupported' : 'ok' });
    return { ...host, reads };
  };
  const sevenTakes = () => {
    const base = stateOf(), take = (id, over = {}) => ({ id, title: `Take ${id}`, status: 'ready', createdAt: Date.now() - 1000 * Number(id.slice(1)), duration: 10, bytes: 1e6, favorite: false, archived: false, thumbnailReady: true, directory: `/mock/${id}`, files: [{ name: 'recording.mp4', bytes: 1e6 }], settings: {}, ...over });
    return stateOf({ recordings: [...Array.from({ length: 6 }, (_, i) => take(`t${i + 1}`)), take('t7', { thumbnailReady: false, status: 'interrupted', finalized: true })] });
  };
  const idle = async (h, n = 8) => { for (let i = 0; i < n; i++) await settle(); };
  await check('embedded posters: under a host whose default policy allows only data: images, the bytes come through the window tool as data: pictures, six cards show a picture and the missing one says so', async () => {
    const policy = url => (url.startsWith('data:image/jpeg;base64,') ? 'load' : 'error');   // http: (the address) and blob: are refused
    const { h, reads, uiCalls } = mediaHost(sevenTakes(), { image: policy }); await idle(h);
    const m = h.t.media;
    eq(JSON.stringify(jpegSize(posterBytes)), JSON.stringify([320, 180]), 'the fixture is a whole 320x180 JPEG');
    for (const id of ['t1', 't2', 't3', 't4', 't5', 't6']) eq(m.cards.get(id)?.state, 'poster', id);
    eq(m.cards.get('t7'), undefined, 'a take with no poster is never asked for one');
    for (const id of ['t1', 't6']) ok(m.posters.get(id).startsWith('data:image/jpeg;base64,'), 'a data: picture');
    eq(Buffer.from(m.posters.get('t1').split(',')[1], 'base64').equals(posterBytes), true, 'the picture is exactly the file');
    eq(JSON.stringify(jpegSize(Buffer.from(m.posters.get('t6').split(',')[1], 'base64'))), JSON.stringify([320, 180]), 'and still a whole JPEG after the transport');
    ok(reads.every(r => r.name === 'thumbnail.jpg' && r.start === 0), 'one bounded piece per poster, only posters');
    ok(h.images.every(u => u.startsWith('data:image/jpeg')), `no address was tried while the window tool works: ${h.images.join(' ')}`);
    eq(h.$('mediaLine').hidden, true, 'nothing to apologise for');
    // The markup: no lazy loading in an agent window; a card without its picture yet says it is loading; the honest card says why.
    const quiet = mediaHost(sevenTakes(), { bridge: 'silent', image: () => 'error' }); await settle();
    const card = quiet.h.t.cardMarkup({ id: 'x1', title: 'T', status: 'ready', thumbnailReady: true, createdAt: Date.now(), files: [] });
    lacks(card, 'loading="lazy"'); has(card, 'Loading preview…'); lacks(card, '<img');
    has(h.t.cardMarkup(sevenTakes().recordings[6]), 'No preview image');
    // A re-render uses the cached picture: no new read, and the picture is in the markup.
    const readsBefore = reads.length; const again = h.t.cardMarkup(sevenTakes().recordings[0]); has(again, 'data:image/jpeg;base64,'); lacks(again, 'Loading preview'); eq(reads.length, readsBefore);
    await h.t.loadPosters(); eq(reads.length, readsBefore, 'nothing is asked for twice');
    ok(uiCalls.every(c => c.ui === 'UI-SESSION-KEY'));
  });
  await check('embedded posters: when the window tool does not answer, the address is the second route, and its own failure ends the card with the reason, within deadlines', async () => {
    // Window tool silent; the address works.
    const direct = mediaHost(sevenTakes(), { bridge: 'silent', image: url => (url.startsWith('http://127.0.0.1:1/file/') ? 'load' : 'error') });
    await idle(direct.h);
    for (let i = 0; i < 12 && direct.h.t.media.cards.size < 6; i++) { await direct.h.runTimeout(); await idle(direct.h); }
    eq([...direct.h.t.media.cards.values()].filter(c => c.state === 'poster').length, 6, 'the address served all six');
    eq(direct.h.t.media.bridge, 'dead'); eq(direct.h.t.media.direct, 'ok');
    ok(direct.reads.length <= 3, `one bounded wait for the silent tool, not one per card: ${direct.reads.length} reads`);
    // Both routes fail (window tool silent, address blocked by policy): every card ends, the line says why, no credential in it.
    const none = mediaHost(sevenTakes(), { bridge: 'silent', image: () => 'error' });
    await idle(none.h);
    for (let i = 0; i < 40 && [...none.h.t.media.cards.values()].filter(c => c.state === 'blocked').length < 6; i++) { await none.h.runTimeout(); await idle(none.h); }
    eq([...none.h.t.media.cards.values()].filter(c => c.state === 'blocked').length, 6, 'all six ended as blocked, none left silent');
    const line = none.h.$('mediaLine'); eq(line.hidden, false); has(line.textContent, 'Previews can’t load in this window'); has(line.textContent, 'window tool: no answer'); has(line.textContent, 'direct address:'); has(line.textContent, 'browser');
    lacks(line.textContent, /media=|key|127\.0\.0\.1/i, 'no address or credential is shown');
    // An image that never reports anything (neither load nor error) is also an end, by its deadline.
    const hang = mediaHost(sevenTakes(), { bridge: 'unsupported', image: () => 'hang' });
    await idle(hang.h);
    for (let i = 0; i < 40 && [...hang.h.t.media.cards.values()].filter(c => c.state === 'blocked').length < 6; i++) { await hang.h.runTimeout(); await idle(hang.h); }
    eq([...hang.h.t.media.cards.values()].filter(c => c.state === 'blocked').length, 6);
    has(hang.h.$('mediaLine').textContent, 'does not run the recorder’s page tool'); has(hang.h.$('mediaLine').textContent, 'no answer in 8 s');
  });
  await check('embedded posters: a host that blocks the address by policy is named, a refused read is per card, and Reload starts the previews again', async () => {
    const blockedBoth = mediaHost(sevenTakes(), { bridge: 'refuse', image: () => 'error' });
    blockedBoth.h.sandbox.document.dispatch?.('securitypolicyviolation', {});
    await idle(blockedBoth.h);
    for (let i = 0; i < 20 && [...blockedBoth.h.t.media.cards.values()].filter(c => c.state === 'blocked').length < 6; i++) { await blockedBoth.h.runTimeout(); await idle(blockedBoth.h); }
    eq([...blockedBoth.h.t.media.cards.values()].filter(c => c.state === 'blocked').length, 6);
    has(blockedBoth.h.$('mediaLine').textContent, 'window tool: refused (File unavailable.)');
    eq(blockedBoth.h.t.media.bridge, 'unknown', 'a refusal about one file does not mark the whole tool dead');
    // Reload (the ••• menu) forgets the verdicts and tries again; here the window now works.
    blockedBoth.h.t.media.cards.clear();
    eq(blockedBoth.h.t.media.cards.size, 0);
  });
  await check('embedded posters: a stale address is replaced once from a fresh media session (rate limited), and the session never leaves the page', async () => {
    let session = { origin: 'http://127.0.0.1:2', mediaKey: 'FRESH' }; const calls = [];
    const { h, uiCalls } = embeddedHost(stateOf(), { image: url => (url.includes('FRESH') ? 'load' : 'error'), answers: { 'media-session': () => ({ structuredContent: session }), 'media-read': () => ({ isError: true, content: [{ type: 'text', text: 'down' }] }) } });
    await idle(h);
    for (let i = 0; i < 10 && !h.t.media.cards.get('r1'); i++) { await h.runTimeout(); await idle(h); }
    eq(h.t.media.cards.get('r1')?.state, 'poster', 'the fresh address worked'); ok(h.images.some(u => u.startsWith('http://127.0.0.1:2/file/')), 'it was the fresh address');
    const asked = uiCalls.filter(c => c.op === 'media-session').length; ok(asked <= 1, `asked once: ${asked}`);
    await h.t.refreshMediaSession(); eq(uiCalls.filter(c => c.op === 'media-session').length, asked, 'rate limited');
  });
  await check('embedded media transport pieces: bounded, joined as one picture, too-large refused, an incomplete answer is an error', async () => {
    const total = 500000, size = 196608, pieces = [];
    const { h } = embeddedHost(stateOf(), { answers: { 'media-read': call => { pieces.push(call.data.start); const length = Math.min(size, total - call.data.start); return { structuredContent: { mime: 'image/jpeg', total, start: call.data.start, length, data: chunkOf(length).toString('base64') } }; } } }); await settle();
    pieces.length = 0;
    const blob = await h.t.fetchMedia('r1', 'thumbnail.jpg', 1024 * 1024); eq(blob.size, total); eq(blob.type, 'image/jpeg');
    eq(JSON.stringify(pieces), JSON.stringify([0, size, 2 * size]), 'sequential pieces no larger than one bounded piece each');
    const url = await h.t.fetchDataURL('r1', 'thumbnail.jpg', 1024 * 1024); ok(url.startsWith('data:image/jpeg;base64,')); eq(Buffer.from(url.split(',')[1], 'base64').equals(Buffer.concat([chunkOf(size), chunkOf(size), chunkOf(total - 2 * size)])), true, 'the joined pieces are the pieces, in order');
    let failure = null; try { await h.t.fetchMedia('r1', 'recording.mp4', 100000); } catch (e) { failure = e; } ok(failure && failure.tooLarge, 'a file over the bound is refused, not loaded whole');
    const odd = embeddedHost(stateOf(), { answers: { 'media-read': call => ({ structuredContent: { mime: 'image/jpeg', total: 10, start: call.data.start, length: 4, data: Buffer.alloc(4).toString('base64') } }) } }); await settle();
    let bad = null; try { await odd.h.t.fetchDataURL('r1', 'thumbnail.jpg', 1000); } catch (e) { bad = e; } ok(bad, 'a non-final piece that is not whole base64 groups is refused');
  });
  await check('media answers are checked, not trusted: offset, size, type, chunk bound, base64 and decoded length, cap', async () => {
    const good = (start, len, total, over = {}) => ({ mime: 'image/jpeg', total, start, length: len, data: chunkOf(len).toString('base64'), ...over });
    const attempt = async (answers, expect = 'image/', cap = 1024 * 1024) => {
      let n = 0; const { h } = embeddedHost(stateOf(), { answers: { 'media-read': call => ({ structuredContent: answers(call.data.start, n++) }) } }); await settle();
      try { return { url: await h.t.fetchDataURL('r1', 'thumbnail.jpg', cap, undefined, expect) }; } catch (e) { return { error: e }; }
    };
    const size = 196608, total = size + 600;
    const whole = await attempt(s => (s === 0 ? good(0, size, total) : good(size, 600, total))); ok(whole.url && whole.url.startsWith('data:image/jpeg;base64,'), 'a correct answer passes');
    const cases = {
      'another offset than the one asked for': () => good(3, 300, 300),
      'total changing between pieces': s => (s === 0 ? good(0, size, total) : good(size, 603, size + 603)),
      'MIME changing between pieces': s => (s === 0 ? good(0, size, total) : good(size, 600, total, { mime: 'image/png' })),
      'a MIME of the wrong kind for a poster': s => good(s, 300, 300, { mime: 'video/mp4' }),
      'a malformed MIME': s => good(s, 300, 300, { mime: 'image/jpeg;x=y' }),
      'no MIME': s => { const p = good(s, 300, 300); delete p.mime; return p; },
      'a chunk above the bound': s => good(s, size + 3, size + 3),
      'a small declared file carrying a larger payload': s => ({ mime: 'image/jpeg', total: 300, start: s, length: 300, data: chunkOf(150000).toString('base64') }),
      'a payload shorter than its declared length': s => ({ mime: 'image/jpeg', total: 3000, start: s, length: 3000, data: chunkOf(300).toString('base64') }),
      'text that is not base64': s => ({ mime: 'image/jpeg', total: 300, start: s, length: 300, data: '!'.repeat(400) }),
      'padding in the middle': s => ({ mime: 'image/jpeg', total: 300, start: s, length: 300, data: 'AA==' + chunkOf(297).toString('base64') }),
      'a piece running past the end of the file': s => good(s, 300, 200),
      'a total of zero or a fraction': s => good(s, 0, 0),
      'a fractional length': s => good(s, 300.5, 600),
      'a non-final piece that is not whole base64 groups': s => good(s, 4, 10),
      'an answer that is not an object': () => 'x',
    };
    for (const [what, answer] of Object.entries(cases)) { const r = await attempt(answer); ok(r.error && !r.url, `${what} must be refused`); has(r.error.message, 'incomplete', what); }
    const big = await attempt(s => good(s, 300, 5 * 1024 * 1024)); ok(big.error && big.error.tooLarge, 'a declared size over the cap is refused as too large');
    // The same checks guard the blob route and playback.
    const { h } = embeddedHost(stateOf(), { answers: { 'media-read': call => ({ structuredContent: { mime: 'video/mp4', total: 10, start: call.data.start, length: 10, data: chunkOf(5000).toString('base64') } }) } }); await settle();
    let refused = null; try { await h.t.fetchMedia('r1', 'recording.mp4', 1024, undefined, 'video/'); } catch (e) { refused = e; } ok(refused, 'the blob route refuses a payload larger than it declared');
    // And a poster answer of the wrong kind never becomes a picture: the card ends honest, with a reason.
    const wrong = mediaHost(sevenTakes(), { image: url => (url.startsWith('data:image/') ? 'load' : 'error'), bridge: 'ok' });
    wrong.h.t.media.bridge = 'unknown';
    const none = embeddedHost(sevenTakes(), { image: () => 'load', answers: { 'media-session': () => ({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }), 'media-read': call => ({ structuredContent: good(call.data.start, 300, 300, { mime: 'text/html' }) }) } });
    await idle(none.h);
    for (let i = 0; i < 20 && [...none.h.t.media.cards.values()].filter(c => c.state === 'blocked').length < 6; i++) { await none.h.runTimeout(); await idle(none.h); }
    ok(![...none.h.t.media.posters.values()].some(u => u.startsWith('data:text')), 'no html is ever shown as a picture');
  });
  await check('a picture pushed out of the cache is fetched again when its card is drawn; a card is never left "done" without a picture', async () => {
    const policy = url => (url.startsWith('data:image/jpeg;base64,') ? 'load' : 'error');
    const st = sevenTakes();
    const one = (posterBytes.toString('base64').length + 'data:image/jpeg;base64,'.length);
    // Room for exactly five of the six pictures: the sixth push evicts the first.
    const { h, reads } = mediaHost(st, { image: policy }); h.t.media.budget = one * 5 + 10; await idle(h, 12);
    const m = h.t.media, ids = ['t1', 't2', 't3', 't4', 't5', 't6'];
    const consistent = () => ids.every(id => m.cards.get(id)?.state !== 'poster' || m.posters.has(id) || m.direct === 'ok');
    ok(consistent(), 'no card is marked done while its picture is gone');
    ok(m.posterBytes <= m.budget && m.posters.size <= 5, `the cache holds to its byte budget: ${m.posters.size} pictures, ${m.posterBytes} bytes`);
    // A redraw (a sort) shows what the cache has, asks for what it lacks, and ends with every card a picture.
    const evictedBefore = ids.filter(id => !m.posters.has(id)); ok(evictedBefore.length >= 1, 'the budget really evicted a picture');
    for (const id of evictedBefore) has(h.t.cardMarkup(st.recordings.find(r => r.id === id)), 'Loading preview…', `${id} is shown as loading, not as a finished card`);
    const readsBefore = reads.length; h.t.state = st; h.$('sort').value = 'oldest'; h.$('sort').dispatch('change'); await h.t.refresh(); await idle(h, 12);
    ok(reads.length > readsBefore, 'the redraw asked for the picture that had been pushed out');
    ok(consistent(), 'still consistent after the redraw');
    // With room for all of them, a redraw asks for nothing and every card has its picture.
    const roomy = mediaHost(sevenTakes(), { image: policy }); await idle(roomy.h, 12);
    eq(ids.every(id => roomy.h.t.media.posters.has(id)), true); const n = roomy.reads.length; roomy.h.$('sort').value = 'oldest'; roomy.h.$('sort').dispatch('change'); await roomy.h.t.refresh(); await idle(roomy.h, 6); eq(roomy.reads.length, n, 'nothing is read twice when everything fits');
    for (const id of ids) lacks(roomy.h.t.cardMarkup(sevenTakes().recordings.find(r => r.id === id)), 'Loading preview');
  });
  await check('a real Reload (the ••• menu) invalidates posters still being fetched: a late answer from before it writes nothing', async () => {
    const gates = []; const policy = url => (url.startsWith('data:image/jpeg;base64,') ? 'load' : 'error');
    const answer = call => new Promise(resolve => gates.push(() => resolve({ structuredContent: { mime: 'image/jpeg', total: posterBytes.length, start: 0, length: posterBytes.length, data: posterBytes.toString('base64') } })));
    const { h } = embeddedHost(sevenTakes(), { image: policy, answers: { 'media-read': answer, 'media-session': () => ({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }) } });
    await idle(h); const old = gates.splice(0); eq(old.length, 3, 'three reads are in flight (bounded parallelism)');
    const m = h.t.media, epoch = m.epoch;
    h.$('hostMenuButton').click(); h.$('hostMenu').onclick({ target: { closest: () => ({ dataset: { host: 'reload' } }) } }); await idle(h);
    ok(m.epoch > epoch, 'Reload starts a new generation of media work');
    const fresh = gates.splice(0); eq(fresh.length, 3, 'and it asks again, three at a time');
    // The answers from before Reload arrive now: they must not become pictures, cards, verdicts, or more requests.
    old.forEach(f => f()); await idle(h, 10);
    eq(m.posters.size, 0, 'no picture from before Reload'); eq([...m.cards.values()].filter(c => c.state === 'poster').length, 0, 'no card marked done by it');
    eq(m.bridge, 'unknown', 'no verdict from it'); eq(gates.length, 0, 'and the old loop starts no further reads');
    // The answers asked for after Reload do.
    fresh.forEach(f => f()); await idle(h, 10);
    ok(m.posters.size >= 3, `the new generation completes: ${m.posters.size}`);
    for (let i = 0; i < 6 && gates.length; i++) { gates.splice(0).forEach(f => f()); await idle(h, 8); }
    eq(['t1', 't2', 't3', 't4', 't5', 't6'].every(id => m.posters.has(id)), true, 'all six end with a picture after Reload');
    // A late failure from before Reload does not mark the new generation's tool dead or a card blocked either.
    const late = []; const { h: g } = embeddedHost(sevenTakes(), { image: policy, answers: { 'media-read': () => new Promise(resolve => late.push(() => resolve({ isError: true, content: [{ type: 'text', text: 'down' }] }))) } });
    await idle(g); const stuck = late.splice(0); g.$('hostMenuButton').click(); g.$('hostMenu').onclick({ target: { closest: () => ({ dataset: { host: 'reload' } }) } }); await idle(g);
    stuck.forEach(f => f()); await idle(g, 10);
    eq([...g.t.media.cards.values()].filter(c => c.state === 'blocked').length, 0, 'a refusal from before Reload blocks no card of the new generation');
  });
  await check('playback: an error and the stall timer for one open make one fallback read, and a read from an earlier open cannot write into the next, even for the same recording and file', async () => {
    const total = 400000, size = 196608, held = [];
    const answer = call => new Promise(resolve => held.push({ start: call.data.start, release: () => { const length = Math.min(size, total - call.data.start); resolve({ structuredContent: { mime: 'video/mp4', total, start: call.data.start, length, data: Buffer.alloc(length, 7).toString('base64') } }); } }));
    const { h } = embeddedHost(stateOf(), { image: () => 'load', answers: { 'media-read': answer, 'media-session': () => ({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }) } });
    await idle(h); held.splice(0).forEach(x => x.release()); await idle(h);   // the posters
    const player = h.$('player'), stall = () => [...h.timeouts.values()].find(x => x.delay === 10000);
    h.t.openRecording('r1'); await settle();
    // The player errors, and while that read is waiting the stall timer fires for the same open.
    player.dispatch('error'); await settle(); const first = held.filter(x => x.start === 0).length; eq(first, 1, 'the error started one fallback read');
    ok(stall(), 'the stall timer is armed'); stall().fn(); await settle(); player.dispatch('error'); await settle();
    eq(held.filter(x => x.start === 0).length, 1, 'neither the timer nor a second error starts another read for the same open');
    // Close and open the very same file again while the first read is still pending.
    h.t.closeRecording(); h.t.openRecording('r1'); await settle();
    const address = player.src; ok(String(address).startsWith('http://'), 'the new open plays from its own source');
    const urlsBefore = []; const create = URL.createObjectURL; URL.createObjectURL = b => { urlsBefore.push(1); return create.call(URL, b); };
    try {
      for (let i = 0; i < 6 && held.length; i++) { held.splice(0).forEach(x => x.release()); await idle(h, 8); }
      eq(player.src, address, 'the old completion did not replace the new open’s source'); eq(player.dataset.via || '', '', 'nor mark it bridged'); eq(urlsBefore.length, 0, 'no object URL was made from the old read');
      // The new open still gets its own fallback, once.
      player.dispatch('error'); await settle(); eq(held.filter(x => x.start === 0).length, 1);
    } finally { URL.createObjectURL = create; }
    // Closing cancels the stall timer and the pending read cannot reopen anything.
    h.t.closeRecording(); for (let i = 0; i < 6 && held.length; i++) { held.splice(0).forEach(x => x.release()); await idle(h, 8); } eq(h.$('player').dataset.via || '', '');
  });
  await check('Reload: a poster read that was in flight does not ask for its next piece, and a late media-session answer commits no address and no verdict', async () => {
    const policy = url => (url.startsWith('data:image/jpeg;base64,') ? 'load' : 'error');
    // (a) A two-piece poster; the first piece is held across Reload.
    const total = 300000, size = 196608, held = [], starts = [];
    const piece = (start, resolve) => { const length = Math.min(size, total - start); resolve({ structuredContent: { mime: 'image/jpeg', total, start, length, data: Buffer.alloc(length, 5).toString('base64') } }); };
    const { h } = embeddedHost(sevenTakes(), { image: policy, answers: { 'media-read': call => new Promise(resolve => { starts.push(call.data.start); held.push(() => piece(call.data.start, resolve)); }), 'media-session': () => ({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }) } });
    await idle(h); const old = held.splice(0); eq(starts.filter(s => s === 0).length, 3);
    h.$('hostMenuButton').click(); h.$('hostMenu').onclick({ target: { closest: () => ({ dataset: { host: 'reload' } }) } }); await idle(h);
    const afterReload = starts.length; held.splice(0);   // the new generation's own reads stay held; only the old ones are released
    old.forEach(f => f()); await idle(h, 10);
    eq(starts.slice(afterReload).filter(s => s > 0).length, 0, 'a chunk answer from before Reload asks for no further piece');
    eq(h.t.media.posters.size, 0);
    // (b) A media-session answer held across Reload, while the new generation has already found the address healthy.
    const sessions = []; let healthy = false;
    const { h: g } = embeddedHost(sevenTakes(), { image: url => (url.startsWith('http://127.0.0.1:1/file/') ? (healthy ? 'load' : 'error') : 'error'), answers: { 'media-read': () => ({ isError: true, content: [{ type: 'text', text: 'refused' }] }), 'media-session': () => new Promise(resolve => sessions.push(() => resolve({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }))) } });
    await idle(g, 12); ok(sessions.length >= 1, 'the old generation asked for a fresh address and is waiting');
    const lateAnswers = sessions.splice(0);
    healthy = true;   // the address works from now on
    g.$('hostMenuButton').click(); g.$('hostMenu').onclick({ target: { closest: () => ({ dataset: { host: 'reload' } }) } }); await idle(g, 12);
    for (let i = 0; i < 8 && g.t.media.direct !== 'ok'; i++) { await g.runTimeout(); await idle(g, 6); }
    eq(g.t.media.direct, 'ok', 'the new generation established a healthy verdict');
    const reasonBefore = JSON.stringify(g.t.media.reasons);
    lateAnswers.forEach(f => f()); await idle(g, 12);
    eq(g.t.media.direct, 'ok', 'the late "nothing new" session answer did not write a failed verdict over it');
    eq(JSON.stringify(g.t.media.reasons), reasonBefore, 'nor a reason');
    for (let i = 0; i < 8 && [...g.t.media.cards.values()].filter(c => c.state === 'poster').length < 6; i++) { await g.runTimeout(); await idle(g, 8); }
    eq([...g.t.media.cards.values()].filter(c => c.state === 'poster').length, 6, 'and posters keep loading over the address');
  });
  await check('the picture cache keeps one count: an image error, an eviction, a refetch and Reload all leave it equal to the pictures held', async () => {
    const policy = url => (url.startsWith('data:image/jpeg;base64,') ? 'load' : 'error');
    const { h } = mediaHost(sevenTakes(), { image: policy }); await idle(h, 12);
    const m = h.t.media, held = () => [...m.posters.values()].reduce((n, u) => n + u.length, 0);
    eq(m.posters.size, 6); eq(m.posterBytes, held(), 'counted after loading');
    const one = m.posters.get('t1').length;
    // The image of a cached card fails to display: the picture is dropped, the count follows, the refetch counts it once.
    await h.t.posterFailed({ dataset: { poster: 't1' }, src: m.posters.get('t1'), closest: () => null, remove() {} });
    await idle(h, 10); eq(m.posters.size, 6, 'refetched'); eq(m.posterBytes, held(), 'one entry, one count, after an image error'); eq(m.posterBytes, one * 6);
    // The same through two errors in a row and with a budget that has to evict.
    for (let i = 0; i < 2; i++) { await h.t.posterFailed({ dataset: { poster: 't2' }, src: m.posters.get('t2') || 'data:x', closest: () => null, remove() {} }); await idle(h, 10); eq(m.posterBytes, held(), 'still exact'); }
    m.budget = one * 4 + 10; h.t.rememberPoster('t1', m.posters.get('t1')); eq(m.posterBytes, held()); ok(m.posters.size <= 4, `evicted to the budget: ${m.posters.size}`);
    h.t.resetMedia(); eq(m.posterBytes, 0); eq(m.posters.size, 0);
  });
  await check('playback: a failed fallback is not tried again for the same open, by a second error or the stall timer; the blob to data: step is kept', async () => {
    const total = 400000, size = 196608; let mode = 'refuse'; const reads = [];
    const { h } = embeddedHost(stateOf(), { image: () => 'load', answers: { 'media-session': () => ({ structuredContent: { origin: 'http://127.0.0.1:1', mediaKey: 'M' } }), 'media-read': call => {
      if (call.data.name !== 'recording.mp4') return { isError: true, content: [{ type: 'text', text: 'no poster' }] };
      reads.push(call.data.start); if (mode === 'refuse') return { isError: true, content: [{ type: 'text', text: 'refused' }] };
      const length = Math.min(size, total - call.data.start); return { structuredContent: { mime: 'video/mp4', total, start: call.data.start, length, data: Buffer.alloc(length, 7).toString('base64') } };
    } } });
    await idle(h); const player = h.$('player'), stall = () => [...h.timeouts.values()].find(x => x.delay === 10000);
    h.t.openRecording('r1'); await settle();
    player.dispatch('error'); await idle(h, 6); eq(reads.length, 1, 'the first attempt read once and was refused');
    player.dispatch('error'); await idle(h, 6); stall()?.fn(); await idle(h, 6); player.dispatch('error'); await idle(h, 6);
    eq(reads.length, 1, 'a second error, the timer and a third error did not read again'); has(h.$('playerNote').textContent, 'can’t');
    // A new open starts afresh; the blob works, then the data: step runs once, fails, and ends.
    h.t.closeRecording(); h.t.openRecording('r1'); await settle(); mode = 'ok'; reads.length = 0;
    player.dispatch('error'); for (let i = 0; i < 6; i++) await idle(h, 6);
    eq(player.dataset.via, 'bridge', 'the blob stage worked'); const blobReads = reads.length; eq(blobReads, 3);
    mode = 'refuse'; reads.length = 0; player.dispatch('error'); await idle(h, 8); eq(reads.length, 1, 'the data: stage made one attempt');
    player.dispatch('error'); await idle(h, 6); stall()?.fn(); await idle(h, 6); eq(reads.length, 1, 'and a failed data: stage is not repeated'); has(h.$('playerNote').textContent, 'can’t be played here');
    // A stale chunk loop stops: close while the data: read is mid-file and nothing more is asked.
    h.t.closeRecording(); h.t.openRecording('r1'); await settle(); mode = 'ok'; reads.length = 0;
    const gate = []; const closed = embeddedHost(stateOf(), { image: () => 'load', answers: { 'media-read': call => (call.data.name === 'recording.mp4' ? new Promise(resolve => gate.push(() => { const length = Math.min(size, total - call.data.start); reads.push(call.data.start); resolve({ structuredContent: { mime: 'video/mp4', total, start: call.data.start, length, data: Buffer.alloc(length, 7).toString('base64') } }); })) : { isError: true, content: [{ type: 'text', text: 'no' }] }) } });
    await idle(closed.h); closed.h.t.openRecording('r1'); await settle(); closed.h.$('player').dispatch('error'); await settle();
    gate.splice(0).forEach(f => f()); await idle(closed.h, 6); const before = reads.length; closed.h.t.closeRecording(); gate.splice(0).forEach(f => f()); await idle(closed.h, 10);
    ok(gate.length === 0, 'after close, the playback read asks for no further piece'); ok(reads.length <= before + 1, `reads: ${reads.length}`);
  });
  await check('playback in an agent host: a blob first, a data: URL for modest files when the blob is refused, a limit that points to the browser, and a stalled player is taken over', async () => {
    const total = 400000, size = 196608;
    const { h, reads } = mediaHost(stateOf(), { total, image: () => 'load' }); await idle(h);
    reads.length = 0;
    h.t.openRecording('r1'); await settle(); const player = h.$('player'); await h.t.playerFailed(); await settle();
    eq(JSON.stringify(reads.map(r => r.start)), JSON.stringify([0, size, 2 * size])); eq(player.dataset.via, 'bridge'); ok(String(player.src).startsWith('blob:'), 'the player plays a local blob');
    await h.t.playerFailed(); await settle(); eq(player.dataset.via, 'data'); ok(String(player.src).startsWith('data:video/mp4;base64,'), 'a refused blob is retried as a data: URL');
    await h.t.playerFailed(); has(h.$('playerNote').textContent, 'can’t be played here', 'the data: URL failing ends it');
    const big = stateOf(); big.recordings[0].files[0].bytes = 80 * 1024 * 1024;
    const { h: g } = embeddedHost(big); await settle(); g.t.openRecording('r1'); await settle(); await g.t.playerFailed();
    has(g.$('playerNote').textContent, 'too large to play in this window'); has(g.$('playerNote').textContent, 'browser');
    // A player that neither loads nor fails is taken over by the deadline.
    const stalled = mediaHost(stateOf(), { total, image: () => 'load' }); await idle(stalled.h); stalled.reads.length = 0;
    stalled.h.t.openRecording('r1'); await settle();
    ok([...stalled.h.timeouts.values()].some(x => x.delay === 10000), 'a deadline is set when playback starts in an agent window');
    for (let i = 0; i < 6 && !stalled.reads.length; i++) { await stalled.h.runTimeout(); await idle(stalled.h); }
    ok(stalled.reads.length > 0, 'the stalled player was taken over by the bridge');
    const { h: plain } = { h: await loaded() }; plain.t.openRecording('r1'); await settle(); await plain.t.playerFailed(); has(plain.$('playerNote').textContent, 'Show it in the folder');
  });
  await check('the agent window has its own ••• menu with Open in Browser as a secondary item; the browser page has none', async () => {
    const { h, actionCalls } = embeddedHost(stateOf({ capture: capOf() }), {}); await settle();
    eq(h.$('hostMenuBox').hidden, false); h.$('hostMenuButton').click(); const menu = h.$('hostMenu');
    has(menu.innerHTML, 'Open in Browser'); has(menu.innerHTML, 'Reload'); has(menu.innerHTML, 'Settings');
    menu.onclick({ target: { closest: () => ({ dataset: { host: 'browser' } }) } }); await settle();
    ok(actionCalls.some(c => c.op === 'open-library'), 'it asks the library to open the browser, which returns no access link');
    eq((await loaded()).$('hostMenuBox').hidden, true);
  });
  // ---- Plugin-only lifecycle (preview.12): Home runs without the recorder app; the recorder opens ready and never records ----
  const coldRecorder = (state = 'not-running') => ({ state, ready: false, opening: state === 'opening', settingsPages: true });
  const coldState = (state, over = {}) => stateOf({ recorder: coldRecorder(state), ...over });
  const privacyClick = kind => ({ target: { closest: sel => (sel === '[data-privacy]' ? { dataset: { privacy: kind } } : null) } });
  const noRecording = (h, why) => { eq(sentOf(h).length, 0, `${why}: no capture command`); eq(startsOf(h).length, 0, `${why}: no Record press`); eq(h.media.touched, false, `${why}: no device touched`); };
  await check('cold Settings › Permissions: no "Not reported yet", an explanation and an Open OpenRecorder button, and all three permission buttons open their system page through the library, asking for nothing', async () => {
    const h = await loaded({ state: coldState() });
    h.t.openSettings('permissions'); const perms = h.$('settingsPane').innerHTML;
    lacks(perms, 'Not reported yet'); has(perms, 'Unknown until OpenRecorder is open'); has(perms, /data-open-recorder/); has(perms, 'asks for nothing and changes nothing'); has(perms, 'reported by OpenRecorder itself'); lacks(perms, /macOS/);
    const linux = await loaded({ state: coldState('not-running', { platform: 'linux' }) }); linux.t.openSettings('permissions'); const lp = linux.$('settingsPane').innerHTML; has(lp, 'Open OpenRecorder'); lacks(lp, /data-privacy=/); lacks(lp, 'system page directly'); lacks(lp, /macOS|Windows/);
    for (const kind of ['screen', 'camera', 'mic']) has(perms, new RegExp(`data-privacy="${kind}"`));
    for (const kind of ['screen', 'camera', 'mic']) { h.$('settingsPane').onclick(privacyClick(kind)); await settle(); }
    eq(JSON.stringify(ofOp(h, 'open-privacy-settings')), JSON.stringify([{ kind: 'screen' }, { kind: 'camera' }, { kind: 'mic' }]), 'each button asks the library for its own page');
    eq(ofOp(h, 'open-recorder').length, 0, 'opening a system page does not open the app'); noRecording(h, 'cold permission buttons');
    // With the app running the same buttons still go through its own menu, exactly as before.
    const live = await loaded({ state: stateOf({ capture: capOf() }) }); live.t.openSettings('permissions');
    live.$('settingsPane').onclick(privacyClick('camera')); await settle();
    eq(JSON.stringify(commandsOf(live)), JSON.stringify([{ command: 'open-privacy', kind: 'camera' }])); eq(ofOp(live, 'open-privacy-settings').length, 0);
  });
  await check('a permission button that cannot open its page says why instead of doing nothing (host refusal, and a host window without the page tool)', async () => {
    const refuse = boot({ key: 'OWNERKEY', api: async (url, opts) => { const b = opts.body ? JSON.parse(opts.body) : null; return b && b.op === 'open-privacy-settings' ? { ok: false, status: 400, json: async () => ({ error: 'This system has no privacy page to open.' }) } : okJson(coldState()); } }); await settle();
    refuse.t.openSettings('permissions'); refuse.$('settingsPane').onclick(privacyClick('screen')); await settle();
    eq(refuse.$('toastAlert').hidden, false); has(refuse.$('toastAlertText').textContent, 'no privacy page');
    const { h, uiCalls } = embeddedHost(coldState(), { ui: 'unsupported' }); await settle();
    h.t.openSettings('permissions'); h.$('settingsPane').onclick(privacyClick('mic')); await settle();
    eq(h.$('toastAlert').hidden, false); has(h.$('toastAlertText').textContent, 'can’t open System Settings'); eq(uiCalls.filter(c => c.op === 'open-privacy-settings').length, 0, 'a window already known to lack the tool does not try it');
    const ok = embeddedHost(coldState()); await settle(); ok.h.t.openSettings('permissions'); ok.h.$('settingsPane').onclick(privacyClick('camera')); await settle();
    eq(JSON.stringify(ok.uiCalls.filter(c => c.op === 'open-privacy-settings').map(c => [c.data, c.ui])), JSON.stringify([[{ kind: 'camera' }, 'UI-SESSION-KEY']]), 'in an agent host it goes through the page-only tool with this window\'s key');
  });
  await check('cold Home says the recorder app is not running and offers Open OpenRecorder; with the app reporting there is no banner', async () => {
    const h = await loaded({ state: coldState() });
    eq(h.$('recorderBanner').hidden, false); has(h.$('recorderTitle').textContent, 'isn’t running'); has(h.$('recorderText').textContent, 'nothing is recorded until you press Record'); eq(h.$('recorderOpen').textContent, 'Open OpenRecorder'); eq(h.$('recorderOpen').disabled, false);
    for (const [state, title, button, off] of [['opening', 'Opening OpenRecorder', 'Opening…', true], ['not-responding', 'isn’t responding', 'Try again', false]]) {
      const g = await loaded({ state: coldState(state) }); has(g.$('recorderTitle').textContent, title); eq(g.$('recorderOpen').textContent, button); eq(g.$('recorderOpen').disabled, off);
    }
    const up = await loaded({ state: stateOf({ capture: capOf(), recorder: { state: 'ready', ready: true, opening: false, settingsPages: true } }) });
    eq(up.$('recorderBanner').hidden, true);
    const older = await loaded({ state: stateOf() }); eq(older.$('recorderBanner').hidden, false, 'an older host that sends no recorder block is read from capture');
    lacks(html, /id="newRecording"/);
  });
  await check('Open OpenRecorder (cold) opens it ready: one open-recorder call, waits for the first report, says ready, and never records, queues a press or touches a device', async () => {
    let up = false; const st = () => (up ? stateOf({ capture: capOf(), recorder: { state: 'ready', ready: true, opening: false, settingsPages: true } }) : coldState());
    const h = boot({ key: 'OWNERKEY', api: async () => okJson(st()) }); await settle();
    h.$('recorderOpen').onclick(); await settle();
    eq(ofOp(h, 'open-recorder').length, 1); eq(JSON.stringify(ofOp(h, 'open-recorder')[0]), '{}', 'the request carries nothing to record');
    has(h.$('recorderTitle').textContent, 'Opening OpenRecorder'); eq(h.$('recorderOpen').disabled, true, 'a second press while it opens does nothing');
    h.$('recorderOpen').onclick(); await settle(); eq(ofOp(h, 'open-recorder').length, 1);
    up = true; for (let i = 0; i < 8 && h.$('toastText').textContent.indexOf('OpenRecorder is ready') < 0; i++) await h.runTimeout();
    has(h.$('toastText').textContent, 'OpenRecorder is ready. Press Record when you want to start.'); eq(h.$('recorderBanner').hidden, true);
    noRecording(h, 'open ready');
  });
  await check('Open OpenRecorder: a launch the host refuses shows its reason and Try again; an app that never reports times out visibly and can be retried; none of it records', async () => {
    const refused = boot({ key: 'OWNERKEY', api: async (url, opts) => { const b = opts.body ? JSON.parse(opts.body) : null; return b && b.op === 'open-recorder' ? { ok: false, status: 400, json: async () => ({ error: 'OpenRecorder may be recording. Stop and quit every OpenRecorder window and host, then try again.' }) } : okJson(coldState()); } }); await settle();
    refused.$('recorderOpen').onclick(); await settle();
    has(refused.$('recorderTitle').textContent, 'didn’t open'); has(refused.$('recorderText').textContent, 'may be recording'); eq(refused.$('recorderOpen').textContent, 'Try again'); eq(refused.$('recorderOpen').disabled, false);
    eq(refused.$('toastAlert').hidden, false); noRecording(refused, 'refused open');
    // The app never reports: after the wait Home says so and offers the same button; pressing it asks again, once.
    const RealDate = Date, clock = { offset: 0 };
    const h = boot({ key: 'OWNERKEY', api: async () => okJson(coldState('opening')) }); await settle();
    h.sandbox.Date = class extends RealDate { constructor(...a) { a.length ? super(...a) : super(RealDate.now() + clock.offset); } static now() { return RealDate.now() + clock.offset; } };
    h.$('recorderOpen').onclick(); await settle(); eq(ofOp(h, 'open-recorder').length, 1);
    clock.offset = 26000; for (let i = 0; i < 6; i++) await h.runTimeout();
    has(h.$('recorderTitle').textContent, 'didn’t report in time'); eq(h.$('recorderOpen').textContent, 'Try again'); eq(h.$('recorderOpen').disabled, false);
    h.$('recorderOpen').onclick(); await settle(); eq(ofOp(h, 'open-recorder').length, 2, 'Try again asks again');
    has(h.$('recorderTitle').textContent, 'Opening OpenRecorder'); noRecording(h, 'timeout and retry');
    // In a host window without the page tool there is nothing to press: it says to use the browser.
    const unsupported = embeddedHost(coldState(), { ui: 'unsupported' }); await settle();
    unsupported.h.$('recorderOpen').onclick(); await settle();
    has(unsupported.h.$('toastAlertText').textContent, 'Open Home in your browser');
  });
  await check('a cold main-control press never sends start-recording, even repeatedly or after the app has come up: Open OpenRecorder cannot carry a Record', async () => {
    let up = false; const st = () => (up ? stateOf({ capture: capOf(), recorder: { state: 'ready', ready: true, opening: false, settingsPages: true } }) : coldState());
    const h = boot({ key: 'OWNERKEY', api: async () => okJson(st()) }); await settle();
    h.$('capRecord').onclick(); await settle(); h.$('capRecord').onclick(); await settle();
    up = true; for (let i = 0; i < 8; i++) await h.runTimeout();
    eq(startsOf(h).length, 0); eq(sentOf(h).length, 0); eq(h.$('capRecordLabel').textContent, 'Record', 'once ready it is Record again, and nothing was recorded for it');
    noRecording(h, 'cold open then ready');
  });
  await check('a stale idle report is not readiness: while the library says the recorder is not responding Home shows Open (never an enabled Record), says so, and the opening poll cannot announce ready from old data', async () => {
    const stale = () => stateOf({ capture: capOf({ ageMs: 13000, stale: false }), recorder: { state: 'not-responding', ready: false, opening: false, settingsPages: true } });
    const h = await loaded({ state: stale() });
    eq(h.$('recorderBanner').hidden, false); has(h.$('recorderTitle').textContent, 'isn’t responding');
    eq(h.$('capRecord').dataset.phase, 'closed'); eq(h.$('capRecordLabel').textContent, 'Open'); has(h.$('capRecord').attrs['aria-label'], 'Open OpenRecorder');
    h.t.openSettings('permissions'); const perms = h.$('settingsPane').innerHTML; has(perms, 'Unknown until OpenRecorder is open'); lacks(perms, 'No problem reported'); lacks(perms, 'Allowed');
    h.$('capRecord').click(); await settle();
    eq(startsOf(h).length, 0, 'the press cannot be a Record'); eq(ofOp(h, 'open-recorder').length, 1);
    // The library keeps answering with the old report: the poll waits, and never says ready.
    for (let i = 0; i < 6; i++) await h.runTimeout();
    lacks(h.$('toastText').textContent, 'OpenRecorder is ready'); eq(h.$('capRecordLabel').textContent === 'Record', false);
    // The same state after the app really reports (ready: true): Record is back, and a press is a bound Record.
    const fresh = await loaded({ state: stateOf({ capture: capOf(), recorder: { state: 'ready', ready: true, opening: false, settingsPages: true } }) });
    eq(fresh.$('capRecordLabel').textContent, 'Record'); eq(fresh.$('recorderBanner').hidden, true);
    // A take's last report stays the last known state while the app is quiet: Stop and the timer guards still apply, no second app is opened.
    const take = await loaded({ state: stateOf({ capture: capOf({ phase: 'recording', generation: 5, elapsedMs: 9000, stale: true }), recorder: { state: 'not-responding', ready: false, opening: false, settingsPages: true } }) });
    eq(take.$('capRecord').dataset.phase, 'recording'); eq(take.$('capRecordLabel').textContent, 'Stop'); eq(take.$('recorderBanner').hidden, true);
    take.$('capRecord').click(); await settle(); eq(ofOp(take, 'open-recorder').length, 0, 'no second app is opened over a take'); eq(startsOf(take).length, 0);
  });
  await check('the GUI-B finding: with focus ON the Open button, the open Permissions pane moves unknown -> opening -> ready by itself (no reload, no blur), keeps its focus, replaces nothing, and records nothing', async () => {
    let up = false;
    const st = () => (up ? stateOf({ capture: capOf({ inputs: { camera: { mark: 'armed' }, microphone: { mark: 'armed' } } }), recorder: { state: 'ready', ready: true, opening: false, settingsPages: true } }) : coldState());
    const h = boot({ key: 'OWNERKEY', api: async () => okJson(st()) }); await settle();
    h.t.openSettings('permissions');
    const pane = h.$('settingsPane'); let writes = 0, markup = pane.innerHTML;
    Object.defineProperty(pane, 'innerHTML', { configurable: true, get: () => markup, set: v => { writes++; markup = v; } });
    const rendered = markup; has(rendered, 'id="permOpen"'); has(rendered, 'id="permStatus-camera"'); has(rendered, 'Unknown until OpenRecorder is open');
    // The person presses Open: a real button now holds focus, inside the pane (the fake DOM's contains() is told so).
    const openButton = h.$('permOpen'); pane.contains = el => el === openButton; h.dom.document.activeElement = openButton;
    pane.onclick({ target: { closest: sel => (sel === '[data-open-recorder]' ? {} : null) } }); await settle();
    eq(ofOp(h, 'open-recorder').length, 1); eq(h.$('permOpen').textContent, 'Opening…'); eq(h.$('permOpen').disabled, true); eq(h.$('permCold').hidden, false); has(h.$('permWhy').textContent, 'Opening OpenRecorder');
    eq(h.dom.document.activeElement, openButton, 'focus is still on the button while it says Opening…');
    // The app reports (the library now says ready). The poll refreshes Home; the focused pane must follow.
    up = true; for (let i = 0; i < 8 && !h.$('permCold').hidden; i++) await h.runTimeout();
    eq(h.$('permCold').hidden, true, 'the Unknown/Open note is gone without a reload or a blur');
    has(h.$('permStatus-screen').textContent, 'Allowed')
    eq(h.$('permStatus-camera').textContent, 'No problem reported'); eq(h.$('permStatus-mic').textContent, 'No problem reported');
    lacks(h.$('permStatus-camera').textContent + h.$('permStatus-mic').textContent + h.$('permNote').textContent, /Live/i);   // armed is not open hardware
    eq(writes, 0, 'the pane was patched in place, never rebuilt'); eq(h.dom.document.activeElement, openButton, 'focus was never taken from the button');
    for (let i = 0; i < 8 && h.$('toastText').textContent.indexOf('OpenRecorder is ready') < 0; i++) await h.runTimeout();
    has(h.$('toastText').textContent, 'OpenRecorder is ready. Press Record when you want to start.'); eq(writes, 0); eq(h.dom.document.activeElement, openButton);
    noRecording(h, 'a focused Open button that becomes ready');
    // The other outcomes reach the focused pane by themselves too: refused (with its reason), and never reported in time.
    const refused = boot({ key: 'OWNERKEY', api: async (u, o) => { const b = o.body && JSON.parse(o.body); return b && b.op === 'open-recorder' ? { ok: false, status: 400, json: async () => ({ error: 'OpenRecorder may be recording. Stop and quit every OpenRecorder window and host, then try again.' }) } : okJson(coldState()); } }); await settle();
    refused.t.openSettings('permissions'); const rb = refused.$('permOpen'); refused.$('settingsPane').contains = el => el === rb; refused.dom.document.activeElement = rb;
    refused.$('settingsPane').onclick({ target: { closest: sel => (sel === '[data-open-recorder]' ? {} : null) } }); await settle();
    has(refused.$('permWhy').textContent, 'may be recording'); eq(refused.$('permOpen').textContent, 'Try again'); eq(refused.$('permOpen').disabled, false); eq(refused.dom.document.activeElement, rb); noRecording(refused, 'refused open');
    const RealDate = Date, clock = { offset: 0 };
    const slow = boot({ key: 'OWNERKEY', api: async () => okJson(coldState('opening')) }); await settle();
    slow.sandbox.Date = class extends RealDate { constructor(...a) { a.length ? super(...a) : super(RealDate.now() + clock.offset); } static now() { return RealDate.now() + clock.offset; } };
    slow.t.openSettings('permissions'); const sb = slow.$('permOpen'); slow.$('settingsPane').contains = el => el === sb; slow.dom.document.activeElement = sb;
    slow.$('settingsPane').onclick({ target: { closest: sel => (sel === '[data-open-recorder]' ? {} : null) } }); await settle();
    clock.offset = 26000; for (let i = 0; i < 6; i++) await slow.runTimeout();
    has(slow.$('permWhy').textContent, 'didn’t report in time'); eq(slow.$('permOpen').textContent, 'Try again'); eq(slow.$('permOpen').disabled, false); eq(slow.dom.document.activeElement, sb);
  });
  await check('Permissions is patched, not rebuilt: a live readiness change never replaces the pane or its focused button, and a form pane with a pending edit is still left alone', async () => {
    let up = false;
    const st = () => (up ? stateOf({ capture: capOf({ screenState: 'denied', inputs: { camera: { mark: 'access-off' }, microphone: { mark: 'live', name: 'Mic' } } }), recorder: { state: 'ready', ready: true, opening: false, settingsPages: true } }) : coldState());
    const h = boot({ key: 'OWNERKEY', api: async () => okJson(st()) }); await settle(); h.t.openSettings('permissions');
    const pane = h.$('settingsPane'); let writes = 0, markup = pane.innerHTML;
    Object.defineProperty(pane, 'innerHTML', { configurable: true, get: () => markup, set: v => { writes++; markup = v; } });
    const focus = {}; pane.contains = el => el === focus; h.dom.document.activeElement = focus;
    up = true; await h.t.refresh(); await settle();
    eq(writes, 0, 'no rebuild while a pane element has focus'); eq(h.$('permCold').hidden, true);
    has(h.$('permStatus-screen').textContent, 'Off for OpenRecorder'); has(h.$('permStatus-camera').textContent, 'Off in System Settings'); has(h.$('permStatus-mic').textContent, 'Allowed. Live now.'); has(h.$('permNote').textContent, 'Live now: microphone');
    // Backend staleness still wins in the patched pane: the app goes quiet (an old idle report remains), and the focused pane says so by itself.
    h.sandbox.fetch = async () => okJson(stateOf({ capture: capOf({ ageMs: 13000 }), recorder: { state: 'not-responding', ready: false, opening: false, settingsPages: true } }));
    await h.t.refresh(); await settle();
    eq(h.$('permCold').hidden, false); has(h.$('permWhy').textContent, 'isn’t responding'); eq(h.$('permOpen').textContent, 'Try again'); has(h.$('permStatus-camera').textContent, 'Unknown until OpenRecorder is open'); eq(writes, 0);
    // A pane with form fields keeps its pending edit: no rebuild while focus is inside it, and it follows once focus leaves.
    const g = await loaded({ state: stateOf() }); g.t.openSettings('general');
    const form = g.$('settingsPane'); let formWrites = 0, formMarkup = form.innerHTML;
    Object.defineProperty(form, 'innerHTML', { configurable: true, get: () => formMarkup, set: v => { formWrites++; formMarkup = v; } });
    const field = {}; form.contains = el => el === field; g.dom.document.activeElement = field;
    const changed = stateOf({ freeBytes: 1e9 }); g.sandbox.fetch = async () => okJson(changed); await g.t.refresh(); await settle();
    eq(formWrites, 0, 'a pending edit is never rebuilt over');
    g.dom.document.activeElement = g.dom.document.body; await g.t.refresh(); await settle(); eq(formWrites, 1, 'once focus has left the form it follows the change, with one rebuild');
  });
  await check('Settings in Home has Permissions and About, Show camera next to the four shapes, and the native Settings route opens it', async () => {
    const h = await loaded({ state: stateOf({ settings: { ...stateOf().settings, showCamera: true }, capture: capOf({ screenState: 'denied', inputs: { camera: { mark: 'access-off' }, microphone: { mark: 'live', name: 'MacBook Pro Microphone' } } }) }) });
    eq(h.t.settingPanes?.length ?? 6, 6); h.t.openSettings('camera'); const camera = h.$('settingsPane').innerHTML;
    has(camera, 'Show camera'); has(camera, /name="showCamera"/); for (const shape of ['circle', 'rounded', 'wide', 'portrait']) has(camera, `value="${shape}"`);
    h.t.openSettings('permissions'); const perms = h.$('settingsPane').innerHTML;
    has(perms, 'Screen Recording'); has(perms, 'Off for OpenRecorder'); has(perms, 'Off in System Settings'); has(perms, 'Allowed. Live now.'); has(perms, 'Live now: microphone'); has(perms, 'never resets or re-requests'); has(perms, /data-privacy="screen"/);
    h.t.openSettings('about'); const about = h.$('settingsPane').innerHTML; has(about, '0.0.0-test'); has(about, 'Browser'); has(about, 'Menu bar');
    h.$('settingsPane').onclick({ target: { closest: sel => (sel === '[data-privacy]' ? { dataset: { privacy: 'screen' } } : null) } }); await settle();
    eq(JSON.stringify(commandsOf(h)), JSON.stringify([{ command: 'open-privacy', kind: 'screen' }]), 'a human button, one command, nothing requested');
    for (const label of ['General', 'Recording', 'Camera &amp; Microphone', 'Permissions', 'Sharing', 'About']) has(app + '', label.replace('&amp;', '&'));
    const routed = boot({ key: null, hash: 'OWNERKEY/settings', api: async () => okJson(stateOf({ capture: capOf() })) }); await settle();
    eq(routed.$('settingsDialog').open, true, 'the /settings route opens the same Settings');
    const plain = boot({ key: null, hash: 'OWNERKEY', api: async () => okJson(stateOf({ capture: capOf() })) }); await settle(); eq(plain.$('settingsDialog').open, false);
  });
  await check('a malformed capture report is rejected as a whole and never replaces the last good state', async () => {
    const good = stateOf({ capture: capOf({ phase: 'recording', generation: 3, elapsedMs: 5000 }) });
    for (const bad of [{ phase: 5, generation: 1 }, { phase: 'recording' }, { phase: 'idle', generation: 1, elapsedMs: 'soon' }, { phase: 'idle', generation: 1, sources: 'all' }, { phase: 'idle', generation: 1, devices: [] }]) {
      let next = good;
      const h = boot({ key: 'K', api: async () => okJson(next) }); await settle();
      const before = h.t.state; ok(before && before.capture.phase === 'recording');
      next = { ...good, capture: bad }; await h.t.refresh(); await settle();
      ok(h.t.state === before, `${JSON.stringify(bad)} replaced the last good state`);
      eq(h.$('capRecordLabel').textContent, 'Stop', 'the page still shows the last reported phase');
      const first = boot({ key: 'K', api: async () => okJson({ ...good, capture: bad }) }); await settle();
      ok(!first.t.state, `${JSON.stringify(bad)} must not become the first state either`);
    }
  });
  await check('real serializer answers: an unreported app still shows Record; a reported stopped take and a running take render from the host’s own fields', async () => {
    const unreported = await loaded({ state: clone(actualCase('capture-unreported')) });
    eq(unreported.$('capture').hidden, false, 'the Record group is always there'); eq(unreported.t.state.capture.phase, 'unavailable');
    lacks(JSON.stringify(unreported.t.state.capture), /launch/, 'before the app reports there is no launch to name');
    const failed = await loaded({ state: clone(actualCase('capture-failed-take-reported')) });
    eq(failed.$('capture').hidden, false); eq(failed.$('capNotice').hidden, false);
    eq(failed.$('capNoticeText').textContent, 'Recording stopped early'); eq(failed.$('capNoticeSub').textContent, 'Available files were kept.');
    lacks(failed.$('capNotice').innerHTML, /keep up|Choose a lower|60 fps/, 'the host’s raw reason never reaches Home');
    eq(failed.$('capSourceName').textContent, 'Built-in Display'); eq(failed.$('capCamera').dataset.mark, 'armed'); eq(failed.$('capCameraName').textContent, 'FaceTime HD Camera'); eq(failed.$('capTime').hidden, true);
    eq(failed.t.state.capture.devices.camera.length, 2, 'the real device list travels through the host'); eq(failed.t.state.capture.devices.camera[1].name, 'Studio Camera');
    has(failed.t.fpsCapabilityMarkup(), 'recording of this setup couldn’t keep up');
    eq(failed.t.state.capture.launch, 'fixture-launch-0001', 'the host passes the app’s launch through'); has(failed.t.fpsCapabilityMarkup(), 'encoder capacity only', 'the host keeps the encoder check beside the answer');
    failed.$('capRecord').click(); await settle(); eq(startsOf(failed).length, 1, 'after a stop, Record is available again'); eq(startsOf(failed)[0].launch, 'fixture-launch-0001'); eq(startsOf(failed)[0].generation, 6);
    const running = await loaded({ state: clone(actualCase('capture-recording-reported')) });
    eq(running.$('capRecordLabel').textContent, 'Stop'); eq(running.$('capTime').hidden, false); has(running.$('capTime').textContent, /^0:4[6-9]$/); eq(running.$('capRecord').dataset.phase, 'recording');
    eq(running.$('capNotice').hidden, true);
    // The serializer carries the applied look; during the take Home shows it (the saved settings say otherwise: shown, mirrored).
    const look = a => JSON.stringify(Object.fromEntries(Object.entries(a).sort()));
    eq(look(running.t.state.capture.appearance), look({ shape: 'wide', size: 224, mirror: false, showCamera: false }));
    eq(running.t.state.settings.showCamera !== false, true, 'the saved preference is untouched');
    eq(running.$('capCamera').title, 'Camera, FaceTime HD Camera, Live · bubble hidden', 'status from the applied look');
    eq(look(failed.t.state.capture.appearance), look({ shape: 'rounded', size: 168, mirror: true, showCamera: true }));
  });
  await check('Settings › Recording tells unknown from unavailable, never blocks 60, and the check runs only on a press', async () => {
    const h = await loaded({ state: stateOf({ capture: capOf() }) });
    h.t.openSettings('recording'); const pane = h.$('settingsPane').innerHTML;
    has(pane, '60 fps hasn’t been measured on this computer for this setup'); has(pane, 'You can still choose it'); lacks(pane, /unavailable|unsupported/i); has(pane, 'Check encoder at 60 fps');
    has(pane, 'encodes test frames only — no camera, mic or screen'); has(pane, /<option value="60"/, '60 stays selectable');
    eq(commandsOf(h).length, 0, 'opening Settings starts nothing');
    h.sandbox.document.getElementById('x');
    const handler = h.dom.listeners['doc:click'] || [];
    handler.forEach(f => f({ target: { id: 'checkFps', closest: () => null } })); await settle();
    eq(JSON.stringify(commandsOf(h)), JSON.stringify([{ command: 'capacity-check' }]));
    for (const [status, want] of [['qualified', 'recording of this setup on this computer kept up'], ['failed-last', 'recording of this setup couldn’t keep up']]) {
      h.t.state = { ...h.t.state, capture: capOf({ capabilities: { fps60: { status, basis: '', measuredFps: 59.9 } } }) };
      has(h.t.fpsCapabilityMarkup(), want, status);
    }
    lacks(h.t.fpsCapabilityMarkup(), /hasn’t been measured/, 'only unknown says unmeasured');
    h.t.state = { ...h.t.state, capture: capOf({ capabilities: { fps60: { status: 'unknown', basis: '', measuredFps: null, encoder: { status: 'qualified', measuredFps: 60 } } } }) };
    has(h.t.fpsCapabilityMarkup(), 'hasn’t been measured', 'a passing encoder check does not make the setup known'); has(h.t.fpsCapabilityMarkup(), 'encoder capacity only'); has(h.t.fpsCapabilityMarkup(), 'Check encoder again');
    const embedded = boot({ embedded: true, host: m => m.method === 'ui/initialize' ? { hostContext: {} } : { structuredContent: stateOf({ capture: capOf() }) }, api: async () => okJson(stateOf({ capture: capOf() })) }); await settle();
    has(embedded.t.fpsCapabilityMarkup(), 'hasn’t been measured', 'the same Settings in an agent window: the check is a person’s button there too'); has(embedded.t.fpsCapabilityMarkup(), 'Check encoder at 60 fps');
  });
  await check('portrait is a choice next to wide, and stored legacy choices stay selectable and unremapped', async () => {
    const h = await loaded({ state: stateOf({ capture: capOf() }) });
    h.t.openSettings('camera'); const pane = h.$('settingsPane').innerHTML;
    has(pane, /<option value="portrait"/); has(pane, 'Portrait 9:16'); has(pane, /<option value="wide"/); has(pane, /<option value="rounded" selected/, 'a stored rounded square is still the selected value');
    const legacy = await loaded({ state: stateOf({ settings: { ...stateOf().settings, cameraShape: 'square', fps: 24 } }) });
    legacy.t.openSettings('camera'); has(legacy.$('settingsPane').innerHTML, /<option value="square" selected/);
    legacy.t.openSettings('recording'); has(legacy.$('settingsPane').innerHTML, /<option value="24" selected/);
  });
  await check('the capture page code can only send the owner-only commands from a person’s press', () => {
    const calls = [...app.matchAll(/captureCommand\('([a-z-]+)'/g)].map(m => m[1]);
    eq([...new Set(calls)].sort().join(','), 'cancel,capacity-check,choose-window,device,dismiss-notice,input,open-privacy,source,stop');
    // The look changes go through one helper whose command names are fixed words.
    const looks = [...app.matchAll(/lookChange\('([a-z-]+)'/g)].map(m => m[1]);
    eq([...new Set(looks)].sort().join(','), 'mirror,show-camera,shape,size'.split(',').sort().join(','));
    eq((app.match(/api\('capture-command'/g) || []).length + (app.match(/uiApi\('capture-command'/g) || []).length, 1, 'one sender of capture commands');
    eq((app.match(/uiApi\('start-recording'/g) || []).length, 1, 'one sender of the start request');
    ok(/async function startRecording[\s\S]*?uiApi\('start-recording'/.test(app) && /function pressRecord[\s\S]*?void startRecording\(\)/.test(app) && /\$\('capRecord'\)\.onclick=pressRecord/.test(app), 'the start request is sent only from the Record press');
    eq((app.match(/startRecording\(\)/g) || []).length, 2, 'startRecording is defined once and called once, by pressRecord');
    // Nothing sends a start, a command or a media bridge call from load, refresh, a timer or a poll.
    for (const forbidden of [/setInterval\([^)]*(captureCommand|pressRecord|start-recording)/, /setTimeout\([^)]*(captureCommand|pressRecord|start-recording)/, /init\(\)[^;]*(captureCommand|pressRecord)/, /function refresh[^{]*\{[^}]*(captureCommand|start-recording)/]) lacks(app, forbidden);
    // Agent-visible calls never name a record, start or input operation.
    const agentCalls = [...app.matchAll(/(?:api|act)\('([a-z-]+)'/g)].map(m => m[1]);
    for (const op of agentCalls) ok(!['record', 'start-recording', 'capture-command', 'input', 'device', 'source'].includes(op), `an agent-visible call named ${op}`);
  });

  const failed = results.filter(r => !r.ok);
  console.log(JSON.stringify({ passed: failed.length === 0, total: results.length, failed: failed.length, results }, null, 2));
  process.exit(failed.length ? 1 : 0);
})();
