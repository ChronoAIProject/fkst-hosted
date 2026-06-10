// Mirror the unified product version into backend/Cargo.toml AND backend/Cargo.lock.
//
// The root package.json "version" is the single source of truth for the
// product version; backend/Cargo.toml's [workspace.package] version line
// mirrors it (the member crates inherit via `version.workspace = true`).
// This script rewrites ONLY the first `version = "..."` line whose active
// TOML table is [package] or [workspace.package]; dependency version strings
// (e.g. under [dependencies]) are never touched.
//
// Because backend/Cargo.lock is committed and every CI/build consumer runs
// with `--locked`, the lock's pinned workspace-member versions must move in
// lockstep: the CLI also rewrites the `version = "..."` line of every
// [[package]] block in Cargo.lock that has NO `source = ` line (source-less
// blocks are exactly the local workspace members; registry/git dependencies
// always carry a `source`). Re-running with the values already present yields
// byte-identical files (zero diff).
//
// No external dependencies — Node builtins only.

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
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

/**
 * Rewrite the `version = "..."` line of every [[package]] block WITHOUT a
 * `source = ` line (the local workspace members) in a Cargo.lock string to
 * `newVersion`. Pure: returns the rewritten content. Registry/git packages
 * (which always carry a `source`) and the unquoted lockfile-format
 * `version = N` header are never touched. Throws when no source-less
 * [[package]] block exists.
 */
export function mirrorCargoLockVersion(content, newVersion) {
  const lines = String(content).split('\n');

  // Collect the [from, to) line ranges of every [[package]] block; a block
  // ends at the next [...] / [[...]] header or EOF.
  const blocks = [];
  let start = -1;
  for (let i = 0; i < lines.length; i += 1) {
    if (!/^\s*\[\[?[^\]]+\]\]?\s*(?:#.*)?$/.test(lines[i])) continue;
    if (start !== -1) blocks.push([start, i]);
    start = /^\s*\[\[package\]\]\s*(?:#.*)?$/.test(lines[i]) ? i : -1;
  }
  if (start !== -1) blocks.push([start, lines.length]);

  let rewroteAny = false;
  for (const [from, to] of blocks) {
    if (lines.slice(from, to).some((l) => /^\s*source\s*=/.test(l))) continue;
    for (let i = from; i < to; i += 1) {
      const m = lines[i].match(/^(\s*version\s*=\s*")[^"]*(".*)$/);
      if (m) {
        lines[i] = `${m[1]}${newVersion}${m[2]}`;
        rewroteAny = true;
        break;
      }
    }
  }
  if (!rewroteAny) {
    throw new Error('no source-less [[package]] block (workspace member) found in Cargo.lock');
  }
  return lines.join('\n');
}

/** Read a file, apply a pure rewrite, and write only when the content changed. */
function rewriteFile(path, newVersion, rewrite) {
  let before;
  try {
    before = readFileSync(path, 'utf8');
  } catch {
    throw new Error(`cannot read ${path}`);
  }
  const after = rewrite(before, newVersion);
  if (after === before) {
    console.log(`INFO: ${path} already at version ${newVersion}; nothing to write.`);
  } else {
    writeFileSync(path, after);
    console.log(`INFO: mirrored version ${newVersion} into ${path}.`);
  }
}

// --- CLI ---------------------------------------------------------------------
const isMain = import.meta.url === pathToFileURL(argv[1] || '').href;
if (isMain) {
  try {
    const version = argv[2];
    const manifest = argv[3] || 'backend/Cargo.toml';
    if (!version) throw new Error('usage: mirror-cargo-version.mjs <newVersion> [cargoTomlPath]');
    rewriteFile(manifest, version, mirrorCargoVersion);
    // The committed lockfile must move in lockstep or every `--locked`
    // consumer (rust-ci, the Dockerfile, the build gate) hard-fails on the
    // stale member pin. A missing lock next to a present manifest is itself
    // a broken state in a committed-lock repo, so it fails loudly too.
    const lock = join(dirname(manifest), 'Cargo.lock');
    rewriteFile(lock, version, mirrorCargoLockVersion);
  } catch (err) {
    console.error(`ERROR: ${err.message}`);
    exit(1);
  }
}
