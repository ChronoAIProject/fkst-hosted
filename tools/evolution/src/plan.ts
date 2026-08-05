// The artifact plan — the generator's declaration of what it produces.
//
// WHY this lives in `tools/evolution/` and not under `.fkst/evolution/`:
// section 12.3 classifies everything under the Evolution root as human intent or
// Evolution output. The plan is neither — it is generator INPUT, describing
// which artifacts the producer roles emit and which capabilities each one
// represents. Putting it under the root would make the generator's own
// configuration part of the output fingerprint it computes, which is circular.
//
// In the eventual package composition (section 28.1) this file is replaced by
// the producer roles' own declarations in `fkst-packages`.

import { parse } from 'yaml';
import type { ManagedOutputClass } from './config.ts';

export const PLAN_SCHEMA_VERSION = 1;

export interface PlannedArtifact {
  id: string;
  kind: ManagedOutputClass;
  locale: string;
  audience: string;
  capabilities: string[];
  journeys: string[];
  /** Repository path, for artifacts committed to Git. */
  repositoryPath?: string;
  /**
   * Release asset identity for a rendered binary (section 24.3).
   *
   * There is deliberately no `tag` field. Section 24.2 DERIVES the tag from the
   * input fingerprint (`fkst-evolution/<first-16-hex>`), so letting the plan
   * name one would allow generator input to point an artifact at an unrelated
   * Release — and the asset name is likewise derived from the content hash.
   */
  release?: { repository: string; assetBaseName: string; localPath: string };
  status: string;
  required: boolean;
  verification: string;
}

export interface PlannedCheck {
  id: string;
  status: string;
  evidence: string;
  checkRunName: string;
  provenance: string;
}

export interface ArtifactPlan {
  schemaVersion: number;
  generator: {
    /** `owner/repo@{commit}:path` — `{commit}` is substituted with the observed head. */
    manifestRef: string;
    packages: { directory: string; ref: string }[];
    engineVersion: string;
    model: string;
  };
  verification: PlannedCheck[];
  artifacts: PlannedArtifact[];
}

export class PlanError extends Error {}

/** Parse and check an artifact plan. */
export function parsePlan(yamlText: string): ArtifactPlan {
  const raw = parse(yamlText) as ArtifactPlan;
  if (raw?.schemaVersion !== PLAN_SCHEMA_VERSION) {
    throw new PlanError(`unsupported plan schemaVersion ${String(raw?.schemaVersion)}`);
  }
  if (!Array.isArray(raw.artifacts) || raw.artifacts.length === 0) {
    throw new PlanError('plan.artifacts must be a non-empty list');
  }
  const seen = new Set<string>();
  for (const artifact of raw.artifacts) {
    if (seen.has(artifact.id)) throw new PlanError(`duplicate artifact id: ${artifact.id}`);
    seen.add(artifact.id);
    if (!artifact.repositoryPath && !artifact.release) {
      throw new PlanError(`artifact ${artifact.id} has neither repositoryPath nor release`);
    }
    if (artifact.repositoryPath && artifact.release) {
      // One artifact, one home. Two would make condition 3 ambiguous about which
      // bytes it is re-hashing.
      throw new PlanError(`artifact ${artifact.id} declares both repositoryPath and release`);
    }
  }
  if (!Array.isArray(raw.verification) || raw.verification.length === 0) {
    throw new PlanError('plan.verification must declare at least one check');
  }
  return raw;
}
