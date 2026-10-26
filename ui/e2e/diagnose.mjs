// Starts the app through msedgedriver directly (no tauri-driver) with its verbose log on, so a failing
// session shows what msedgedriver launched, with which arguments, and what it was waiting for.
//   node ui/e2e/diagnose.mjs        (APP_EXE and EDGEDRIVER override the defaults)
// The log lands in ui/e2e/.tools/logs/msedgedriver.log and its tail is printed.
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
const log = join(logDir, 'msedgedriver.log');
mkdirSync(logDir, { recursive: true });

const port = await new Promise((res) => {
  const s = createServer().listen(0, '127.0.0.1', () => {
    const { port } = s.address();
    s.close(() => res(port));
  });
});

const work = mkdtempSync(join(tmpdir(), 'srf-diag-'));
const env = { ...process.env, RECORDING_FIXER_FOLDERS: work, WEBVIEW2_USER_DATA_FOLDER: join(work, '.webview') };
const proc = spawn(edgedriver, [`--port=${port}`, '--verbose', `--log-path=${log}`], { stdio: 'ignore', env });
await new Promise((r) => setTimeout(r, 1500));

let code = 0;
try {
  if (!existsSync(exe)) throw new Error(`app not found: ${exe}`);
  const driver = await new Builder()
    .usingServer(`http://127.0.0.1:${port}/`)
    .withCapabilities({ browserName: 'webview2', 'ms:edgeOptions': { binary: exe, webviewOptions: {} } })
    .build();
  console.log('session created; title:', await driver.getTitle());
  await driver.quit();
} catch (e) {
  code = 1;
  console.error('session failed:', e.message.split('\n')[0]);
} finally {
  proc.kill();
  await new Promise((r) => setTimeout(r, 500));
  rmSync(work, { recursive: true, force: true });
}
if (existsSync(log)) console.log(`--- ${log} (last 60 lines)\n${readFileSync(log, 'utf8').split('\n').slice(-60).join('\n')}`);
process.exit(code);
