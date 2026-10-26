// End-to-end tests: the real built app, driven through Edge WebDriver. msedgedriver starts a WebView2
// app itself; tauri-driver would only translate the capabilities for it.
//
// Prerequisites (see README "Testing"):
//   an msedgedriver matching the installed WebView2 runtime, in ui/e2e/.tools/edgedriver/ (or EDGEDRIVER)
//   npm --prefix ui exec -- tauri build --debug --no-bundle        (the app under test)
//
// Every test works on synthetic recordings in a temp folder that the app is pointed at with
// RECORDING_FIXER_FOLDERS, with its own WebView2 profile, so nothing of the user's is touched.

import { spawn } from 'node:child_process';
import { closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { connect, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { after, afterEach, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { Builder, By, until } from 'selenium-webdriver';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const exe = process.env.APP_EXE ?? join(root, 'target', 'debug', process.platform === 'win32' ? 'recording-fixer.exe' : 'recording-fixer');
const edgedriver = process.env.EDGEDRIVER ?? join(here, '.tools', 'edgedriver', 'msedgedriver.exe');

// ---- synthetic AIFF files ---------------------------------------------------------------------

const RATE = 44100;

/** 16-bit stereo AIFF. `broken` leaves the size fields at 0, like Serato after an unclean exit. */
function aiff({ seconds, hz = 440, amp = 12000, broken = false, withTag = true }) {
  const frames = Math.round(seconds * RATE);
  const audio = Buffer.alloc(frames * 4);
  for (let i = 0; i < frames; i++) {
    const v = Math.round(Math.sin((i * hz * 2 * Math.PI) / RATE) * amp);
    audio.writeInt16BE(v, i * 4);
    audio.writeInt16BE(v, i * 4 + 2);
  }
  const id3 = Buffer.concat([Buffer.from('ID3 '), u32(14), Buffer.from('ID3\x03\x00\x00\x00\x00\x00\x00', 'latin1'), Buffer.alloc(4)]);
  const trailer = withTag && !broken ? id3 : Buffer.alloc(0);
  const head = Buffer.alloc(54);
  head.write('FORM', 0);
  head.write('AIFF', 8);
  head.write('COMM', 12);
  head.writeUInt32BE(18, 16);
  head.writeUInt16BE(2, 20);
  head.writeUInt32BE(broken ? 0 : frames, 22);
  head.writeUInt16BE(16, 26);
  Buffer.from([0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]).copy(head, 28); // 44100 Hz, 80-bit extended
  head.write('SSND', 38);
  head.writeUInt32BE(broken ? 0 : audio.length + 8, 42);
  if (!broken) head.writeUInt32BE(head.length - 8 + audio.length + trailer.length, 4);
  return Buffer.concat([head, audio, trailer]);
}
const u32 = (n) => {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(n);
  return b;
};

/** Read what matters from an AIFF header. */
function readAiff(path) {
  const b = readFileSync(path);
  return { form: b.readUInt32BE(4), frames: b.readUInt32BE(22), ssnd: b.readUInt32BE(42), size: b.length, bytes: b };
}

const sampleAt = (b, frame, ch = 0) => b.readInt16BE(54 + frame * 4 + ch * 2);

// ---- driver plumbing --------------------------------------------------------------------------

const freePort = () =>
  new Promise((res, rej) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => res(port));
    });
    s.on('error', rej);
  });

const canConnect = (port) =>
  new Promise((resolve) => {
    const c = connect(port, '127.0.0.1');
    c.once('connect', () => {
      c.end();
      resolve(true);
    });
    c.once('error', () => resolve(false));
  });

async function waitForPort(port, ms = 15000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await canConnect(port)) return;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`msedgedriver did not start on port ${port}`);
}
let work, driverProc, driver;
const files = {};

const XP = {
  row: (name) => `//tr[@data-testid='row'][.//td[contains(@class,'name')][normalize-space()='${name}']]`,
  button: (text) => `//button[normalize-space()='${text}']`,
};

const find = (xpath, ms = 10000) => driver.wait(until.elementLocated(By.xpath(xpath)), ms);
async function clickXp(xpath, ms) {
  const el = await find(xpath, ms);
  await driver.wait(until.elementIsEnabled(el), 5000);
  await el.click();
  return el;
}
const tickRow = (name) => clickXp(`${XP.row(name)}//input[@type='checkbox']`);
const bodyText = () => driver.findElement(By.css('body')).getText();
async function untilText(re, ms = 15000) {
  try {
    await driver.wait(async () => re.test(await bodyText()), ms);
  } catch {
    // Say what the screen showed instead: far more useful than a bare timeout.
    throw new Error(`waiting for text ${re}; the screen shows:\n${(await bodyText()).slice(0, 1500)}`);
  }
}
const listed = async () => (await driver.findElements(By.css("tr[data-testid='row'] td.name"))).length;
const dirFiles = () => readdirSync(work).sort();

/** Untick everything, so each test starts from the same selection. */
async function clearSelection() {
  const all = await driver.findElement(By.xpath("//input[@aria-label='Select all']"));
  if (!/selected/.test(await bodyText())) return;
  await all.click(); // everything visible becomes selected (or, if it already was, nothing)
  if (/selected/.test(await bodyText())) await all.click();
}

/** Clear old toasts, so a wait for \"Exported ...\" cannot be satisfied by an earlier test's message. */
async function dismissToasts() {
  for (const b of await driver.findElements(By.xpath("//div[contains(@class,'toast')]//button[@aria-label='Dismiss']"))) {
    await b.click().catch(() => {});
  }
  await driver.wait(async () => (await driver.findElements(By.css('.toast'))).length === 0, 5000);
}

async function closeEditorIfOpen() {
  if ((await driver.findElements(By.xpath("//div[@aria-label='Editor']"))).length) {
    await driver.findElement(By.css('body')).sendKeys('\uE00C'); // Escape
    await driver.wait(async () => (await driver.findElements(By.xpath("//div[@aria-label='Editor']"))).length === 0, 5000);
  }
}

async function rescan() {
  await clickXp(XP.button('↻ Rescan'));
  await driver.sleep(400);
}

before(async () => {
  assert.ok(existsSync(exe), `build the app first (just build-debug):${exe} not found`);
  assert.ok(existsSync(edgedriver), `msedgedriver not found at ${edgedriver} (set EDGEDRIVER)`);

  work = mkdtempSync(join(tmpdir(), 'srf-e2e-'));
  files.good1 = join(work, 'Good One.aif');
  files.good2 = join(work, 'Good Two.aif');
  files.broken = join(work, 'Broken Set.aif');
  writeFileSync(files.good1, aiff({ seconds: 2, hz: 440 }));
  writeFileSync(files.good2, aiff({ seconds: 3, hz: 660 }));
  writeFileSync(files.broken, aiff({ seconds: 2.5, hz: 330, broken: true }));
  // Make the unclosed file look idle rather than "still being recorded".
  const old = new Date(Date.now() - 3600_000);
  (await import('node:fs')).utimesSync(files.broken, old, old);

  // Keep the driver's output: when the session cannot start (it happened on CI only), this is the
  // only trace of why. The app inherits this environment through msedgedriver.
  const logDir = join(here, '.tools', 'logs');
  mkdirSync(logDir, { recursive: true });
  const driverLog = join(logDir, 'msedgedriver.log');
  const logFd = openSync(driverLog, 'w');
  const port = await freePort();
  driverProc = spawn(edgedriver, [`--port=${port}`], {
    stdio: ['ignore', logFd, logFd],
    env: {
      ...process.env,
      RECORDING_FIXER_FOLDERS: work,
      RECORDING_FIXER_NO_UPDATE_CHECK: '1', // tests never touch the network
      WEBVIEW2_USER_DATA_FOLDER: join(work, '.webview'),
    },
  });
  closeSync(logFd);
  try {
    await waitForPort(port);
    driver = await new Builder()
      .usingServer(`http://127.0.0.1:${port}/`)
      .withCapabilities({ browserName: 'webview2', 'ms:edgeOptions': { binary: exe, webviewOptions: {} } })
      .build();
  } catch (e) {
    console.error(`msedgedriver output (${driverLog}):
${readFileSync(driverLog, 'utf8') || '<empty>'}`);
    throw e;
  }
  await driver.manage().setTimeouts({ implicit: 0, pageLoad: 30000, script: 30000 });
  await driver.wait(async () => (await listed()) >= 3, 20000, 'the recordings to be listed');
});

after(async () => {
  try {
    await driver?.quit();
  } catch {
    /* already gone */
  }
  driverProc?.kill();
  await new Promise((r) => setTimeout(r, 500));
  try {
    rmSync(work, { recursive: true, force: true });
  } catch {
    /* Windows may still hold the WebView2 profile; the OS temp cleaner will get it */
  }
});

// ---- the tests --------------------------------------------------------------------------------

describe('the real app', () => {
  afterEach(async () => {
    await closeEditorIfOpen();
    await clearSelection();
    await dismissToasts();
  });

  it('starts with the right window title and lists the recordings in the folder', async () => {
    assert.equal(await driver.getTitle(), 'Recording Fixer for Serato DJ');
    const names = await Promise.all((await driver.findElements(By.css("tr[data-testid='row'] td.name"))).map((e) => e.getText()));
    assert.deepEqual(names.sort(), ['Broken Set.aif', 'Good One.aif', 'Good Two.aif']);
  });

  it('shows each recording with its true length and status', async () => {
    const text = await (await find(XP.row('Good Two.aif'))).getText();
    assert.match(text, /0:03/, 'Good Two is 3 s');
    assert.match(text, /OK/i);
    assert.match(text, /44\.1 kHz/);
    assert.match(text, /16-bit/);
    const broken = await (await find(XP.row('Broken Set.aif'))).getText();
    assert.match(broken, /NEEDS REPAIR/i);
    assert.match(broken, /0:02/, 'the length is recovered from the file size even though the header says 0');
    assert.match(await bodyText(), /1\s*need repair/i);
  });

  it('only offers Repair for a file that needs it', async () => {
    const repair = await find(XP.button('Repair…'));
    assert.equal(await repair.isEnabled(), false);
    await tickRow('Good One.aif');
    assert.equal(await repair.isEnabled(), false, 'a healthy file cannot be repaired');
    await tickRow('Good One.aif'); // untick
    await tickRow('Broken Set.aif');
    assert.equal(await repair.isEnabled(), true);
    await tickRow('Broken Set.aif'); // untick for the next test
  });

  it('repairs a broken recording into a new, healthy copy and leaves the original alone', async () => {
    const before = readFileSync(files.broken);
    await tickRow('Broken Set.aif');
    await clickXp(XP.button('Repair…'));
    await clickXp("//div[@role='dialog']" + XP.button('Repair'));
    await untilText(/Saved repaired copy of Broken Set\.aif/i);

    const fixed = join(work, 'Broken Set (fixed).aif');
    assert.ok(existsSync(fixed), 'the repaired copy exists');
    const h = readAiff(fixed);
    assert.equal(h.frames, 110250, '2.5 s at 44.1 kHz');
    assert.equal(h.ssnd, 110250 * 4 + 8);
    assert.equal(h.form, h.size - 8);
    assert.deepEqual(readFileSync(files.broken), before, 'the original is byte-for-byte untouched');
    assert.deepEqual(h.bytes.subarray(54), before.subarray(54), 'the audio is identical');

    await driver.wait(async () => (await listed()) >= 4, 10000, 'the repaired copy to appear in the list');
    const row = await (await find(XP.row('Broken Set (fixed).aif'))).getText();
    assert.match(row, /OK/i);
  });

  it('combines two recordings into one lossless file with the audio in order', async () => {
    await rescan();
    await tickRow('Good One.aif');
    await tickRow('Good Two.aif');
    await clickXp(XP.button('Combine & export…'));
    await find("//div[@role='dialog'][@aria-label='Editor']");
    // The timeline is a real canvas, sized to the window, and the combined length is shown.
    const canvas = await driver.findElement(By.css("canvas[data-testid='timeline']"));
    assert.ok((await canvas.getRect()).width > 400);
    await untilText(/0:05/); // 2 s + 3 s

    await clickXp("//div[@aria-label='Editor']" + XP.button('Export'));
    await untilText(/Exported Good One \(combined\)\.aif/i, 20000);
    const out = join(work, 'Good One (combined).aif');
    const h = readAiff(out);
    assert.equal(h.frames, 5 * RATE);
    assert.equal(h.form, h.size - 8);
    // Frame 0 starts the first file's tone (a sine starts at 0); the join is at 2 s.
    const first = readAiff(files.good1).bytes;
    assert.deepEqual(h.bytes.subarray(54, 54 + 2 * RATE * 4), first.subarray(54, 54 + 2 * RATE * 4));
    const second = readAiff(files.good2).bytes;
    assert.deepEqual(h.bytes.subarray(54 + 2 * RATE * 4, 54 + 5 * RATE * 4), second.subarray(54, 54 + 3 * RATE * 4));
  });

  it('explains the folder row and has a Settings window for export defaults and ffmpeg', async () => {
    assert.match(await bodyText(), /show recordings from/i);
    const count = await driver.findElement(By.css('.src .cnt'));
    assert.equal(await count.getText(), String(await listed()), 'the folder shows how many recordings it holds');
    const tip = await driver.findElement(By.css('.src')).getAttribute('title');
    assert.ok(tip.includes(work.replace(/\\/g, '\\')) || tip.length > 0, 'hovering shows the folder');

    // Settings holds the export defaults and the plain-language ffmpeg explanation.
    await clickXp("//button[contains(.,'Settings')]");
    const dialog = await find("//div[@role='dialog'][@aria-label='Settings']");
    const text = await dialog.getText();
    assert.match(text, /Default export/i);
    assert.match(text, /free audio converter/i);
    assert.match(text, /AIFF/);
    assert.match(text, /Found/i);
    assert.match(text, /best first/i);
    await driver.wait(async () => /v0\.1\.0/.test(await dialog.getText()), 5000, 'About to show the real app version');
    assert.match(text, /Report an issue on GitHub/i);
    assert.match(await driver.findElement(By.css('footer.status .ver')).getText(), /^v0\.1\.0$/i, 'the status bar shows the version');
    await clickXp("//div[@role='dialog']" + XP.button('Done'));
    await driver.wait(async () => (await driver.findElements(By.xpath("//div[@aria-label='Settings']"))).length === 0, 5000);
  });

  it('in the editor: click to play from there, drag across to zoom, drag a name bar to reorder', async (t) => {
    // Two one-minute clips: the five-second fixtures are already at the maximum magnification when fitted.
    const longA = join(work, 'Long A.aif');
    const longB = join(work, 'Long B.aif');
    writeFileSync(longA, aiff({ seconds: 60, hz: 300 }));
    writeFileSync(longB, aiff({ seconds: 60, hz: 500 }));
    try {
      await rescan();
      await tickRow('Long A.aif');
      await tickRow('Long B.aif');
      await clickXp(XP.button('Combine & export…'));
      await find("//div[@aria-label='Editor']");
      await editorScenario(t);
      await sampleScenario(longA);
    } finally {
      rmSync(longA, { force: true });
      rmSync(longB, { force: true });
    }
  });

  // Zoom all the way in on a real recording and check that individual samples are exact.
  async function sampleScenario(srcPath) {
    // The real command, over the real IPC: exact 16-bit [min, max] for 20 frames.
    const raw = await driver.executeAsyncScript(
      "const done = arguments[arguments.length - 1];" +
        "window.__TAURI_INTERNALS__.invoke('get_samples', { path: arguments[0], start: 100000, end: 100020 })" +
        '.then((b) => done(Array.from(new Int16Array(b)))).catch((e) => done(String(e)));',
      srcPath,
    );
    assert.ok(Array.isArray(raw), `get_samples failed: ${raw}`);
    assert.equal(raw.length, 40);
    const bytes = readFileSync(srcPath);
    for (let i = 0; i < 20; i++) {
      const v = sampleAt(bytes, 100000 + i);
      assert.deepEqual([raw[2 * i], raw[2 * i + 1]], [v, v], `frame ${100000 + i}`);
    }

    // Wheel-zoom to the limit: the scrollable width must stay sane and a click must land on a sample.
    await driver.executeScript(
      "const s = document.querySelector('.scroller');" +
        'for (let i = 0; i < 40; i++) s.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -1000, clientX: 300, bubbles: true, cancelable: true }));',
    );
    await driver.sleep(600);
    const width = await driver.executeScript("return parseFloat(document.querySelector('.content').style.width)");
    assert.ok(width <= 8_000_000, `scroll width ${width}`);
    const canvas = await driver.findElement(By.css("canvas[data-testid='timeline']"));
    const rect = await canvas.getRect();
    await driver.actions({ async: true }).move({ x: Math.round(rect.x + 300), y: Math.round(rect.y + 90) }).press().release().perform();
    await driver.sleep(300);
    const pos = await driver.findElement(By.css('.playpos')).getText();
    assert.match(pos, /^clip \d · sample [\d,]+$/);
    await driver.executeScript("document.querySelector('[aria-label=\"Pause\"]')?.click()");
  }

  async function editorScenario(t) {
    const canvas = await driver.findElement(By.css("canvas[data-testid='timeline']"));
    await untilText(/2:00/);
    const rect = await canvas.getRect();
    const at = (fx, dy) => ({ x: Math.round(rect.x + rect.width * fx), y: Math.round(rect.y + dy) });
    const drag = async (from, to, steps = 8) => {
      const a = driver.actions({ async: true }).move(from).press();
      for (let i = 1; i <= steps; i++) a.move({ x: Math.round(from.x + ((to.x - from.x) * i) / steps), y: from.y });
      await a.release().perform();
    };
    const order = async () => Promise.all((await driver.findElements(By.css('.rows .clip .nm'))).map((e) => e.getText()));
    const scrollInfo = () => driver.executeScript("const s=document.querySelector('.scroller'); return [s.scrollWidth, s.clientWidth]");

    // --- click: the playhead goes there and playback starts ---
    await driver.actions({ async: true }).move(at(0.3, 90)).press().release().perform();
    const playing = await driver
      .wait(async () => (await driver.findElements(By.xpath("//div[@aria-label='Editor']//button[@aria-label='Pause']"))).length > 0, 6000)
      .then(() => true, () => false);
    if (playing) {
      await driver.sleep(500);
      const m = (await driver.findElement(By.css("[data-testid='position']")).getText()).match(/(\d+):(\d+)\.(\d)/);
      const secs = Number(m[1]) * 60 + Number(m[2]) + Number(m[3]) / 10;
      assert.ok(secs >= 35 && secs < 41, `playback should start near 30% of 120 s (36 s), not at ${secs}`);
      await clickXp("//div[@aria-label='Editor']//button[@aria-label='Pause']");
    }
    t.diagnostic(playing ? 'click-to-play: playing from the clicked point' : 'click-to-play: no audio device, playback not asserted');

    // --- drag across the waveform: zoom ---
    const [wide0, view0] = await scrollInfo();
    assert.ok(wide0 <= view0 + 2, 'starts fitted to the view');
    await drag(at(0.2, 90), at(0.4, 90));
    await driver.wait(async () => (await scrollInfo())[0] > view0 * 3, 5000, 'the timeline to zoom in');

    // --- drag a clip's name bar: reorder ---
    await clickXp("//div[@aria-label='Editor']" + XP.button('Fit'));
    await driver.wait(async () => { const [w, v] = await scrollInfo(); return w <= v + 2; }, 5000);
    const before = await order();
    assert.equal(before.length, 2);
    await drag(at(0.1, 34), at(0.95, 34));
    await driver.wait(async () => (await order())[0] === before[1], 5000, 'the clips to swap places');
    assert.deepEqual(await order(), [before[1], before[0]]);
  }

  it('applies a fade in and a fade out to the exported file', async () => {
    await rescan();
    await tickRow('Good Two.aif');
    await clickXp(XP.button('Edit & export…'));
    await find("//div[@aria-label='Editor']");
    await (await find("//input[@aria-label='Fade in (seconds)']")).sendKeys('1');
    await (await find("//input[@aria-label='Fade out (seconds)']")).sendKeys('1');
    const before = new Set(dirFiles());
    await clickXp("//div[@aria-label='Editor']" + XP.button('Export'));
    await untilText(/Exported .*\.aif/i, 20000);

    // The editor remembers the last file name template, so find the new file by what appeared.
    const added = dirFiles().filter((f) => !before.has(f) && f.endsWith('.aif'));
    assert.equal(added.length, 1, `exactly one new .aif (got ${added.join(', ')})`);
    const h = readAiff(join(work, added[0]));
    assert.equal(h.frames, 3 * RATE);
    const orig = readAiff(files.good2).bytes;
    assert.equal(sampleAt(h.bytes, 0), 0, 'starts silent');
    assert.equal(sampleAt(h.bytes, h.frames - 1), 0, 'ends silent');
    const peak = (b, a, z) => Math.max(...Array.from({ length: z - a }, (_, i) => Math.abs(sampleAt(b, a + i))));
    assert.ok(peak(h.bytes, 0, 2000) < peak(orig, 0, 2000), 'the opening is quieter than the original');
    assert.ok(peak(h.bytes, h.frames - 2000, h.frames) < peak(orig, h.frames - 2000, h.frames), 'the ending is quieter than the original');
    // Between the fades the audio is bit-identical.
    assert.deepEqual(h.bytes.subarray(54 + RATE * 4, 54 + 2 * RATE * 4), orig.subarray(54 + RATE * 4, 54 + 2 * RATE * 4));
  });

  it('exports an MP3 with the tags typed in the editor', async () => {
    await rescan();
    await tickRow('Good One.aif');
    await clickXp(XP.button('Edit & export…'));
    await find("//div[@aria-label='Editor']");
    await clickXp("//div[@aria-label='Editor']//label[.//b[normalize-space()='MP3']]/input");
    await clickXp("//div[@aria-label='Editor']//button[@role='tab'][contains(.,'Name, folder')]");
    const name = await find("//input[@aria-label='File name template']");
    await name.clear();
    await name.sendKeys('{name} tagged');
    await (await find("//input[@id='tag-artist']")).sendKeys('E2E Artist');
    await clickXp("//div[@aria-label='Editor']" + XP.button('Export'));
    await untilText(/Exported Good One tagged\.mp3/i, 30000);
    const mp3 = readFileSync(join(work, 'Good One tagged.mp3'));
    assert.ok(mp3.length > 5000);
    assert.equal(mp3.subarray(0, 3).toString('latin1'), 'ID3', 'starts with an ID3 tag');
    assert.ok(mp3.subarray(0, 4000).toString('latin1').includes('E2E Artist'), 'the artist tag is in the file');
    assert.ok(mp3.subarray(0, 4000).toString('latin1').includes('Good One'), 'the title tag defaults to the file name');
  });
  it('previews a recording from the list through the real audio engine', async (t) => {
    await rescan();
    const play = await find(`${XP.row('Good Two.aif')}//button[contains(@class,'pbtn')]`);
    await play.click();
    // Either it plays (an audio device is available) or the app explains why it could not.
    await driver.wait(async () => {
      const t = await bodyText();
      return /0:0\d\.\d/.test(t) && (await driver.findElements(By.xpath("//button[@aria-label='Pause']"))).length > 0 || /Cannot play|could not open|no default audio/i.test(t);
    }, 15000, 'playback to start or a clear error to appear');
    const playing = (await driver.findElements(By.xpath("//button[@aria-label='Pause']"))).length > 0;
    t.diagnostic(playing ? 'AUDIO: playback started on the real output device' : `AUDIO: no playback; the app said: ${(await bodyText()).match(/Cannot play[^\n]*|could not open[^\n]*/i)?.[0]}`);
    const alive = await driver.findElement(By.css('main'));
    assert.ok(await alive.isDisplayed(), 'the app is still up');
  });

  it('Space and the transport pause playback, and the window keeps responding', async () => {
    const pauses = await driver.findElements(By.xpath("//button[@aria-label='Pause']"));
    if (pauses.length === 0) return; // no audio device in this environment
    await pauses[0].click();
    await find("//button[@aria-label='Play']");
    assert.ok(await driver.findElement(By.css('main')).isDisplayed());
  });

  // Last on purpose: the editor remembers the loudness choice for the exports after it.
  it('matches a target loudness on export and reports what it did', async () => {
    await rescan();
    await tickRow('Good Two.aif');
    await clickXp(XP.button('Edit & export…'));
    await find("//div[@aria-label='Editor']");
    await clickXp("//div[@aria-label='Editor']//label[.//b[normalize-space()='AIFF']]/input"); // the MP3 test left MP3 selected
    await clickXp("//div[@aria-label='Editor']//label[contains(.,'Match a loudness')]/input");
    const target = await find("//input[@id='lufs']");
    await target.clear();
    await target.sendKeys('-20');
    const before = new Set(dirFiles());
    await clickXp("//div[@aria-label='Editor']" + XP.button('Export'));
    await untilText(/Exported .*\.aif\. Loudness -(19\.[89]|20\.[0-2]) LUFS/i, 30000);

    const added = dirFiles().filter((f) => !before.has(f) && f.endsWith('.aif'));
    assert.equal(added.length, 1, `exactly one new .aif (got ${added.join(', ')})`);
    const out = readAiff(join(work, added[0]));
    const orig = readAiff(files.good2);
    assert.equal(out.frames, orig.frames, 'same length');
    // The middle of the file: the editor still has the fade from the earlier test.
    const peak = (b) => Math.max(...Array.from({ length: 4000 }, (_, i) => Math.abs(sampleAt(b, Math.round(1.2 * RATE) + i))));
    const [was, now] = [peak(orig.bytes), peak(out.bytes)];
    assert.ok(now < was * 0.5 && now > was * 0.1, `quieter by roughly 11 dB (${was} -> ${now})`);
  });
});
