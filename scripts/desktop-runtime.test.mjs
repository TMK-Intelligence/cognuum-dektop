import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('../src-tauri/src/desktop-runtime.js', import.meta.url), 'utf8');
const origin = 'https://access.cognuum.com';
const metadata = { version: '0.3.0-preview.1', voice: { protocol: 1, language: 'en' } };
function install({ pageOrigin = origin, ready = true } = {}) {
  const browserSpeech = function BrowserSpeech() {};
  const window = { SpeechRecognition: browserSpeech, webkitSpeechRecognition: browserSpeech };
  const listeners = [];
  const document = {
    documentElement: ready ? { dataset: {} } : null,
    addEventListener: (...args) => listeners.push(args),
  };
  runInNewContext(`${source}(${JSON.stringify(metadata)}, ${JSON.stringify(origin)});`, {
    window, document, location: { origin: pageOrigin },
  });
  return { window, document, listeners, browserSpeech };
}

test('native windows advertise Max while the unsupported browser speech mic is absent', () => {
  const { window, document } = install();
  assert.equal(window.__COGNUUM_DESKTOP__.voice.protocol, 1);
  assert.equal(window.SpeechRecognition ?? window.webkitSpeechRecognition ?? null, null);
  assert.equal(document.documentElement.dataset.appVersion, metadata.version);
  assert.equal(Object.isFrozen(window.__COGNUUM_DESKTOP__), true);
  assert.throws(() => Object.defineProperty(window, 'SpeechRecognition', { value: function () {} }));
});

test('untrusted and fallback origins retain their browser APIs and receive no native hint', () => {
  for (const pageOrigin of ['https://access.cognuum.com.evil.test', 'https://other.test', 'tauri://localhost']) {
    const { window, document, browserSpeech } = install({ pageOrigin });
    assert.equal(window.__COGNUUM_DESKTOP__, undefined);
    assert.equal(window.SpeechRecognition, browserSpeech);
    assert.equal(window.webkitSpeechRecognition, browserSpeech);
    assert.deepEqual(document.documentElement.dataset, {});
  }
});

test('document-start initialization marks the DOM when it becomes available', () => {
  const { document, listeners, window } = install({ ready: false });
  assert.equal(window.SpeechRecognition, undefined);
  assert.equal(listeners.length, 1);
  assert.equal(listeners[0][0], 'DOMContentLoaded');
  assert.equal(listeners[0][2].once, true);
  document.documentElement = { dataset: {} };
  listeners[0][1]();
  assert.equal(document.documentElement.dataset.tauri, 'true');
});
