// Takes the pictures used in the README and the user guide, from the real app, so they can be redone
// whenever the interface changes:
//   npm --prefix ui exec -- tauri build --debug --no-bundle          (the app to photograph)
//   node ui/e2e/screenshots.mjs                                       (writes docs/images/*.png)
//
// The recordings shown are made from the committed synthetic test set (tests/fixtures/beeps-set.aif):
// nothing of anyone's real music or library appears. APP_EXE and EDGEDRIVER override the defaults.
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Builder, By, until } from 'selenium-webdriver';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const exe = process.env.APP_EXE ?? join(root, 'target', 'debug', process.platform === 'win32' ? 'recording-fixer.exe' : 'recording-fixer');
const edgedriver = process.env.EDGEDRIVER ?? join(here, '.tools', 'edgedriver', 'msedgedriver.exe');
const outDir = join(root, 'docs', 'images');

// ---- recordings from the synthetic set ---------------------------------------------------------

const source = readFileSync(join(root, 'tests', 'fixtures', 'beeps-set.aif'));
const HEAD = 54;
const RATE = 22050;
const FRAME = 4;
const pcm = source.subarray(HEAD, HEAD + source.readUInt32BE(42) - 8);

function id3(recorded) {
  const frame = (id, body) => Buffer.concat([Buffer.from(id), Buffer.from(new Uint8Array([(body.length >>> 24) & 255, (body.length >>> 16) & 255, (body.length >>> 8) & 255, body.length & 255, 0, 0])), body]);
  const frames = Buffer.concat([
    frame('TIT2', Buffer.concat([Buffer.from([0]), Buffer.from('Serato Recording')])),
    frame('COMM', Buffer.concat([Buffer.from([0]), Buffer.from('eng'), Buffer.from([0]), Buffer.from(recorded)])),
  ]);
  const n = frames.length;
  const tag = Buffer.concat([Buffer.from('ID3\x03\x00\x00', 'latin1'), Buffer.from([(n >> 21) & 127, (n >> 14) & 127, (n >> 7) & 127, n & 127]), frames]);
  const chunk = Buffer.concat([Buffer.from('ID3 '), Buffer.from([(tag.length >>> 24) & 255, (tag.length >>> 16) & 255, (tag.length >>> 8) & 255, tag.length & 255]), tag]);
  return chunk.length % 2 ? Buffer.concat([chunk, Buffer.alloc(1)]) : chunk;
}

/** A slice of the set as a Serato-style AIFF; `broken` leaves the size fields at 0 and writes no tag. */
function recording(fromSec, toSec, recorded, broken = false) {
  const audio = pcm.subarray(Math.round(fromSec * RATE) * FRAME, Math.round(toSec * RATE) * FRAME);
  const head = Buffer.from(source.subarray(0, HEAD));
  const trailer = broken ? Buffer.alloc(0) : id3(recorded);
  if (!broken) {
    head.writeUInt32BE(audio.length / FRAME, 22);
    head.writeUInt32BE(audio.length + 8, 42);
    head.writeUInt32BE(HEAD - 8 + audio.length + trailer.length, 4);
  } else {
    head.writeUInt32BE(0, 4);
    head.writeUInt32BE(0, 22);
    head.writeUInt32BE(0, 42);
  }
  return Buffer.concat([head, audio, trailer]);
}

// ---- driving the app -----------------------------------------------------------------------------

const freePort = () =>
  new Promise((res) => {
    const s = createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => res(port));
    });
  });

if (!existsSync(exe)) throw new Error(`app not found: ${exe}`);
if (!existsSync(edgedriver)) throw new Error(`msedgedriver not found: ${edgedriver}`);

const work = mkdtempSync(join(tmpdir(), 'srf-shots-'));
const rec = join(work, 'Recording');
const temp = join(work, 'Recording temp');
mkdirSync(rec);
mkdirSync(temp);
writeFileSync(join(rec, 'Serato Recording 1.aif'), recording(0, 46, 'Recorded 2026-10-24 21:58:12'));
writeFileSync(join(rec, 'Serato Recording 2.aif'), recording(0, 30, 'Recorded 2026-10-24 23:20:41'));
writeFileSync(join(rec, 'Serato Recording 3.aif'), recording(16, 46, 'Recorded 2026-10-25 00:41:09'));
writeFileSync(join(temp, 'Serato Recording 4.aif'), recording(8, 38, '', true));
const old = new Date(Date.now() - 6 * 3600_000);
utimesSync(join(temp, 'Serato Recording 4.aif'), old, old);

const port = await freePort();
const driverProc = spawn(edgedriver, [`--port=${port}`], {
  stdio: 'ignore',
  env: {
    ...process.env,
    RECORDING_FIXER_FOLDERS: [rec, temp].join(delimiter),
    RECORDING_FIXER_NO_UPDATE_CHECK: '1',
    WEBVIEW2_USER_DATA_FOLDER: join(work, '.webview'),
  },
});
await new Promise((r) => setTimeout(r, 1500));
const driver = await new Builder()
  .usingServer(`http://127.0.0.1:${port}/`)
  .withCapabilities({ browserName: 'webview2', 'ms:edgeOptions': { binary: exe, webviewOptions: {} } })
  .build();

const find = (xpath, ms = 10000) => driver.wait(until.elementLocated(By.xpath(xpath)), ms);
const click = async (xpath) => {
  const el = await find(xpath);
  await driver.wait(until.elementIsEnabled(el), 5000);
  await el.click();
};
const button = (text) => `//button[normalize-space()='${text}']`;
const row = (name) => `//tr[@data-testid='row'][.//td[contains(@class,'name')][normalize-space()='${name}']]`;
const tick = (name) => click(`${row(name)}//input[@type='checkbox']`);
const editor = "//div[@aria-label='Editor']";
const tab = (name) => click(`${editor}//button[@role='tab'][contains(.,'${name}')]`);
const pause = (ms) => driver.sleep(ms);
const bodyText = () => driver.findElement(By.css('body')).getText();

async function shot(name) {
  await pause(700); // let animations and canvases settle
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, `${name}.png`), Buffer.from(await driver.takeScreenshot(), 'base64'));
  console.log(`docs/images/${name}.png`);
}

async function theme(label) {
  for (let i = 0; i < 3; i++) {
    const btn = await find("//button[contains(@class,'theme')]");
    if ((await btn.getText()).toLowerCase().includes(label)) return;
    await btn.click();
    await pause(200);
  }
}

async function until_(re, ms = 20000) {
  await driver.wait(async () => re.test(await bodyText()), ms, `waiting for ${re}`);
}

try {
  await driver.manage().window().setRect({ width: 1180, height: 800 });
  await find(row('Serato Recording 1.aif'));
  await pause(800);
  try {
    await click(button('Got it'));
  } catch {
    /* the folder help may already be gone */
  }
  await theme('dark');

  // 1. the list
  await tick('Serato Recording 2.aif');
  await tick('Serato Recording 3.aif');
  await shot('list');
  await tick('Serato Recording 2.aif');
  await tick('Serato Recording 3.aif');

  // 2. the repair choice
  await tick('Serato Recording 4.aif');
  await click(button('Repair…'));
  await find("//div[@role='dialog']");
  await shot('repair');
  await click("//div[@role='dialog']" + button('Cancel'));
  await pause(300);
  await tick('Serato Recording 4.aif');

  // 3. the editor on the whole set (the beatmix, the quiet track, the breakdown and the drop are all in it)
  await tick('Serato Recording 1.aif');
  await click(button('Edit & export…'));
  await find(editor);
  await pause(1500);
  await shot('editor');

  // 4. loudness, with its graph
  await tab('Loudness');
  await click(`${editor}//label[contains(.,'Match a loudness')]/input`);
  await until_(/Playing with it on/i);
  await pause(1500);
  await shot('loudness');

  // 5. fades (turn the processing off first so the picture is about fades)
  await click(`${editor}//label[contains(.,'Leave as recorded')]/input`);
  await tab('Fades');
  await click(`${editor}//button[@aria-label='Fade in 2 seconds']`);
  await click(`${editor}//button[@aria-label='Fade out 5 seconds']`);
  await shot('fades');

  // 6. format and quality
  await tab('Format');
  await click(`${editor}//label[.//b[normalize-space()='MP3']]/input`);
  await shot('format');
  await driver.findElement(By.css('body')).sendKeys('');
  await pause(500);

  // 7. settings
  await click("//button[contains(.,'Settings')]");
  await find("//div[@role='dialog'][@aria-label='Settings']");
  await shot('settings');
  await driver.findElement(By.css('body')).sendKeys('');
  await pause(300);

  // 8. the light theme, for the README
  await theme('light');
  await tick('Serato Recording 3.aif');
  await shot('list-light');
} finally {
  await driver.quit().catch(() => {});
  driverProc.kill();
  await new Promise((r) => setTimeout(r, 500));
  rmSync(work, { recursive: true, force: true });
}
