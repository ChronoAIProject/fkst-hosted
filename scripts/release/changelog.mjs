// CHANGELOG.md manipulation for the fkst-hosted release pipeline.
//
// CHANGELOG.md uses two HTML-comment markers to delimit the "pending" release
// section (the in-progress release being prepared on a develop -> main PR):
//
//   <!-- BEGIN PENDING -->
//   ## vX.Y.Z — DATE
//   <release notes>
//   <!-- END PENDING -->
//   ## v... (older, frozen releases below)
//
// Two operations:
//   set-pending : (re)write the pending block from a release-note file. Idempotent.
//   freeze      : move the pending entry out below END (permanent) and empty the
//                 pending block, so the next release prepends above it.
//
// No external dependencies — Node builtins only.

import { readFileSync, writeFileSync } from 'node:fs';
import { argv, exit } from 'node:process';
import { pathToFileURL } from 'node:url';

export const BEGIN = '<!-- BEGIN PENDING -->';
export const END = '<!-- END PENDING -->';
const CHANGELOG = 'CHANGELOG.md';

/** Remove a single leading HTML comment block (the template's instructions). */
export function stripLeadingComment(note) {
  let s = String(note).replace(/^﻿/, '').trimStart();
  if (s.startsWith('<!--')) {
    const i = s.indexOf('-->');
    if (i !== -1) s = s.slice(i + 3).trimStart();
  }
  return s;
}

function normalize(doc) {
  return doc.replace(/\n{3,}/g, '\n\n').replace(/\s*$/, '\n');
}

function split(changelog) {
  const b = changelog.indexOf(BEGIN);
  const e = changelog.indexOf(END);
  if (b === -1 || e === -1 || e < b) {
    throw new Error(`${CHANGELOG} is missing the ${BEGIN} / ${END} markers`);
  }
  return {
    head: changelog.slice(0, b),
    inner: changelog.slice(b + BEGIN.length, e),
    tail: changelog.slice(e + END.length),
  };
}

/** Replace the pending block with the given release as `## vX.Y.Z — DATE` + notes. */
export function setPending(changelog, version, date, note) {
  const { head, tail } = split(changelog);
  const body = stripLeadingComment(note).trim();
  const entry = `## v${version} — ${date}\n\n${body}`;
  return normalize(`${head}${BEGIN}\n${entry}\n${END}${tail}`);
}

/** Move the current pending entry below END (frozen) and empty the pending block. */
export function freeze(changelog) {
  const { head, inner, tail } = split(changelog);
  const entry = inner.trim();
  if (entry === '') return normalize(`${head}${BEGIN}\n${END}${tail}`);
  return normalize(`${head}${BEGIN}\n${END}\n\n${entry}\n${tail}`);
}

// --- CLI ---------------------------------------------------------------------
const isMain = import.meta.url === pathToFileURL(argv[1] || '').href;
if (isMain) {
  try {
    const cmd = argv[2];
    if (cmd === 'set-pending') {
      const [version, date, noteFile] = argv.slice(3);
      if (!version || !date || !noteFile) {
        throw new Error('usage: changelog.mjs set-pending <version> <date> <noteFile>');
      }
      const out = setPending(readFileSync(CHANGELOG, 'utf8'), version, date, readFileSync(noteFile, 'utf8'));
      writeFileSync(CHANGELOG, out);
      console.log(`INFO: set pending CHANGELOG section to v${version} — ${date}`);
    } else if (cmd === 'freeze') {
      writeFileSync(CHANGELOG, freeze(readFileSync(CHANGELOG, 'utf8')));
      console.log('INFO: froze the pending CHANGELOG section');
    } else {
      throw new Error(`unknown command: ${cmd ?? '(none)'} (expected set-pending|freeze)`);
    }
  } catch (err) {
    console.error(`ERROR: ${err.message}`);
    exit(1);
  }
}
