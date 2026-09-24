import test from 'node:test';
import assert from 'node:assert/strict';
import { previewConfig, previewMetadata } from './preview-assets.mjs';

test('preview packaging requires explicit preview identity and both installers', () => {
  assert.throws(() => previewConfig('0.1.0'));
  assert.throws(() => previewConfig('staging'));
  const config = previewConfig('0.1.0-preview.1');
  assert.equal(config.bundle.createUpdaterArtifacts, false);
  const input = { version: config.version, files: ['Cognuum.dmg', 'Cognuum.exe'], read: file => Buffer.from(file) };
  const metadata = previewMetadata(input);
  assert.equal(metadata.tag, 'v0.1.0-preview.1');
  assert.equal(metadata.checksums.mac_dmg_sha256.length, 64);
  assert.equal(metadata.files.length, 2);
  assert.throws(() => previewMetadata({ ...input, files: ['Cognuum.dmg'] }));
  assert.throws(() => previewMetadata({ ...input, files: [...input.files, 'other.exe'] }));
});
