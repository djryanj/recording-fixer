import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { checksumFor } from './fetch-ffmpeg.mjs';

describe('checksum files', () => {
  const sums = [
    'c1044833f3e9b30de2e3d80faef815998624d568fee0493eb49ecb13e5ef0980  ffmpeg-n9.0-latest-win64-lgpl-9.0.zip',
    'AAAA33f3e9b30de2e3d80faef815998624d568fee0493eb49ecb13e5ef0980ff *ffmpeg-n9.0-latest-win64-gpl-9.0.zip',
    'not a checksum line',
  ].join('\r\n');

  it('finds the hash for exactly the named file', () => {
    assert.equal(checksumFor(sums, 'ffmpeg-n9.0-latest-win64-lgpl-9.0.zip'), 'c1044833f3e9b30de2e3d80faef815998624d568fee0493eb49ecb13e5ef0980');
    assert.equal(checksumFor(sums, 'ffmpeg-n9.0-latest-win64-gpl-9.0.zip'), 'aaaa33f3e9b30de2e3d80faef815998624d568fee0493eb49ecb13e5ef0980ff');
  });

  it('does not match a longer or different name, or invent one', () => {
    assert.equal(checksumFor(sums, 'ffmpeg-n9.0-latest-win64-lgpl-9.0.zip.sig'), null);
    assert.equal(checksumFor(sums, 'lgpl-9.0.zip'), null);
    assert.equal(checksumFor('', 'x'), null);
  });
});
