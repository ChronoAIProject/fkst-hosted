// Path selection for the source and output fingerprints (spec section 17.3, 17.6).
//
// Section 17.3 is explicit that its exclusions are unconditional and that
// configuration MUST NOT override them. The previous spec draft expressed the
// same exclusions twice — once as prose and once as a config list — without
// saying which prevailed; revision 2 resolved that in favour of the prose. This
// module therefore applies configuration FIRST and the unconditional rules
// LAST, so no include pattern can reach back into a reserved prefix.

import picomatch from 'picomatch';

/** The single Evolution root (section 12.1). */
export const EVOLUTION_ROOT = '.fkst/evolution/';
/** The pre-existing repo-local workflow catalog, independent of Evolution (section 33.4). */
export const PACKAGES_ROOT = '.fkst/packages/';
/** Owner-controlled policy — hashed into `productRelevant` only. */
export const CONFIG_PATH = `${EVOLUTION_ROOT}config.yaml`;
/** Owner-controlled intent — hashed into `productRelevant` only. */
export const INTENT_PREFIX = `${EVOLUTION_ROOT}intent/`;
/** Excluded from the output fingerprint to avoid a circular hash (section 17.6). */
export const MANIFEST_PATH = `${EVOLUTION_ROOT}manifest.json`;

export interface PathSelector {
  include: string[];
  exclude?: string[];
}

// `dot: true` because every reserved prefix here begins with a dot. Without it a
// `coverage.include: ["**"]` would silently skip `.github/`, `.fkst/` and
// friends, and the coverage fingerprint would quietly under-report.
const MATCH_OPTIONS = { dot: true } as const;

function compile(selector: PathSelector): (path: string) => boolean {
  const isIncluded = picomatch(selector.include, MATCH_OPTIONS);
  const isExcluded =
    selector.exclude && selector.exclude.length > 0
      ? picomatch(selector.exclude, MATCH_OPTIONS)
      : () => false;
  return (path: string) => isIncluded(path) && !isExcluded(path);
}

/** True for the human-owned paths that section 17.3 folds into `productRelevant`. */
export function isOwnerIntentPath(path: string): boolean {
  return path === CONFIG_PATH || path.startsWith(INTENT_PREFIX);
}

function isReserved(path: string): boolean {
  return path.startsWith(EVOLUTION_ROOT) || path.startsWith(PACKAGES_ROOT);
}

/**
 * `productRelevantFingerprint` membership (section 17.3).
 *
 * Owner intent is added back after the unconditional removal because it is
 * genuinely a convergence input: an owner who rewrites product positioning must
 * cause regeneration. Leaving it out is the failure mode section 17.3 calls out
 * explicitly for companion setups.
 */
export function productRelevantMatcher(selector: PathSelector): (path: string) => boolean {
  const configured = compile(selector);
  return (path: string) => {
    if (isOwnerIntentPath(path)) return true;
    if (isReserved(path)) return false;
    return configured(path);
  };
}

/** `coverageFingerprint` membership (section 17.3) — provenance only, no owner-intent add-back. */
export function coverageMatcher(selector: PathSelector): (path: string) => boolean {
  const configured = compile(selector);
  return (path: string) => {
    if (isReserved(path)) return false;
    return configured(path);
  };
}

/**
 * `outputFingerprint` file membership (section 17.6): everything Evolution wrote.
 *
 * `manifest.json` is excluded because it CONTAINS the fingerprint being
 * computed; section 17.6 folds it back in as a canonical projection instead.
 * `config.yaml` and `intent/**` are excluded because Evolution never writes
 * them — they are inputs, and hashing them as output would make an owner's edit
 * look like generator drift.
 */
export function managedOutputMatcher(): (path: string) => boolean {
  return (path: string) =>
    path.startsWith(EVOLUTION_ROOT) && !isOwnerIntentPath(path) && path !== MANIFEST_PATH;
}

/**
 * The section 25.8 / 12.1.1 write-boundary test.
 *
 * Deliberately a fixed prefix comparison with no configuration input: this is
 * the check that makes security objective 25.1(3) true rather than aspirational.
 * A symlink is compared by its own path and never by its target.
 */
export function isWritableByEvolution(path: string): boolean {
  // A `..` segment is refused outright. A Git tree never produces one, so this
  // can only arrive from generator-supplied input such as the artifact plan —
  // and `.fkst/evolution/../../backend/src/main.rs` passes a naive prefix test
  // while naming a file outside the root.
  if (path.split('/').includes('..')) return false;
  return path.startsWith(EVOLUTION_ROOT) && !isOwnerIntentPath(path);
}
