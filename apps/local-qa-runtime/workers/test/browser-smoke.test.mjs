import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserSmokeWorkerError,
  parseBrowserSmokeRequest,
  runBrowserSmoke,
  serializeBrowserSmokeResult,
} from "../dist/index.js";

const requestValue = {
  version: "local-qa-browser-smoke/request-v1",
  fixtureUrl: "http://127.0.0.1:43123/fixed-page.html",
  selector: '[data-local-qa="status"]',
  expectedText: "READY",
  timeoutMs: 5000,
};
const requestJson = JSON.stringify(requestValue);
const sanitizedObservationRef = {
  kind: "local-sanitized-observation",
  id: "observation/0",
  schema_version: "qa.local-evidence/v1",
  content_digest: `sha256:${"a".repeat(64)}`,
};
const screenshotEvidenceRef = {
  kind: "local-evidence-object",
  id: "evidence/0",
  schema_version: "qa.local-evidence/v1",
  content_digest: "sha256:4c4b6a3be1314ab86138bef4314dde022e600960d8689a2c8f8631802d20dab6",
};
const runnerLogEvidenceRef = {
  kind: "local-evidence-object",
  id: "evidence/1",
  schema_version: "qa.local-evidence/v1",
  content_digest: "sha256:bb9c62cc84fc533e52193a8961778b0be251cd8f19a89b3fa836e94043a0075e",
};
const expectedSerialized = '{"version":"local-qa-browser-smoke/result-v1","outcome":"passed","observation":{"fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","finalUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","observedText":"READY","sanitizedObservationRef":{"kind":"local-sanitized-observation","id":"observation/0","schema_version":"qa.local-evidence/v1","content_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},"startedAt":"2026-01-02T03:04:05.000Z","finishedAt":"2026-01-02T03:04:05.012Z","durationMs":12,"evidence":[{"objectId":"evidence/0","role":"screenshot","artifactRef":{"kind":"local-evidence-object","id":"evidence/0","schema_version":"qa.local-evidence/v1","content_digest":"sha256:4c4b6a3be1314ab86138bef4314dde022e600960d8689a2c8f8631802d20dab6"}},{"objectId":"evidence/1","role":"runner-log","artifactRef":{"kind":"local-evidence-object","id":"evidence/1","schema_version":"qa.local-evidence/v1","content_digest":"sha256:bb9c62cc84fc533e52193a8961778b0be251cd8f19a89b3fa836e94043a0075e"}}]}';

test("walks the fixed request through pure policy in exact order", async () => {
  const harness = createHarness();
  const bundle = await runBrowserSmoke(requestJson, harness.ports);

  assert.deepEqual(harness.events, [
    "clock.now",
    "clock.monotonicMs",
    "session.run",
    "evidence.stageGeneratedLog",
    "session.close",
    "clock.now",
    "clock.monotonicMs",
  ]);
  assert.equal(harness.calls.run, 1);
  assert.equal(harness.calls.stage, 1);
  assert.equal(harness.calls.close, 1);
  assert.deepEqual(harness.requests, [requestValue]);
  assert.equal(harness.staged.length, 1);
  assert.equal(harness.staged[0].name, "runner.log");
  assert.equal(harness.staged[0].mediaType, "text/plain; charset=utf-8");
  assert.deepEqual(
    [...harness.staged[0].bytes],
    [...new TextEncoder().encode("navigation accepted\nassertion passed\n")],
  );
  assert.equal(harness.staged[0].bytes.byteLength, 37);
  assert.equal("objects" in bundle, false);
  assert.deepEqual(bundle.result.observation.sanitizedObservationRef, sanitizedObservationRef);
  assert.deepEqual(bundle.result.evidence, [
    { objectId: "evidence/0", role: "screenshot", artifactRef: screenshotEvidenceRef },
    { objectId: "evidence/1", role: "runner-log", artifactRef: runnerLogEvidenceRef },
  ]);
  assert.equal(new TextDecoder().decode(serializeBrowserSmokeResult(bundle.result)), expectedSerialized);
});

test("accepts JSON whitespace, member reordering, and both port boundaries", () => {
  for (const port of [1, 65535]) {
    const source = ` \n\t { "timeoutMs" : 5000, "expectedText" : "READY", "selector" : "[data-local-qa=\\"status\\"]", "fixtureUrl" : "http://127.0.0.1:${port}/fixed-page.html", "version" : "local-qa-browser-smoke/request-v1" } \r `;
    assert.deepEqual(parseBrowserSmokeRequest(source), {
      ...requestValue,
      fixtureUrl: `http://127.0.0.1:${port}/fixed-page.html`,
    });
  }
});

test("rejects each missing request field", () => {
  for (const field of Object.keys(requestValue)) {
    assertRequestError(JSON.stringify(omit(requestValue, field)), "request.missing_field");
  }
});

test("rejects each request field with every representative wrong type", () => {
  for (const field of Object.keys(requestValue)) {
    for (const value of [null, true, [], {}]) {
      assertRequestError(JSON.stringify({ ...requestValue, [field]: value }), "request.wrong_type");
    }
  }
  assertRequestError(JSON.stringify({ ...requestValue, timeoutMs: "5000" }), "request.wrong_type");
  for (const field of ["version", "fixtureUrl", "selector", "expectedText"]) {
    assertRequestError(JSON.stringify({ ...requestValue, [field]: 5000 }), "request.wrong_type");
  }
});

test("rejects changed fixed values and non-canonical timeout tokens", () => {
  const rejected = [
    requestJson.replace("request-v1", "request-v2"),
    requestJson.replace("status", "other"),
    requestJson.replace("READY", "ready"),
    requestJson.replace("5000}", "4999}"),
    requestJson.replace("5000}", "5000.0}"),
    requestJson.replace("5000}", "5e3}"),
  ];
  for (const source of rejected) {
    assertRequestError(source, "request.unsupported_value");
  }
});

test("rejects every fixture URL escape from the fixed loopback shape", () => {
  const rejectedUrls = [
    "http://user@127.0.0.1:43123/fixed-page.html",
    "https://127.0.0.1:43123/fixed-page.html",
    "http://localhost:43123/fixed-page.html",
    "http://[::1]:43123/fixed-page.html",
    "http://127.0.0.2:43123/fixed-page.html",
    "http://127.0.0.1/fixed-page.html",
    "http://127.0.0.1:0/fixed-page.html",
    "http://127.0.0.1:65536/fixed-page.html",
    "http://127.0.0.1:abc/fixed-page.html",
    "http://127.0.0.1:43123/",
    "http://127.0.0.1:43123/fixed-page.html/extra",
    "http://127.0.0.1:43123/%66ixed-page.html",
    "http://127.0.0.1:43123/fixed%2Dpage.html",
    "http://127.0.0.1:43123/fixed-page.html?query=1",
    "http://127.0.0.1:43123/fixed-page.html#fragment",
  ];
  for (const fixtureUrl of rejectedUrls) {
    assertRequestError(JSON.stringify({ ...requestValue, fixtureUrl }), "request.unsupported_value");
  }
});

test("rejects duplicate decoded keys in multiple positions", () => {
  const duplicates = [
    '{"version":"local-qa-browser-smoke/request-v1","version":"local-qa-browser-smoke/request-v1","fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","timeoutMs":5000}',
    '{"version":"local-qa-browser-smoke/request-v1","fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","timeoutMs":5000,"timeoutMs":5000}',
    '{"ver\\u0073ion":"local-qa-browser-smoke/request-v1","version":"local-qa-browser-smoke/request-v1","fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","timeoutMs":5000}',
  ];
  for (const source of duplicates) {
    assertRequestError(source, "request.duplicate_key");
  }
});

test("rejects malformed JSON, non-object roots, unknown fields, and trailing data", () => {
  for (const source of [
    "",
    "{",
    '{"version":"unterminated}',
    '{"version":"bad\nstring"}',
    requestJson.replace('"timeoutMs":5000', '"timeoutMs":01'),
    requestJson.replace('"timeoutMs":5000', '"timeoutMs":+5000'),
    requestJson.replace('"timeoutMs":5000', '"timeoutMs":5000,'),
    requestJson.replace('"selector":', '"selector" '),
  ]) {
    assertRequestError(source, "request.invalid_json");
  }
  for (const source of ["null", "true", "5000", '"object"', "[]"]) {
    assertRequestError(source, "request.root_not_object");
  }
  for (const field of ["evaluate", "javascript", "cdp", "command", "executable", "cwd", "env", "downloadPath", "outputPath", "implementation"]) {
    assertRequestError(JSON.stringify({ ...requestValue, [field]: "forbidden" }), "request.unknown_field");
  }
  for (const suffix of ["x", "{}", "\u0000"]) {
    assertRequestError(requestJson + suffix, "request.trailing_data");
  }
});

test("maps session rejection without exposing untrusted error text", async () => {
  const harness = createHarness({ runError: new Error("secret browser failure") });
  const error = await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "session.run_failed");
  assert.equal(error.message.includes("secret browser failure"), false);
  assert.deepEqual(harness.events, ["clock.now", "clock.monotonicMs", "session.run", "session.close"]);
});

test("rejects every invalid session response shape before staging", async () => {
  const valid = validSessionResult();
  const withSymbol = { ...valid, [Symbol("raw")]: true };
  const invalidResponses = [
    null,
    [],
    ...Object.keys(valid).map((field) => omit(valid, field)),
    { ...valid, screenshotBytes: new Uint8Array([1]) },
    { ...valid, browserHandle: {} },
    { ...valid, finalUrl: 1 },
    { ...valid, observedText: null },
    withSymbol,
  ];
  for (const sessionResult of invalidResponses) {
    const harness = createHarness({ sessionResult });
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "session.invalid_response");
    assert.equal(harness.calls.run, 1);
    assert.equal(harness.calls.stage, 0);
    assert.equal(harness.calls.close, 1);
    assert.equal(harness.calls.now, 1);
    assert.equal(harness.calls.monotonic, 1);
  }
});

test("rejects every final URL change and skips staging", async () => {
  const rejectedUrls = [
    "https://127.0.0.1:43123/fixed-page.html",
    "http://localhost:43123/fixed-page.html",
    "http://127.0.0.1:1/fixed-page.html",
    "http://127.0.0.1:43123/other.html",
    "http://127.0.0.1:43123/fixed-page.html?query=1",
    "http://127.0.0.1:43123/fixed-page.html#fragment",
    "http://user@127.0.0.1:43123/fixed-page.html",
  ];
  for (const finalUrl of rejectedUrls) {
    const harness = createHarness({ sessionResult: validSessionResult({ finalUrl }) });
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "policy.final_url_rejected");
    assert.deepEqual(harness.events.slice(-1), ["session.close"]);
    assert.equal(harness.calls.stage, 0);
    assert.equal(harness.calls.now, 1);
  }
});

test("rejects observed-text mismatch and non-string observations", async () => {
  for (const observedText of ["ready", " READY", "READY ", "READY\n"]) {
    const harness = createHarness({ sessionResult: validSessionResult({ observedText }) });
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "policy.assertion_failed");
    assert.equal(harness.calls.stage, 0);
    assert.equal(harness.calls.close, 1);
  }
  for (const observedText of [null, 5000, true, {}, []]) {
    const harness = createHarness({ sessionResult: validSessionResult({ observedText }) });
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "session.invalid_response");
    assert.equal(harness.calls.stage, 0);
    assert.equal(harness.calls.close, 1);
  }
});

test("rejects the complete canonical reference failure matrix", async () => {
  const matrices = [
    ["sanitizedObservationRef", sanitizedObservationRef, "local-evidence-object", "qa.local-evidence/v2"],
    ["screenshotEvidenceRef", screenshotEvidenceRef, "local-sanitized-observation", "qa.local-evidence/v2"],
  ];
  for (const [field, reference, wrongKind, wrongSchema] of matrices) {
    for (const invalidReference of invalidReferenceVariants(reference, wrongKind, wrongSchema)) {
      const harness = createHarness({
        sessionResult: validSessionResult({ [field]: invalidReference }),
      });
      await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "evidence.invalid_reference");
      assert.equal(harness.calls.stage, 0);
      assert.equal(harness.calls.close, 1);
    }
  }
});

test("accepts optional reference versions and forwards every value unchanged", async () => {
  const versionedObservation = { ...sanitizedObservationRef, version: "obs-v1" };
  const versionedScreenshot = { ...screenshotEvidenceRef, version: "screen-v1" };
  const versionedLog = { ...runnerLogEvidenceRef, version: "log-v1" };
  const harness = createHarness({
    sessionResult: validSessionResult({
      sanitizedObservationRef: versionedObservation,
      screenshotEvidenceRef: versionedScreenshot,
    }),
    stagingResult: versionedLog,
  });
  const bundle = await runBrowserSmoke(requestJson, harness.ports);

  assert.deepEqual(bundle.result.observation.sanitizedObservationRef, versionedObservation);
  assert.deepEqual(bundle.result.evidence[0].artifactRef, versionedScreenshot);
  assert.deepEqual(bundle.result.evidence[1].artifactRef, versionedLog);
  const serialized = new TextDecoder().decode(serializeBrowserSmokeResult(bundle.result));
  for (const reference of [versionedObservation, versionedScreenshot, versionedLog]) {
    const fragment = `"content_digest":${JSON.stringify(reference.content_digest)},"version":${JSON.stringify(reference.version)}`;
    assert.equal(serialized.includes(fragment), true);
  }
  assert.equal(serialized.endsWith("\n"), false);
});

test("maps staging rejection, validates staged references, and always finalizes", async () => {
  const rejected = createHarness({ stageError: new Error("private storage failure") });
  const stagingError = await captureWorkerError(
    () => runBrowserSmoke(requestJson, rejected.ports),
    "evidence.staging_failed",
  );
  assert.equal(stagingError.message.includes("private storage failure"), false);
  assert.equal(rejected.calls.stage, 1);
  assert.equal(rejected.calls.close, 1);

  for (const invalidReference of invalidReferenceVariants(
    runnerLogEvidenceRef,
    "local-sanitized-observation",
    "qa.local-evidence/v2",
  )) {
    const harness = createHarness({ stagingResult: invalidReference });
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "evidence.invalid_reference");
    assert.equal(harness.calls.stage, 1);
    assert.equal(harness.calls.close, 1);
  }
});

test("does not acquire session capabilities after parsing or initial clock failure", async () => {
  const invalidRequest = createHarness();
  await captureWorkerError(
    () => runBrowserSmoke(requestJson.replace("request-v1", "request-v2"), invalidRequest.ports),
    "request.unsupported_value",
  );
  assert.deepEqual(invalidRequest.events, []);

  const cases = [
    [{ nowErrorAt: 1 }, "clock.failed"],
    [{ nowValues: [5000] }, "clock.invalid_value"],
    [{ monotonicErrorAt: 1 }, "clock.failed"],
    [{ monotonicValues: [1.5] }, "clock.invalid_value"],
    [{ monotonicValues: [Number.MAX_SAFE_INTEGER + 1] }, "clock.invalid_value"],
  ];
  for (const [options, code] of cases) {
    const harness = createHarness(options);
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), code);
    assert.equal(harness.calls.run, 0);
    assert.equal(harness.calls.stage, 0);
    assert.equal(harness.calls.close, 0);
  }
});

test("rejects invalid calendar timestamps and wall-clock duration mismatches", async () => {
  const cases = [
    { nowValues: ["2026-02-30T03:04:05.000Z"] },
    { nowValues: ["2026-01-02T03:04:05Z"] },
    { nowValues: ["2026-01-02T03:04:05.000Z", "2026-01-02T03:04:05.011Z"] },
  ];
  for (const options of cases) {
    const harness = createHarness(options);
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), "clock.invalid_value");
  }
});

test("rejects failed or invalid finished clock reads after finalization", async () => {
  const cases = [
    [{ nowErrorAt: 2 }, "clock.failed"],
    [{ nowValues: ["2026-01-02T03:04:05.000Z", null] }, "clock.invalid_value"],
    [{ monotonicErrorAt: 2 }, "clock.failed"],
    [{ monotonicValues: [1000, 999] }, "clock.invalid_value"],
    [{ monotonicValues: [-Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER] }, "clock.invalid_value"],
    [{ monotonicValues: [1000, 1000.5] }, "clock.invalid_value"],
  ];
  for (const [options, code] of cases) {
    const harness = createHarness(options);
    await captureWorkerError(() => runBrowserSmoke(requestJson, harness.ports), code);
    assert.equal(harness.calls.run, 1);
    assert.equal(harness.calls.stage, 1);
    assert.equal(harness.calls.close, 1);
    assert.equal(harness.events.indexOf("session.close") < harness.events.lastIndexOf("clock.now") || harness.calls.now === 1, true);
  }
});

test("finalization failure overrides success and earlier post-run failures", async () => {
  const successfulPolicy = createHarness({ closeError: new Error("private close failure") });
  const successError = await captureWorkerError(
    () => runBrowserSmoke(requestJson, successfulPolicy.ports),
    "session.finalization_failed",
  );
  assert.equal(successError.message.includes("private close failure"), false);
  assert.equal(successfulPolicy.calls.stage, 1);
  assert.equal(successfulPolicy.calls.close, 1);
  assert.equal(successfulPolicy.calls.now, 1);

  const failedPolicy = createHarness({
    sessionResult: validSessionResult({ observedText: "NOT READY" }),
    closeError: new Error("private close failure"),
  });
  await captureWorkerError(
    () => runBrowserSmoke(requestJson, failedPolicy.ports),
    "session.finalization_failed",
  );
  assert.equal(failedPolicy.calls.stage, 0);
  assert.equal(failedPolicy.calls.close, 1);
  assert.equal(failedPolicy.calls.now, 1);
});

test("serialization is compact, ordered, escaped, and capability-free", async () => {
  const bundle = await runBrowserSmoke(requestJson, createHarness().ports);
  const bytes = serializeBrowserSmokeResult(bundle.result);
  const serialized = new TextDecoder().decode(bytes);

  assert.equal(serialized, expectedSerialized);
  assert.deepEqual([...bytes], [...new TextEncoder().encode(expectedSerialized)]);
  assert.deepEqual(Object.keys(JSON.parse(serialized)), [
    "version",
    "outcome",
    "observation",
    "startedAt",
    "finishedAt",
    "durationMs",
    "evidence",
  ]);
  assert.deepEqual(Object.keys(JSON.parse(serialized).observation), [
    "fixtureUrl",
    "finalUrl",
    "selector",
    "expectedText",
    "observedText",
    "sanitizedObservationRef",
  ]);
  for (const forbidden of [
    '"objects"',
    '"bytes"',
    '"byteLength"',
    '"ownership"',
    '"path"',
    '"storageToken"',
    '"quarantineId"',
    "file:",
    "/tmp/",
    "C:\\\\",
  ]) {
    assert.equal(serialized.includes(forbidden), false, forbidden);
  }
});

function createHarness(options = {}) {
  const events = [];
  const calls = { run: 0, stage: 0, close: 0, now: 0, monotonic: 0 };
  const requests = [];
  const staged = [];
  const nowValues = [...(options.nowValues ?? ["2026-01-02T03:04:05.000Z", "2026-01-02T03:04:05.012Z"])];
  const monotonicValues = [...(options.monotonicValues ?? [1000, 1012])];
  const sessionResult = Object.prototype.hasOwnProperty.call(options, "sessionResult")
    ? options.sessionResult
    : validSessionResult();
  const stagingResult = Object.prototype.hasOwnProperty.call(options, "stagingResult")
    ? options.stagingResult
    : runnerLogEvidenceRef;

  return {
    events,
    calls,
    requests,
    staged,
    ports: {
      session: {
        async run(request) {
          events.push("session.run");
          calls.run += 1;
          requests.push(request);
          if (options.runError !== undefined) {
            throw options.runError;
          }
          return sessionResult;
        },
        async close() {
          events.push("session.close");
          calls.close += 1;
          if (options.closeError !== undefined) {
            throw options.closeError;
          }
        },
      },
      evidence: {
        async stageGeneratedLog(input) {
          events.push("evidence.stageGeneratedLog");
          calls.stage += 1;
          staged.push(input);
          if (options.stageError !== undefined) {
            throw options.stageError;
          }
          return stagingResult;
        },
      },
      clock: {
        now() {
          events.push("clock.now");
          calls.now += 1;
          if (options.nowErrorAt === calls.now) {
            throw new Error("private wall clock failure");
          }
          return nowValues.shift();
        },
        monotonicMs() {
          events.push("clock.monotonicMs");
          calls.monotonic += 1;
          if (options.monotonicErrorAt === calls.monotonic) {
            throw new Error("private monotonic clock failure");
          }
          return monotonicValues.shift();
        },
      },
    },
  };
}

function validSessionResult(overrides = {}) {
  return {
    finalUrl: requestValue.fixtureUrl,
    observedText: "READY",
    sanitizedObservationRef,
    screenshotEvidenceRef,
    ...overrides,
  };
}

function invalidReferenceVariants(reference, wrongKind, wrongSchema) {
  const withSymbol = { ...reference, [Symbol("raw")]: true };
  return [
    null,
    [],
    ...["kind", "id", "schema_version", "content_digest"].map((field) => omit(reference, field)),
    { ...reference, rawPath: "/tmp/evidence" },
    withSymbol,
    { ...reference, kind: wrongKind },
    { ...reference, kind: 1 },
    { ...reference, id: "" },
    { ...reference, id: reference.id === "observation/0" ? "observation/1" : "evidence/9" },
    { ...reference, id: 1 },
    { ...reference, schema_version: wrongSchema },
    { ...reference, schema_version: 1 },
    { ...reference, content_digest: "a".repeat(64) },
    { ...reference, content_digest: `sha25:${"a".repeat(64)}` },
    { ...reference, content_digest: `SHA256:${"a".repeat(64)}` },
    { ...reference, content_digest: `sha256:${"A".repeat(64)}` },
    { ...reference, content_digest: `sha256:${"g".repeat(64)}` },
    { ...reference, content_digest: `sha256:${"a".repeat(63)}` },
    { ...reference, content_digest: `sha256:${"a".repeat(65)}` },
    { ...reference, content_digest: 1 },
    { ...reference, version: "" },
    { ...reference, version: 1 },
  ];
}

function omit(value, field) {
  return Object.fromEntries(Object.entries(value).filter(([key]) => key !== field));
}

function assertRequestError(source, code) {
  assert.throws(
    () => parseBrowserSmokeRequest(source),
    (error) => error instanceof BrowserSmokeWorkerError && error.code === code,
    source,
  );
}

async function captureWorkerError(operation, code) {
  let caught;
  try {
    await operation();
  } catch (error) {
    caught = error;
  }
  assert.equal(caught instanceof BrowserSmokeWorkerError, true);
  assert.equal(caught.code, code);
  return caught;
}
