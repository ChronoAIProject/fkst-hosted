import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  ContractError,
  buildInitialRunAcceptance,
  canonicalBytes,
  contractContentDigest,
  contractContentProjection,
  contractRegistry,
  validateLocalQARunRequest,
  validateRunAcceptance,
  verifyContractContentDigest,
  type ValidatedValue,
} from "../src/index.js";

interface Fixture {
  readonly schema_version: string;
  readonly request: Record<string, unknown>;
  readonly builder_inputs: {
    readonly accepted_at: string;
    readonly producer_version: string;
  };
  readonly expected_request_utf8: string;
  readonly expected_request_projection_utf8: string;
  readonly expected_request_digest: string;
  readonly expected_acceptance_utf8: string;
  readonly expected_acceptance_projection_utf8: string;
  readonly expected_acceptance_digest: string;
}

const fixture = JSON.parse(
  readFileSync(
    new URL("../../fixtures/qa.local-run-admission/v1/happy-path.json", import.meta.url),
    "utf8",
  ),
) as Fixture;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

function raw(value: unknown): Uint8Array {
  return encoder.encode(JSON.stringify(value));
}

function mutatedRequest(field: string, value: unknown): Record<string, unknown> {
  return { ...structuredClone(fixture.request), [field]: value };
}

test("walks the shared LocalQARunRequest fixture into exact RunAcceptance bytes", () => {
  assert.equal(fixture.schema_version, "qa.local-run-admission-fixture/v1");
  const registry = contractRegistry();
  assert.deepEqual(registry.types.LocalQARunRequest, {
    schema: "qa.local-run-admission/v1",
    pointer: "#/$defs/LocalQARunRequest",
  });
  assert.deepEqual(registry.types.RunAcceptance, {
    schema: "qa.local-run-admission/v1",
    pointer: "#/$defs/RunAcceptance",
  });

  const request = validateLocalQARunRequest(raw(fixture.request));
  assert.equal(decoder.decode(canonicalBytes(request)), fixture.expected_request_utf8);
  assert.equal(
    decoder.decode(contractContentProjection(request)),
    fixture.expected_request_projection_utf8,
  );
  assert.equal(contractContentDigest(request), fixture.expected_request_digest);
  verifyContractContentDigest(request);

  const acceptance = buildInitialRunAcceptance(
    request,
    fixture.builder_inputs.accepted_at,
    fixture.builder_inputs.producer_version,
  );
  const acceptanceBytes = canonicalBytes(acceptance);
  assert.equal(decoder.decode(acceptanceBytes), fixture.expected_acceptance_utf8);
  assert.equal(
    decoder.decode(contractContentProjection(acceptance)),
    fixture.expected_acceptance_projection_utf8,
  );
  assert.equal(contractContentDigest(acceptance), fixture.expected_acceptance_digest);
  verifyContractContentDigest(validateRunAcceptance(acceptanceBytes));
});

test("rejects unsupported profile, unknown root members, and digest mismatch", () => {
  assert.throws(
    () => validateLocalQARunRequest(raw(mutatedRequest("profile", "local_qa_host_mvp"))),
    ContractError,
  );
  assert.throws(
    () => validateLocalQARunRequest(raw(mutatedRequest("unknown", true))),
    ContractError,
  );
  assert.throws(
    () => validateLocalQARunRequest(raw(mutatedRequest("producer_version", "changed/1"))),
    (error: unknown) =>
      error instanceof ContractError && error.rejection.code === "contract.digest_mismatch",
  );
});

test("treats acceptedAt equal to expires_at as expired without acceptance", () => {
  const request = validateLocalQARunRequest(raw(fixture.request));
  let acceptance: ValidatedValue | undefined;
  assert.throws(() => {
    acceptance = buildInitialRunAcceptance(
      request,
      "2026-08-14T04:05:00Z",
      fixture.builder_inputs.producer_version,
    );
  }, ContractError);
  assert.equal(acceptance, undefined);
});
