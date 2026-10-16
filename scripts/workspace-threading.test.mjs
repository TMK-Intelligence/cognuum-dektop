import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const desktop = readFileSync(new URL('../src-tauri/src/desktop.rs', import.meta.url), 'utf8');
const main = readFileSync(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
function assertThreading(source, setup) {
  const save = source.slice(source.indexOf('pub fn save('), source.indexOf('fn save_on_main_thread('));
  assert.match(save, /run_on_main_thread\(move \|\| save_on_main_thread/);
  assert.doesNotMatch(save, /save_window_state\(/);
  assert.equal((source.match(/save_window_state\(/g) ?? []).length, 1);
  const persist = source.slice(source.indexOf('fn save_on_main_thread('), source.indexOf('pub fn restore('));
  assert.match(persist, /save_window_state\(/);
  assert.match(setup, /0\.\.workspace::MAX_WINDOWS/);
  assert.match(setup, /skip_initial_state/);
  const create = source.slice(source.indexOf('pub fn create('), source.indexOf('pub fn window_event('));
  assert.match(create, /\.visible\(false\)/);
  assert.match(create, /run_on_main_thread\(move \|\| \{\s*let _ = shown_window.restore_state/);
  assert.ok(create.indexOf('shown_window.restore_state') < create.indexOf('shown_window.show()'));
}
test('workspace save and restore keep plugin locks on the event loop', () => assertThreading(desktop, main));
for (const newline of ['\n', '\r\n']) {
  test(`red proof: worker persistence and restore are rejected with ${JSON.stringify(newline)} lines`, () => {
    const source = desktop.replace(/\r?\n/g, newline);
    assertThreading(source, main);
    assert.throws(() => assertThreading(source.replace('run_on_main_thread(move || save_on_main_thread', 'spawn(move || save_on_main_thread'), main));
    assert.throws(() => assertThreading(source, main.replace('skip_initial_state', 'restore_on_worker')));
    const workerRestore = source.replace(/run_on_main_thread(?=\(move \|\| \{\s*let _ = shown_window.restore_state)/, 'spawn');
    assert.notEqual(workerRestore, source, 'the worker-restore violation must actually be planted');
    assert.throws(() => assertThreading(workerRestore, main));
  });
}
