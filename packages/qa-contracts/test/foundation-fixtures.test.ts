import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  admitJson,
  canonicalAdmittedBytes,
  canonicalBytes,
  contractContentDigest,
  contractContentProjection,
  contractRegistry,
  ContractError,
  type FoundationType,
  foundationTypeNames,
  sha256Digest,
  validateFoundation,
  verifyContractContentDigest,
} from "../src/index.js";

interface ExpectedRejection {
  readonly category: string;
  readonly code?: string;
  readonly reason: string;
  readonly path: string;
}

interface RfcFixture {
  readonly schema_version: string;
  readonly gate_id: string;
  readonly valid_cases: readonly {
    readonly case_id: string;
    readonly source_utf8_base64: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  }[];
  readonly invalid_cases: readonly {
    readonly case_id: string;
    readonly source_utf8_base64: string;
    readonly expected: ExpectedRejection;
  }[];
}

interface FoundationFixture {
  readonly schema_version: string;
  readonly gate_id: string;
  readonly valid_cases: readonly {
    readonly case_id: string;
    readonly foundation_type: FoundationType;
    readonly source: unknown;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  }[];
  readonly invalid_cases: readonly {
    readonly case_id: string;
    readonly foundation_type: FoundationType;
    readonly source: unknown;
    readonly expected: ExpectedRejection;
  }[];
  readonly projection_cases: readonly {
    readonly case_id: string;
    readonly foundation_type: FoundationType;
    readonly source: Record<string, unknown>;
    readonly expected_projection_utf8_base64: string;
    readonly expected_sha256: string;
  }[];
  readonly digest_mismatch_cases: readonly {
    readonly case_id: string;
    readonly foundation_type: FoundationType;
    readonly source: Record<string, unknown>;
    readonly expected_projection_sha256: string;
    readonly expected: ExpectedRejection;
  }[];
}

const fixturesRoot = new URL("../../../../fixtures/", import.meta.url);
const rfcFixture = loadJson<RfcFixture>(new URL("rfc8785-v1.json", fixturesRoot));
const foundationFixture = loadJson<FoundationFixture>(
  new URL("qa/contract-foundation-v1.json", fixturesRoot),
);

const fixtureIds = [
  ...rfcFixture.valid_cases.map((fixtureCase) => fixtureCase.case_id),
  ...rfcFixture.invalid_cases.map((fixtureCase) => fixtureCase.case_id),
  ...foundationFixture.valid_cases.map((fixtureCase) => fixtureCase.case_id),
  ...foundationFixture.invalid_cases.map((fixtureCase) => fixtureCase.case_id),
  ...foundationFixture.projection_cases.map((fixtureCase) => fixtureCase.case_id),
  ...foundationFixture.digest_mismatch_cases.map((fixtureCase) => fixtureCase.case_id),
];

test("contract registry and fixture metadata", () => {
  const registry = contractRegistry();
  assert.equal(registry.registry_version, "qa.contract-registry/v1");
  assert.equal(registry.profile, "local_qa_host_mvp");
  assert.deepEqual(Object.keys(registry.schemas), ["qa.contract-foundation/v1"]);
  assert.deepEqual(Object.keys(registry.types).sort(), [...foundationTypeNames()].sort());
  assert.equal(registry.types.ProjectionSpecimen?.fixture_only, true);
  assert.equal(registry.types.StrictUnionSpecimen?.fixture_only, true);
  assert.equal(registry.types.ContractMeta?.fixture_only, undefined);
  assert.ok(Object.isFrozen(foundationTypeNames()));
  assert.equal(rfcFixture.schema_version, "qa.rfc8785-fixtures/v1");
  assert.equal(foundationFixture.schema_version, "qa.contract-foundation-fixtures/v1");
  assert.equal(rfcFixture.gate_id, "P0-02-CONTRACT-FOUNDATION");
  assert.equal(foundationFixture.gate_id, "P0-02-CONTRACT-FOUNDATION");
  assert.equal(new Set(fixtureIds).size, fixtureIds.length, "fixture case IDs are unique");

  const mutableRegistry = registry as unknown as Record<string, unknown>;
  assert.ok(Object.isFrozen(registry));
  assert.throws(() => {
    mutableRegistry.profile = "mutated";
  }, TypeError);
});

for (const fixtureCase of rfcFixture.valid_cases) {
  test(fixtureCase.case_id, () => {
    const raw = Buffer.from(fixtureCase.source_utf8_base64, "base64");
    const admitted = admitJson(raw);
    const canonical = canonicalAdmittedBytes(admitted);
    assert.equal(
      Buffer.from(canonical).toString("base64"),
      fixtureCase.expected_canonical_utf8_base64,
    );
    assert.equal(sha256Digest(canonical), fixtureCase.expected_sha256);
  });
}

for (const fixtureCase of rfcFixture.invalid_cases) {
  test(fixtureCase.case_id, () => {
    const raw = Buffer.from(fixtureCase.source_utf8_base64, "base64");
    assert.throws(
      () => admitJson(raw),
      (error: unknown) => rejectionMatches(error, fixtureCase.expected, fixtureCase.case_id),
    );
  });
}

for (const fixtureCase of foundationFixture.valid_cases) {
  test(fixtureCase.case_id, () => {
    const validated = validateFoundation(
      Buffer.from(JSON.stringify(fixtureCase.source)),
      fixtureCase.foundation_type,
    );
    const canonical = canonicalBytes(validated);
    assert.equal(
      Buffer.from(canonical).toString("base64"),
      fixtureCase.expected_canonical_utf8_base64,
    );
    assert.equal(sha256Digest(canonical), fixtureCase.expected_sha256);
  });
}

for (const fixtureCase of foundationFixture.invalid_cases) {
  test(fixtureCase.case_id, () => {
    assert.throws(
      () =>
        validateFoundation(
          Buffer.from(JSON.stringify(fixtureCase.source)),
          fixtureCase.foundation_type,
        ),
      (error: unknown) => rejectionMatches(error, fixtureCase.expected, fixtureCase.case_id),
    );
  });
}

for (const fixtureCase of foundationFixture.projection_cases) {
  test(fixtureCase.case_id, () => {
    const validated = validateFoundation(
      Buffer.from(JSON.stringify(fixtureCase.source)),
      fixtureCase.foundation_type,
    );
    const projection = contractContentProjection(validated);
    assert.equal(
      Buffer.from(projection).toString("base64"),
      fixtureCase.expected_projection_utf8_base64,
    );
    assert.equal(contractContentDigest(validated), fixtureCase.expected_sha256);
    verifyContractContentDigest(validated);
  });
}

for (const fixtureCase of foundationFixture.digest_mismatch_cases) {
  test(fixtureCase.case_id, () => {
    const validated = validateFoundation(
      Buffer.from(JSON.stringify(fixtureCase.source)),
      fixtureCase.foundation_type,
    );
    assert.equal(contractContentDigest(validated), fixtureCase.expected_projection_sha256);
    assert.throws(
      () => verifyContractContentDigest(validated),
      (error: unknown) => rejectionMatches(error, fixtureCase.expected, fixtureCase.case_id),
    );
  });
}

test("admitted and validated values are deeply immutable", () => {
  const admitted = admitJson(Buffer.from('{"nested":{"value":1}}'));
  const admittedValue = admitted.value() as Record<string, unknown>;
  const admittedNested = admittedValue.nested as Record<string, unknown>;
  const admittedCanonical = canonicalAdmittedBytes(admitted);
  assert.ok(Object.isFrozen(admittedValue));
  assert.ok(Object.isFrozen(admittedNested));
  assert.throws(() => {
    admittedNested.value = 2;
  }, TypeError);
  assert.deepEqual(canonicalAdmittedBytes(admitted), admittedCanonical);

  const projectionCase = foundationFixture.projection_cases[0]!;
  const validated = validateFoundation(
    Buffer.from(JSON.stringify(projectionCase.source)),
    projectionCase.foundation_type,
  );
  const validatedValue = validated.value() as Record<string, unknown>;
  const payload = validatedValue.payload as Record<string, unknown>;
  const digest = contractContentDigest(validated);
  assert.ok(Object.isFrozen(validatedValue));
  assert.ok(Object.isFrozen(payload));
  assert.throws(() => {
    payload.a = 99;
  }, TypeError);
  assert.equal(contractContentDigest(validated), digest);
});

function loadJson<T>(url: URL): T {
  return JSON.parse(readFileSync(url, "utf8")) as T;
}

function rejectionMatches(error: unknown, expected: ExpectedRejection, caseId: string): boolean {
  assert.ok(error instanceof ContractError, `${caseId}: expected ContractError`);
  assert.deepEqual(error.rejection, expected, `${caseId}: rejection`);
  return true;
}
