import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { nextVersion, highestBump } from './next-version.mjs';

test('patch bump increments patch', () => assert.equal(nextVersion('1.2.3', 'patch'), '1.2.4'));
test('minor bump resets patch', () => assert.equal(nextVersion('1.2.3', 'minor'), '1.3.0'));
test('major bump resets minor and patch', () => assert.equal(nextVersion('1.2.3', 'major'), '2.0.0'));
test('strips a leading v from the base', () => assert.equal(nextVersion('v0.0.0', 'minor'), '0.1.0'));
test('invalid base throws', () => assert.throws(() => nextVersion('nope', 'patch')));

test('highestBump picks the strongest bump and ignores README', () => {
  const dir = mkdtempSync(join(tmpdir(), 'cs-'));
  try {
    writeFileSync(join(dir, 'README.md'), '"fkst-hosted": major\n');
    writeFileSync(join(dir, 'a.md'), '---\n"fkst-hosted": patch\n---\nfix\n');
    writeFileSync(join(dir, 'b.md'), '---\n"fkst-hosted": minor\n---\nfeature\n');
    assert.equal(highestBump(dir), 'minor');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('highestBump returns null when there are no changesets', () => {
  const dir = mkdtempSync(join(tmpdir(), 'cs-'));
  try {
    writeFileSync(join(dir, 'README.md'), 'readme\n');
    assert.equal(highestBump(dir), null);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
