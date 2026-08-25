import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  buildInitialRunAcceptanceV2,
  canonicalBytes,
  contractContentDigest,
  validateLocalQARunRequestV2,
  validateRunAcceptanceV2,
} from "../src/index.js";

const fixture = JSON.parse(readFileSync(new URL("../../fixtures/qa.local-run-admission/v2/happy-path.json", import.meta.url), "utf8"));
const encoder = new TextEncoder();
const decoder = new TextDecoder();

test("validates the v2 admission walking skeleton vectors", () => {
  const request = validateLocalQARunRequestV2(encoder.encode(`${fixture.expected_request_utf8}\n`));
  assert.equal(contractContentDigest(request), fixture.request.content_digest);
  assert.equal(decoder.decode(canonicalBytes(request)), fixture.expected_request_utf8);

  const acceptance = buildInitialRunAcceptanceV2(request, fixture.accepted_at, "fkst-local-qa-host/0.1.0");
  assert.equal(decoder.decode(canonicalBytes(acceptance)), fixture.expected_acceptance_utf8);
  assert.equal(contractContentDigest(acceptance), "sha256:c590e3ffd6ca7d36e1a62e4ebb8f5799f7f879d0abff82422497c1bcba0f399d");
  validateRunAcceptanceV2(encoder.encode(fixture.expected_acceptance_utf8));
});
