// Assemble `manifest.json` from repository state plus the artifact plan.
//
// Ordering note: the manifest's `outputFingerprint` covers a canonical
// projection of the manifest itself, which sounds circular and is not — section
// 17.6 excludes `outputFingerprint` from the projection precisely so the value
// can be computed last and written into an otherwise-complete document.

import { execFile } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { basename, extname, join } from 'node:path';
import { promisify } from 'node:util';
import { requiredClasses, type EvolutionConfig } from './config.ts';
import {
  directoryTreeFingerprint, generatorEnvFingerprint, generatorPinnedFingerprint,
  inputFingerprint, outputFingerprint, readManagedOutputs, sourceFingerprints,
  type ReleaseAssetRef,
} from './fingerprints.ts';
import { contentHash } from './hash.ts';
import { readTree } from './gittree.ts';
import { log } from './log.ts';
import {
  manifestProjection, MANIFEST_SCHEMA_VERSION,
  type ArtifactRecord, type Manifest, type VerificationCheck,
} from './manifest.ts';
import { isWritableByEvolution } from './selector.ts';
import type { ArtifactPlan } from './plan.ts';

const execFileAsync = promisify(execFile);

/** Schema versions hashed into `generatorPinnedFingerprint` (section 17.4 order). */
export const SCHEMA_VERSIONS: [number, number, number, number] = [1, 1, 1, MANIFEST_SCHEMA_VERSION];

async function versionOf(cmd: string, args: string[], pick: (out: string) => string): Promise<string> {
  try {
    const { stdout } = await execFileAsync(cmd, args, { encoding: 'utf8' });
    return pick(stdout);
  } catch (error) {
    // Provenance-only: a missing renderer must not stop manifest assembly, but
    // it must be visible rather than silently recorded as some default version.
    log.warn('toolchain version unavailable', { cmd, error: String(error) });
    return 'unavailable';
  }
}

/** Detect the renderer and media tool versions recorded as provenance (section 17.4). */
export async function detectToolchain(repoRoot: string): Promise<Record<string, string>> {
  const [ffmpeg, playwright] = await Promise.all([
    versionOf('ffmpeg', ['-version'], (o) => o.split('\n')[0].replace('ffmpeg version ', '').split(' ')[0]),
    versionOf('npx', ['--no-install', 'playwright', '--version'], (o) => o.trim().replace('Version ', '')),
  ]);
  return {
    node: process.version,
    ffmpeg,
    playwright,
    slideRenderer: `chromium-pdf via playwright ${playwright}`,
    repoRoot: basename(repoRoot),
  };
}

/**
 * Section 24.2 Release tag: `fkst-evolution/<first-16-hex-of-input-fingerprint>`.
 *
 * Derived, never configured. The full fingerprint stays in the manifest, so the
 * shortened tag is not the sole identity.
 */
export function releaseTag(inputFingerprint: string): string {
  const hex = inputFingerprint.replace(/^sha256:/, '').slice(0, 16);
  if (hex.length !== 16) throw new Error(`input fingerprint too short for a tag: ${inputFingerprint}`);
  return `fkst-evolution/${hex}`;
}

/** Section 24.3 asset name: `<base>.sha256-<16 hex>.<ext>`. */
export function releaseAssetName(baseName: string, hash: string, extension: string): string {
  const hex = hash.replace(/^sha256:/, '').slice(0, 16);
  if (hex.length !== 16) throw new Error(`content hash too short for an asset name: ${hash}`);
  // 16 hex characters (64 bits), not 8: at 32 bits a collision becomes likely
  // within tens of thousands of assets, and the collision is unrecoverable by
  // design — the name is taken, the bytes differ, and the only remedy the spec
  // offers is "flag inconsistency".
  return `${baseName}.sha256-${hex}${extension}`;
}

export interface BuildOptions {
  repoRoot: string;
  config: EvolutionConfig;
  plan: ArtifactPlan;
  revision: string;
  repository: string;
  /** ISO-8601 timestamp recorded on every artifact and on verification. */
  timestamp: string;
}

/** Build a complete manifest for the given revision. */
export async function buildManifest(options: BuildOptions): Promise<Manifest> {
  const { repoRoot, config, plan, revision, repository, timestamp } = options;

  const source = await sourceFingerprints(repoRoot, config, revision);
  const head = source.observedHead;

  const packages = await Promise.all(
    plan.generator.packages.map(async (pkg) => ({
      ref: pkg.ref.replace('{commit}', head),
      treeFingerprint: await directoryTreeFingerprint(repoRoot, head, pkg.directory),
    }))
  );
  const pinned = generatorPinnedFingerprint({
    manifestRef: plan.generator.manifestRef.replace('{commit}', head),
    packages,
    schemaVersions: SCHEMA_VERSIONS,
    generatorEpoch: config.generatorEpoch,
  });
  const toolchain = await detectToolchain(repoRoot);
  const envFingerprint = generatorEnvFingerprint({
    engineVersion: plan.generator.engineVersion,
    model: plan.generator.model,
    toolchain,
  });
  const input = inputFingerprint(source.productRelevant, pinned);

  // Committed artifact bytes come from the Git TREE, not the working directory:
  // an uncommitted edit must not be able to produce a manifest that describes
  // bytes no reviewer will ever see in the pull request.
  const managed = await readManagedOutputs(repoRoot, head);
  const managedByPath = new Map(managed.map((e) => [e.path, e]));

  const enabled = new Set(requiredClasses(config));
  const artifacts: ArtifactRecord[] = [];
  const assets: ReleaseAssetRef[] = [];

  for (const planned of plan.artifacts) {
    // Section 17.7.2 rule 1: a disabled class, or one whose producer role the
    // owner declared absent, contributes no REQUIRED entries. The artifact is
    // still recorded — with provenance — it is simply not quantified over.
    const required = planned.required && enabled.has(planned.kind);

    let hash: string;
    let release: ArtifactRecord['release'];
    let repositoryPath: string | undefined;

    if (planned.repositoryPath) {
      if (!isWritableByEvolution(planned.repositoryPath)) {
        // Section 12.1.1 / 25.8: enforced here as well as at merge, because a
        // plan is generator input and generator input must not be able to steer
        // a write outside the root.
        throw new Error(`plan targets a path outside the Evolution write boundary: ${planned.repositoryPath}`);
      }
      const entry = managedByPath.get(planned.repositoryPath);
      if (!entry) {
        throw new Error(`planned artifact ${planned.id} is not committed at ${planned.repositoryPath}`);
      }
      hash = contentHash(entry.content);
      repositoryPath = planned.repositoryPath;
    } else if (planned.release) {
      const bytes = await readFile(join(repoRoot, planned.release.localPath));
      hash = contentHash(bytes);
      const asset = releaseAssetName(
        planned.release.assetBaseName, hash, extname(planned.release.localPath)
      );
      release = { repository: planned.release.repository, tag: releaseTag(input), asset };
      assets.push({ tag: release.tag, asset, contentHash: hash });
    } else {
      throw new Error(`planned artifact ${planned.id} has no destination`);
    }

    artifacts.push({
      id: planned.id,
      kind: planned.kind,
      locale: planned.locale,
      audience: planned.audience,
      capabilities: planned.capabilities,
      journeys: planned.journeys,
      sourceCommit: head,
      inputFingerprint: input,
      generatorPinnedFingerprint: pinned,
      ...(repositoryPath ? { repositoryPath } : {}),
      ...(release ? { release } : {}),
      contentHash: hash,
      status: planned.status as ArtifactRecord['status'],
      required,
      verification: planned.verification,
      updatedAt: timestamp,
    });
  }

  const checks: VerificationCheck[] = plan.verification.map((check) => ({
    id: check.id,
    status: check.status,
    evidence: check.evidence,
    checkRun: {
      repository,
      headSha: head,
      // Null until a control plane publishes the run. Section 17.7.1 makes a
      // null id a corroboration failure rather than a pass, which is the honest
      // outcome while no control plane exists.
      id: null,
      name: check.checkRunName,
    },
    provenance: check.provenance,
  }));

  const manifest: Manifest = {
    schemaVersion: MANIFEST_SCHEMA_VERSION,
    source: {
      repository,
      branch: config.source.branch,
      observedHead: head,
      previousCoveredHead: head,
      historyRelation: 'fast-forward',
      productRelevantFingerprint: source.productRelevant,
      coverageFingerprint: source.coverage,
      inputFingerprint: input,
      lastFullRebuildFor: null,
    },
    artifactRepository: { repository, branch: config.source.branch },
    generator: {
      manifestRef: plan.generator.manifestRef.replace('{commit}', head),
      packages: packages.map((p) => p.ref),
      generatorEpoch: config.generatorEpoch,
      pinnedFingerprint: pinned,
      provenanceOnly: ['engineVersion', 'model', 'toolchain', 'envFingerprint'],
      engineVersion: plan.generator.engineVersion,
      model: plan.generator.model,
      toolchain,
      envFingerprint,
    },
    outputFingerprint: '',
    verification: {
      status: plan.verification.every((c) => c.status === 'passed') ? 'passed' : 'failed',
      verifiedAt: timestamp,
      checks,
    },
    artifacts,
  };

  manifest.outputFingerprint = outputFingerprint(managed, assets, manifestProjection(manifest));
  log.info('manifest assembled', {
    artifacts: artifacts.length,
    required: artifacts.filter((a) => a.required).length,
    input,
    output: manifest.outputFingerprint,
  });
  return manifest;
}

/** Freshly hash every managed file at a revision — condition 3's integrity input. */
export async function presentManagedFiles(
  repoRoot: string,
  revision: string
): Promise<Map<string, string>> {
  const entries = await readTree(repoRoot, revision, (p) => p.startsWith('.fkst/evolution/'));
  return new Map(entries.map((e) => [e.path, contentHash(e.content)]));
}
