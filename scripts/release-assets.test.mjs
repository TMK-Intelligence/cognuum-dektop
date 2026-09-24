import test from 'node:test';
import assert from 'node:assert/strict';
import { releaseMetadata } from './release-assets.mjs';
const files = ['a/Cognuum.dmg', 'b/Cognuum.exe', 'a/Cognuum.app.tar.gz', 'a/Cognuum.app.tar.gz.sig', 'b/Cognuum.exe.sig'];
const fixture = { version: '0.1.0', channel: 'production', files, read: () => Buffer.from('signed-fixture'), now: '2026-09-24T00:00:00Z' };
test('release manifest covers both Mac architectures and Windows with signatures', () => {
  const metadata = releaseMetadata(fixture);
  assert.equal(metadata.tag, 'v0.1.0');
  assert.deepEqual(Object.keys(metadata.updater.platforms).sort(), ['darwin-aarch64', 'darwin-x86_64', 'windows-x86_64']);
  assert.equal(metadata.checksums.mac_dmg_sha256.length, 64);
});
test('red proof: no partial or unsigned release can produce a manifest', () => {
  for (const removed of files) assert.throws(() => releaseMetadata({ ...fixture, files: files.filter(file => file !== removed) }));
  assert.throws(() => releaseMetadata({ ...fixture, files: [...files, 'extra/Cognuum.exe'] }));
  assert.throws(() => releaseMetadata({ ...fixture, read: () => Buffer.from('') }));
});
