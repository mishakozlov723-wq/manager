// The update manifest (latest.json) the manager's updater reads (src-tauri/src/upd.rs): which version is out,
// where its exe is, its size and SHA-256.
//   node scripts/manifest.mjs <exe> <version> [url] [out.json]
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';

const [exe, ver, url0, out] = process.argv.slice(2);
if (!exe || !ver) {
  console.error('usage: node scripts/manifest.mjs <exe> <version> [url] [out.json]');
  process.exit(2);
}
const data = readFileSync(exe);
const sha256 = createHash('sha256').update(data).digest('hex');
// the notes: NOTES, or the first plain line of RELEASE.md
let notes = process.env.NOTES;
if (notes === undefined) {
  try {
    notes = readFileSync('RELEASE.md', 'utf8').split(/\r?\n/).map(s => s.trim()).find(s => s && !s.startsWith('#')) || '';
    notes = notes.replace(/\*\*|__|`/g, '').replace(/^[-*] /, '');
  } catch { notes = ''; }
}
const url = url0 || `https://github.com/mishakozlov723-wq/manager/releases/download/v${ver}/Retocker-Manager-Setup.exe`;
const size = (data.length / 1048576).toFixed(1).replace('.', ',') + ' МБ';
const json = JSON.stringify({ version: ver, notes, size, bytes: data.length, url, sha256 }, null, 2) + '\n';
if (out) writeFileSync(out, json); else process.stdout.write(json);
