// Mirror the unified product version into backend/Cargo.toml.
//
// The root package.json "version" is the single source of truth for the
// product version; backend/Cargo.toml's [workspace.package] version line
// mirrors it (the member crates inherit via `version.workspace = true`).
// This script rewrites ONLY the first `version = "..."` line whose active
// TOML table is [package] or [workspace.package]; dependency version strings
// (e.g. under [dependencies]) are never touched. Re-running with the value
// already present yields a byte-identical file (zero diff).
//
// No external dependencies — Node builtins only.

import { readFileSync, writeFileSync } from 'node:fs';
import { argv, exit } from 'node:process';
import { pathToFileURL } from 'node:url';

const VERSIONED_TABLES = new Set(['package', 'workspace.package']);

/**
 * Rewrite the [package]/[workspace.package] `version = "..."` line of a
 * Cargo.toml string to `newVersion`. Pure: returns the rewritten content.
 * Throws when no eligible version line exists.
 */
export function mirrorCargoVersion(content, newVersion) {
  const lines = String(content).split('\n');
  let table = '';
  let replaced = false;
  for (let i = 0; i < lines.length; i += 1) {
    const header = lines[i].match(/^\s*\[([^\]]+)\]\s*(?:#.*)?$/);
    if (header) {
      table = header[1].trim();
      continue;
    }
    if (replaced || !VERSIONED_TABLES.has(table)) continue;
    const m = lines[i].match(/^(\s*version\s*=\s*")[^"]*(".*)$/);
    if (m) {
      lines[i] = `${m[1]}${newVersion}${m[2]}`;
      replaced = true;
    }
  }
  if (!replaced) {
    throw new Error('no version line found under [package] or [workspace.package]');
  }
  return lines.join('\n');
}

// --- CLI ---------------------------------------------------------------------
const isMain = import.meta.url === pathToFileURL(argv[1] || '').href;
if (isMain) {
  try {
    const version = argv[2];
    const manifest = argv[3] || 'backend/Cargo.toml';
    if (!version) throw new Error('usage: mirror-cargo-version.mjs <newVersion> [cargoTomlPath]');
    let before;
    try {
      before = readFileSync(manifest, 'utf8');
    } catch {
      throw new Error(`cannot read ${manifest}`);
    }
    const after = mirrorCargoVersion(before, version);
    if (after === before) {
      console.log(`INFO: ${manifest} already at version ${version}; nothing to write.`);
    } else {
      writeFileSync(manifest, after);
      console.log(`INFO: mirrored version ${version} into ${manifest}.`);
    }
  } catch (err) {
    console.error(`ERROR: ${err.message}`);
    exit(1);
  }
}
