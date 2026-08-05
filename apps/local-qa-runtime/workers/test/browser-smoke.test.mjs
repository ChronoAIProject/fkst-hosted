import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserSmokeWorkerError,
  parseBrowserSmokeRequest,
  runBrowserSmoke,
  serializeBrowserSmokeResult,
} from "../dist/index.js";

const requestJson = '{"version":"local-qa-browser-smoke/request-v1","fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","timeoutMs":5000}';
const sanitizedObservationRef = {
  kind: "sanitized-observation",
  id: "observation/0",
  schema_version: "qa.sanitized-observation/v1",
  content_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
};
const screenshotArtifactRef = {
  kind: "artifact-pointer",
  id: "artifact/0",
  schema_version: "qa.artifact-pointer/v1",
  content_digest: "4c4b6a3be1314ab86138bef4314dde022e600960d8689a2c8f8631802d20dab6",
};
const runnerLogArtifactRef = {
  kind: "artifact-pointer",
  id: "artifact/1",
  schema_version: "qa.artifact-pointer/v1",
  content_digest: "bb9c62cc84fc533e52193a8961778b0be251cd8f19a89b3fa836e94043a0075e",
};
const expectedSerialized = '{"version":"local-qa-browser-smoke/result-v1","outcome":"passed","observation":{"fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","finalUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","observedText":"READY","sanitizedObservationRef":{"kind":"sanitized-observation","id":"observation/0","schema_version":"qa.sanitized-observation/v1","content_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},"startedAt":"2026-01-02T03:04:05.000Z","finishedAt":"2026-01-02T03:04:05.012Z","durationMs":12,"evidence":[{"objectId":"evidence/0","role":"screenshot","artifactRef":{"kind":"artifact-pointer","id":"artifact/0","schema_version":"qa.artifact-pointer/v1","content_digest":"4c4b6a3be1314ab86138bef4314dde022e600960d8689a2c8f8631802d20dab6"}},{"objectId":"evidence/1","role":"runner-log","artifactRef":{"kind":"artifact-pointer","id":"artifact/1","schema_version":"qa.artifact-pointer/v1","content_digest":"bb9c62cc84fc533e52193a8961778b0be251cd8f19a89b3fa836e94043a0075e"}}]}';

test("walks the fixed browser smoke request through policy and Host-attested evidence", async () => {
  let runCalls = 0;
  let closeCalls = 0;
  let stagingCalls = 0;
  const nowValues = ["2026-01-02T03:04:05.000Z", "2026-01-02T03:04:05.012Z"];
  const monotonicValues = [1000, 1012];
  const bundle = await runBrowserSmoke(requestJson, {
    session: {
      async run(request) {
        runCalls += 1;
        assert.deepEqual(request, {
          version: "local-qa-browser-smoke/request-v1",
          fixtureUrl: "http://127.0.0.1:43123/fixed-page.html",
          selector: '[data-local-qa="status"]',
          expectedText: "READY",
          timeoutMs: 5000,
        });
        return {
          finalUrl: request.fixtureUrl,
          observedText: "READY",
          sanitizedObservationRef,
          screenshotArtifactRef,
        };
      },
      async close() {
        closeCalls += 1;
      },
    },
    evidence: {
      async stageGeneratedLog(input) {
        stagingCalls += 1;
        assert.equal(input.name, "runner.log");
        assert.equal(input.mediaType, "text/plain; charset=utf-8");
        assert.deepEqual(
          [...input.bytes],
          [...new TextEncoder().encode("navigation accepted\nassertion passed\n")],
        );
        return runnerLogArtifactRef;
      },
    },
    clock: {
      now() {
        return nowValues.shift();
      },
      monotonicMs() {
        return monotonicValues.shift();
      },
    },
  });

  assert.equal(runCalls, 1);
  assert.equal(closeCalls, 1);
  assert.equal(stagingCalls, 1);
  assert.equal("objects" in bundle, false);
  assert.deepEqual(bundle.result.observation.sanitizedObservationRef, sanitizedObservationRef);
  assert.deepEqual(bundle.result.evidence, [
    { objectId: "evidence/0", role: "screenshot", artifactRef: screenshotArtifactRef },
    { objectId: "evidence/1", role: "runner-log", artifactRef: runnerLogArtifactRef },
  ]);
  const serialized = new TextDecoder().decode(serializeBrowserSmokeResult(bundle.result));
  assert.equal(serialized, expectedSerialized);
  for (const forbidden of ['"objects"', '"bytes"', '"byteLength"', '"sha256"', '"ownership"', "file:", "/tmp/", '"storageToken"']) {
    assert.equal(serialized.includes(forbidden), false);
  }
});

test("strict request parsing accepts member reordering and rejects every open capability", () => {
  assert.deepEqual(
    parseBrowserSmokeRequest('{"timeoutMs":5000,"expectedText":"READY","selector":"[data-local-qa=\\"status\\"]","fixtureUrl":"http://127.0.0.1:1/fixed-page.html","version":"local-qa-browser-smoke/request-v1"}'),
    {
      version: "local-qa-browser-smoke/request-v1",
      fixtureUrl: "http://127.0.0.1:1/fixed-page.html",
      selector: '[data-local-qa="status"]',
      expectedText: "READY",
      timeoutMs: 5000,
    },
  );

  const rejected = [
    ["{", "request.invalid_json"],
    [requestJson + "x", "request.trailing_data"],
    ['{"version":"local-qa-browser-smoke/request-v1","version":"local-qa-browser-smoke/request-v1","fixtureUrl":"http://127.0.0.1:43123/fixed-page.html","selector":"[data-local-qa=\\"status\\"]","expectedText":"READY","timeoutMs":5000}', "request.duplicate_key"],
    ["[]", "request.root_not_object"],
    [requestJson.replace("}", ',"evaluate":"alert(1)"}'), "request.unknown_field"],
    [requestJson.replace(',"timeoutMs":5000', ""), "request.missing_field"],
    [requestJson.replace('"READY"', "true"), "request.wrong_type"],
    [requestJson.replace("request-v1", "request-v2"), "request.unsupported_value"],
    [requestJson.replace("127.0.0.1", "localhost"), "request.unsupported_value"],
    [requestJson.replace(":43123/", ":0/"), "request.unsupported_value"],
    [requestJson.replace(":43123/", ":65536/"), "request.unsupported_value"],
    [requestJson.replace("fixed-page.html", "fixed-page.html?x=1"), "request.unsupported_value"],
    [requestJson.replace("status", "other"), "request.unsupported_value"],
    [requestJson.replace("READY", "ready"), "request.unsupported_value"],
    [requestJson.replace("5000}", "5000.0}"), "request.unsupported_value"],
    [requestJson.replace("5000}", "5e3}"), "request.unsupported_value"],
  ];
  for (const [source, code] of rejected) {
    assert.throws(
      () => parseBrowserSmokeRequest(source),
      (error) => error instanceof BrowserSmokeWorkerError && error.code === code,
      source,
    );
  }
});

test("rejects malformed Host evidence references without transferring raw fields", async () => {
  const invalidReferences = [
    { ...sanitizedObservationRef, kind: "artifact-pointer" },
    { ...sanitizedObservationRef, schema_version: "qa.sanitized-observation/v2" },
    { ...sanitizedObservationRef, content_digest: "A".repeat(64) },
    { ...sanitizedObservationRef, storageToken: "forbidden" },
  ];
  for (const invalidReference of invalidReferences) {
    let closeCalls = 0;
    await assert.rejects(
      runBrowserSmoke(requestJson, {
        session: {
          async run() {
            return {
              finalUrl: "http://127.0.0.1:43123/fixed-page.html",
              observedText: "READY",
              sanitizedObservationRef: invalidReference,
              screenshotArtifactRef,
            };
          },
          async close() {
            closeCalls += 1;
          },
        },
        evidence: {
          async stageGeneratedLog() {
            assert.fail("evidence staging must not run");
          },
        },
        clock: fixedClock(),
      }),
      (error) => error instanceof BrowserSmokeWorkerError && error.code === "evidence.invalid_reference",
    );
    assert.equal(closeCalls, 1);
  }
});

test("session finalization runs exactly once after run or policy failure", async () => {
  for (const mode of ["run", "policy"]) {
    let closeCalls = 0;
    await assert.rejects(
      runBrowserSmoke(requestJson, {
        session: {
          async run() {
            if (mode === "run") {
              throw new Error("untrusted port detail");
            }
            return {
              finalUrl: "http://127.0.0.1:43123/other.html",
              observedText: "READY",
              sanitizedObservationRef,
              screenshotArtifactRef,
            };
          },
          async close() {
            closeCalls += 1;
          },
        },
        evidence: {
          async stageGeneratedLog() {
            assert.fail("evidence staging must not run");
          },
        },
        clock: fixedClock(),
      }),
      (error) => error instanceof BrowserSmokeWorkerError,
    );
    assert.equal(closeCalls, 1);
  }
});

function fixedClock() {
  return {
    now: () => "2026-01-02T03:04:05.000Z",
    monotonicMs: () => 1000,
  };
}
