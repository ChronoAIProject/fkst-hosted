// Change-impact analysis for captures (spec section 32.2).
//
// The property under test is the one that makes media artifacts viable at all:
// a run that changed nothing must leave the committed bytes exactly as they
// were, even though the freshly captured bytes differ. Headless Chromium is not
// bit-reproducible, so "the new bytes differ" is the normal case, not a signal.

import { strict as assert } from 'node:assert';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { adoptCaptures, captureIdOf, currentJourneyHashes } from '../src/captures.ts';
import { contentHash } from '../src/hash.ts';
import type { Manifest } from '../src/manifest.ts';

const COMMITTED = Buffer.from('committed capture bytes', 'utf8');
const FRESH = Buffer.from('freshly rasterized bytes, visually identical', 'utf8');
const JOURNEY_SRC = Buffer.from('journey spec v1', 'utf8');
const INPUT = `sha256:${'1'.repeat(64)}`;

const SHOT_PATH = '.fkst/evolution/screenshots/sessions-level.png';
const JOURNEY_PATH = '.fkst/evolution/journeys/queue-work-item.spec.ts';

function manifest(overrides: { input?: string; journeyHash?: string } = {}): Manifest {
  const base = {
    locale: 'en', audience: 'a', capabilities: ['cap_1'], journeys: ['jny_1'],
    sourceCommit: 'a'.repeat(40), inputFingerprint: INPUT,
    generatorPinnedFingerprint: `sha256:${'5'.repeat(64)}`,
    status: 'current' as const, required: true, verification: 'passed',
    updatedAt: '2026-01-01T00:00:00Z',
  };
  return {
    schemaVersion: 1,
    source: {
      repository: 'acme/app', branch: 'main', observedHead: 'a'.repeat(40),
      previousCoveredHead: 'a'.repeat(40), historyRelation: 'fast-forward',
      productRelevantFingerprint: `sha256:${'3'.repeat(64)}`,
      coverageFingerprint: `sha256:${'4'.repeat(64)}`,
      inputFingerprint: overrides.input ?? INPUT,
    },
    artifactRepository: { repository: 'acme/app', branch: 'main' },
    generator: {
      manifestRef: 'x', packages: [], generatorEpoch: 1,
      pinnedFingerprint: `sha256:${'5'.repeat(64)}`, provenanceOnly: [],
      engineVersion: '0.1.0', model: 'test', toolchain: {},
      envFingerprint: `sha256:${'6'.repeat(64)}`,
    },
    outputFingerprint: `sha256:${'2'.repeat(64)}`,
    verification: { status: 'passed', verifiedAt: '2026-01-01T00:00:00Z', checks: [] },
    artifacts: [
      {
        ...base, id: 'screenshot.sessions-level', kind: 'screenshots',
        repositoryPath: SHOT_PATH, contentHash: contentHash(COMMITTED),
      },
      {
        ...base, id: 'journey.queue-work-item', kind: 'journeys',
        repositoryPath: JOURNEY_PATH,
        contentHash: overrides.journeyHash ?? contentHash(JOURNEY_SRC),
      },
    ],
  };
}

/** A scratch repository with a committed capture, a journey, and a fresh capture. */
async function scratch(opts: { withFresh?: boolean; withCommitted?: boolean } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'fkst-captures-'));
  await mkdir(join(root, '.fkst/evolution/screenshots'), { recursive: true });
  await mkdir(join(root, '.fkst/evolution/journeys'), { recursive: true });
  await mkdir(join(root, 'tools/evolution/out/captures'), { recursive: true });
  if (opts.withCommitted !== false) await writeFile(join(root, SHOT_PATH), COMMITTED);
  await writeFile(join(root, JOURNEY_PATH), JOURNEY_SRC);
  if (opts.withFresh !== false) {
    await writeFile(join(root, 'tools/evolution/out/captures/sessions-level.png'), FRESH);
  }
  return root;
}

const freshDir = (root: string) => join(root, 'tools/evolution/out/captures');

test('an unchanged run keeps the committed bytes even though fresh bytes differ', async () => {
  const root = await scratch();
  try {
    const m = manifest();
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m, currentInputFingerprint: INPUT,
      currentJourneyHashes: await currentJourneyHashes(root, m),
      freshCaptureDir: freshDir(root),
    });
    assert.equal(decisions[0].adopted, false);
    assert.equal(decisions[0].reason, 'unchanged-inputs');
    // The whole point: byte-for-byte untouched, so the output fingerprint holds.
    assert.deepEqual(await readFile(join(root, SHOT_PATH)), COMMITTED);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('a moved input fingerprint adopts the fresh capture', async () => {
  const root = await scratch();
  try {
    const m = manifest();
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m,
      currentInputFingerprint: `sha256:${'9'.repeat(64)}`,
      currentJourneyHashes: await currentJourneyHashes(root, m),
      freshCaptureDir: freshDir(root),
    });
    assert.equal(decisions[0].adopted, true);
    assert.equal(decisions[0].reason, 'input-fingerprint-moved');
    assert.deepEqual(await readFile(join(root, SHOT_PATH)), FRESH);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('a changed journey adopts the fresh capture even when the input is unchanged', async () => {
  // The journey lives under the Evolution root, so editing it does NOT move the
  // input fingerprint. Without this branch a rewritten journey would keep
  // publishing screenshots of the flow it no longer performs.
  const root = await scratch();
  try {
    const m = manifest({ journeyHash: `sha256:${'7'.repeat(64)}` });
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m, currentInputFingerprint: INPUT,
      currentJourneyHashes: await currentJourneyHashes(root, m),
      freshCaptureDir: freshDir(root),
    });
    assert.equal(decisions[0].adopted, true);
    assert.equal(decisions[0].reason, 'journey-changed');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('a missing committed capture is always adopted', async () => {
  const root = await scratch({ withCommitted: false });
  try {
    const m = manifest();
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m, currentInputFingerprint: INPUT,
      currentJourneyHashes: await currentJourneyHashes(root, m),
      freshCaptureDir: freshDir(root),
    });
    assert.equal(decisions[0].adopted, true);
    assert.equal(decisions[0].reason, 'no-committed-bytes');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('a baseline run with no manifest adopts everything', async () => {
  const root = await scratch();
  try {
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: null, currentInputFingerprint: INPUT,
      currentJourneyHashes: new Map(), freshCaptureDir: freshDir(root),
    });
    // No manifest means no capture records to iterate, so nothing is decided —
    // the baseline's captures are adopted by the journey writing them and the
    // first manifest recording them.
    assert.equal(decisions.length, 0);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('wanting a refresh with no fresh bytes keeps the last known good artifact', async () => {
  // Section 26.5: a failed run preserves the last known good canonical artifact.
  // It must be visible, though — the artifact is now older than its inputs.
  const root = await scratch({ withFresh: false });
  try {
    const m = manifest();
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m,
      currentInputFingerprint: `sha256:${'9'.repeat(64)}`,
      currentJourneyHashes: await currentJourneyHashes(root, m),
      freshCaptureDir: freshDir(root),
    });
    assert.equal(decisions[0].adopted, false);
    assert.equal(decisions[0].reason, 'no-fresh-capture');
    assert.deepEqual(await readFile(join(root, SHOT_PATH)), COMMITTED);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('an unreadable journey source is treated as changed, not as unchanged', async () => {
  // Failing towards regeneration is recoverable. Failing towards "keep" would
  // let a stale capture outlive the journey that justified it.
  const root = await scratch();
  try {
    const m = manifest();
    const hashes = await currentJourneyHashes(root, m);
    hashes.delete('jny_1');
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m, currentInputFingerprint: INPUT,
      currentJourneyHashes: hashes, freshCaptureDir: freshDir(root),
    });
    assert.equal(decisions[0].reason, 'journey-changed');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('only capture kinds are considered', async () => {
  const root = await scratch();
  try {
    const m = manifest();
    const decisions = await adoptCaptures({
      repoRoot: root, manifest: m, currentInputFingerprint: INPUT,
      currentJourneyHashes: await currentJourneyHashes(root, m),
      freshCaptureDir: freshDir(root),
    });
    // The journey artifact is authored content, not a capture — it must never be
    // overwritten by this path.
    assert.deepEqual(decisions.map((d) => d.artifactId), ['screenshot.sessions-level']);
    assert.deepEqual(await readFile(join(root, JOURNEY_PATH)), JOURNEY_SRC);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('captureIdOf maps an artifact id to the journey capture name', () => {
  assert.equal(captureIdOf('screenshot.sessions-level'), 'sessions-level');
  assert.equal(captureIdOf('screenshot.work.composer'), 'work.composer');
  assert.throws(() => captureIdOf('nodot'), /must be/);
});
