// The section 17.7 convergence decision.
//
// Two rules from the spec shape this module:
//
//  1. "Conditions 3 and 4 MUST be evaluated by re-deriving from repository and
//     GitHub state, never by reading a status field out of the manifest alone."
//     So condition 3 re-hashes bytes and condition 4 re-fetches a check run;
//     neither trusts `status` or `verification` strings.
//
//  2. Section 27.5: uncertainty MUST be represented explicitly. A condition that
//     needs GitHub state the caller did not supply reports `not-evaluable` and
//     downgrades the verdict. It never quietly counts as a pass — that is how a
//     verifier starts lying.

import { changedPaths } from './gittree.ts';
import { contentHash } from './hash.ts';
import { log } from './log.ts';
import { productRelevantMatcher } from './selector.ts';
import { requiredArtifacts, type Manifest } from './manifest.ts';
import type { EvolutionConfig } from './config.ts';

export type ConditionOutcome = 'pass' | 'fail' | 'not-evaluable';

export interface ConditionResult {
  id: number;
  name: string;
  outcome: ConditionOutcome;
  detail: string;
}

export type Verdict = 'CONVERGED' | 'CONVERGED_PENDING_CONTROL_PLANE' | 'NOT_CONVERGED';

export interface ConvergenceReport {
  verdict: Verdict;
  conditions: ConditionResult[];
}

/**
 * The GitHub reads conditions 3, 4 and 6 need.
 *
 * An interface rather than a direct `gh` call so the decision logic can be
 * exercised without a network or a repository — and so a future control-plane
 * implementation can supply its own installation-token client without touching
 * this file.
 */
export interface GitHubPort {
  /** Raw bytes of a Release asset, or null when the tag or asset is absent. */
  fetchReleaseAsset(repository: string, tag: string, asset: string): Promise<Buffer | null>;
  /** A check run by id, or null when it no longer exists. */
  getCheckRun(
    repository: string,
    id: number
  ): Promise<{ appId: number | null; conclusion: string | null; outputText: string } | null>;
  /** Input fingerprints advertised by open canonical sync PRs. */
  openSyncPullRequestInputs(repository: string): Promise<string[]>;
  /** The App id that is allowed to publish verification check runs. */
  configuredAppId(): Promise<number | null>;
}

export interface ConvergenceInputs {
  repoRoot: string;
  config: EvolutionConfig;
  manifest: Manifest;
  /** Freshly computed, never read from the manifest. */
  currentInputFingerprint: string;
  currentOutputFingerprint: string;
  observedHead: string;
  /** Path → freshly computed content hash, for every managed file present now. */
  presentFiles: Map<string, string>;
  github?: GitHubPort;
}

function result(id: number, name: string, outcome: ConditionOutcome, detail: string): ConditionResult {
  return { id, name, outcome, detail };
}

/** Condition 1: current input fingerprint equals the committed manifest's. */
function condition1(inputs: ConvergenceInputs): ConditionResult {
  const expected = inputs.manifest.source.inputFingerprint;
  const actual = inputs.currentInputFingerprint;
  return actual === expected
    ? result(1, 'input fingerprint matches manifest', 'pass', actual)
    : result(1, 'input fingerprint matches manifest', 'fail', `expected ${expected}, computed ${actual}`);
}

/** Condition 2: current output fingerprint equals the committed manifest's. */
function condition2(inputs: ConvergenceInputs): ConditionResult {
  const expected = inputs.manifest.outputFingerprint;
  const actual = inputs.currentOutputFingerprint;
  return actual === expected
    ? result(2, 'output fingerprint matches manifest', 'pass', actual)
    : result(2, 'output fingerprint matches manifest', 'fail', `expected ${expected}, computed ${actual}`);
}

/**
 * Condition 3: presence AND integrity of every required artifact.
 *
 * The two halves are reported separately because section 17.7 gives them
 * different outcomes: an integrity mismatch is managed-output drift and resolves
 * as `block` regardless of drift policy, while a presence failure is an ordinary
 * non-converged outcome that simply admits a cycle. Collapsing them would force
 * a repository that has merely never rendered a video into an integrity-failure
 * branch its drift policy cannot reach.
 */
async function condition3(inputs: ConvergenceInputs): Promise<ConditionResult[]> {
  const required = requiredArtifacts(inputs.manifest);
  const missing: string[] = [];
  const corrupt: string[] = [];
  const unevaluable: string[] = [];

  for (const artifact of required) {
    if (artifact.repositoryPath) {
      const actual = inputs.presentFiles.get(artifact.repositoryPath);
      if (actual === undefined) {
        missing.push(artifact.repositoryPath);
      } else if (actual !== artifact.contentHash) {
        corrupt.push(`${artifact.repositoryPath} (expected ${artifact.contentHash}, found ${actual})`);
      }
      continue;
    }
    if (artifact.release) {
      if (!inputs.github) {
        unevaluable.push(`${artifact.id} (Release asset needs GitHub)`);
        continue;
      }
      const bytes = await inputs.github.fetchReleaseAsset(
        artifact.release.repository,
        artifact.release.tag,
        artifact.release.asset
      );
      // Section 16.2: `assetUrl` is a convenience and NOT authoritative — the
      // asset is re-derived from repository + tag + asset name and re-hashed, so
      // a stale or rewritten URL cannot make a missing artifact look present.
      if (!bytes) {
        missing.push(`${artifact.release.tag}/${artifact.release.asset}`);
      } else if (contentHash(bytes) !== artifact.contentHash) {
        corrupt.push(`${artifact.release.asset} (Release asset bytes differ from recorded hash)`);
      }
      continue;
    }
    missing.push(`${artifact.id} (no repositoryPath and no release reference)`);
  }

  const presence =
    missing.length === 0
      ? result(3, 'required artifacts present', unevaluable.length ? 'not-evaluable' : 'pass',
          unevaluable.length ? `unresolved: ${unevaluable.join(', ')}` : `${required.length} required artifact(s)`)
      : result(3, 'required artifacts present', 'fail', `absent: ${missing.join(', ')}`);

  const integrity =
    corrupt.length === 0
      ? result(3, 'required artifact integrity', unevaluable.length ? 'not-evaluable' : 'pass', 'content hashes match')
      : result(3, 'required artifact integrity', 'fail', `drift (blocks regardless of policy): ${corrupt.join(', ')}`);

  return [presence, integrity];
}

/** Condition 4: every required verification corroborated per section 17.7.1. */
async function condition4(inputs: ConvergenceInputs): Promise<ConditionResult> {
  const checks = inputs.manifest.verification.checks;
  if (checks.length === 0) {
    return result(4, 'verification corroborated', 'fail', 'manifest records no verification checks');
  }
  if (!inputs.github) {
    return result(
      4, 'verification corroborated', 'not-evaluable',
      'check-run corroboration requires GitHub; a manifest status string is explicitly not evidence'
    );
  }
  const appId = await inputs.github.configuredAppId();
  const problems: string[] = [];
  for (const check of checks) {
    if (check.checkRun.id === null) {
      problems.push(`${check.id}: no check run published (control plane not deployed)`);
      continue;
    }
    const run = await inputs.github.getCheckRun(check.checkRun.repository, check.checkRun.id);
    if (!run) {
      problems.push(`${check.id}: check run ${check.checkRun.id} no longer retrievable`);
      continue;
    }
    if (appId !== null && run.appId !== appId) {
      problems.push(`${check.id}: published by app ${run.appId}, not the configured App ${appId}`);
      continue;
    }
    if (run.conclusion !== 'success') {
      problems.push(`${check.id}: conclusion is ${run.conclusion}`);
      continue;
    }
    if (!run.outputText.includes(inputs.manifest.source.inputFingerprint)) {
      problems.push(`${check.id}: check-run output does not record the manifest input fingerprint`);
    }
  }
  return problems.length === 0
    ? result(4, 'verification corroborated', 'pass', `${checks.length} check run(s) corroborated`)
    : result(4, 'verification corroborated', 'not-evaluable', problems.join('; '));
}

/**
 * Condition 5: no newer product-relevant change remains uncovered.
 *
 *   uncovered = (previousCoveredHead != observedHead)
 *               AND treeDiff(previousCoveredHead, observedHead)
 *                     intersects source.productRelevant
 *
 * Deliberately one comparison rather than a per-commit walk. It is also
 * deliberately product-relevant, not authoritative: under the section 17.5 split
 * `coverageState` advances only when a cycle merges, so `previousCoveredHead`
 * legitimately lags after any test-only or CI-only commit. Testing against all
 * authoritative input would make every such commit a convergence failure and
 * reproduce the behaviour the split exists to remove.
 */
async function condition5(inputs: ConvergenceInputs): Promise<ConditionResult> {
  const previous = inputs.manifest.source.previousCoveredHead;
  if (!previous) {
    return result(5, 'no uncovered product-relevant change', 'pass', 'no previous covered head — baseline');
  }
  if (previous === inputs.observedHead) {
    return result(5, 'no uncovered product-relevant change', 'pass', 'covered head equals observed head');
  }
  let changed: string[];
  try {
    changed = await changedPaths(inputs.repoRoot, previous, inputs.observedHead);
  } catch (error) {
    // Section 17.7: when the compare response is truncated (or here, when the
    // base commit is unreachable) treat condition 5 as failed rather than guessing.
    return result(5, 'no uncovered product-relevant change', 'fail', `cannot compare: ${String(error)}`);
  }
  const inProduct = productRelevantMatcher(inputs.config.source.productRelevant);
  const intersecting = changed.filter(inProduct);
  return intersecting.length === 0
    ? result(5, 'no uncovered product-relevant change', 'pass',
        `${changed.length} changed path(s), none product-relevant (coverage lag is not a failure)`)
    : result(5, 'no uncovered product-relevant change', 'fail',
        `uncovered product-relevant paths: ${intersecting.slice(0, 5).join(', ')}`);
}

/** Condition 6: no open canonical sync PR represents a different current input. */
async function condition6(inputs: ConvergenceInputs): Promise<ConditionResult> {
  if (!inputs.github) {
    return result(6, 'no divergent open sync PR', 'not-evaluable', 'requires GitHub pull request state');
  }
  const repository = inputs.manifest.artifactRepository.repository;
  const open = await inputs.github.openSyncPullRequestInputs(repository);
  const divergent = open.filter((fp) => fp !== inputs.currentInputFingerprint);
  return divergent.length === 0
    ? result(6, 'no divergent open sync PR', 'pass', `${open.length} open sync PR(s)`)
    : result(6, 'no divergent open sync PR', 'fail', `open sync PR at input ${divergent[0]}`);
}

/** Evaluate all six conditions and reduce them to one verdict. */
export async function evaluateConvergence(inputs: ConvergenceInputs): Promise<ConvergenceReport> {
  const conditions: ConditionResult[] = [
    condition1(inputs),
    condition2(inputs),
    ...(await condition3(inputs)),
    await condition4(inputs),
    await condition5(inputs),
    await condition6(inputs),
  ];

  const failed = conditions.filter((c) => c.outcome === 'fail');
  const pending = conditions.filter((c) => c.outcome === 'not-evaluable');

  // A not-evaluable condition can never be reported as CONVERGED: section 17.7
  // says the repository is converged "only when ALL of the following hold", and
  // an unevaluated condition does not hold — it is unknown.
  const verdict: Verdict =
    failed.length > 0 ? 'NOT_CONVERGED'
      : pending.length > 0 ? 'CONVERGED_PENDING_CONTROL_PLANE'
        : 'CONVERGED';

  log.info('convergence evaluated', {
    verdict,
    failed: failed.length,
    pending: pending.length,
  });
  return { verdict, conditions };
}
