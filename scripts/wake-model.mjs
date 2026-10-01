// Pinned Apache-2.0 model; never download models at application runtime.
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, mkdtemp, cp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
const name = 'sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20';
const checksum = '68447f4fbc67e70eee3a93961f36e81e98f47aef73ce7e7ca00885c6cd3616a6';
const root = fileURLToPath(new URL('../src-tauri/wake-model/', import.meta.url));
const files = ["encoder-epoch-13-avg-2-chunk-8-left-64.int8.onnx", "decoder-epoch-13-avg-2-chunk-8-left-64.onnx", "joiner-epoch-13-avg-2-chunk-8-left-64.int8.onnx", "tokens.txt", "encoder-epoch-13-avg-2-chunk-16-left-64.onnx", "decoder-epoch-13-avg-2-chunk-16-left-64.onnx", "joiner-epoch-13-avg-2-chunk-16-left-64.onnx"];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const manifest = JSON.parse(await readFile(new URL('./wake-model-hashes.json', import.meta.url)));
async function valid() {
  try { for (const file of files) if (digest(await readFile(join(root, file))) !== manifest[file]) return false; return true; }
  catch { return false; }
}
if (!await valid()) {
  const tmp = await mkdtemp(join(tmpdir(), 'cognuum-voice-'));
  try {
    const archive = join(tmp, 'model.tar.bz2');
    const bytes = process.argv[2] ? await readFile(process.argv[2]) : await (async () => {
      const response = await fetch(`https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/${name}.tar.bz2`, { signal: AbortSignal.timeout(300000) });
      if (!response.ok) throw new Error(`Speech model download failed (${response.status})`);
      return Buffer.from(await response.arrayBuffer());
    })();
    if (digest(bytes) !== checksum) throw new Error('Speech model checksum mismatch');
    await writeFile(archive, bytes);
    execFileSync('tar', ['-xjf', archive, '-C', tmp, ...files.map(file => `${name}/${file}`)]);
    await rm(root, { recursive: true, force: true }); // Generated resources only.
    await mkdir(root, { recursive: true });
    for (const file of files) await cp(join(tmp, name, file), join(root, file));
    if (!await valid()) throw new Error('Speech model files failed integrity verification');
  } finally { await rm(tmp, { recursive: true, force: true }); }
}
await cp(new URL('../assets/VOICE-LICENSE.txt', import.meta.url), join(root, 'LICENSE.txt'));
await cp(new URL('../assets/WAKE-MODEL.md', import.meta.url), join(root, 'README.md'));
console.log('Local Max phonetic wake model verified.');
