// Configuration validation (spec section 13.3) and path selection (17.3, 17.6).
//
// Every case here is a FAIL-CLOSED case or a boundary case. Section 13.3.1 is
// explicit that this validation is an input check and not a control, so the
// point of these tests is that a bad config is refused loudly rather than
// defaulted quietly.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { ConfigError, parseConfig, requiredClasses } from '../src/config.ts';
import {
  coverageMatcher, isWritableByEvolution, managedOutputMatcher, productRelevantMatcher,
} from '../src/selector.ts';

const BASE = `
schemaVersion: 1
enabled: true
source:
  branch: "@default"
  productRelevant:
    include: ["backend/src/**", "frontend/src/**"]
    exclude: ["**/*_tests.rs"]
  coverage:
    include: ["**"]
    exclude: [".git/**"]
artifactRepository: "."
intent:
  product: ".fkst/evolution/intent/product.md"
  overrides: ".fkst/evolution/intent/overrides.yaml"
managedOutputs:
  documentation: { enabled: true }
  skills: { enabled: true }
  journeys: { enabled: true }
  screenshots: { enabled: true }
  slides: { enabled: true }
  video: { enabled: true, storage: "github-release" }
absentProducerRoles: []
locales: ["en"]
triggers:
  defaultBranchPush: true
publication:
  mode: "propose"
  requireCurrentSource: true
  requireChecks: true
  allowDirectPush: false
drift:
  policy: "block"
generatorEpoch: 1
retention:
  renderedSnapshots: 10
security:
  runPullRequestCode: false
  allowProductionData: false
  allowProductionCredentials: false
`;

const withLine = (from: string, to: string) => BASE.replace(from, to);

test('the baseline config parses', () => {
  const config = parseConfig(BASE);
  assert.equal(config.source.branch, '@default');
  assert.equal(config.publication.mode, 'propose');
  assert.equal(config.generatorEpoch, 1);
});

test('an unsupported schemaVersion fails closed', () => {
  assert.throws(() => parseConfig(withLine('schemaVersion: 1', 'schemaVersion: 2')), ConfigError);
});

test('an absent productRelevant set fails closed — no default exists', () => {
  const noSet = BASE.replace(
    /  productRelevant:\n    include: \[.*\]\n    exclude: \[.*\]\n/,
    ''
  );
  assert.throws(() => parseConfig(noSet), /source.productRelevant is required/);
});

test('an empty productRelevant include fails closed', () => {
  // An empty set silently disables ALL cycle admission — the failure mode
  // section 13.3 names explicitly.
  assert.throws(
    () => parseConfig(withLine('include: ["backend/src/**", "frontend/src/**"]', 'include: []')),
    /must not be empty/
  );
});

test('explicitly naming the Evolution root in an include fails closed', () => {
  assert.throws(
    () => parseConfig(withLine('include: ["backend/src/**", "frontend/src/**"]',
      'include: ["backend/src/**", ".fkst/evolution/docs/**"]')),
    /may not explicitly name .fkst\/evolution/
  );
});

test('a broad wildcard is permitted and simply narrowed later', () => {
  // Section 13.3 draws the line at EXPLICIT re-inclusion; `**` is fine.
  const config = parseConfig(withLine('include: ["backend/src/**", "frontend/src/**"]', 'include: ["**"]'));
  assert.equal(config.source.productRelevant.include[0], '**');
});

test('a managedOutputs entry carrying a destination fails closed', () => {
  assert.throws(
    () => parseConfig(withLine('documentation: { enabled: true }',
      'documentation: { enabled: true, path: "docs/custom" }')),
    /unknown field/
  );
});

test('a non-GitHub-native video storage fails closed', () => {
  assert.throws(
    () => parseConfig(withLine('video: { enabled: true, storage: "github-release" }',
      'video: { enabled: true, storage: "s3" }')),
    /must be "github-release"/
  );
});

test('requesting direct push fails closed', () => {
  assert.throws(() => parseConfig(withLine('allowDirectPush: false', 'allowDirectPush: true')), ConfigError);
});

test('a merge policy that cannot honor required checks fails closed', () => {
  assert.throws(() => parseConfig(withLine('requireChecks: true', 'requireChecks: false')), ConfigError);
});

test('requesting privileged execution of an untrusted PR head fails closed', () => {
  assert.throws(
    () => parseConfig(withLine('runPullRequestCode: false', 'runPullRequestCode: true')),
    /must be false/
  );
});

test('permitting production data or credentials fails closed', () => {
  assert.throws(() => parseConfig(withLine('allowProductionData: false', 'allowProductionData: true')), ConfigError);
  assert.throws(
    () => parseConfig(withLine('allowProductionCredentials: false', 'allowProductionCredentials: true')),
    ConfigError
  );
});

test('an unknown top-level field is rejected rather than ignored', () => {
  // Silent acceptance would let MISSPELLED safety policy appear active.
  assert.throws(() => parseConfig(`${BASE}\nallowProdData: true\n`), /unknown field/);
});

test('an unknown publication mode fails closed', () => {
  assert.throws(() => parseConfig(withLine('mode: "propose"', 'mode: "yolo"')), /publication.mode must be one of/);
});

test('requiredClasses drops disabled classes', () => {
  const config = parseConfig(withLine('slides: { enabled: true }', 'slides: { enabled: false }'));
  assert.ok(!requiredClasses(config).includes('slides'));
  assert.ok(requiredClasses(config).includes('documentation'));
});

test('requiredClasses drops classes whose producer role is declared absent', () => {
  // Section 17.7.2 rule 1: the verifier must not report an undeployed role's
  // artifacts as missing.
  const config = parseConfig(withLine('absentProducerRoles: []', 'absentProducerRoles: ["artifact-renderer"]'));
  assert.ok(!requiredClasses(config).includes('video'));
  assert.ok(requiredClasses(config).includes('screenshots'));
});

// ---- selectors -------------------------------------------------------------

test('productRelevant always includes owner intent, even outside the include set', () => {
  const config = parseConfig(BASE);
  const match = productRelevantMatcher(config.source.productRelevant);
  assert.ok(match('.fkst/evolution/config.yaml'));
  assert.ok(match('.fkst/evolution/intent/product.md'));
  assert.ok(match('backend/src/main.rs'));
});

test('productRelevant excludes generated Evolution output and the packages catalog', () => {
  const config = parseConfig(BASE);
  const match = productRelevantMatcher(config.source.productRelevant);
  assert.ok(!match('.fkst/evolution/docs/anything.md'));
  assert.ok(!match('.fkst/evolution/manifest.json'));
  assert.ok(!match('.fkst/packages/whatever.toml'));
});

test('a configured exclude still applies inside the include set', () => {
  const config = parseConfig(BASE);
  const match = productRelevantMatcher(config.source.productRelevant);
  assert.ok(!match('backend/src/config_tests.rs'));
});

test('coverage sees dotfiles under a bare ** but never the reserved prefixes', () => {
  const config = parseConfig(BASE);
  const match = coverageMatcher(config.source.coverage);
  assert.ok(match('.github/workflows/rust-ci.yml'), 'dot: true is required or coverage under-reports');
  assert.ok(match('README.md'));
  assert.ok(!match('.fkst/evolution/docs/x.md'));
});

test('coverage does NOT get the owner-intent add-back', () => {
  // Only productRelevant folds intent back in; coverage is provenance.
  const config = parseConfig(BASE);
  assert.ok(!coverageMatcher(config.source.coverage)('.fkst/evolution/config.yaml'));
});

test('managed-output matcher excludes the manifest and owner-owned inputs', () => {
  const match = managedOutputMatcher();
  assert.ok(match('.fkst/evolution/docs/page.md'));
  assert.ok(!match('.fkst/evolution/manifest.json'), 'excluded to avoid a circular hash');
  assert.ok(!match('.fkst/evolution/config.yaml'));
  assert.ok(!match('.fkst/evolution/intent/product.md'));
  assert.ok(!match('backend/src/main.rs'));
});

test('the write boundary is a fixed prefix comparison', () => {
  assert.ok(isWritableByEvolution('.fkst/evolution/docs/a.md'));
  assert.ok(!isWritableByEvolution('.fkst/evolution/config.yaml'));
  assert.ok(!isWritableByEvolution('.fkst/evolution/intent/overrides.yaml'));
  assert.ok(!isWritableByEvolution('.fkst/packages/x'));
  assert.ok(!isWritableByEvolution('backend/src/main.rs'));
  // A traversal attempt is compared by its own literal path and never resolved.
  assert.ok(!isWritableByEvolution('.fkst/evolution/../../backend/src/main.rs'));
});
