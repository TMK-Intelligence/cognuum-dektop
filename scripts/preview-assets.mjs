import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, mkdirSync, copyFileSync, writeFileSync } from 'node:fs';
import { join, basename } from 'node:path';

export function previewConfig(version) {
  if (!/^\d+\.\d+\.\d+-preview\.[1-9]\d*$/.test(version ?? '')) throw new Error('Expected an explicit preview version');
  return { version, bundle: { createUpdaterArtifacts: false, macOS: { signingIdentity: '-' } } };
}

export function previewMetadata({ version, files, read }) {
  previewConfig(version);
  const pick = suffix => {
    const matches = files.filter(file => file.endsWith(suffix));
    if (matches.length !== 1) throw new Error(`Expected exactly one ${suffix} installer`);
    return matches[0];
  };
  const mac = pick('.dmg'), windows = pick('.exe');
  const checksums = {
    mac_dmg_sha256: createHash('sha256').update(read(mac)).digest('hex'),
    windows_exe_sha256: createHash('sha256').update(read(windows)).digest('hex'),
  };
  return { tag: `v${version}`, files: [mac, windows], checksums };
}

if (process.argv[1]?.replaceAll('\\', '/').endsWith('/preview-assets.mjs')) {
  const version = process.env.PREVIEW_VERSION;
  if (process.argv[2] === 'prepare') {
    writeFileSync('preview-config.json', JSON.stringify(previewConfig(version), null, 2));
  } else if (process.argv[2] === 'assemble') {
    const root = 'preview-artifacts';
    const walk = dir => readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? walk(join(dir, entry.name)) : [join(dir, entry.name)]);
    const metadata = previewMetadata({ version, files: walk(root), read: readFileSync });
    const upload = join(root, 'upload'); mkdirSync(upload);
    for (const file of metadata.files) copyFileSync(file, join(upload, basename(file)));
    writeFileSync(join(root, 'tag.txt'), metadata.tag);
    writeFileSync(join(root, 'notes.md'), `<!-- COGNUUM_UNSIGNED_PREVIEW -->\n# Unsigned preview\n\nCognuum ${version} for macOS 13+ (Apple Silicon and Intel) and Windows 11 x64.\n\nThese preview installers have no verified publisher signature. The Mac app is ad-hoc signed, not Apple notarized; Windows has no Authenticode signature. Your operating system may block installation. Automatic updates are unavailable. Signed releases will follow once developer enrollment and certificates are ready.\n\nThe app connects to production at https://access.cognuum.com and requires an internet connection and your existing account. Account access rules still apply.\n\n<!-- CHECKSUMS\n${JSON.stringify(metadata.checksums, null, 2)}\n-->\n`);
  } else throw new Error('Expected prepare or assemble');
}
