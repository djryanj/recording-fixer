import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { cargoVersion, isPrerelease, releaseNotes, SEMVER, setCargoVersion, stampChangelog } from './release.mjs';

const CARGO = `[workspace]
members = ["a"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
serde = { version = "1" }
`;

describe('versions', () => {
  it('accepts releases and pre-releases, nothing else', () => {
    for (const v of ['0.1.0', '1.20.3', '0.1.0-rc.1', '2.0.0-beta.2', '1.0.0-alpha']) assert.ok(SEMVER.test(v), v);
    for (const v of ['1', '1.0', 'v1.0.0', '1.0.0-', '1.0.0-rc..1', '1.0.0+build', 'x.y.z', '']) assert.ok(!SEMVER.test(v), v);
    assert.ok(isPrerelease('1.0.0-rc.1') && !isPrerelease('1.0.0'));
  });

  it('changes only the workspace package version', () => {
    const out = setCargoVersion(CARGO, '0.1.0-rc.1');
    assert.equal(cargoVersion(out), '0.1.0-rc.1');
    assert.ok(out.includes('serde = { version = "1" }'));
    assert.equal(out.replace('0.1.0-rc.1', '0.1.0'), CARGO);
  });

  it('keeps Windows line endings', () => {
    const out = setCargoVersion(CARGO.replace(/\n/g, '\r\n'), '0.2.0');
    assert.ok(out.includes('\r\n') && !/[^\r]\n/.test(out));
  });

  it('refuses a Cargo.toml without one', () => {
    assert.throws(() => setCargoVersion('[workspace]\nmembers = []\n', '1.0.0'), /no version/);
  });
});

describe('closing the changelog', () => {
  const entries = `# Changelog

## [Unreleased]

### Added

- A thing.

## [0.1.0] - 2026-01-01

- Old.
`;

  it('dates the open section and opens a new Unreleased above it', () => {
    const out = stampChangelog(entries, '0.2.0', '2026-10-30');
    assert.match(out, /## \[Unreleased\]\n\n## \[0\.2\.0\] - 2026-10-30\n\n### Added\n\n- A thing\./);
    assert.ok(out.includes('## [0.1.0] - 2026-01-01'));
    assert.equal(out.match(/## \[Unreleased\]/g).length, 1);
  });

  it('turns a "Not yet released" section named for the version into the release (the first one)', () => {
    const first = `# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - Not yet released\n\nFirst version, not yet tagged or published.\n\n### Added\n\n- Everything.\n`;
    const out = stampChangelog(first, '0.1.0', '2026-10-30');
    assert.match(out, /## \[Unreleased\]\n\n## \[0\.1\.0\] - 2026-10-30\n\nFirst public version\./);
    assert.equal(out.match(/## \[Unreleased\]/g).length, 1);
  });

  it('adds the Unreleased heading when there was none', () => {
    const out = stampChangelog(`# Changelog\n\n## [0.1.0] - Not yet released\n\n- x\n`, '0.1.0', '2026-10-30');
    assert.match(out, /## \[Unreleased\]\n\n## \[0\.1\.0\] - 2026-10-30/);
  });

  it('refuses to release nothing', () => {
    assert.throws(() => stampChangelog(`# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-01-01\n\n- Old.\n`, '0.2.0', '2026-10-30'), /nothing to release/);
  });

  it('keeps Windows line endings', () => {
    const out = stampChangelog(entries.replace(/\n/g, '\r\n'), '0.2.0', '2026-10-30');
    assert.ok(out.includes('\r\n') && !/[^\r]\n/.test(out));
  });
});

describe('release notes', () => {
  const log = `# Changelog

## [Unreleased]

- Pending.

## [0.2.0] - 2026-02-02

- Two.

## [0.1.0] - 2026-01-01

- One.
`;

  it("is the version's own section", () => {
    assert.equal(releaseNotes(log, '0.2.0'), '- Two.');
    assert.equal(releaseNotes(log, '0.1.0'), '- One.');
  });

  it('for a release candidate, is the section of the version it leads to, else what is unreleased', () => {
    assert.equal(releaseNotes(log, '0.2.0-rc.1'), '- Two.');
    assert.equal(releaseNotes(log, '0.3.0-rc.1'), '- Pending.');
  });

  it('is empty rather than wrong when there is no changelog section', () => {
    assert.equal(releaseNotes('# Changelog\n', '1.0.0'), '');
  });
});
