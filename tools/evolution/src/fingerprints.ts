// The six fingerprints of spec section 17.1.
//
//   productRelevant  §17.3  cycle admission, merge staleness
//   coverage         §17.3  provenance only
//   generatorPinned  §17.4  cycle admission
//   generatorEnv     §17.4  provenance only
//   input            §17.5  convergence condition 1
//   output           §17.6  convergence condition 2
//
// WHY the split matters here and not only in the spec: an earlier draft used one
// input fingerprint over `include: ["**"]`, which made "any byte changed" the
// admission rule while section 21.4 simultaneously required "source has not
// moved" to merge. On an active repository those never both hold. Keeping the
// two families genuinely separate in code is what stops that from creeping back.

import { readTree, resolveRevision } from './gittree.ts';
import {
  contentHash, domainConcat, formatDigest, namedLeaf, treeFingerprint, type FileEntry,
} from './hash.ts';
import { log } from './log.ts';
import { coverageMatcher, managedOutputMatcher, productRelevantMatcher } from './selector.ts';
import type { EvolutionConfig } from './config.ts';

export const DOMAIN_GENPIN = 'fkst-evolution-genpin-v1';
export const DOMAIN_INPUT = 'fkst-evolution-input-v2';
export const DOMAIN_OUTPUT = 'fkst-evolution-output-v1';
export const DOMAIN_GENENV = 'fkst-evolution-genenv-v1';

/** A package reference resolved to an immutable commit (section 28.4). */
export interface ResolvedPackageRef {
  /** `owner/repo@<resolved-commit-sha>:path` — never a symbolic branch or tag. */
  ref: string;
  /** Tree fingerprint of that package directory at the resolved commit. */
  treeFingerprint: string;
}

export interface GeneratorPinnedInputs {
  manifestRef: string;
  packages: ResolvedPackageRef[];
  /** capabilities, journeys, changes, manifest — in that order (section 17.4). */
  schemaVersions: [number, number, number, number];
  generatorEpoch: number;
}

/** Deployment facts the repository does not control — provenance only (section 17.4). */
export interface GeneratorEnvInputs {
  engineVersion: string;
  model: string;
  toolchain: Record<string, string>;
}

function digestOf(prefixed: string): Buffer {
  const [scheme, hex] = prefixed.split(':');
  if (scheme !== 'sha256' || !/^[0-9a-f]{64}$/.test(hex ?? '')) {
    throw new Error(`expected a sha256:<hex> digest, got ${JSON.stringify(prefixed)}`);
  }
  return Buffer.from(hex, 'hex');
}

/**
 * `generatorPinnedFingerprint` (section 17.4).
 *
 * Leaf ORDER is fixed by the spec and reproduced literally: `manifestRef`, then
 * every `package[i]` sorted by full ref string, then every `packageTree[i]` in
 * that same order, then `schemaVersions`, then `generatorEpoch`. The two package
 * loops are separate rather than interleaved because that is how section 17.4
 * lays them out.
 */
export function generatorPinnedFingerprint(inputs: GeneratorPinnedInputs): string {
  const sorted = [...inputs.packages].sort((a, b) => (a.ref < b.ref ? -1 : a.ref > b.ref ? 1 : 0));
  const leaves = [namedLeaf('manifestRef', inputs.manifestRef)];
  sorted.forEach((pkg, i) => leaves.push(namedLeaf(`package[${i}]`, pkg.ref)));
  sorted.forEach((pkg, i) => leaves.push(namedLeaf(`packageTree[${i}]`, pkg.treeFingerprint)));
  leaves.push(namedLeaf('schemaVersions', inputs.schemaVersions.map(String).join(',')));
  // Decimal, no padding — section 17.4 pins the serialization because "1" and
  // "01" would otherwise be different generators.
  leaves.push(namedLeaf('generatorEpoch', String(inputs.generatorEpoch)));
  return formatDigest(domainConcat(DOMAIN_GENPIN, leaves));
}

/** `generatorEnvFingerprint` (section 17.4) — recorded, never a convergence input. */
export function generatorEnvFingerprint(inputs: GeneratorEnvInputs): string {
  const leaves = [
    namedLeaf('engineVersion', inputs.engineVersion),
    namedLeaf('model', inputs.model),
  ];
  for (const key of Object.keys(inputs.toolchain).sort()) {
    leaves.push(namedLeaf(`toolchain.${key}`, inputs.toolchain[key]));
  }
  return formatDigest(domainConcat(DOMAIN_GENENV, leaves));
}

/** `inputFingerprint` (section 17.5). */
export function inputFingerprint(productRelevant: string, generatorPinned: string): string {
  return formatDigest(
    domainConcat(DOMAIN_INPUT, [digestOf(productRelevant), digestOf(generatorPinned)])
  );
}

/** Content hashes of Release assets the manifest references (section 17.6). */
export interface ReleaseAssetRef {
  tag: string;
  asset: string;
  contentHash: string;
}

/**
 * `outputFingerprint` (section 17.6): the managed file tree, plus referenced
 * Release asset hashes, plus a canonical projection of the manifest.
 *
 * Section 17.6 states the three components but not their serialization, and
 * section 17.5 requires that the exact serialization be documented and covered
 * by test vectors. This is it — a domain-tagged concatenation of three named
 * leaves, with assets sorted by `<tag>/<asset>` so upload order cannot change
 * the fingerprint.
 *
 * The manifest projection is REQUIRED, not optional. Under the previous MAY,
 * `verification` and `artifacts` sat outside the hash while conditions 3 and 4
 * read presence and verification status out of them — so editing those strings
 * changed the answer to "is this converged?" without changing any fingerprint.
 */
export function outputFingerprint(
  managedFiles: FileEntry[],
  assets: ReleaseAssetRef[],
  manifestProjectionJcs: string
): string {
  const assetLine = [...assets]
    .map((a) => `${a.tag}/${a.asset}=${a.contentHash}`)
    .sort()
    .join('\n');
  const leaves = [
    namedLeaf('outputTree', formatDigest(treeFingerprint(managedFiles))),
    namedLeaf('releaseAssets', assetLine),
    namedLeaf('manifestProjection', manifestProjectionJcs),
  ];
  return formatDigest(domainConcat(DOMAIN_OUTPUT, leaves));
}

export interface SourceFingerprints {
  observedHead: string;
  productRelevant: string;
  coverage: string;
  productRelevantPaths: string[];
  coveragePaths: string[];
}

/** Compute both source-tree fingerprints at one revision (section 17.3). */
export async function sourceFingerprints(
  repoRoot: string,
  config: EvolutionConfig,
  revision: string
): Promise<SourceFingerprints> {
  const observedHead = await resolveRevision(repoRoot, revision);
  const inProduct = productRelevantMatcher(config.source.productRelevant);
  const inCoverage = coverageMatcher(config.source.coverage);

  // One tree read serving both selectors: two reads would double the cost of
  // every reconcile for no benefit.
  const entries = await readTree(repoRoot, observedHead, (p) => inProduct(p) || inCoverage(p));
  const productEntries = entries.filter((e) => inProduct(e.path));
  const coverageEntries = entries.filter((e) => inCoverage(e.path));

  log.info('source fingerprints computed', {
    head: observedHead.slice(0, 12),
    productRelevantFiles: productEntries.length,
    coverageFiles: coverageEntries.length,
  });

  return {
    observedHead,
    productRelevant: formatDigest(treeFingerprint(productEntries)),
    coverage: formatDigest(treeFingerprint(coverageEntries)),
    productRelevantPaths: productEntries.map((e) => e.path).sort(),
    coveragePaths: coverageEntries.map((e) => e.path).sort(),
  };
}

/** Read the managed-output file set that section 17.6 hashes. */
export async function readManagedOutputs(
  repoRoot: string,
  revision: string
): Promise<FileEntry[]> {
  const keep = managedOutputMatcher();
  return readTree(repoRoot, revision, keep);
}

/** Tree fingerprint of one directory at a revision — used for `packageTree[i]`. */
export async function directoryTreeFingerprint(
  repoRoot: string,
  revision: string,
  directory: string
): Promise<string> {
  const prefix = directory.endsWith('/') ? directory : `${directory}/`;
  const entries = await readTree(repoRoot, revision, (p) => p.startsWith(prefix));
  if (entries.length === 0) {
    throw new Error(`no files under ${prefix} at ${revision} — cannot pin an empty generator`);
  }
  return formatDigest(treeFingerprint(entries));
}

export { contentHash };
