#!/usr/bin/env node
/**
 * Measure every route the application can render at the widths that matter,
 * in a real browser, and report the ones that scroll sideways.
 *
 * `M1-06` asks whether the layout works at 320 CSS pixels. That cannot be
 * answered by a component test: jsdom has no layout engine, so a passing test
 * there says nothing about whether anything overflows and `scrollWidth` is not
 * even defined. It needs a browser that actually computes boxes, at a real
 * device width — so this drives one over the DevTools Protocol and measures.
 *
 * The browser is Chromium, found in the Playwright cache or named by `CHROME`.
 * There are no npm dependencies: Node 22 and later ship a global `WebSocket`,
 * and the protocol is spoken directly. That is deliberate — this repository's
 * frontend install is not something to add a browser-automation dependency to
 * for one measurement.
 *
 *   node frontend/scripts/measure-viewport.mjs --base http://127.0.0.1:8120 \
 *     --work <work-id> --chapter <chapter-id> [--sign-in]
 *
 * Exits 1 if any route scrolls horizontally at any measured width, and prints
 * the widest offending elements rather than only the count, because "it
 * overflows" is not a bug report.
 */

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const WIDTHS = [320, 360, 768, 1280];
const HEIGHT = 720;

// ---------------------------------------------------------------- arguments

function parseArgs(argv) {
  const args = { widths: WIDTHS, signIn: false, settle: 250 };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === '--sign-in') args.signIn = true;
    else if (flag.startsWith('--')) args[flag.slice(2)] = argv[++i];
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));

if (!args.base) {
  console.error('usage: measure-viewport.mjs --base <url> [--work <id>] [--chapter <id>] [--sign-in]');
  process.exit(2);
}

const BASE = args.base.replace(/\/+$/, '');
const SETTLE = Number(args.settle);

// ------------------------------------------------------------------- routes

/**
 * Every route the shell can resolve, with the ones that need an identifier
 * supplied by the caller.
 *
 * The planned and not-found routes are included on purpose: a page that says
 * "not built yet" is still a page a reader can land on at 320 pixels, and the
 * navigation that reaches it has to hold.
 */
const ROUTES = [
  ['/', 'home'],
  ['/sign-in', 'sign in'],
  ['/register', 'register'],
  ['/password-reset', 'password reset'],
  ['/account', 'account'],
  ['/pseud', 'own pseuds'],
  ...[args.handle ? [`/pseud/${args.handle}`, 'public profile'] : []],
  ['/write', 'writing desk'],
  ...[args.work ? [`/write/${args.work}`, 'work editor'] : []],
  ['/import', 'import'],
  ['/library', 'library'],
  ['/library/history', 'history'],
  ['/exports', 'exports'],
  ['/jobs', 'jobs'],
  ['/admin/jobs', 'admin jobs'],
  ...[args.work ? [`/works/${args.work}`, 'work page'] : []],
  ...[args.work && args.chapter ? [`/works/${args.work}/chapters/${args.chapter}`, 'reader'] : []],
  ['/discover', 'planned: discover'],
  ['/no-such-route', 'not found'],
];

// --------------------------------------------------------------- the browser

function findChrome() {
  if (process.env.CHROME) return process.env.CHROME;
  const cache = join(process.env.HOME ?? '', '.cache', 'ms-playwright');
  if (existsSync(cache)) {
    for (const entry of readdirSync(cache).sort().reverse()) {
      if (!entry.startsWith('chromium-')) continue;
      for (const sub of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
        const candidate = join(cache, entry, sub);
        if (existsSync(candidate)) return candidate;
      }
    }
  }
  throw new Error('no Chromium found; set CHROME to a browser binary');
}

class Devtools {
  #socket;
  #nextId = 1;
  #pending = new Map();
  #listeners = new Map();

  static async launch(chrome) {
    const profile = mkdtempSync(join(tmpdir(), 'lorehaven-viewport-'));
    const child = spawn(
      chrome,
      [
        '--headless=new',
        '--disable-gpu',
        '--no-sandbox',
        '--no-first-run',
        '--disable-extensions',
        '--hide-scrollbars',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        'about:blank',
      ],
      { stdio: ['ignore', 'ignore', 'pipe'] },
    );

    const endpoint = await new Promise((resolve, reject) => {
      let buffered = '';
      const timer = setTimeout(() => reject(new Error('Chromium did not report a debugging port')), 30_000);
      child.stderr.on('data', (chunk) => {
        buffered += chunk.toString();
        const match = buffered.match(/DevTools listening on (ws:\/\/\S+)/);
        if (match) {
          clearTimeout(timer);
          resolve(match[1]);
        }
      });
      child.on('exit', (code) => {
        clearTimeout(timer);
        reject(new Error(`Chromium exited with ${code}`));
      });
    });

    const devtools = new Devtools(endpoint, child, profile);
    await devtools.#connect();
    return devtools;
  }

  constructor(endpoint, child, profile) {
    this.endpoint = endpoint;
    this.child = child;
    this.profile = profile;
    this.version = null;
  }

  #connect() {
    return new Promise((resolve, reject) => {
      this.#socket = new WebSocket(this.endpoint);
      this.#socket.addEventListener('open', () => resolve());
      this.#socket.addEventListener('error', () => reject(new Error('could not open the DevTools socket')));
      this.#socket.addEventListener('message', (event) => this.#receive(JSON.parse(event.data)));
    });
  }

  #receive(message) {
    if (message.id && this.#pending.has(message.id)) {
      const { resolve, reject } = this.#pending.get(message.id);
      this.#pending.delete(message.id);
      if (message.error) reject(new Error(`${message.error.message} (${message.error.code})`));
      else resolve(message.result);
      return;
    }
    const key = `${message.sessionId ?? ''}:${message.method}`;
    for (const listener of this.#listeners.get(key) ?? []) listener(message.params);
    for (const listener of this.#listeners.get(`*:${message.method}`) ?? []) listener(message.params, message.sessionId);
  }

  send(method, params = {}, sessionId) {
    const id = this.#nextId++;
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      this.#socket.send(JSON.stringify(payload));
    });
  }

  /** Subscribe to an event; the returned function unsubscribes. */
  on(method, listener, sessionId) {
    const key = `${sessionId ?? ''}:${method}`;
    this.#listeners.set(key, [...(this.#listeners.get(key) ?? []), listener]);
    return () => {
      this.#listeners.set(key, (this.#listeners.get(key) ?? []).filter((entry) => entry !== listener));
    };
  }

  once(method, sessionId) {
    return new Promise((resolve) => {
      const key = `${sessionId ?? ''}:${method}`;
      const listener = (params) => {
        const list = this.#listeners.get(key) ?? [];
        this.#listeners.set(key, list.filter((entry) => entry !== listener));
        resolve(params);
      };
      this.#listeners.set(key, [...(this.#listeners.get(key) ?? []), listener]);
    });
  }

  async close() {
    try {
      this.#socket?.close();
      this.child.kill('SIGKILL');
    } finally {
      rmSync(this.profile, { recursive: true, force: true });
    }
  }
}

/** What actually overflows, in the order a reader would meet it. */
const MEASURE = `(() => {
  const root = document.documentElement;
  // The reference is the layout viewport. Under mobile emulation innerWidth is
  // larger than it, so measuring against innerWidth would both miss offenders
  // between the two widths and pass pages that overflow.
  const vw = root.clientWidth;
  const wide = [];
  const offCanvas = [];
  // An element inside a deliberate horizontal scroller — the narrow-screen
  // navigation strip is one — is not overflowing the document, and counting it
  // would bury the real offenders in noise.
  const inScroller = (element) => {
    for (let n = element.parentElement; n && n !== document.body; n = n.parentElement) {
      const overflow = getComputedStyle(n).overflowX;
      if (overflow === 'auto' || overflow === 'scroll' || overflow === 'hidden') return true;
    }
    return false;
  };
  for (const element of document.querySelectorAll('body *')) {
    const rect = element.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) continue;
    const style = getComputedStyle(element);
    if (style.visibility === 'hidden' || style.display === 'none') continue;
    if (inScroller(element)) continue;
    const label = element.tagName.toLowerCase()
      + (element.id ? '#' + element.id : '')
      + (element.classList.length ? '.' + [...element.classList].slice(0, 2).join('.') : '');
    if (rect.right > vw + 1) {
      wide.push({ label, left: Math.round(rect.left), right: Math.round(rect.right), width: Math.round(rect.width) });
    } else if (rect.left < -1) {
      offCanvas.push({ label, left: Math.round(rect.left), right: Math.round(rect.right) });
    }
  }
  wide.sort((a, b) => b.right - a.right);
  return {
    // The comparison is against the *layout viewport*, not innerWidth. Under
    // mobile emulation innerWidth is inflated by the device viewport (320
    // reports 355), so scrollWidth <= innerWidth is true whatever the page
    // does — which is exactly how a hand measurement once recorded that the
    // document did not scroll horizontally at 320 when it did.
    layoutWidth: vw,
    innerWidth: window.innerWidth,
    scrollWidth: root.scrollWidth,
    bodyScrollWidth: document.body.scrollWidth,
    documentHeight: root.scrollHeight,
    overflowing: wide.length,
    worst: wide.slice(0, 6),
    offCanvas: offCanvas.slice(0, 4),
    title: document.title,
  };
})()`;

// ---------------------------------------------------------------------- main

const devtools = await Devtools.launch(findChrome());
const { targetId } = await devtools.send('Target.createTarget', { url: 'about:blank' });
const { sessionId } = await devtools.send('Target.attachToTarget', { targetId, flatten: true });

await devtools.send('Page.enable', {}, sessionId);
await devtools.send('Runtime.enable', {}, sessionId);
await devtools.send('Network.enable', {}, sessionId);

/*
 * Whether the page has stopped fetching.
 *
 * The shell boots and then asks the server who is signed in, so measuring a
 * fixed 700 ms after `load` measures whichever of those two states the machine
 * happened to reach — and the signed-in header is 179 CSS pixels wider than the
 * signed-out one. That is how a run can report a different overflow on two
 * pages showing the same header. Waiting for the network to go quiet, and then
 * a beat for the render that follows it, measures the settled page.
 */
let inFlight = 0;
let lastActivity = Date.now();
devtools.on('Network.requestWillBeSent', () => {
  inFlight += 1;
  lastActivity = Date.now();
}, sessionId);
for (const done of ['Network.loadingFinished', 'Network.loadingFailed']) {
  devtools.on(done, () => {
    inFlight = Math.max(0, inFlight - 1);
    lastActivity = Date.now();
  }, sessionId);
}

async function quiet(quietMs = 400, capMs = 8000) {
  const started = Date.now();
  while (Date.now() - started < capMs) {
    if (inFlight === 0 && Date.now() - lastActivity >= quietMs) return true;
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  return false;
}

async function navigate(url) {
  inFlight = 0;
  lastActivity = Date.now();
  const loaded = devtools.once('Page.loadEventFired', sessionId);
  await devtools.send('Page.navigate', { url }, sessionId);
  await loaded;
  const settled = await quiet();
  if (!settled) console.log(`  (note) ${url} never went quiet; measured anyway`);
  // A short extra beat: Svelte patches the DOM after the last response lands,
  // and a fetch that resolved is not yet a rendered header.
  await new Promise((resolve) => setTimeout(resolve, SETTLE));
  const { result } = await devtools.send(
    'Runtime.evaluate',
    { expression: 'document.readyState', returnByValue: true },
    sessionId,
  );
  return result.value;
}

if (args.signIn) {
  await navigate(`${BASE}/sign-in`);
  const email = process.env.LH_EMAIL ?? 'dev@lorehaven.local';
  const password = process.env.LH_PASSWORD;
  if (!password) {
    console.error('--sign-in needs LH_PASSWORD in the environment (this instance\'s development password)');
    await devtools.close();
    process.exit(2);
  }
  const outcome = await devtools.send(
    'Runtime.evaluate',
    {
      awaitPromise: true,
      returnByValue: true,
      expression: `(async () => {
        const token = document.cookie.match(/(?:^|; )lorehaven_csrf=([^;]+)/)?.[1];
        // The API base is /api/v1, and the CSRF cookie is issued by this very
        // request (sign-in is not yet cookie-authenticated), so the header is
        // sent when one is already present and omitted when it is not.
        const response = await fetch('/api/v1/auth/login', {
          method: 'POST',
          headers: { 'content-type': 'application/json', ...(token ? { 'x-csrf-token': decodeURIComponent(token) } : {}) },
          body: JSON.stringify({ email: ${JSON.stringify(email)}, password: ${JSON.stringify(password)} }),
        });
        return { status: response.status, body: (await response.text()).slice(0, 200) };
      })()`,
    },
    sessionId,
  );
  const { status, body } = outcome.result.value;
  if (status !== 200) {
    console.error(`sign in failed with ${status}: ${body}`);
    await devtools.close();
    process.exit(2);
  }
  console.log(`signed in as ${email}\n`);
}

const failures = [];

for (const width of WIDTHS) {
  // A phone for the narrow widths and a desktop layout viewport for the rest,
  // so the `meta viewport` and the mobile stylesheet are exercised the way a
  // reader's device would exercise them.
  await devtools.send(
    'Emulation.setDeviceMetricsOverride',
    { width, height: HEIGHT, deviceScaleFactor: 1, mobile: width <= 480 },
    sessionId,
  );

  console.log(`${width} CSS px${width <= 480 ? ' (mobile emulation)' : ''}`);
  console.log('-'.repeat(78));

  for (const [path, label] of ROUTES) {
    await navigate(`${BASE}${path}`);
    const { result } = await devtools.send('Runtime.evaluate', { expression: MEASURE, returnByValue: true }, sessionId);
    const m = result.value;
    const ok = m.scrollWidth <= m.layoutWidth;
    if (!ok) failures.push({ width, path, label, m });
    const verdict = ok ? 'ok ' : 'OVERFLOW';
    const over = m.scrollWidth > m.layoutWidth ? ` (+${m.scrollWidth - m.layoutWidth})` : '';
    console.log(
      `  ${verdict.padEnd(9)} ${String(path).padEnd(44)} scrollWidth=${String(m.scrollWidth).padStart(4)} of ${String(m.layoutWidth).padStart(4)}${over}`,
    );
    if (!ok) {
      for (const element of m.worst) {
        console.log(`              ${element.label} spans ${element.left}..${element.right} (${element.width} wide)`);
      }
    }
    if (m.offCanvas.length > 0 && ok) {
      console.log(`              (${m.offCanvas.length} element(s) parked off-canvas, not counted as overflow)`);
    }
  }
  console.log('');
}

await devtools.close();

if (failures.length > 0) {
  console.log(`${failures.length} route/width pair(s) scroll horizontally:`);
  for (const failure of failures) {
    console.log(`  ${failure.width}px  ${failure.path}  (${failure.label})`);
    for (const element of failure.m.worst) console.log(`      ${element.label} ${element.left}..${element.right}`);
  }
  process.exit(1);
}

console.log(`all ${ROUTES.length} routes hold at ${WIDTHS.join(', ')} CSS pixels`);
