import { writeFileSync } from 'node:fs';
import { releaseConfig } from './config.mjs';
const channel = process.argv[2];
try {
  writeFileSync('release-config.json', JSON.stringify(releaseConfig(channel, process.platform, process.env), null, 2));
} catch (error) { console.error(error.message); process.exitCode = 1; }
