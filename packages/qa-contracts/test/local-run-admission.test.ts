import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  ContractError,
  buildInitialRunAcceptance,
  canonicalAdmittedBytes,
  canonicalBytes,
  contractContentDigest,
  contractContentProjection,
  contractRegistry,
  admitJson,
  sha256Digest,
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

interface ExpectedRejection {
  readonly category: "canonicalization" | "contract" | "validation";
  readonly code?: string;
  readonly reason: string;
  readonly path: string;
}

interface ConformanceFixture {
  readonly schema_version: string;
  readonly missing_request_members: readonly string[];
  readonly request_cases: readonly {
    readonly name: string;
    readonly path: string;
    readonly value: unknown;
    readonly refresh_digest?: boolean;
    readonly expected: ExpectedRejection;
  }[];
  readonly nested_unknown_members: readonly string[];
  readonly identity_kinds: Readonly<Record<string, string>>;
  readonly request_digest_bound_paths: readonly string[];
  readonly builder_cases: readonly { readonly name: string; readonly accepted_at: string; readonly accepted: boolean }[];
  readonly missing_acceptance_members: readonly string[];
  readonly acceptance_digest_bound_paths: readonly string[];
  readonly raw_cases: readonly {
    readonly name: string;
    readonly target: "request" | "acceptance";
    readonly prefix?: string;
    readonly suffix?: string;
    readonly hex_prefix?: string;
    readonly replace?: readonly [string, string];
    readonly wrap_depth?: number;
    readonly expected: ExpectedRejection;
  }[];
}

const fixture = JSON.parse(
  readFileSync(
    new URL("../../fixtures/qa.local-run-admission/v1/happy-path.json", import.meta.url),
    "utf8",
  ),
) as Fixture;
const conformance = JSON.parse(
  readFileSync(
    new URL("../../fixtures/qa.local-run-admission/v1/conformance.json", import.meta.url),
    "utf8",
  ),
) as ConformanceFixture;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

function raw(value: unknown): Uint8Array {
  return encoder.encode(JSON.stringify(value));
}

function mutatedRequest(field: string, value: unknown): Record<string, unknown> {
  return { ...structuredClone(fixture.request), [field]: value };
}

function setPointer(value: Record<string, unknown>, pointer: string, replacement: unknown): void {
  const tokens = pointer.slice(1).split("/");
  let current: Record<string, unknown> = value;
  for (const token of tokens.slice(0, -1)) current = current[token] as Record<string, unknown>;
  current[tokens.at(-1)!] = replacement;
}

function removePointer(value: Record<string, unknown>, pointer: string): void {
  delete value[pointer.slice(1)];
}

function refreshDigest(value: Record<string, unknown>): void {
  const projected = structuredClone(value);
  delete projected.content_digest;
  delete projected.signature;
  value.content_digest = sha256Digest(canonicalAdmittedBytes(admitJson(raw(projected))));
}

function assertRejection(action: () => unknown, expected: ExpectedRejection): void {
  assert.throws(action, (error: unknown) => {
    assert.ok(error instanceof ContractError);
    assert.deepEqual(error.rejection, expected);
    return true;
  });
}

function digestMutation(path: string): unknown {
  if (path.endsWith("/content_digest")) return `sha256:${"8".repeat(64)}`;
  if (path.endsWith("/schema_version")) return "qa.changed/v2";
  if (path === "/expires_at") return "2026-08-14T04:06:00Z";
  if (path === "/nonce") return "bm9uY2UtMDAwMDAy";
  if (path === "/idempotency_key") return "idem_0002";
  return "changed-2";
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

test("applies the shared request rejection corpus", () => {
  assert.equal(conformance.schema_version, "qa.local-run-admission-conformance/v1");
  for (const member of conformance.missing_request_members) {
    const request = structuredClone(fixture.request);
    removePointer(request, `/${member}`);
    assertRejection(() => validateLocalQARunRequest(raw(request)), {
      category: "validation", reason: "schema_violation", path: "/",
    });
  }
  for (const entry of conformance.request_cases) {
    const request = structuredClone(fixture.request);
    setPointer(request, entry.path, entry.value);
    if (entry.refresh_digest === true) refreshDigest(request);
    assertRejection(() => validateLocalQARunRequest(raw(request)), entry.expected);
  }
  for (const member of conformance.nested_unknown_members) {
    const request = structuredClone(fixture.request);
    setPointer(request, `/source/${member}`, member === "bytes" ? [1] : "secret");
    assertRejection(() => validateLocalQARunRequest(raw(request)), {
      category: "validation", reason: "schema_violation", path: "/source",
    });
  }
  for (const identity of Object.keys(conformance.identity_kinds)) {
    const request = structuredClone(fixture.request);
    setPointer(request, `/${identity}/kind`, "wrong-kind");
    assertRejection(() => validateLocalQARunRequest(raw(request)), {
      category: "validation", reason: "schema_violation", path: `/${identity}/kind`,
    });
  }
});

test("binds every request projection class and preserves nested digests", () => {
  for (const path of conformance.request_digest_bound_paths) {
    const request = structuredClone(fixture.request);
    setPointer(request, path, digestMutation(path));
    assertRejection(() => validateLocalQARunRequest(raw(request)), {
      category: "contract", code: "contract.digest_mismatch", reason: "digest_mismatch", path: "/content_digest",
    });
  }
});

test("applies builder boundaries with no partial acceptance", () => {
  const request = validateLocalQARunRequest(raw(fixture.request));
  for (const entry of conformance.builder_cases) {
    let acceptance: ValidatedValue | undefined;
    if (entry.accepted) {
      acceptance = buildInitialRunAcceptance(request, entry.accepted_at, fixture.builder_inputs.producer_version);
      assert.equal((acceptance.value() as { accepted_at: string }).accepted_at, entry.accepted_at);
    } else {
      assertRejection(() => {
        acceptance = buildInitialRunAcceptance(request, entry.accepted_at, fixture.builder_inputs.producer_version);
      }, { category: "contract", code: "contract.invalid_relation", reason: "accepted_at_out_of_window", path: "/accepted_at" });
      assert.equal(acceptance, undefined);
    }
  }
});

test("rejects acceptance mutations and created_at mismatch", () => {
  const acceptance = JSON.parse(fixture.expected_acceptance_utf8) as Record<string, unknown>;
  for (const member of conformance.missing_acceptance_members) {
    const changed = structuredClone(acceptance);
    removePointer(changed, `/${member}`);
    assertRejection(() => validateRunAcceptance(raw(changed)), { category: "validation", reason: "schema_violation", path: "/" });
  }
  const unknown = { ...acceptance, unknown: true };
  assertRejection(() => validateRunAcceptance(raw(unknown)), { category: "validation", reason: "schema_violation", path: "/" });
  for (const path of conformance.acceptance_digest_bound_paths) {
    const changed = structuredClone(acceptance);
    if (path === "/state") setPointer(changed, path, "running");
    else if (path.endsWith("_at")) setPointer(changed, path, "2026-08-14T04:00:02Z");
    else if (path.includes("digest")) setPointer(changed, path, `sha256:${"8".repeat(64)}`);
    else setPointer(changed, path, "changed-2");
    assert.throws(() => validateRunAcceptance(raw(changed)), ContractError);
  }
  const mismatch = structuredClone(acceptance);
  mismatch.created_at = "2026-08-14T04:00:02Z";
  refreshDigest(mismatch);
  assertRejection(() => validateRunAcceptance(raw(mismatch)), {
    category: "contract", code: "contract.invalid_relation", reason: "accepted_at_mismatch", path: "/created_at",
  });
});

test("applies strict raw admission and canonicalizes harmless formatting", () => {
  for (const entry of conformance.raw_cases) {
    let text = entry.target === "request" ? fixture.expected_request_utf8 : fixture.expected_acceptance_utf8;
    if (entry.replace !== undefined) text = text.replace(entry.replace[0], entry.replace[1]);
    if (entry.wrap_depth !== undefined) text = `${"[".repeat(entry.wrap_depth)}${text}${"]".repeat(entry.wrap_depth)}`;
    const bytes = new Uint8Array([
      ...Buffer.from(entry.hex_prefix ?? "", "hex"),
      ...encoder.encode(`${entry.prefix ?? ""}${text}${entry.suffix ?? ""}`),
    ]);
    assertRejection(() => entry.target === "request" ? validateLocalQARunRequest(bytes) : validateRunAcceptance(bytes), entry.expected);
  }
  const spaced = fixture.expected_request_utf8.replace(/,"/g, ', "').replace(/":/g, '": ');
  assert.equal(decoder.decode(canonicalBytes(validateLocalQARunRequest(encoder.encode(spaced)))), fixture.expected_request_utf8);
});
