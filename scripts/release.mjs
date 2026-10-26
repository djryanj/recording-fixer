// Cutting a release, the same way every time. Run through `just` (see the justfile):
//   just prepare-release 0.1.0-rc.1    a release/v0.1.0-rc.1 branch with the versions bumped, pushed, ready for a PR
//   just tag-release 0.1.0-rc.1        after that PR is merged: tag main and push the tag (this starts the release)
//
// Where the version lives: [workspace.package] in the root Cargo.toml is the source of truth (the app, the
// status bar and the update check read it); ui/package.json is kept equal to it. tauri.conf.json has none.
//
// The changelog is closed out only for a final release. A pre-release (1.2.3-rc.1) leaves it open, so the
// final version collects everything the candidates did.
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$/;
export const isPrerelease = (v) => v.includes('-');

/** Set `version` inside the [workspace.package] table of a Cargo.toml. */
export function setCargoVersion(text, version) {
  const crlf = text.includes('\r\n');
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  let inTable = false;
  let done = false;
  const out = lines.map((line) => {
    if (/^\[.*\]\s*$/.test(line)) inTable = /^\[workspace\.package\]\s*$/.test(line);
    if (inTable && !done && /^version\s*=/.test(line)) {
      done = true;
      return `version = "${version}"`;
    }
    return line;
  });
  if (!done) throw new Error('no version in [workspace.package] of Cargo.toml');
  return out.join(crlf ? '\r\n' : '\n');
}

export function cargoVersion(text) {
  const m = /\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/ms.exec(text.replace(/\r\n/g, '\n'));
  return m ? m[1] : null;
}

/**
 * Close out the changelog's open section for `version`, dated `date` (YYYY-MM-DD), and leave a fresh
 * empty "Unreleased" above it. The open section is "Unreleased" if it has entries, else a heading
 * already named for this version and marked "Not yet released" (how the first release is prepared).
 */
export function stampChangelog(text, version, date) {
  const crlf = text.includes('\r\n');
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const heading = (i) => /^## /.test(lines[i] ?? '');
  const sectionEnd = (from) => {
    let i = from + 1;
    while (i < lines.length && !heading(i)) i++;
    return i;
  };
  const unreleased = lines.findIndex((l) => /^## \[Unreleased\]\s*$/i.test(l));
  const named = lines.findIndex((l) => new RegExp(`^## \\[${version.replace(/\./g, '\\.')}\\] - (Not yet released|Unreleased)\\s*$`, 'i').test(l));
  const hasEntries = (from) => lines.slice(from + 1, sectionEnd(from)).some((l) => l.trim() !== '');

  let out;
  if (unreleased !== -1 && hasEntries(unreleased)) {
    out = [...lines];
    out.splice(unreleased, 1, '## [Unreleased]', '', `## [${version}] - ${date}`);
  } else if (named !== -1) {
    out = [...lines];
    out[named] = `## [${version}] - ${date}`;
    if (unreleased === -1) out.splice(named, 0, '## [Unreleased]', '');
  } else {
    throw new Error('the changelog has nothing to release: add entries under "## [Unreleased]" first');
  }
  const text2 = out.join('\n').replace('First version, not yet tagged or published.', 'First public version.');
  return crlf ? text2.replace(/\n/g, '\r\n') : text2;
}

/**
 * The changelog text that goes into the GitHub release for `version`: its own dated section, else (for a
 * pre-release) the section of the version it leads up to, else what is still under "Unreleased".
 */
export function releaseNotes(text, version) {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const base = version.split('-')[0];
  const esc = (v) => v.replace(/\./g, '\\.');
  const find = (re) => lines.findIndex((l) => re.test(l));
  let at = find(new RegExp(`^## \\[${esc(version)}\\]`));
  if (at === -1 && isPrerelease(version)) at = find(new RegExp(`^## \\[${esc(base)}\\]`));
  if (at === -1) at = find(/^## \[Unreleased\]/i);
  if (at === -1) return '';
  let end = at + 1;
  while (end < lines.length && !/^## /.test(lines[end])) end++;
  return lines.slice(at + 1, end).join('\n').trim();
}

// ---- the command line ------------------------------------------------------------------------

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const win = process.platform === 'win32';

function run(cmd, args, { quiet = false, allowFail = false } = {}) {
  // npm and cargo are .cmd/.exe shims on Windows and need a shell; the arguments here are fixed words and a version that passed SEMVER.
  const stdio = quiet ? ['ignore', 'pipe', 'pipe'] : ['ignore', 'pipe', 'inherit'];
  const r = win && cmd !== 'git' ? spawnSync([cmd, ...args].join(' '), { cwd: root, encoding: 'utf8', shell: true, stdio }) : spawnSync(cmd, args, { cwd: root, encoding: 'utf8', stdio });
  if (r.status !== 0 && !allowFail) throw new Error(`${cmd} ${args.join(' ')} failed${r.stderr ? `: ${r.stderr.trim()}` : ''}`);
  return (r.stdout ?? '').trim();
}
const git = (...args) => run('git', args, { quiet: true });
const read = (f) => readFileSync(join(root, f), 'utf8');
const write = (f, t) => writeFileSync(join(root, f), t);

function requireCleanMain({ pull }) {
  if (git('status', '--porcelain') !== '') throw new Error('the working tree has uncommitted changes: commit or stash them first');
  if (git('rev-parse', '--abbrev-ref', 'HEAD') !== 'main') throw new Error('run this from the main branch');
  if (pull) git('pull', '--ff-only', 'origin', 'main');
  else {
    git('fetch', 'origin', 'main');
    if (git('rev-parse', 'HEAD') !== git('rev-parse', 'origin/main')) throw new Error('local main is not the same as origin/main: pull (or push) first');
  }
}

function prepare(version, { filesOnly }) {
  if (!filesOnly) {
    requireCleanMain({ pull: true });
    if (git('branch', '--list', `release/v${version}`) !== '' || git('ls-remote', '--heads', 'origin', `release/v${version}`) !== '') {
      throw new Error(`release/v${version} already exists`);
    }
    if (git('tag', '--list', `v${version}`) !== '') throw new Error(`the tag v${version} already exists`);
    git('checkout', '-b', `release/v${version}`);
  }
  write('Cargo.toml', setCargoVersion(read('Cargo.toml'), version));
  // npm updates package-lock.json's matching fields in the same step.
  run('npm', ['--prefix', 'ui', 'version', version, '--no-git-tag-version', '--allow-same-version'], { quiet: true });
  // Cargo.lock lists the workspace crates with their versions.
  run('cargo', ['update', '--workspace', '--offline'], { quiet: true });
  const files = ['Cargo.toml', 'Cargo.lock', 'ui/package.json', 'ui/package-lock.json'];
  if (isPrerelease(version)) {
    console.log('Pre-release: the changelog is left open for the final version.');
  } else {
    const date = new Date().toISOString().slice(0, 10);
    write('CHANGELOG.md', stampChangelog(read('CHANGELOG.md'), version, date));
    files.push('CHANGELOG.md');
  }
  if (filesOnly) return console.log(`Updated ${files.join(', ')} (no branch, commit or push: --files-only).`);
  git('add', ...files);
  git('commit', '-m', `chore(release): prepare v${version}`);
  git('push', '-u', 'origin', `release/v${version}`);
  console.log(`
Release branch ready. Next:
  1. Open a PR: release/v${version} -> main, and let CI pass
  2. Merge it
  3. git checkout main && git pull && git branch -d release/v${version}
  4. just tag-release ${version}     (tags main and pushes the tag: the last step)`);
}

/** What the release workflow needs to know about the tag it was started by (it prints GitHub output lines). */
function verify(tagName) {
  const cargo = cargoVersion(read('Cargo.toml'));
  const ui = JSON.parse(read('ui/package.json')).version;
  if (cargo !== ui) throw new Error(`Cargo.toml says ${cargo} but ui/package.json says ${ui}`);
  if (tagName && tagName !== `v${cargo}`) throw new Error(`the tag is ${tagName} but the app version is ${cargo}`);
  console.log(`version=${cargo}`);
  console.log(`prerelease=${isPrerelease(cargo)}`);
}

function notes(version) {
  const body = releaseNotes(read('CHANGELOG.md'), version);
  const kind = isPrerelease(version) ? 'Release candidate' : 'Release';
  console.log(`${kind} ${version}.\n\n${body || 'See the changelog.'}`);
}

function tag(version) {
  requireCleanMain({ pull: false });
  const found = { 'Cargo.toml': cargoVersion(read('Cargo.toml')), 'ui/package.json': JSON.parse(read('ui/package.json')).version };
  for (const [file, v] of Object.entries(found)) {
    if (v !== version) throw new Error(`${file} says ${v}, not ${version}: has the release PR been merged?`);
  }
  if (!isPrerelease(version) && !new RegExp(`^## \\[${version.replace(/\./g, '\\.')}\\] - \\d{4}-\\d{2}-\\d{2}`, 'm').test(read('CHANGELOG.md'))) {
    throw new Error(`CHANGELOG.md has no dated "## [${version}]" section`);
  }
  if (git('tag', '--list', `v${version}`) !== '') throw new Error(`the tag v${version} already exists`);
  // Tag first, then push as its own step: if the push fails (offline, or the tag is already on the remote)
  // the annotated tag still exists locally and `git push origin vX` by hand is the whole fix. An existing
  // tag is never moved: a tag that has built and published a release must not be repointed.
  git('tag', '-a', `v${version}`, '-m', `Recording Fixer for Serato DJ ${version}`);
  try {
    git('push', 'origin', `v${version}`);
  } catch (e) {
    throw new Error(`${e.message}
The tag v${version} exists locally. Once the problem is fixed, finish with: git push origin v${version}`);
  }
  console.log(`Pushed v${version}. The release workflow starts from this tag.`);
}

const invokedDirectly = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (invokedDirectly) {
  const [command, version, ...flags] = process.argv.slice(2);
  try {
    if (command === 'verify') verify(version ?? '');
    else if (command === 'notes' && version && SEMVER.test(version)) notes(version);
    else if (!['prepare', 'tag'].includes(command) || !version || !SEMVER.test(version)) {
      throw new Error('usage: node scripts/release.mjs prepare|tag X.Y.Z[-rc.N] [--files-only] | verify [vX.Y.Z] | notes X.Y.Z');
    } else if (command === 'prepare') prepare(version, { filesOnly: flags.includes('--files-only') });
    else tag(version);
  } catch (e) {
    console.error(`Error: ${e.message}`);
    process.exit(1);
  }
}
