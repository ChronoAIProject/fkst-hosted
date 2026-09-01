import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  contractRegistry,
  validateExecutorControlReport,
  validateExecutorControlRequest,
  validateLocalWorkerControlFrame,
} from "../src/index.js";

const fixture = JSON.parse(
  readFileSync(
    new URL("../fixtures/qa.local-executor-control/v1/positive.json", import.meta.url),
    "utf8",
  ),
) as { request: unknown; report: Record<string, unknown> };
const encoder = new TextEncoder();
const workerFixture = JSON.parse(
  readFileSync(
    new URL("../fixtures/qa.local-worker-control/v1/positive.json", import.meta.url),
    "utf8",
  ),
) as Record<string, unknown>;

test("registers and validates the executor control fixture", () => {
  assert.equal(contractRegistry().schemas["qa.local-executor-control/v1"]?.major, 1);
  validateExecutorControlRequest(encoder.encode(JSON.stringify(fixture.request)));
  validateExecutorControlReport(encoder.encode(JSON.stringify(fixture.report)));
});

test("registers and validates every worker control frame", () => {
  assert.equal(contractRegistry().schemas["qa.local-worker-control/v1"]?.major, 1);
  for (const frame of Object.values(workerFixture)) {
    validateLocalWorkerControlFrame(encoder.encode(JSON.stringify(frame)));
  }
});

test("requires a cleanup receipt or sanitized residual", () => {
  const report = structuredClone(fixture.report);
  delete report.cleanup_receipt;
  assert.throws(() => validateExecutorControlReport(encoder.encode(JSON.stringify(report))));
});
