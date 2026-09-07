// The convergence decision (spec section 17.7).
//
// The decision logic is exercised through the `GitHubPort` interface with an
// in-memory double, so every branch — including the ones that need GitHub — is
// testable without a network.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { evaluateConvergence, type ConvergenceInputs, type GitHubPort } from '../src/converge.ts';
import { parseConfig } from '../src/config.ts';
import { contentHash } from '../src/hash.ts';
import type { Manifest } from '../src/manifest.ts';

const CONFIG = parseConfig(`
schemaVersion: 1
enabled: true
source:
  branch: main
  productRelevant:
    include: ["src/**"]
  coverage:
    include: ["**"]
artifactRepository: "."
intent:
  product: ".fkst/evolution/intent/product.md"
  overrides: ".fkst/evolution/intent/overrides.yaml"
managedOutputs:
  documentation: { enabled: true }
  skills: { enabled: false }
  journeys: { enabled: false }
  screenshots: { enabled: false }
  slides: { enabled: false }
  video: { enabled: false }
absentProducerRoles: []
locales: ["en"]
triggers: {}
publication:
  mode: "propose"
  requireCurrentSource: true
  requireChecks: true
  allowDirectPush: false
drift:
  policy: "block"
generatorEpoch: 1
retention: {}
security:
  runPullRequestCode: false
  allowProductionData: false
  allowProductionCredentials: false
`);

const DOC_PATH = '.fkst/evolution/docs/page.md';
const DOC_HASH = contentHash(Buffer.from('doc bytes', 'utf8'));
const INPUT = `sha256:${'1'.repeat(64)}`;
const OUTPUT = `sha256:${'2'.repeat(64)}`;
const HEAD = 'a'.repeat(40);

function manifest(overrides: Partial<Manifest> = {}): Manifest {
  return {
    schemaVersion: 1,
    source: {
      repository: 'acme/app', branch: 'main', observedHead: HEAD, previousCoveredHead: HEAD,
      historyRelation: 'fast-forward',
      productRelevantFingerprint: `sha256:${'3'.repeat(64)}`,
      coverageFingerprint: `sha256:${'4'.repeat(64)}`,
      inputFingerprint: INPUT,
    },
    artifactRepository: { repository: 'acme/app', branch: 'main' },
    generator: {
      manifestRef: 'acme/app@abc:tools/evolution', packages: [], generatorEpoch: 1,
      pinnedFingerprint: `sha256:${'5'.repeat(64)}`,
      provenanceOnly: [], engineVersion: '0.1.0', model: 'test', toolchain: {},
      envFingerprint: `sha256:${'6'.repeat(64)}`,
    },
    outputFingerprint: OUTPUT,
    verification: {
      status: 'passed', verifiedAt: '2026-01-01T00:00:00Z',
      checks: [{
        id: 'jny_test', status: 'passed', evidence: 'j.spec.ts',
        checkRun: { repository: 'acme/app', headSha: HEAD, id: 42, name: 'fkst-evolution/journey.jny_test' },
        provenance: 'transcription',
      }],
    },
    artifacts: [{
      id: 'docs.page', kind: 'documentation', locale: 'en', audience: 'admin',
      capabilities: ['cap_1'], journeys: ['jny_test'], sourceCommit: HEAD,
      inputFingerprint: INPUT, generatorPinnedFingerprint: `sha256:${'5'.repeat(64)}`,
      repositoryPath: DOC_PATH, contentHash: DOC_HASH, status: 'current',
      required: true, verification: 'passed', updatedAt: '2026-01-01T00:00:00Z',
    }],
    ...overrides,
  };
}

function port(overrides: Partial<GitHubPort> = {}): GitHubPort {
  return {
    fetchReleaseAsset: async () => null,
    getCheckRun: async () => ({ appId: 7, conclusion: 'success', outputText: `input=${INPUT}` }),
    openSyncPullRequestInputs: async () => [],
    configuredAppId: async () => 7,
    ...overrides,
  };
}

function inputs(overrides: Partial<ConvergenceInputs> = {}): ConvergenceInputs {
  return {
    repoRoot: '/nonexistent',
    config: CONFIG,
    manifest: manifest(),
    currentInputFingerprint: INPUT,
    currentOutputFingerprint: OUTPUT,
    observedHead: HEAD,
    presentFiles: new Map([[DOC_PATH, DOC_HASH]]),
    github: port(),
    ...overrides,
  };
}

const find = (r: Awaited<ReturnType<typeof evaluateConvergence>>, name: string) =>
  r.conditions.find((c) => c.name === name)!;

test('a fully satisfied repository converges', async () => {
  const report = await evaluateConvergence(inputs());
  assert.equal(report.verdict, 'CONVERGED');
});

test('condition 1 fails when the input fingerprint moved', async () => {
  const report = await evaluateConvergence(
    inputs({ currentInputFingerprint: `sha256:${'9'.repeat(64)}` })
  );
  assert.equal(report.verdict, 'NOT_CONVERGED');
  assert.equal(find(report, 'input fingerprint matches manifest').outcome, 'fail');
});

test('condition 2 fails when the output fingerprint moved', async () => {
  const report = await evaluateConvergence(
    inputs({ currentOutputFingerprint: `sha256:${'9'.repeat(64)}` })
  );
  assert.equal(find(report, 'output fingerprint matches manifest').outcome, 'fail');
});

test('condition 3 presence fails when a required artifact is absent', async () => {
  const report = await evaluateConvergence(inputs({ presentFiles: new Map() }));
  const presence = find(report, 'required artifacts present');
  assert.equal(presence.outcome, 'fail');
  // Presence failure must NOT be reported as integrity drift: it admits an
  // ordinary cycle, while drift blocks regardless of policy.
  assert.equal(find(report, 'required artifact integrity').outcome, 'pass');
});

test('condition 3 integrity fails when bytes disagree with the recorded hash', async () => {
  const report = await evaluateConvergence(
    inputs({ presentFiles: new Map([[DOC_PATH, `sha256:${'0'.repeat(64)}`]]) })
  );
  assert.equal(find(report, 'required artifacts present').outcome, 'pass');
  const integrity = find(report, 'required artifact integrity');
  assert.equal(integrity.outcome, 'fail');
  assert.match(integrity.detail, /blocks regardless of policy/);
});

test('a tombstoned artifact is excluded from the presence test', async () => {
  const m = manifest();
  m.artifacts[0].status = 'removed';
  const report = await evaluateConvergence(inputs({ manifest: m, presentFiles: new Map() }));
  assert.equal(find(report, 'required artifacts present').outcome, 'pass');
});

test('a required-but-failed artifact keeps the repository non-converged', async () => {
  // Section 17.7.2 rule 3: it MUST NOT be silently downgraded to not-required.
  const m = manifest();
  m.artifacts[0].status = 'failed';
  const report = await evaluateConvergence(inputs({ manifest: m, presentFiles: new Map() }));
  assert.equal(report.verdict, 'NOT_CONVERGED');
});

test('a required: false artifact is ignored by conditions 3 and 4', async () => {
  const m = manifest();
  m.artifacts[0].required = false;
  const report = await evaluateConvergence(inputs({ manifest: m, presentFiles: new Map() }));
  assert.equal(find(report, 'required artifacts present').outcome, 'pass');
});

test('condition 4 is not-evaluable without GitHub, never a pass', async () => {
  const report = await evaluateConvergence(inputs({ github: undefined }));
  assert.equal(find(report, 'verification corroborated').outcome, 'not-evaluable');
  assert.equal(report.verdict, 'CONVERGED_PENDING_CONTROL_PLANE');
});

test('condition 4 rejects a check run published by another actor', async () => {
  const report = await evaluateConvergence(
    inputs({ github: port({ getCheckRun: async () => ({ appId: 999, conclusion: 'success', outputText: INPUT }) }) })
  );
  assert.match(find(report, 'verification corroborated').detail, /not the configured App/);
});

test('condition 4 rejects a check run whose conclusion is not success', async () => {
  const report = await evaluateConvergence(
    inputs({ github: port({ getCheckRun: async () => ({ appId: 7, conclusion: 'failure', outputText: INPUT }) }) })
  );
  assert.match(find(report, 'verification corroborated').detail, /conclusion is failure/);
});

test('condition 4 rejects a replayed run recording a different input fingerprint', async () => {
  const report = await evaluateConvergence(
    inputs({ github: port({ getCheckRun: async () => ({ appId: 7, conclusion: 'success', outputText: 'input=sha256:stale' }) }) })
  );
  assert.match(find(report, 'verification corroborated').detail, /does not record the manifest input fingerprint/);
});

test('condition 4 rejects a deleted check run', async () => {
  const report = await evaluateConvergence(inputs({ github: port({ getCheckRun: async () => null }) }));
  assert.match(find(report, 'verification corroborated').detail, /no longer retrievable/);
});

test('condition 4 fails outright when the manifest records no checks', async () => {
  const m = manifest();
  m.verification.checks = [];
  const report = await evaluateConvergence(inputs({ manifest: m }));
  assert.equal(find(report, 'verification corroborated').outcome, 'fail');
  assert.equal(report.verdict, 'NOT_CONVERGED');
});

test('a null check-run id is not corroboration', async () => {
  // The state this proof actually ships in: no control plane has published a
  // run, so the honest verdict is pending, not converged.
  const m = manifest();
  m.verification.checks[0].checkRun.id = null;
  const report = await evaluateConvergence(inputs({ manifest: m }));
  assert.equal(find(report, 'verification corroborated').outcome, 'not-evaluable');
  assert.equal(report.verdict, 'CONVERGED_PENDING_CONTROL_PLANE');
});

test('condition 5 passes at baseline when no previous covered head exists', async () => {
  const m = manifest();
  m.source.previousCoveredHead = null;
  const report = await evaluateConvergence(inputs({ manifest: m }));
  assert.equal(find(report, 'no uncovered product-relevant change').outcome, 'pass');
});

test('condition 5 fails loudly when the range cannot be compared', async () => {
  // Section 17.7: a truncated or unusable compare is a FAILURE, not a guess.
  const m = manifest();
  m.source.previousCoveredHead = 'b'.repeat(40);
  const report = await evaluateConvergence(inputs({ manifest: m, observedHead: HEAD }));
  const c5 = find(report, 'no uncovered product-relevant change');
  assert.equal(c5.outcome, 'fail');
  assert.match(c5.detail, /cannot compare/);
});

test('condition 6 fails when an open sync PR describes a different input', async () => {
  const report = await evaluateConvergence(
    inputs({ github: port({ openSyncPullRequestInputs: async () => [`sha256:${'8'.repeat(64)}`] }) })
  );
  assert.equal(find(report, 'no divergent open sync PR').outcome, 'fail');
});

test('condition 6 tolerates an open sync PR at the SAME input', async () => {
  const report = await evaluateConvergence(
    inputs({ github: port({ openSyncPullRequestInputs: async () => [INPUT] }) })
  );
  assert.equal(find(report, 'no divergent open sync PR').outcome, 'pass');
});

test('a Release-backed artifact is re-derived from tag and name, not assetUrl', async () => {
  const bytes = Buffer.from('video bytes', 'utf8');
  const m = manifest();
  m.artifacts[0] = {
    ...m.artifacts[0], id: 'video.demo', kind: 'video', repositoryPath: undefined,
    release: {
      repository: 'acme/app', tag: 'fkst-evolution/0123456789abcdef', asset: 'demo.sha256-abc.mp4',
      assetUrl: 'https://example.invalid/stale',
    },
    contentHash: contentHash(bytes),
  };
  const report = await evaluateConvergence(inputs({
    manifest: m, presentFiles: new Map(),
    github: port({ fetchReleaseAsset: async () => bytes }),
  }));
  assert.equal(find(report, 'required artifacts present').outcome, 'pass');
  assert.equal(find(report, 'required artifact integrity').outcome, 'pass');
});

test('a Release asset whose bytes changed is an integrity failure', async () => {
  const m = manifest();
  m.artifacts[0] = {
    ...m.artifacts[0], id: 'video.demo', kind: 'video', repositoryPath: undefined,
    release: { repository: 'acme/app', tag: 't', asset: 'demo.sha256-abc.mp4' },
    contentHash: contentHash(Buffer.from('original', 'utf8')),
  };
  const report = await evaluateConvergence(inputs({
    manifest: m, presentFiles: new Map(),
    github: port({ fetchReleaseAsset: async () => Buffer.from('rewritten', 'utf8') }),
  }));
  assert.equal(find(report, 'required artifact integrity').outcome, 'fail');
});

test('a Release-backed artifact is not-evaluable without GitHub', async () => {
  const m = manifest();
  m.artifacts[0] = {
    ...m.artifacts[0], id: 'video.demo', kind: 'video', repositoryPath: undefined,
    release: { repository: 'acme/app', tag: 't', asset: 'demo.mp4' },
  };
  const report = await evaluateConvergence(
    inputs({ manifest: m, presentFiles: new Map(), github: undefined })
  );
  assert.equal(find(report, 'required artifacts present').outcome, 'not-evaluable');
  assert.notEqual(report.verdict, 'CONVERGED');
});
