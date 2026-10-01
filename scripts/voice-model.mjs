// Pinned Apache-2.0 model; never download models at application runtime.
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, mkdtemp, cp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
const name = 'sherpa-onnx-streaming-zipformer-en-2023-06-26';
const checksum = '639e25b578e9e997131402199419c13a941f8e4e198e2da1ce57dbf5cf401282';
const root = fileURLToPath(new URL('../src-tauri/voice-model/', import.meta.url));
const files = ['encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx', 'decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx', 'joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx', 'tokens.txt', 'README.md'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const manifest = JSON.parse(await readFile(new URL('./voice-model-hashes.json', import.meta.url)));
async function valid() {
  try { for (const file of files) if (digest(await readFile(join(root, file))) !== manifest[file]) return false; return true; }
  catch { return false; }
}
if (!await valid()) {
  const tmp = await mkdtemp(join(tmpdir(), 'cognuum-voice-'));
  try {
    const archive = join(tmp, 'model.tar.bz2');
    const bytes = process.argv[2] ? await readFile(process.argv[2]) : await (async () => {
      const response = await fetch(`https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${name}.tar.bz2`, { signal: AbortSignal.timeout(300000) });
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
await cp(new URL('../assets/voice-bpe.vocab', import.meta.url), join(root, 'bpe.vocab'));
await cp(new URL('../assets/VOICE-LICENSE.txt', import.meta.url), join(root, 'LICENSE.txt'));
console.log('Local English voice model verified (68 MiB).');
