// Puts the ffmpeg that ships with the app where Tauri expects a sidecar: src-tauri/binaries/ffmpeg-<target triple>[.exe].
//   node scripts/fetch-ffmpeg.mjs --target x86_64-pc-windows-msvc      download BtbN's LGPL build and verify it
//   node scripts/fetch-ffmpeg.mjs --target universal-apple-darwin      build LGPL ffmpeg from source (macOS only)
//   node scripts/fetch-ffmpeg.mjs --target <triple> --local PATH       use a binary you already have (test builds ONLY)
//
// Only release builds need this (src-tauri/tauri.release.conf.json lists the sidecar); development and CI builds do not.
//
// Windows: the LGPL variant of BtbN's FFmpeg-Builds (https://github.com/BtbN/FFmpeg-Builds), downloaded over HTTPS
// and checked against the checksums.sha256 published in the same release. That is an integrity check against
// corruption, not a pin: the "latest" release moves. What was used is written to FFMPEG-BUILD.txt for the release notes.
// macOS: built from source by scripts/build-ffmpeg-macos.sh (pinned versions and checksums), LGPL only.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, createReadStream, createWriteStream, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'src-tauri', 'binaries');

export const WINDOWS = {
  triple: 'x86_64-pc-windows-msvc',
  release: 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/',
  archive: 'ffmpeg-n9.0-latest-win64-lgpl-9.0.zip',
};

/** The hash listed for `name` in a `sha256sum`-style file ("<hex>  <name>" or "<hex> *<name>"). */
export function checksumFor(text, name) {
  for (const line of text.split(/\r?\n/)) {
    const m = /^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$/.exec(line);
    if (m && m[2] === name) return m[1].toLowerCase();
  }
  return null;
}

export async function sha256(file) {
  const h = createHash('sha256');
  await pipeline(createReadStream(file), h);
  return h.digest('hex');
}

async function download(url, dest) {
  const res = await fetch(url, { redirect: 'follow' });
  if (!res.ok || !res.body) throw new Error(`download failed (${res.status}): ${url}`);
  await pipeline(Readable.fromWeb(res.body), createWriteStream(dest));
}

function findFile(dir, name) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      const hit = findFile(p, name);
      if (hit) return hit;
    } else if (entry.name === name) return p;
  }
  return null;
}

async function windows() {
  const work = mkdtempSync(join(tmpdir(), 'srf-ffmpeg-'));
  try {
    const zip = join(work, WINDOWS.archive);
    console.log(`Downloading ${WINDOWS.archive} ...`);
    await download(WINDOWS.release + WINDOWS.archive, zip);
    const sums = await (await fetch(WINDOWS.release + 'checksums.sha256')).text();
    const want = checksumFor(sums, WINDOWS.archive);
    if (!want) throw new Error(`checksums.sha256 does not list ${WINDOWS.archive}`);
    const got = await sha256(zip);
    if (got !== want) throw new Error(`checksum mismatch for ${WINDOWS.archive}: expected ${want}, got ${got}`);
    console.log(`Checksum ok (${got}).`);
    // bsdtar (built into Windows 10+ and macOS) reads zip files.
    const x = spawnSync('tar', ['-xf', zip, '-C', work], { stdio: 'inherit' });
    if (x.status !== 0) throw new Error('could not unpack the archive');
    const exe = findFile(work, 'ffmpeg.exe');
    if (!exe) throw new Error('ffmpeg.exe is not in the archive');
    mkdirSync(outDir, { recursive: true });
    copyFileSync(exe, join(outDir, `ffmpeg-${WINDOWS.triple}.exe`));
    writeFileSync(
      join(outDir, 'FFMPEG-BUILD.txt'),
      `ffmpeg for Windows (x86_64), LGPL variant, from BtbN FFmpeg-Builds\n${WINDOWS.release}${WINDOWS.archive}\nsha256 ${got}\nretrieved ${new Date().toISOString()}\nSource and build scripts: https://github.com/BtbN/FFmpeg-Builds (ffmpeg source: https://ffmpeg.org/)\n`,
    );
    console.log(`Wrote src-tauri/binaries/ffmpeg-${WINDOWS.triple}.exe`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

function macos(target) {
  if (process.platform !== 'darwin') throw new Error('the macOS ffmpeg can only be built on macOS');
  const arch = target.startsWith('aarch64') ? 'arm64' : target.startsWith('x86_64') ? 'x86_64' : 'universal';
  const r = spawnSync('bash', [join(root, 'scripts', 'build-ffmpeg-macos.sh'), arch], { stdio: 'inherit' });
  if (r.status !== 0) throw new Error('building ffmpeg failed');
}

function useLocal(target, path) {
  if (!existsSync(path)) throw new Error(`no such file: ${path}`);
  mkdirSync(outDir, { recursive: true });
  const dest = join(outDir, `ffmpeg-${target}${target.includes('windows') ? '.exe' : ''}`);
  copyFileSync(path, dest);
  writeFileSync(join(outDir, 'FFMPEG-BUILD.txt'), `TEST BUILD ONLY: ffmpeg copied from ${path} (${statSync(path).size} bytes). Not the LGPL build a release ships.\n`);
  console.warn(`Copied ${path} -> ${dest}\nThis is for a test build only: it is whatever ffmpeg you had, not the pinned LGPL build.`);
}

const invokedDirectly = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (invokedDirectly) {
  const args = process.argv.slice(2);
  const flag = (n) => args[args.indexOf(n) + 1];
  const target = args.includes('--target') ? flag('--target') : null;
  try {
    if (!target) throw new Error('usage: node scripts/fetch-ffmpeg.mjs --target <triple> [--local PATH]');
    if (args.includes('--local')) useLocal(target, flag('--local'));
    else if (target === WINDOWS.triple) await windows();
    else if (target.endsWith('apple-darwin')) macos(target);
    else throw new Error(`no bundled ffmpeg is defined for ${target}`);
  } catch (e) {
    console.error(`Error: ${e.message}`);
    process.exit(1);
  }
}
