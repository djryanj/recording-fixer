// Starts the app through msedgedriver directly (no tauri-driver) with its verbose log on, so a failing
// session shows what msedgedriver launched, with which arguments, and what it was waiting for.
//   node ui/e2e/diagnose.mjs        (APP_EXE and EDGEDRIVER override the defaults)
//
// Note: msedgedriver's `args` capability goes to the *app's* command line, not to WebView2, so it cannot
// be used to pass browser flags. Log lands in ui/e2e/.tools/logs/msedgedriver-plain.log.
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Builder } from 'selenium-webdriver';

const here = dirname(fileURLToPath(import.meta.url));
const exe = process.env.APP_EXE ?? resolve(here, '../../target/debug/recording-fixer.exe');
const edgedriver = process.env.EDGEDRIVER ?? join(here, '.tools', 'edgedriver', 'msedgedriver.exe');
const logDir = join(here, '.tools', 'logs');
mkdirSync(logDir, { recursive: true });

const variants = [{ name: 'plain', args: [] }];

const freePort = () =>
  new Promise((res) => {
    const s = createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => res(port));
    });
  });

async function attempt({ name, args }) {
  const log = join(logDir, `msedgedriver-${name}.log`);
  const port = await freePort();
  const work = mkdtempSync(join(tmpdir(), 'srf-diag-'));
  const env = { ...process.env, RECORDING_FIXER_FOLDERS: work, WEBVIEW2_USER_DATA_FOLDER: join(work, '.webview') };
  const proc = spawn(edgedriver, [`--port=${port}`, '--verbose', `--log-path=${log}`], { stdio: 'ignore', env });
  await new Promise((r) => setTimeout(r, 1500));
  let result;
  try {
    const driver = await new Builder()
      .usingServer(`http://127.0.0.1:${port}/`)
      .withCapabilities({ browserName: 'webview2', 'ms:edgeOptions': { binary: exe, args, webviewOptions: {} } })
      .build();
    result = { ok: true, detail: `title "${await driver.getTitle()}"` };
    await driver.quit();
  } catch (e) {
    result = { ok: false, detail: e.message.split('\n')[0] };
  } finally {
    proc.kill();
    await new Promise((r) => setTimeout(r, 500));
    rmSync(work, { recursive: true, force: true });
  }
  if (!result.ok && existsSync(log)) {
    const tail = readFileSync(log, 'utf8').split('\n').slice(-40).join('\n');
    console.log(`--- ${name}: last lines of ${log}\n${tail}`);
  }
  return { name, args, ...result };
}

if (!existsSync(exe)) throw new Error(`app not found: ${exe}`);
const results = [];
for (const v of variants.filter((v) => !process.env.DIAG_VARIANT || v.name === process.env.DIAG_VARIANT)) {
  const r = await attempt(v);
  results.push(r);
  console.log(`${r.ok ? 'OK  ' : 'FAIL'} ${v.name} [${v.args.join(' ')}]: ${r.detail}`);
  if (r.ok) break;
}
process.exit(results.some((r) => r.ok) ? 0 : 1);
