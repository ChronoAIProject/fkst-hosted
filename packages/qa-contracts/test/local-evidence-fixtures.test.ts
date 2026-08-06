import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  canonicalBytes,
  contractContentDigest,
  contractRegistry,
  sha256Digest,
  validateLocalEvidenceObject,
  validateLocalEvidenceObjectRef,
  validateLocalSanitizedObservation,
  validateLocalSanitizedObservationRef,
  type ValidatedValue,
} from "../src/index.js";

type EvidenceType =
  | "LocalSanitizedObservation"
  | "LocalEvidenceObject"
  | "LocalSanitizedObservationRef"
  | "LocalEvidenceObjectRef";

interface EvidenceCase {
  readonly case_id: string;
  readonly evidence_type: EvidenceType;
  readonly source: Record<string, unknown>;
  readonly expected_canonical_utf8?: string;
  readonly expected_content_digest?: string;
  readonly raw_evidence_case_id?: string;
  readonly referenced_case_id?: string;
}

interface RawEvidenceCase {
  readonly case_id: string;
  readonly utf8: string;
  readonly expected_byte_length: number;
  readonly expected_sha256: string;
}

interface LocalEvidenceFixture {
  readonly schema_version: string;
  readonly raw_evidence_cases: readonly RawEvidenceCase[];
  readonly accepted_cases: readonly EvidenceCase[];
  readonly rejected_binding_cases: readonly EvidenceCase[];
  readonly raw_digest_non_binding_cases: readonly {
    readonly case_id: string;
    readonly raw_evidence_case_id: string;
    readonly referenced_case_id: string;
    readonly digest_form: "unprefixed" | "prefixed";
  }[];
  readonly unknown_type_case: {
    readonly case_id: string;
    readonly evidence_type: string;
    readonly source: Record<string, unknown>;
  };
}

const fixture = JSON.parse(
  readFileSync(new URL("../../../../fixtures/qa/local-evidence-v1.json", import.meta.url), "utf8"),
) as LocalEvidenceFixture;

test("local evidence fixture metadata", () => {
  assert.equal(fixture.schema_version, "qa.local-evidence-fixtures/v1");
  assert.deepEqual(
    fixture.accepted_cases.map(({ case_id, evidence_type }) => [case_id, evidence_type]),
    [
      ["local-observation-accepted", "LocalSanitizedObservation"],
      ["local-runner-log-object-accepted", "LocalEvidenceObject"],
      ["local-observation-ref-accepted", "LocalSanitizedObservationRef"],
      ["local-runner-log-ref-accepted", "LocalEvidenceObjectRef"],
    ],
  );

  const registry = contractRegistry();
  for (const evidenceType of fixture.accepted_cases.map(({ evidence_type }) => evidence_type)) {
    assert.deepEqual(registry.types[evidenceType], {
      schema: "qa.local-evidence/v1",
      pointer: `#/$defs/${evidenceType}`,
    });
  }
});

for (const rawCase of fixture.raw_evidence_cases) {
  test(rawCase.case_id, () => {
    console.log(`case_id=${rawCase.case_id}`);
    const rawBytes = Buffer.from(rawCase.utf8, "utf8");
    assert.equal(rawBytes.byteLength, rawCase.expected_byte_length);
    assert.equal(unprefixedSha256(rawBytes), rawCase.expected_sha256);
  });
}

for (const fixtureCase of fixture.accepted_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    const validated = validateEvidenceCase(fixtureCase.evidence_type, raw(fixtureCase.source));
    assert.deepEqual(validated.value(), fixtureCase.source);
    const canonical = canonicalBytes(validated);

    if (fixtureCase.expected_canonical_utf8 !== undefined) {
      assert.equal(Buffer.from(canonical).toString("utf8"), fixtureCase.expected_canonical_utf8);
      assert.equal(contractContentDigest(validated), fixtureCase.expected_content_digest);
      assert.equal(sha256Digest(canonical), fixtureCase.expected_content_digest);
    }

    if (fixtureCase.raw_evidence_case_id !== undefined) {
      const rawCase = findRawCase(fixtureCase.raw_evidence_case_id);
      assert.equal(fixtureCase.source.byte_length, rawCase.expected_byte_length);
      assert.equal(fixtureCase.source.sha256, rawCase.expected_sha256);
    }

    if (fixtureCase.referenced_case_id !== undefined) {
      assertReferenceBinds(validated, fixtureCase.referenced_case_id);
    }
  });
}

for (const fixtureCase of fixture.rejected_binding_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    const validatedReference = validateEvidenceCase(
      fixtureCase.evidence_type,
      raw(fixtureCase.source),
    );
    assert.doesNotThrow(() => canonicalBytes(validatedReference));
    assert.notEqual(
      referenceContentDigest(validatedReference),
      digestAcceptedCase(fixtureCase.referenced_case_id!),
    );
  });
}

for (const fixtureCase of fixture.raw_digest_non_binding_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    const rawDigest = findRawCase(fixtureCase.raw_evidence_case_id).expected_sha256;
    const candidate = fixtureCase.digest_form === "prefixed" ? `sha256:${rawDigest}` : rawDigest;
    assert.notEqual(candidate, digestAcceptedCase(fixtureCase.referenced_case_id));
  });
}

test(fixture.unknown_type_case.case_id, () => {
  console.log(`case_id=${fixture.unknown_type_case.case_id}`);
  assert.throws(
    () =>
      validateEvidenceCase(
        fixture.unknown_type_case.evidence_type as EvidenceType,
        raw(fixture.unknown_type_case.source),
      ),
    /unsupported evidence fixture type: Unknown/,
  );
});

function validateEvidenceCase(evidenceType: EvidenceType, source: Uint8Array): ValidatedValue {
  switch (evidenceType) {
    case "LocalSanitizedObservation":
      return validateLocalSanitizedObservation(source);
    case "LocalEvidenceObject":
      return validateLocalEvidenceObject(source);
    case "LocalSanitizedObservationRef":
      return validateLocalSanitizedObservationRef(source);
    case "LocalEvidenceObjectRef":
      return validateLocalEvidenceObjectRef(source);
    default: {
      const unsupportedType: never = evidenceType;
      throw new Error(`unsupported evidence fixture type: ${String(unsupportedType)}`);
    }
  }
}

function assertReferenceBinds(reference: ValidatedValue, referencedCaseId: string): void {
  assert.equal(referenceContentDigest(reference), digestAcceptedCase(referencedCaseId));
}

function referenceContentDigest(reference: ValidatedValue): string {
  const value = reference.value();
  assert.ok(value !== null && typeof value === "object" && !Array.isArray(value));
  const contentDigest = (value as Record<string, unknown>).content_digest;
  assert.ok(typeof contentDigest === "string");
  return contentDigest;
}

function digestAcceptedCase(caseId: string): string {
  const target = findAcceptedCase(caseId);
  return contractContentDigest(validateEvidenceCase(target.evidence_type, raw(target.source)));
}

function findAcceptedCase(caseId: string): EvidenceCase {
  const fixtureCase = fixture.accepted_cases.find((candidate) => candidate.case_id === caseId);
  assert.ok(fixtureCase, `unknown accepted case: ${caseId}`);
  return fixtureCase;
}

function findRawCase(caseId: string): RawEvidenceCase {
  const rawCase = fixture.raw_evidence_cases.find((candidate) => candidate.case_id === caseId);
  assert.ok(rawCase, `unknown raw evidence case: ${caseId}`);
  return rawCase;
}

function unprefixedSha256(bytes: Uint8Array): string {
  return sha256Digest(bytes).slice("sha256:".length);
}

function raw(value: unknown): Uint8Array {
  return Buffer.from(JSON.stringify(value));
}
