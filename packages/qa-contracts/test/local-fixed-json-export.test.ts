import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  admitJson, canonicalAdmittedBytes, canonicalBytes, ContractError, contractContentDigest,
  contractRegistry, localFixedJsonPolicy, sha256Digest,
  validateLocalFixedJsonExport, validateLocalFixedJsonReceipt, validateLocalFixedJsonSource,
  LOCAL_FIXED_JSON_MAX_BYTES, LOCAL_FIXED_JSON_POLICY_DIGEST, LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES,
} from "../src/index.js";

type SourceCase = {
  id: string; raw_utf8: string; accepted: boolean; canonical_utf8?: string;
  source_digest?: string; output_digest?: string;
};
const fixture = JSON.parse(readFileSync(new URL(
  "../../fixtures/qa.local-fixed-json-export/v1/conformance.json", import.meta.url,
), "utf8")) as {
  canary: string; source_cases: SourceCase[]; receipt: Record<string, unknown>;
  rejected_bindings: { id: string; patch: Record<string, unknown> }[];
  duplicate_receipt_utf8: string; max_source_bytes: number; max_receipt_bytes: number;
  png_hex: string;
};
const source = Buffer.from(fixture.source_cases[0]!.raw_utf8);
const output = Buffer.from(fixture.source_cases[0]!.canonical_utf8!);
const receipt = Buffer.from(JSON.stringify(fixture.receipt));

function rejectsSafely(operation: () => unknown): void {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof ContractError);
    assert.equal(error.rejection.reason, "invalid_local_fixed_json_export");
    assert.equal(error.rejection.path, "/");
    assert.ok(!`${error} ${JSON.stringify(error)}`.includes(fixture.canary));
    return true;
  });
}

function rehash(value: Record<string, unknown>): Uint8Array {
  const projection = { ...value };
  delete projection.content_digest;
  projection.content_digest = sha256Digest(canonicalAdmittedBytes(admitJson(Buffer.from(JSON.stringify(projection)))));
  return Buffer.from(JSON.stringify(projection));
}

test("shared fixed JSON sources enforce strict validation and private errors", () => {
  for (const entry of fixture.source_cases) {
    const raw = Buffer.from(entry.raw_utf8);
    if (entry.accepted) {
      const validated = validateLocalFixedJsonSource(raw);
      const canonical = canonicalBytes(validated);
      assert.equal(Buffer.from(canonical).toString(), entry.canonical_utf8, entry.id);
      assert.equal(sha256Digest(raw), entry.source_digest, entry.id);
      assert.equal(sha256Digest(canonical), entry.output_digest, entry.id);
      validateLocalFixedJsonSource(canonical);
    } else {
      rejectsSafely(() => validateLocalFixedJsonSource(raw));
    }
  }
  rejectsSafely(() => validateLocalFixedJsonSource(Buffer.from(fixture.png_hex, "hex")));
});

test("shared receipts bind source/output digests, policy, size and identity", () => {
  const validated = validateLocalFixedJsonExport(receipt, source, output);
  assert.deepEqual(validated.value(), fixture.receipt);
  assert.notEqual(fixture.receipt.source_digest, fixture.receipt.output_digest);
  assert.ok(Object.isFrozen(validated.value()));
  for (const entry of fixture.rejected_bindings) {
    rejectsSafely(() => validateLocalFixedJsonExport(rehash({ ...fixture.receipt, ...entry.patch }), source, output));
  }
  rejectsSafely(() => validateLocalFixedJsonReceipt(Buffer.from(fixture.duplicate_receipt_utf8)));
  rejectsSafely(() => validateLocalFixedJsonExport(receipt, output, output));
  rejectsSafely(() => validateLocalFixedJsonExport(receipt, source, source));
});

test("fixed JSON limits apply before parsing at exact byte boundaries", () => {
  assert.equal(fixture.max_source_bytes, LOCAL_FIXED_JSON_MAX_BYTES);
  assert.equal(fixture.max_receipt_bytes, LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES);
  const exact = Buffer.from(`{${" ".repeat(LOCAL_FIXED_JSON_MAX_BYTES - source.length)}${source.toString().slice(1)}`);
  assert.equal(exact.length, LOCAL_FIXED_JSON_MAX_BYTES);
  validateLocalFixedJsonSource(exact);
  rejectsSafely(() => validateLocalFixedJsonSource(Buffer.from(exact.toString().replace("{", "{ "))));
  const exactReceipt = Buffer.from(`{${" ".repeat(LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES - receipt.length)}${receipt.toString().slice(1)}`);
  validateLocalFixedJsonReceipt(exactReceipt);
  rejectsSafely(() => validateLocalFixedJsonReceipt(Buffer.from(exactReceipt.toString().replace("{", "{ "))));
  rejectsSafely(() => validateLocalFixedJsonSource(Buffer.alloc(LOCAL_FIXED_JSON_MAX_BYTES + 1, 255)));
  rejectsSafely(() => validateLocalFixedJsonExport(receipt, source, Buffer.alloc(LOCAL_FIXED_JSON_MAX_BYTES + 1)));
});

test("registry exposes an immutable local built-in policy, not caller authority", () => {
  const registry = contractRegistry();
  for (const type of ["LocalFixedJsonPolicy", "LocalFixedJsonReceipt"]) {
    assert.equal(registry.types[type]?.schema, "qa.local-fixed-json-export/v1");
  }
  const policy = localFixedJsonPolicy();
  assert.equal(contractContentDigest(policy), LOCAL_FIXED_JSON_POLICY_DIGEST);
  const value = policy.value() as Record<string, unknown>;
  assert.equal(value.profile, "local-host-built-in-fixed-observation");
  assert.equal(value.png, "deny");
  assert.equal(value.raw_logs, "deny");
  assert.ok(Object.isFrozen(value));
});
