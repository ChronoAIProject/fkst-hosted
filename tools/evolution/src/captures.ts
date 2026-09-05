// Change-impact analysis for captures (spec section 32.2).
//
// THE PROBLEM THIS SOLVES. Headless Chromium's rasterization is not
// bit-reproducible: re-running a journey against an unchanged product yields
// PNGs that differ by up to 2/255 per pixel. The output fingerprint hashes
// bytes, so a capture rewritten for no reason changes `outputFingerprint`, which
// section 17.7 condition 2 reads as non-convergence and section 17.8 classifies
// as managed-output drift — which, under the bootstrap `block` policy, stops the
// lane. Every media-bearing repository would sit in permanent false drift.
//
// THE RULE. A capture is adopted into the managed subtree only when something it
// depends on actually moved. Section 32.4 already states this for video
// rendering ("SHOULD occur only when its journey, product UI, locale, template,
// narration, or renderer inputs changed"); this applies the same rule to
// screenshots, which the spec leaves implicit.
//
// A capture depends on:
//   * the product surface        -> `inputFingerprint` (section 17.5)
//   * the journey that drives it -> the journey artifact's recorded contentHash
//   * the capture settings       -> pinned in the journey's Playwright config,
//                                   and therefore covered by the journey hash
//
// If none of those moved, the committed bytes stand. This is deliberately NOT a
// perceptual comparison: an artifact's integrity is its content hash, and
// "close enough" is not a hash.

import { copyFile, mkdir, readFile, stat } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { contentHash } from './hash.ts';
import { log } from './log.ts';
import type { Manifest } from './manifest.ts';

/** Why a capture was or was not adopted — surfaced so a run is explainable. */
export type AdoptionReason =
  | 'no-committed-bytes'
  | 'input-fingerprint-moved'
  | 'journey-changed'
  | 'unchanged-inputs'
  | 'no-fresh-capture';

export interface CaptureDecision {
  artifactId: string;
  repositoryPath: string;
  adopted: boolean;
  reason: AdoptionReason;
}

export interface AdoptionInputs {
  repoRoot: string;
  /** The committed manifest, or null when this is a baseline run. */
  manifest: Manifest | null;
  /** Freshly computed input fingerprint at the revision being generated. */
  currentInputFingerprint: string;
  /** Current content hash of each journey artifact, keyed by artifact id. */
  currentJourneyHashes: Map<string, string>;
  /** Directory the journey wrote its fresh captures into. */
  freshCaptureDir: string;
}

/** Artifact kinds whose bytes are a capture rather than authored content. */
const CAPTURE_KINDS = new Set(['screenshots']);

async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

/**
 * Decide, per capture, whether the freshly produced bytes should replace the
 * committed ones — and apply the decision.
 *
 * Returns one decision per capture artifact so the caller can report exactly
 * what moved and why. A run that adopts nothing is the expected steady state,
 * not a failure.
 */
export async function adoptCaptures(inputs: AdoptionInputs): Promise<CaptureDecision[]> {
  const { repoRoot, manifest, currentInputFingerprint, currentJourneyHashes, freshCaptureDir } = inputs;
  const decisions: CaptureDecision[] = [];

  const captures = (manifest?.artifacts ?? []).filter((a) => CAPTURE_KINDS.has(a.kind));
  // A baseline run has no manifest and therefore no committed bytes to protect;
  // everything the journey produced is adopted.
  const inputMoved = manifest === null || manifest.source.inputFingerprint !== currentInputFingerprint;

  for (const artifact of captures) {
    if (!artifact.repositoryPath) continue;
    const committedPath = join(repoRoot, artifact.repositoryPath);
    const freshPath = join(freshCaptureDir, `${captureIdOf(artifact.id)}.png`);

    const hasFresh = await exists(freshPath);
    const hasCommitted = await exists(committedPath);

    const journeyMoved = artifact.journeys.some((journeyId) => {
      const recorded = recordedJourneyHash(manifest, journeyId);
      const current = currentJourneyHashes.get(journeyId);
      // An unknown hash on either side is treated as "moved": failing towards
      // regeneration is recoverable, whereas failing towards "keep" would let a
      // stale capture outlive the journey that justified it.
      return recorded === undefined || current === undefined || recorded !== current;
    });

    let reason: AdoptionReason;
    if (!hasCommitted) reason = 'no-committed-bytes';
    else if (inputMoved) reason = 'input-fingerprint-moved';
    else if (journeyMoved) reason = 'journey-changed';
    else reason = 'unchanged-inputs';

    const wants = reason !== 'unchanged-inputs';
    if (wants && !hasFresh) {
      // Wanted a refresh and has nothing to refresh from. Keeping the committed
      // bytes is right — section 26.5 preserves the last known good artifact —
      // but it must be visible, because the artifact is now older than its inputs.
      decisions.push({ artifactId: artifact.id, repositoryPath: artifact.repositoryPath, adopted: false, reason: 'no-fresh-capture' });
      log.warn('capture needs regeneration but no fresh bytes exist', {
        artifact: artifact.id,
        expected: freshPath,
      });
      continue;
    }

    if (wants) {
      await mkdir(dirname(committedPath), { recursive: true });
      await copyFile(freshPath, committedPath);
      log.info('capture adopted', { artifact: artifact.id, reason });
    } else {
      log.debug('capture unchanged — committed bytes kept', { artifact: artifact.id });
    }
    decisions.push({ artifactId: artifact.id, repositoryPath: artifact.repositoryPath, adopted: wants, reason });
  }

  return decisions;
}

/**
 * The capture id a journey wrote, derived from the artifact id.
 *
 * Artifact ids are `screenshot.<capture-id>` by convention in the plan, and the
 * journey names its files by capture id. Keeping the mapping in one function
 * means a convention change breaks loudly here rather than silently producing
 * "no fresh capture" for everything.
 */
export function captureIdOf(artifactId: string): string {
  const dot = artifactId.indexOf('.');
  if (dot < 0) throw new Error(`capture artifact id must be "<kind>.<capture-id>": ${artifactId}`);
  return artifactId.slice(dot + 1);
}

function recordedJourneyHash(manifest: Manifest | null, journeyId: string): string | undefined {
  if (!manifest) return undefined;
  // The journey's own artifact record carries the hash of the spec file that
  // drove the capture.
  const journeyArtifact = manifest.artifacts.find(
    (a) => a.kind === 'journeys' && a.journeys.includes(journeyId)
  );
  return journeyArtifact?.contentHash;
}

/** Hash the journey source files a manifest declares, for the comparison above. */
export async function currentJourneyHashes(
  repoRoot: string,
  manifest: Manifest | null
): Promise<Map<string, string>> {
  const hashes = new Map<string, string>();
  for (const artifact of manifest?.artifacts ?? []) {
    if (artifact.kind !== 'journeys' || !artifact.repositoryPath) continue;
    try {
      const bytes = await readFile(join(repoRoot, artifact.repositoryPath));
      for (const journeyId of artifact.journeys) hashes.set(journeyId, contentHash(bytes));
    } catch (error) {
      log.warn('journey source unreadable — treating as changed', {
        path: artifact.repositoryPath,
        error: String(error),
      });
    }
  }
  return hashes;
}
