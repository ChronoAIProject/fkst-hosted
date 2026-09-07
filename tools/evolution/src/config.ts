// Parse and validate `.fkst/evolution/config.yaml` (spec sections 13.2, 13.3).
//
// Every check here fails CLOSED. Section 13.3.1 is emphatic that this validation
// is an input check and not a control: the write boundary of section 12.1.1 is
// enforced separately, at every write, precisely because `config.yaml` is
// repository content and a boundary defined by the thing it is supposed to
// bound is no boundary at all.

import { parse } from 'yaml';
import { log } from './log.ts';
import type { PathSelector } from './selector.ts';

export const SUPPORTED_SCHEMA_VERSION = 1;

/** Managed output classes and their schema-fixed subtrees (section 13.2). */
export const MANAGED_OUTPUT_SUBTREES = {
  documentation: 'docs/',
  skills: 'skills/',
  journeys: 'journeys/',
  screenshots: 'screenshots/',
  slides: 'slides/',
  video: null, // rendered binary — a Release asset, not a subtree (section 24.1)
} as const;

export type ManagedOutputClass = keyof typeof MANAGED_OUTPUT_SUBTREES;

/**
 * Which package role (section 28.1) produces each class.
 *
 * Section 17.7.2 rule 1 makes this mapping load-bearing: a class whose producer
 * role the owner declared absent contributes NO required artifacts, so the
 * verifier must not report it missing. The mapping names the role that produces
 * the class's committed bytes — for `slides` that is the narrative producer,
 * whose Markdown source is the artifact; the renderer's PDF is a derived
 * companion recorded as `required: false`.
 */
export const PRODUCER_ROLE_BY_CLASS: Record<ManagedOutputClass, string> = {
  documentation: 'documentation-maintainer',
  skills: 'skill-builder',
  journeys: 'demo-producer',
  screenshots: 'demo-producer',
  slides: 'narrative-producer',
  video: 'artifact-renderer',
};

export interface EvolutionConfig {
  schemaVersion: number;
  enabled: boolean;
  source: { branch: string; productRelevant: PathSelector; coverage: PathSelector };
  artifactRepository: string;
  intent: { product: string; overrides: string };
  managedOutputs: Record<ManagedOutputClass, { enabled: boolean; storage?: string }>;
  absentProducerRoles: string[];
  locales: string[];
  triggers: Record<string, unknown>;
  publication: {
    mode: string;
    requireCurrentSource: boolean;
    requireChecks: boolean;
    allowDirectPush: boolean;
    [k: string]: unknown;
  };
  drift: { policy: string };
  generatorEpoch: number;
  retention: Record<string, unknown>;
  security: {
    runPullRequestCode: boolean;
    allowProductionData: boolean;
    allowProductionCredentials: boolean;
  };
}

export class ConfigError extends Error {}

function requireObject(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new ConfigError(`${field} must be a mapping`);
  }
  return value as Record<string, unknown>;
}

function requireStringArray(value: unknown, field: string): string[] {
  if (!Array.isArray(value) || value.some((v) => typeof v !== 'string')) {
    throw new ConfigError(`${field} must be a list of strings`);
  }
  return value as string[];
}

function rejectUnknownKeys(obj: Record<string, unknown>, allowed: string[], field: string): void {
  const unknown = Object.keys(obj).filter((k) => !allowed.includes(k));
  if (unknown.length > 0) {
    // Section 13.3: silent acceptance would let MISSPELLED SAFETY POLICY appear
    // active — `allowProductionDat: false` would read as a policy that is in
    // fact absent, and the default would silently apply instead.
    throw new ConfigError(`${field} has unknown field(s): ${unknown.join(', ')}`);
  }
}

/**
 * Section 13.3: an include entry may not EXPLICITLY name a reserved prefix, but
 * a general wildcard is fine and is simply narrowed by section 17.3.
 *
 * The test is whether the pattern mentions the prefix literally at all. That
 * catches both `.fkst/evolution/docs/**` and `**​/.fkst/evolution/**`, while a
 * bare `**` passes — which is exactly the line section 13.3 draws.
 */
function assertNoReservedInclude(selector: PathSelector, field: string): void {
  for (const pattern of selector.include) {
    for (const reserved of ['.fkst/evolution', '.fkst/packages']) {
      if (pattern.includes(reserved)) {
        throw new ConfigError(
          `${field}.include may not explicitly name ${reserved} (pattern: ${pattern})`
        );
      }
    }
  }
}

function parseSelector(value: unknown, field: string): PathSelector {
  const obj = requireObject(value, field);
  rejectUnknownKeys(obj, ['include', 'exclude'], field);
  const include = requireStringArray(obj.include, `${field}.include`);
  const exclude = obj.exclude === undefined ? [] : requireStringArray(obj.exclude, `${field}.exclude`);
  return { include, exclude };
}

function parseManagedOutputs(value: unknown): EvolutionConfig['managedOutputs'] {
  const obj = requireObject(value, 'managedOutputs');
  const classes = Object.keys(MANAGED_OUTPUT_SUBTREES) as ManagedOutputClass[];
  rejectUnknownKeys(obj, classes, 'managedOutputs');

  const result = {} as EvolutionConfig['managedOutputs'];
  for (const cls of classes) {
    const entry = obj[cls] === undefined ? { enabled: false } : requireObject(obj[cls], `managedOutputs.${cls}`);
    // Section 13.3: "a managedOutputs entry carries a path, directory, or
    // destination field" is a hard failure — destinations are fixed by schema so
    // that configuration can enable or disable a class but never relocate it.
    rejectUnknownKeys(entry, cls === 'video' ? ['enabled', 'storage'] : ['enabled'], `managedOutputs.${cls}`);
    if (typeof entry.enabled !== 'boolean') {
      throw new ConfigError(`managedOutputs.${cls}.enabled must be a boolean`);
    }
    if (cls === 'video' && entry.enabled) {
      if (entry.storage !== 'github-release') {
        // Section 13.3: "a requested storage mode is not GitHub-native".
        throw new ConfigError(`managedOutputs.video.storage must be "github-release"`);
      }
    }
    result[cls] = { enabled: entry.enabled, storage: entry.storage as string | undefined };
  }
  return result;
}

/** Parse and fully validate a config document. Throws `ConfigError` on any violation. */
export function parseConfig(yamlText: string): EvolutionConfig {
  const raw = requireObject(parse(yamlText), 'config.yaml');
  rejectUnknownKeys(
    raw,
    [
      'schemaVersion', 'enabled', 'source', 'artifactRepository', 'intent', 'managedOutputs',
      'absentProducerRoles', 'locales', 'triggers', 'publication', 'drift', 'generatorEpoch',
      'retention', 'security',
    ],
    'config.yaml'
  );

  if (raw.schemaVersion !== SUPPORTED_SCHEMA_VERSION) {
    throw new ConfigError(
      `unsupported schemaVersion ${String(raw.schemaVersion)} (supported: ${SUPPORTED_SCHEMA_VERSION})`
    );
  }

  const source = requireObject(raw.source, 'source');
  rejectUnknownKeys(source, ['branch', 'productRelevant', 'coverage'], 'source');
  if (typeof source.branch !== 'string' || source.branch.length === 0) {
    throw new ConfigError('source.branch must be a non-empty string');
  }

  // Section 13.3: an absent or empty product-relevant set silently disables all
  // cycle admission, so it is rejected rather than defaulted. Section 13.2 and
  // open question 40.16 are explicit that NO default ships.
  if (source.productRelevant === undefined) {
    throw new ConfigError('source.productRelevant is required — no default set exists (section 13.2)');
  }
  const productRelevant = parseSelector(source.productRelevant, 'source.productRelevant');
  if (productRelevant.include.length === 0) {
    throw new ConfigError('source.productRelevant.include must not be empty');
  }
  const coverage = parseSelector(source.coverage, 'source.coverage');
  assertNoReservedInclude(productRelevant, 'source.productRelevant');
  assertNoReservedInclude(coverage, 'source.coverage');

  const publication = requireObject(raw.publication, 'publication');
  if (publication.allowDirectPush !== false) {
    // Section 13.3 + invariant 41.6: Evolution never pushes the trusted branch.
    throw new ConfigError('publication.allowDirectPush must be false (section 21.6)');
  }
  if (publication.requireChecks !== true) {
    throw new ConfigError('publication.requireChecks must be true — a merge policy that cannot honor required checks is rejected');
  }
  const modes = ['disabled', 'observe', 'propose', 'automerge-managed', 'release-gated'];
  if (typeof publication.mode !== 'string' || !modes.includes(publication.mode)) {
    throw new ConfigError(`publication.mode must be one of: ${modes.join(', ')}`);
  }

  const security = requireObject(raw.security, 'security');
  rejectUnknownKeys(security, ['runPullRequestCode', 'allowProductionData', 'allowProductionCredentials'], 'security');
  if (security.runPullRequestCode !== false) {
    // Section 13.3: "security policy requests privileged execution of an
    // untrusted PR head". Section 19.2 makes PR processing read-only.
    throw new ConfigError('security.runPullRequestCode must be false (section 19.2)');
  }
  if (security.allowProductionData !== false || security.allowProductionCredentials !== false) {
    throw new ConfigError('security.allowProductionData and allowProductionCredentials must be false (sections 25.6, 25.7)');
  }

  const drift = requireObject(raw.drift, 'drift');
  if (!['block', 'repair', 'adopt'].includes(String(drift.policy))) {
    throw new ConfigError('drift.policy must be one of: block, repair, adopt');
  }

  if (typeof raw.generatorEpoch !== 'number' || !Number.isSafeInteger(raw.generatorEpoch)) {
    throw new ConfigError('generatorEpoch must be an integer');
  }
  if (typeof raw.artifactRepository !== 'string' || raw.artifactRepository.length === 0) {
    throw new ConfigError('artifactRepository must be a non-empty string');
  }

  const intent = requireObject(raw.intent, 'intent');
  rejectUnknownKeys(intent, ['product', 'overrides'], 'intent');

  const config: EvolutionConfig = {
    schemaVersion: raw.schemaVersion,
    enabled: raw.enabled !== false,
    source: { branch: source.branch, productRelevant, coverage },
    artifactRepository: raw.artifactRepository,
    intent: { product: String(intent.product), overrides: String(intent.overrides) },
    managedOutputs: parseManagedOutputs(raw.managedOutputs),
    absentProducerRoles:
      raw.absentProducerRoles === undefined
        ? []
        : requireStringArray(raw.absentProducerRoles, 'absentProducerRoles'),
    locales: requireStringArray(raw.locales ?? ['en'], 'locales'),
    triggers: requireObject(raw.triggers ?? {}, 'triggers'),
    publication: publication as EvolutionConfig['publication'],
    drift: { policy: String(drift.policy) },
    generatorEpoch: raw.generatorEpoch,
    retention: requireObject(raw.retention ?? {}, 'retention'),
    security: security as unknown as EvolutionConfig['security'],
  };

  log.debug('config validated', {
    branch: config.source.branch,
    mode: config.publication.mode,
    epoch: config.generatorEpoch,
  });
  return config;
}

/** Classes that contribute required artifacts (section 17.7.2 rules 1). */
export function requiredClasses(config: EvolutionConfig): ManagedOutputClass[] {
  return (Object.keys(config.managedOutputs) as ManagedOutputClass[]).filter((cls) => {
    if (!config.managedOutputs[cls].enabled) return false;
    return !config.absentProducerRoles.includes(PRODUCER_ROLE_BY_CLASS[cls]);
  });
}
