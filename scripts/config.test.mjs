import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { validateConfig, releaseConfig } from './config.mjs';
const base = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url)));
const staging = JSON.parse(readFileSync(new URL('../src-tauri/tauri.staging.conf.json', import.meta.url)));
test('production and staging are separately bound to the intended origin', () => {
  validateConfig(base);
  validateConfig({ ...base, ...staging, plugins: { ...base.plugins, ...staging.plugins } }, 'staging');
});
test('red proof: configuration checker rejects staging in production and exposed native APIs', () => {
  const wrongOrigin = structuredClone(base); wrongOrigin.plugins.desktop.origin = 'https://dev-access.cognuum.com';
  assert.throws(() => validateConfig(wrongOrigin));
  const bridge = structuredClone(base); bridge.app.security.capabilities = ['remote-filesystem'];
  assert.throws(() => validateConfig(bridge));
  const global = structuredClone(base); global.app.withGlobalTauri = true;
  assert.throws(() => validateConfig(global));
});
test('signed releases fail closed without signing credentials', () => {
  assert.throws(() => releaseConfig('production', 'darwin', {}), /Missing release configuration/);
  const env = { TAURI_UPDATER_PUBLIC_KEY: 'dGVzdA==', TAURI_SIGNING_PRIVATE_KEY: 'fixture', WINDOWS_SIGN_COMMAND: 'sign-test %1' };
  assert.match(releaseConfig('production', 'win32', env).plugins.updater.endpoints[0], /cognuum-dektop\/releases\/latest\/download/);
  assert.match(releaseConfig('staging', 'win32', env).plugins.updater.endpoints[0], /releases\/download\/staging/);
  assert.throws(() => releaseConfig('production', 'win32', { ...env, WINDOWS_SIGN_COMMAND: '' }));
});
