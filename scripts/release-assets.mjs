import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, mkdirSync, copyFileSync, writeFileSync } from 'node:fs';
import { join, basename } from 'node:path';
import { REPOSITORY } from './config.mjs';

export function releaseMetadata({ version, channel, files, read, now }) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !['production', 'staging'].includes(channel)) throw new Error('Invalid release identity');
  const pick = suffix => {
    const matches = files.filter(file => file.endsWith(suffix));
    if (matches.length !== 1) throw new Error(`Expected exactly one ${suffix} asset`);
    return matches[0];
  };
  const mac = pick('.dmg'), windows = pick('.exe'), archive = pick('.app.tar.gz');
  const signatures = [archive, windows].map(file => {
    const path = `${file}.sig`;
    if (!files.includes(path)) throw new Error('Missing signed update artifact');
    const signature = read(path).toString().trim();
    if (!signature) throw new Error('Empty update signature');
    return signature;
  });
  const tag = channel === 'production' ? `v${version}` : 'staging';
  const assetUrl = file => `https://github.com/${REPOSITORY}/releases/download/${tag}/${encodeURIComponent(basename(file))}`;
  const macUpdate = { signature: signatures[0], url: assetUrl(archive) };
  const checksums = {
    mac_dmg_sha256: createHash('sha256').update(read(mac)).digest('hex'),
    windows_exe_sha256: createHash('sha256').update(read(windows)).digest('hex'),
  };
  return {
    tag, files: [mac, windows, archive, `${archive}.sig`, `${windows}.sig`], checksums,
    updater: { version, notes: `Cognuum ${version}`, pub_date: now, platforms: {
      'darwin-aarch64': macUpdate, 'darwin-x86_64': macUpdate,
      'windows-x86_64': { signature: signatures[1], url: assetUrl(windows) },
    } },
  };
}

if (process.argv[1]?.endsWith('/release-assets.mjs') || process.argv[1]?.endsWith('\\release-assets.mjs')) {
  const root = 'release-artifacts';
  const walk = dir => readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? walk(join(dir, entry.name)) : [join(dir, entry.name)]);
  const version = JSON.parse(readFileSync('src-tauri/tauri.conf.json')).version;
  const metadata = releaseMetadata({ version, channel: process.env.CHANNEL, files: walk(root), read: readFileSync, now: new Date().toISOString() });
  const upload = join(root, 'upload'); mkdirSync(upload);
  for (const file of metadata.files) copyFileSync(file, join(upload, basename(file)));
  writeFileSync(join(upload, 'latest.json'), JSON.stringify(metadata.updater, null, 2));
  writeFileSync(join(root, 'tag.txt'), metadata.tag);
  writeFileSync(join(root, 'notes.md'), `Cognuum ${version} for macOS (Apple Silicon and Intel) and Windows x64.\n\nProduction: https://access.cognuum.com\n\n<!-- CHECKSUMS\n${JSON.stringify(metadata.checksums, null, 2)}\n-->\n`);
}
