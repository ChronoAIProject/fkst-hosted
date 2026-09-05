// Integration cover for `resolveInputs` — the single definition of the
// convergence-bearing inputs (spec sections 17.3-17.5).
//
// These run against THIS repository's real Git tree rather than a fixture,
// because the property under test is that the generator half of the fingerprint
// is derived from the revision being evaluated. A fixture with a hand-written
// tree would not exercise that.
//
// WHY this file exists: an earlier verifier computed the generator tree at the
// MANIFEST's `observedHead` while computing the source tree at the current
// revision. Condition 1 then compared the manifest's generator fingerprint
// against itself, so a changed generator could never fail convergence and the
// "generatorPinnedFingerprint moved" rebuild trigger could never fire. The bug
// was in wiring, not in the decision logic, so it needs a wiring test.

import { strict as assert } from 'node:assert';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { resolveInputs, resolveRef } from '../src/build.ts';
import { parseConfig } from '../src/config.ts';
import { parsePlan } from '../src/plan.ts';

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');

const config = parseConfig(readFileSync(join(REPO, '.fkst/evolution/config.yaml'), 'utf8'));
const plan = parsePlan(readFileSync(join(REPO, 'tools/evolution/artifact-plan.yaml'), 'utf8'));

const isSha256 = (value: string) => /^sha256:[0-9a-f]{64}$/.test(value);

test('resolveInputs yields well-formed digests at HEAD', async () => {
  const { source, pinned, input } = await resolveInputs(REPO, config, plan, 'HEAD');
  assert.ok(isSha256(source.productRelevant), source.productRelevant);
  assert.ok(isSha256(source.coverage), source.coverage);
  assert.ok(isSha256(pinned), pinned);
  assert.ok(isSha256(input), input);
  assert.match(source.observedHead, /^[0-9a-f]{40}$/);
});

test('the generator is resolved at the REVISION being evaluated, not a fixed head', async () => {
  const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: REPO, encoding: 'utf8' }).trim();
  const parent = execFileSync('git', ['rev-parse', 'HEAD~1'], { cwd: REPO, encoding: 'utf8' }).trim();

  const atHead = await resolveInputs(REPO, config, plan, head);
  const atParent = await resolveInputs(REPO, config, plan, parent);

  assert.equal(atHead.source.observedHead, head);
  assert.equal(atParent.source.observedHead, parent);
  assert.match(atHead.packages[0].ref, /@[0-9a-f]{40}:/, 'the ref must be post-resolution');
});

test('a package ref resolves to the last commit touching it, not the branch head', async () => {
  // The property that makes the post-merge no-op reachable: a commit that does
  // not touch the generator must not move the generator fingerprint. Without it
  // the very commit that writes the manifest invalidates the manifest.
  const expected = execFileSync(
    'git',
    ['log', '-1', '--format=%H', 'HEAD', '--', 'tools/evolution'],
    { cwd: REPO, encoding: 'utf8' }
  ).trim();
  const resolved = await resolveRef(
    REPO,
    'HEAD',
    'ChronoAIProject/fkst-hosted@{commit}:tools/evolution'
  );
  assert.equal(resolved, `ChronoAIProject/fkst-hosted@${expected}:tools/evolution`);
});

test('a ref carrying no placeholder is already resolved and passes through', async () => {
  const fixed = 'ChronoAIProject/fkst-packages@abc123:packages/observer';
  assert.equal(await resolveRef(REPO, 'HEAD', fixed), fixed);
});

test('a placeholder ref with no path component is rejected', async () => {
  await assert.rejects(
    () => resolveRef(REPO, 'HEAD', 'owner/repo@{commit}:'),
    /no path component/
  );
});

test('generatorEpoch moves the pinned and input fingerprints but not the source tree', async () => {
  const base = await resolveInputs(REPO, config, plan, 'HEAD');
  const bumped = await resolveInputs(
    REPO,
    { ...config, generatorEpoch: config.generatorEpoch + 1 },
    plan,
    'HEAD'
  );
  // The owner's deliberate regeneration lever (section 32.3) must reach the
  // convergence-bearing fingerprint...
  assert.notEqual(base.pinned, bumped.pinned);
  assert.notEqual(base.input, bumped.input);
  // ...without pretending the source changed.
  assert.equal(base.source.productRelevant, bumped.source.productRelevant);
});

test('a product-relevant selector change moves the source fingerprint', async () => {
  const base = await resolveInputs(REPO, config, plan, 'HEAD');
  const narrowed = await resolveInputs(
    REPO,
    { ...config, source: { ...config.source, productRelevant: { include: ['backend/src/**'], exclude: [] } } },
    plan,
    'HEAD'
  );
  assert.notEqual(base.source.productRelevant, narrowed.source.productRelevant);
  assert.notEqual(base.input, narrowed.input);
});

test('owner intent is inside the product-relevant fingerprint', async () => {
  // Section 17.3 folds config.yaml and intent/** in unconditionally. Without it
  // an owner could rewrite product positioning and nothing would regenerate.
  const { source } = await resolveInputs(REPO, config, plan, 'HEAD');
  assert.ok(source.productRelevantPaths.includes('.fkst/evolution/config.yaml'));
  assert.ok(source.productRelevantPaths.includes('.fkst/evolution/intent/product.md'));
  assert.ok(source.productRelevantPaths.includes('.fkst/evolution/intent/overrides.yaml'));
});

test('generated output is NOT inside either source fingerprint', async () => {
  const { source } = await resolveInputs(REPO, config, plan, 'HEAD');
  const generated = (p: string) =>
    p.startsWith('.fkst/evolution/docs/') ||
    p.startsWith('.fkst/evolution/screenshots/') ||
    p === '.fkst/evolution/manifest.json';
  assert.equal(source.productRelevantPaths.filter(generated).length, 0);
  assert.equal(source.coveragePaths.filter(generated).length, 0);
});
