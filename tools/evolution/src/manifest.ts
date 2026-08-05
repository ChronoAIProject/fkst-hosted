// The Evolution manifest: schema, canonical projection, and serialization
// (spec sections 16.2, 16.4, 17.6).

import canonicalize from 'canonicalize';
import { parseStrictJson } from './jsonstrict.ts';
import type { ManagedOutputClass } from './config.ts';

export const MANIFEST_SCHEMA_VERSION = 1;

/** Artifact freshness (section 16.3). Disjoint from the capability lifecycle of §14.2. */
export type ArtifactStatus =
  | 'current' | 'current-unverified' | 'stale' | 'blocked' | 'failed' | 'deprecated' | 'removed';

export interface ReleaseRef {
  repository: string;
  tag: string;
  asset: string;
  assetUrl?: string;
}

export interface ArtifactRecord {
  id: string;
  kind: ManagedOutputClass;
  locale: string;
  audience: string;
  capabilities: string[];
  journeys: string[];
  sourceCommit: string;
  inputFingerprint: string;
  generatorPinnedFingerprint: string;
  repositoryPath?: string;
  release?: ReleaseRef;
  contentHash: string;
  status: ArtifactStatus;
  /** Section 17.7.2: conditions 3 and 4 quantify over exactly the `true` entries. */
  required: boolean;
  verification: string;
  updatedAt: string;
}

export interface CheckRunRef {
  repository: string;
  headSha: string;
  id: number | null;
  name: string;
}

export interface VerificationCheck {
  id: string;
  status: string;
  evidence: string;
  checkRun: CheckRunRef;
  /** Section 17.7.1: transcription vs independent observation must be legible. */
  provenance: string;
}

export interface Manifest {
  schemaVersion: number;
  source: {
    repository: string;
    branch: string;
    observedHead: string;
    previousCoveredHead: string | null;
    historyRelation: string;
    productRelevantFingerprint: string;
    coverageFingerprint: string;
    inputFingerprint: string;
    lastFullRebuildFor?: string | null;
  };
  artifactRepository: { repository: string; branch: string };
  generator: {
    manifestRef: string;
    packages: string[];
    generatorEpoch: number;
    pinnedFingerprint: string;
    provenanceOnly: string[];
    engineVersion: string;
    model: string;
    toolchain: Record<string, string>;
    envFingerprint: string;
  };
  outputFingerprint: string;
  verification: { status: string; verifiedAt: string; checks: VerificationCheck[] };
  artifacts: ArtifactRecord[];
}

/**
 * The canonical projection that enters the output fingerprint (section 17.6):
 * every field EXCEPT `outputFingerprint`, serialized as JCS (RFC 8785).
 *
 * RFC 8785 is used rather than "sorted keys, no whitespace" because these bytes
 * are hashed. Number formatting, Unicode escaping and normalization, and
 * duplicate-key handling are all unspecified by that looser phrasing, so two
 * conforming implementations would disagree on the fingerprint of one manifest.
 */
export function manifestProjection(manifest: Manifest): string {
  const { outputFingerprint: _omitted, ...rest } = manifest;
  const jcs = canonicalize(rest);
  if (typeof jcs !== 'string') {
    throw new Error('manifest could not be canonicalized (RFC 8785)');
  }
  return jcs;
}

/**
 * Serialize a manifest for committing.
 *
 * Pretty-printed with a trailing newline because a human reviews this file in a
 * pull request diff; the hash never reads these bytes — it reads the JCS
 * projection — so formatting here is free.
 */
export function serializeManifest(manifest: Manifest): string {
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

/** Parse a committed manifest, rejecting duplicate keys per section 17.6. */
export function parseManifest(text: string): Manifest {
  const manifest = parseStrictJson<Manifest>(text);
  if (manifest.schemaVersion !== MANIFEST_SCHEMA_VERSION) {
    // Section 33.3 / 16.5: an unsupported manifest is never silently replaced.
    throw new Error(
      `unsupported manifest schemaVersion ${String(manifest.schemaVersion)} ` +
        `(supported: ${MANIFEST_SCHEMA_VERSION})`
    );
  }
  return manifest;
}

/**
 * Artifacts that conditions 3 and 4 quantify over (section 17.7.2).
 *
 * `removed` entries are tombstones and are excluded by construction — their
 * bytes are permitted to be absent. Entries that are `blocked` or `failed` stay
 * in the set on purpose: they keep the repository non-converged, truthfully,
 * until they generate or their class is disabled.
 */
export function requiredArtifacts(manifest: Manifest): ArtifactRecord[] {
  return manifest.artifacts.filter((a) => a.required && a.status !== 'removed');
}
