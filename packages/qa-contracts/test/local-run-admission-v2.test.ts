import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  buildInitialRunAcceptanceV2,
  canonicalBytes,
  contractContentDigest,
  ContractError,
  validateLocalQARunRequestV2,
  validateRunAcceptanceV2,
} from "../src/index.js";

const fixture = JSON.parse(readFileSync(new URL("../../fixtures/qa.local-run-admission/v2/happy-path.json", import.meta.url), "utf8"));
const encoder = new TextEncoder();
const decoder = new TextDecoder();

test("validates the v2 admission walking skeleton vectors", () => {
  assert.equal(fixture.request.attempt_binding.fence_token, "dGVzdC1mZW5jZS0wMDAwMDAwMg");
  assert.equal(Buffer.from(fixture.request.attempt_binding.fence_token, "base64url").toString("utf8"), "test-fence-00000002");
  const request = validateLocalQARunRequestV2(encoder.encode(fixture.expected_request_utf8));
  assert.equal(contractContentDigest(request), fixture.request.content_digest);
  assert.equal(decoder.decode(canonicalBytes(request)), fixture.expected_request_utf8);

  const acceptance = buildInitialRunAcceptanceV2(request, fixture.accepted_at, "fkst-local-qa-host/0.1.0");
  assert.equal(decoder.decode(canonicalBytes(acceptance)), fixture.expected_acceptance_utf8);
  assert.equal(contractContentDigest(acceptance), "sha256:193ec4c65b3c5a16334c2ae2688c827e6246170a51e5f1987305fe28ce7b5ef5");
  validateRunAcceptanceV2(encoder.encode(fixture.expected_acceptance_utf8));
});

function assertRejection(raw: string, category: string, code: string | undefined, path: string): void {
  assert.throws(
    () => validateLocalQARunRequestV2(encoder.encode(raw)),
    (error: unknown) =>
      error instanceof ContractError &&
      error.rejection.category === category &&
      error.rejection.code === code &&
      error.rejection.path === path,
  );
}

test("rejects non-canonical v2 encoded identities", () => {
  assertRejection(
    fixture.expected_request_utf8.replace("dGVzdC1mZW5jZS0wMDAwMDAwMg", "test-fence-00000002"),
    "contract",
    "contract.invalid_encoding",
    "/attempt_binding/fence_token",
  );
  assertRejection(
    fixture.expected_request_utf8.replace("bm9uY2UtMDAwMDAwMDAy", "test-fence-00000002"),
    "contract",
    "contract.invalid_encoding",
    "/nonce",
  );
});

test("enforces the v2 producer version UTF-8 byte limit", () => {
  assertRejection(
    fixture.expected_request_utf8.replace("fkst-local-qa-host/0.1.0", "é".repeat(65)),
    "validation",
    undefined,
    "/producer_version",
  );
});

test("enforces the v2 acceptance producer version UTF-8 byte limit", () => {
  assert.throws(
    () =>
      validateRunAcceptanceV2(
        encoder.encode(
          fixture.expected_acceptance_utf8.replace("fkst-local-qa-host/0.1.0", "é".repeat(65)),
        ),
      ),
    (error: unknown) =>
      error instanceof ContractError &&
      error.rejection.category === "validation" &&
      error.rejection.code === undefined &&
      error.rejection.path === "/producer_version",
  );
});
