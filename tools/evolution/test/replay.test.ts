// The Phase 1 synthetic-baseline replay (spec section 37).
//
// Run against this repository's real history rather than a fixture: the point of
// the oracle is to measure what a selector would have done over commits nobody
// arranged for it, and a hand-built range would measure only the arrangement.

import { strict as assert } from 'node:assert';
import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { formatReport, replay, type Candidate } from '../src/replay.ts';
import { isOwnerIntentPath } from '../src/selector.ts';

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');

const changedPathsOf = async (sha: string): Promise<string[]> =>
  execFileSync('git', ['diff-tree', '--no-commit-id', '--name-only', '-r', '-m', '--first-parent', sha], {
    cwd: REPO,
    encoding: 'utf8',
  })
    .split('\n')
    .filter(Boolean);
const COVERAGE = { include: ['**'], exclude: ['.git/**'] };

const CANDIDATES: Candidate[] = [
  { name: 'backend only', selector: { include: ['backend/src/**'], exclude: [] } },
  { name: 'everything', selector: { include: ['**'], exclude: [] } },
];

test('replay scores every candidate over the range', async () => {
  const report = await replay(REPO, 'acme/app', 'HEAD~20..HEAD', CANDIDATES, COVERAGE);
  assert.equal(report.commits, report.timeline.length);
  assert.ok(report.commits > 0 && report.commits <= 20);
  assert.equal(report.candidates.length, 2);
  for (const c of report.candidates) {
    assert.equal(c.admitted + c.coverageOnly, report.commits);
    assert.ok(c.admissionRate >= 0 && c.admissionRate <= 1, String(c.admissionRate));
  }
});

test('a broader selector never admits fewer commits than a narrower one it contains', async () => {
  // `everything` is a strict superset of `backend only`, so its admission count
  // is a monotone upper bound. A violation would mean the matcher is not
  // actually applying the include set.
  const report = await replay(REPO, 'acme/app', 'HEAD~20..HEAD', CANDIDATES, COVERAGE);
  const narrow = report.candidates.find((c) => c.name === 'backend only')!;
  const broad = report.candidates.find((c) => c.name === 'everything')!;
  assert.ok(broad.admitted >= narrow.admitted, `${broad.admitted} >= ${narrow.admitted}`);
});

test('a reserved-only commit admits ONLY when it touched owner intent', async () => {
  // `reservedOnly` means the commit touched nothing the COVERAGE selector sees,
  // because section 17.3 removes `.fkst/evolution/**` and `.fkst/packages/**`
  // from both fingerprints. But that same section folds `config.yaml` and
  // `intent/**` back into productRelevant — an owner who rewrites product
  // positioning MUST cause regeneration. So such a commit is reserved-only for
  // coverage and still admissible, and the two must not be conflated.
  const report = await replay(REPO, 'acme/app', 'HEAD~20..HEAD', CANDIDATES, COVERAGE);
  const reserved = report.timeline.filter((e) => e.reservedOnly);
  for (const entry of reserved) {
    const admitted = Object.values(entry.admits).some(Boolean);
    if (!admitted) continue;
    const changed = await changedPathsOf(entry.sha);
    assert.ok(
      changed.every(isOwnerIntentPath),
      `${entry.sha} admitted while touching non-intent reserved paths: ${changed.join(', ')}`
    );
  }
});

test('every timeline entry carries a verdict for every candidate', async () => {
  const report = await replay(REPO, 'acme/app', 'HEAD~10..HEAD', CANDIDATES, COVERAGE);
  for (const entry of report.timeline) {
    assert.deepEqual(Object.keys(entry.admits).sort(), ['backend only', 'everything']);
    assert.match(entry.sha, /^[0-9a-f]{40}$/);
    assert.match(entry.date, /^\d{4}-\d{2}-\d{2}$/);
  }
});

test('an empty range is an error rather than a zero-commit report', async () => {
  // A silent "0 commits, 0% admission" would read as a measurement result.
  await assert.rejects(() => replay(REPO, 'acme/app', 'HEAD..HEAD', CANDIDATES, COVERAGE), /no commits/);
});

test('formatReport renders every candidate and its drivers', async () => {
  const report = await replay(REPO, 'acme/app', 'HEAD~10..HEAD', CANDIDATES, COVERAGE);
  const text = formatReport(report);
  assert.match(text, /commits replayed/);
  for (const c of report.candidates) assert.ok(text.includes(c.name), c.name);
});
