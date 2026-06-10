import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mirrorCargoVersion } from './mirror-cargo-version.mjs';

const SAMPLE = [
  '[workspace]',
  'members = ["fkst-hosted-api"]',
  'resolver = "2"',
  '',
  '[workspace.package]',
  'version = "0.0.0"',
  'edition = "2021"',
  '',
  '[dependencies]',
  'axum = "0.7"',
  'serde = { version = "1" }',
  '',
].join('\n');

test('rewrites only the [workspace.package] version line', () => {
  const out = mirrorCargoVersion(SAMPLE, '1.2.3');
  assert.match(out, /\[workspace\.package\]\nversion = "1\.2\.3"\n/);
  assert.ok(out.includes('axum = "0.7"'), 'dependency string version must stay untouched');
  assert.ok(out.includes('serde = { version = "1" }'), 'inline-table dependency version must stay untouched');
  assert.ok(!out.includes('version = "0.0.0"'), 'old workspace version must be gone');
});

test('idempotent: re-applying the same version is byte-identical', () => {
  const once = mirrorCargoVersion(SAMPLE, '1.2.3');
  const twice = mirrorCargoVersion(once, '1.2.3');
  assert.equal(twice, once);
});

test('missing target version line throws', () => {
  assert.throws(
    () => mirrorCargoVersion('[dependencies]\naxum = "0.7"\n', '1.2.3'),
    /no version line found/
  );
});
