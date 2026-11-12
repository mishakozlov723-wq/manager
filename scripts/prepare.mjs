// Gets the app ready for a build: stamps the version.
// node scripts/prepare.mjs [version]
import { readFileSync, writeFileSync } from 'node:fs';

const ver = process.argv[2];
if (ver) {
  const p = 'src-tauri/tauri.conf.json';
  const c = JSON.parse(readFileSync(p, 'utf8'));
  c.version = ver;
  writeFileSync(p, JSON.stringify(c, null, 2) + '\n');
}
console.log('ready', ver || '');
