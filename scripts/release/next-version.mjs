// Compute the next SemVer version for fkst-hosted from the pending changesets.
//
// Changesets here drive ONLY the version number (release notes live in
// release-notes/ + CHANGELOG.md). We read every .changeset/*.md (excluding
// README.md), take the highest declared bump (major > minor > patch), and apply
// it to a base version supplied by the caller (the latest git tag, or the root
// package.json when there is no tag yet).
//
// No external dependencies — Node builtins only.

import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { argv, exit } from 'node:process';
import { pathToFileURL } from 'node:url';

const RANK = { patch: 0, minor: 1, major: 2 };
const NAMES = ['patch', 'minor', 'major'];

/** Highest bump level among the pending changesets, or null if there are none. */
export function highestBump(changesetDir) {
  let files;
  try {
    files = readdirSync(changesetDir);
  } catch {
    return null;
  }
  let best = -1;
  for (const f of files) {
    if (!f.endsWith('.md') || f.toLowerCase() === 'readme.md') continue;
    const content = readFileSync(join(changesetDir, f), 'utf8');
    // frontmatter lines look like:  "fkst-hosted": minor
    for (const m of content.matchAll(/:\s*(major|minor|patch)\b/gi)) {
      const r = RANK[m[1].toLowerCase()];
      if (r > best) best = r;
    }
  }
  return best === -1 ? null : NAMES[best];
}

/** Apply a bump level to a base version, returning the new `x.y.z` string. */
export function nextVersion(base, level) {
  const m = String(base).trim().replace(/^v/, '').match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!m) throw new Error(`invalid base version: ${base}`);
  let [major, minor, patch] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (level === 'major') (major += 1), (minor = 0), (patch = 0);
  else if (level === 'minor') (minor += 1), (patch = 0);
  else if (level === 'patch') patch += 1;
  else throw new Error(`invalid bump level: ${level}`);
  return `${major}.${minor}.${patch}`;
}

// --- CLI ---------------------------------------------------------------------
const isMain = import.meta.url === pathToFileURL(argv[1] || '').href;
if (isMain) {
  try {
    const base = argv[2];
    const dir = argv[3] || '.changeset';
    if (!base) throw new Error('usage: next-version.mjs <baseVersion> [changesetDir]');
    const level = highestBump(dir);
    if (!level) {
      console.error('ERROR: no pending changesets found — nothing to release');
      exit(1);
    }
    console.log(nextVersion(base, level));
  } catch (err) {
    console.error(`ERROR: ${err.message}`);
    exit(1);
  }
}
