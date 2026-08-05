import assert from "node:assert/strict";
import test from "node:test";

import {
  ContractError,
  contractRegistry,
  validateLocalSanitizedObservation,
} from "../src/index.js";

const ACCEPTED_JSON =
  '{"schema_version":"qa.local-evidence/v1","run_id":"run_1","attempt":1,"fixture_url":"http://127.0.0.1:3210/fixed-page.html","final_url":"http://127.0.0.1:3210/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expected_text":"READY","observed_text":"READY"}';
const DUPLICATE_RUN_ID_JSON =
  '{"schema_version":"qa.local-evidence/v1","run_id":"run_1","run_id":"run_2","attempt":1,"fixture_url":"http://127.0.0.1:3210/fixed-page.html","final_url":"http://127.0.0.1:3210/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expected_text":"READY","observed_text":"READY"}';

const acceptedObservation = Object.freeze({
  schema_version: "qa.local-evidence/v1",
  run_id: "run_1",
  attempt: 1,
  fixture_url: "http://127.0.0.1:3210/fixed-page.html",
  final_url: "http://127.0.0.1:3210/fixed-page.html",
  selector: '[data-local-qa="status"]',
  expected_text: "READY",
  observed_text: "READY",
});

const prohibitedUrls = [
  "http://localhost:3210/fixed-page.html",
  "http://127.0.0.1:0/fixed-page.html",
  "http://127.0.0.1:65536/fixed-page.html",
  "http://user@127.0.0.1:3210/fixed-page.html",
  "http://[::1]:3210/fixed-page.html",
  "http://127.0.0.1:3210/fixed-page.html?x=1",
  "http://127.0.0.1:3210/fixed-page.html#status",
  "http://127.0.0.1:3210/%66ixed-page.html",
  "http://127.0.0.1:3210/fixed-page.html/extra",
] as const;

test("fixed passing MVP observation walks registry and TypeScript validator", () => {
  const registry = contractRegistry();
  assert.deepEqual(registry.schemas["qa.local-evidence/v1"], {
    path: "contracts/qa.local-evidence/v1/schema.json",
    id: "urn:chronoai:fkst:qa-contracts:qa.local-evidence:v1",
    major: 1,
  });
  assert.deepEqual(registry.types.LocalSanitizedObservation, {
    schema: "qa.local-evidence/v1",
    pointer: "#/$defs/LocalSanitizedObservation",
  });

  const validated = validateLocalSanitizedObservation(Buffer.from(ACCEPTED_JSON));
  assert.deepEqual(validated.value(), acceptedObservation);
  assert.ok(Object.isFrozen(validated.value()));
});

test("LocalSanitizedObservation rejects unequal URLs and unknown fields", () => {
  assertRejection(
    () =>
      validateLocalSanitizedObservation(
        rawWith({ final_url: "http://127.0.0.1:3211/fixed-page.html" }),
      ),
    "contract",
    "contract.invalid_relation",
    "fixture_url_mismatch",
    "/final_url",
  );
  assert.throws(() => validateLocalSanitizedObservation(rawWith({ uploadable: true })));
});

test("LocalSanitizedObservation retains strict duplicate-key admission", () => {
  assertRejection(
    () => validateLocalSanitizedObservation(Buffer.from(DUPLICATE_RUN_ID_JSON)),
    "canonicalization",
    "canonicalization.duplicate_member",
    "duplicate_member",
    "/",
  );
});

test("LocalSanitizedObservation rejects malformed identifiers, attempts, and literals", () => {
  for (const replacement of [
    { run_id: "-run" },
    { run_id: "a".repeat(65) },
    { attempt: 1.5 },
    { attempt: 0 },
    { attempt: 9_007_199_254_740_992 },
    { schema_version: "qa.local-evidence/v2" },
    { selector: '[data-local-qa="other"]' },
    { expected_text: "WAIT" },
    { observed_text: "WAIT" },
  ]) {
    assert.throws(() => validateLocalSanitizedObservation(rawWith(replacement)));
  }
});

test("LocalSanitizedObservation rejects every prohibited URL form", () => {
  for (const url of prohibitedUrls) {
    assert.throws(() => validateLocalSanitizedObservation(rawWith({ fixture_url: url })));
    assert.throws(() => validateLocalSanitizedObservation(rawWith({ final_url: url })));
  }
});

function rawWith(replacement: Record<string, unknown>): Uint8Array {
  return Buffer.from(JSON.stringify({ ...acceptedObservation, ...replacement }));
}

function assertRejection(
  operation: () => unknown,
  category: "canonicalization" | "contract" | "validation",
  code: string,
  reason: string,
  path: string,
): void {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof ContractError);
    assert.deepEqual(error.rejection, {
      category,
      code,
      reason,
      path,
    });
    return true;
  });
}
