import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  contractRegistry,
  validateExecutorControlReport,
  validateExecutorControlRequest,
  validateLocalWorkerControlFrame,
} from "../src/index.js";

const encoder = new TextEncoder();
const executorPositive = fixture("qa.local-executor-control/v1/positive.json");
const executorNegative = fixture("qa.local-executor-control/v1/negative.json");
const workerPositive = fixture("qa.local-worker-control/v1/positive.json");
const workerNegative = fixture("qa.local-worker-control/v1/negative.json");
const cancellation = fixture("qa.local-cancellation/v1/conformance.json");

test("keeps control registry metadata and fixture indexes closed", () => {
  const registry = contractRegistry();
  assert.deepEqual(registry.schemas["qa.local-worker-control/v1"], {
    path: "contracts/qa.local-worker-control/v1/schema.json",
    id: "urn:chronoai:fkst:qa-contracts:qa.local-worker-control:v1",
    major: 1,
  });
  assert.deepEqual(registry.schemas["qa.local-executor-control/v1"], {
    path: "contracts/qa.local-executor-control/v1/schema.json",
    id: "urn:chronoai:fkst:qa-contracts:qa.local-executor-control:v1",
    major: 1,
  });
  assert.deepEqual(Object.keys(executorPositive).sort(), ["reports", "request"]);
  assert.deepEqual(Object.keys(executorNegative).sort(), ["relation_cases", "schema_cases"]);
  assert.deepEqual(Object.keys(workerPositive).sort(), ["enums", "frames"]);
  assert.deepEqual(Object.keys(workerNegative).sort(), ["relation_cases", "schema_cases"]);
  assert.deepEqual(Object.keys(cancellation).sort(), [
    "cleanup_outcome",
    "cleanup_receipt",
    "control_status",
    "effect_disposition",
    "independent_outcome",
    "sanitized_residual",
  ]);
  assert.deepEqual(record(cancellation.control_status).positive, [
    "accepted",
    "too_late",
    "rejected",
    "failed",
  ]);
  assert.deepEqual(record(cancellation.effect_disposition).positive, [
    "not_started",
    "completed",
    "uncertain",
  ]);
  assert.deepEqual(record(cancellation.independent_outcome).positive, [
    "not_started",
    "succeeded",
    "failed",
    "cancelled",
    "lost_or_inconclusive",
  ]);
  assert.deepEqual(record(cancellation.cleanup_outcome).positive, [
    "not_required",
    "completed",
    "blocked",
  ]);
  assert.equal(array(executorNegative.schema_cases).length, 16);
  assert.equal(array(executorNegative.relation_cases).length, 7);
  assert.equal(array(workerNegative.schema_cases).length, 15);
  assert.equal(array(workerNegative.relation_cases).length, 3);
});

test("validates every control frame report and enum value", () => {
  validateExecutorControlRequest(raw(executorPositive.request));
  for (const report of Object.values(record(executorPositive.reports))) {
    validateExecutorControlReport(raw(report));
  }

  assert.deepEqual(Object.keys(record(executorPositive.reports)).sort(), ["cleanup_receipt", "residual"]);
  assert.deepEqual(
    record(record(executorPositive.reports).cleanup_receipt).cleanup_receipt,
    record(cancellation.cleanup_receipt).positive,
  );
  assert.deepEqual(
    record(record(executorPositive.reports).residual).residual,
    record(cancellation.sanitized_residual).positive,
  );
  const baseline = record(record(executorPositive.reports).cleanup_receipt);
  for (const [field, vocabulary] of [
    ["status", record(cancellation.control_status).positive],
    ["effect_disposition", record(cancellation.effect_disposition).positive],
    ["execution_outcome", record(cancellation.independent_outcome).positive],
    ["evidence_outcome", record(cancellation.independent_outcome).positive],
    ["upload_outcome", record(cancellation.independent_outcome).positive],
    ["cleanup_outcome", record(cancellation.cleanup_outcome).positive],
  ] as const) {
    for (const value of array(vocabulary)) {
      validateExecutorControlReport(raw({ ...baseline, [field]: value }));
    }
  }

  assert.deepEqual(Object.keys(record(workerPositive.frames)).sort(), [
    "abort",
    "cancel_ack.accepted",
    "cancel_ack.too_late",
    "control_failure.control.conflict",
    "control_failure.control.deadline_elapsed",
    "control_failure.control.invalid_frame",
    "control_failure.control.invalid_invocation",
  ]);
  for (const frame of Object.values(record(workerPositive.frames))) {
    validateLocalWorkerControlFrame(raw(frame));
  }
});

test("rejects the shared closed-schema matrix", () => {
  for (const fixtureCase of array(executorNegative.schema_cases).map(record)) {
    const target = fixtureCase.target === "request"
      ? executorPositive.request
      : record(executorPositive.reports).cleanup_receipt;
    const rejected = fixtureCase.target === "request"
      ? () => validateExecutorControlRequest(raw(mutate(target, fixtureCase)))
      : () => validateExecutorControlReport(raw(mutate(target, fixtureCase)));
    assert.throws(rejected, String(fixtureCase.case_id));
  }

  const report = record(record(executorPositive.reports).cleanup_receipt);
  for (const [field, vocabulary] of [
    ["status", record(cancellation.control_status).negative],
    ["effect_disposition", record(cancellation.effect_disposition).negative],
    ["execution_outcome", record(cancellation.independent_outcome).negative],
    ["evidence_outcome", record(cancellation.independent_outcome).negative],
    ["upload_outcome", record(cancellation.independent_outcome).negative],
    ["cleanup_outcome", record(cancellation.cleanup_outcome).negative],
  ] as const) {
    for (const value of array(vocabulary)) {
      assert.throws(() => validateExecutorControlReport(raw({ ...report, [field]: value })));
    }
  }
  for (const fixtureCase of array(record(cancellation.cleanup_receipt).negative).map(record)) {
    assert.throws(() => validateExecutorControlReport(raw({
      ...report,
      cleanup_receipt: mutate(record(cancellation.cleanup_receipt).positive, fixtureCase),
    })));
  }
  const residualReport = record(record(executorPositive.reports).residual);
  for (const fixtureCase of array(record(cancellation.sanitized_residual).negative).map(record)) {
    assert.throws(() => validateExecutorControlReport(raw({
      ...residualReport,
      residual: mutate(record(cancellation.sanitized_residual).positive, fixtureCase),
    })));
  }

  for (const fixtureCase of array(workerNegative.schema_cases).map(record)) {
    const frame = record(workerPositive.frames)[String(fixtureCase.frame)];
    assert.throws(
      () => validateLocalWorkerControlFrame(raw(mutate(frame, fixtureCase))),
      String(fixtureCase.case_id),
    );
  }
});

test("applies the shared identity-relation matrix", () => {
  for (const fixtureCase of array(executorNegative.relation_cases).map(record)) {
    const report = structuredClone(record(record(executorPositive.reports).cleanup_receipt));
    if (typeof fixtureCase.field === "string") report[fixtureCase.field] = fixtureCase.value;
    assert.equal(
      executorRelationValid(record(executorPositive.request), report),
      fixtureCase.valid,
      String(fixtureCase.case_id),
    );
  }

  for (const fixtureCase of array(workerNegative.relation_cases).map(record)) {
    const acknowledgement = structuredClone(record(record(workerPositive.frames)["cancel_ack.accepted"]));
    if (typeof fixtureCase.field === "string") acknowledgement[fixtureCase.field] = fixtureCase.value;
    assert.equal(
      workerRelationValid(record(record(workerPositive.frames).abort), acknowledgement),
      fixtureCase.valid,
      String(fixtureCase.case_id),
    );
  }
});

function fixture(relativePath: string): Record<string, unknown> {
  return record(JSON.parse(readFileSync(new URL(`../fixtures/${relativePath}`, import.meta.url), "utf8")));
}

function raw(value: unknown): Uint8Array {
  return encoder.encode(JSON.stringify(value));
}

function record(value: unknown): Record<string, unknown> {
  assert.equal(typeof value, "object");
  assert.notEqual(value, null);
  assert.equal(Array.isArray(value), false);
  return value as Record<string, unknown>;
}

function array(value: unknown): unknown[] {
  assert.equal(Array.isArray(value), true);
  return value as unknown[];
}

function mutate(baseline: unknown, fixtureCase: Record<string, unknown>): Record<string, unknown> {
  const value = structuredClone(record(baseline));
  const field = String(array(fixtureCase.path)[0]);
  if (fixtureCase.operation === "remove") delete value[field];
  else value[field] = fixtureCase.value;
  return value;
}

function executorRelationValid(request: Record<string, unknown>, report: Record<string, unknown>): boolean {
  const selection = record(request.selection);
  return request.control_id === report.control_id
    && request.run_id === report.run_id
    && request.executor_run_id === report.executor_run_id
    && selection.executor_id === report.executor_id
    && selection.executor_version === report.executor_version
    && selection.capability_digest === report.capability_digest;
}

function workerRelationValid(abort: Record<string, unknown>, acknowledgement: Record<string, unknown>): boolean {
  return abort.control_id === acknowledgement.control_id
    && abort.invocation_id === acknowledgement.invocation_id;
}
