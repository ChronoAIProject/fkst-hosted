import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { validateLocalWorkerProtocolFailure } from "../../../../packages/qa-contracts/dist/index.js";
import { WorkerControlState } from "../dist/protocol-worker.js";

const fixture = JSON.parse(await readFile(new URL("../../../../packages/qa-contracts/fixtures/qa.local-worker-protocol/v1/happy-path.json", import.meta.url), "utf8"));
const fromHex = (value) => Buffer.from(value, "hex");
const controlEncoder = new TextEncoder();
const abort = {
  protocol: "qa.local-worker-control/v1",
  kind: "abort",
  control_id: "00000000-0000-0000-0000-000000000001",
  invocation_id: "invocation/0",
  deadline_utc: "2026-09-02T12:00:00Z",
};

test("identical abort is idempotent and fences capabilities", () => {
  const state = new WorkerControlState("invocation/0");
  const raw = controlEncoder.encode(JSON.stringify(abort));
  assert.equal(state.acceptAbort(raw).status, "accepted");
  assert.equal(state.acceptAbort(raw).status, "accepted");
  assert.equal(state.cancelled(), true);
  assert.throws(() => state.assertCapabilityAllowed(), /control.cancelled/);
});

test("conflicting control fails closed", () => {
  const state = new WorkerControlState("invocation/0");
  state.acceptAbort(controlEncoder.encode(JSON.stringify(abort)));
  assert.throws(
    () =>
      state.acceptAbort(
        controlEncoder.encode(
          JSON.stringify({ ...abort, deadline_utc: "2026-09-02T12:00:01Z" }),
        ),
      ),
    /control.conflict/,
  );
});

test("terminal before abort reports too late without cancellation", () => {
  const state = new WorkerControlState("invocation/0");
  state.markTerminal();
  assert.equal(
    state.acceptAbort(controlEncoder.encode(JSON.stringify(abort))).status,
    "too_late",
  );
  assert.equal(state.cancelled(), false);
  state.assertCapabilityAllowed();
});

test("walks one fragmented invocation through the fixed worker process", async () => {
  const child = spawn(process.execPath, [fileURLToPath(new URL("../dist/worker-main.js", import.meta.url))], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout = createFrameReader(child.stdout);
  const stderr = [];
  child.stderr.on("data", (chunk) => stderr.push(chunk));

  await writeFragmented(child.stdin, fromHex(fixture.frames[0].wire_hex));
  for (let index = 1; index < 15; index += 2) {
    assert.deepEqual(await stdout.read(), fromHex(fixture.frames[index].wire_hex));
    await writeFragmented(child.stdin, fromHex(fixture.frames[index + 1].wire_hex));
  }
  child.stdin.end();
  assert.deepEqual(await stdout.read(), fromHex(fixture.frames[15].wire_hex));
  const exitCode = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  assert.equal(exitCode, 0);
  assert.equal(Buffer.concat(stderr).length, 0);
  assert.equal(stdout.trailing().length, 0);
});

test("emits one sanitized failure for malformed and truncated input", async () => {
  const cases = [
    Buffer.from("00000001ff", "hex"),
    Buffer.from("000000057b7d", "hex"),
  ];
  for (const input of cases) {
    const outcome = await runFailingWorker([input]);
    assert.equal(outcome.exitCode, 1);
    assertFailureOutput(outcome.stdout, "protocol.invalid_frame");
    assert.match(outcome.stderr, /^fkst-local-qa-worker: protocol\.invalid_frame\n$/);
    assert.equal(outcome.stderr.length <= 128, true);
    assert.equal(outcome.stderr.includes("/private/"), false);
    assert.equal(outcome.stderr.includes("Error:"), false);
  }
});

test("rejects a complete frame followed by a partial extra frame", async () => {
  const next = fromHex(fixture.frames[2].wire_hex);
  const outcome = await runFailingWorker([
    Buffer.concat([fromHex(fixture.frames[0].wire_hex), next.subarray(0, 2)]),
  ]);
  assert.equal(outcome.exitCode, 1);
  assertFailureOutput(outcome.stdout, "protocol.invalid_sequence");
});

test("rejects coalesced extra frames with one failure and no capability request", async () => {
  const outcome = await runFailingWorker([
    Buffer.concat([fromHex(fixture.frames[0].wire_hex), fromHex(fixture.frames[0].wire_hex)]),
  ]);
  assert.equal(outcome.exitCode, 1);
  assertFailureOutput(outcome.stdout, "protocol.invalid_sequence");
  assert.match(outcome.stderr, /^fkst-local-qa-worker: protocol\.invalid_sequence\n$/);
});

test("rejects trailing input instead of publishing a success terminal", async () => {
  const child = spawnWorker();
  const stdout = createFrameReader(child.stdout);
  const stderr = [];
  child.stderr.on("data", (chunk) => stderr.push(chunk));
  child.stdin.write(fromHex(fixture.frames[0].wire_hex));
  for (let index = 1; index < 15; index += 2) {
    assert.deepEqual(await stdout.read(), fromHex(fixture.frames[index].wire_hex));
    child.stdin.write(fromHex(fixture.frames[index + 1].wire_hex));
  }
  child.stdin.write(fromHex(fixture.frames[0].wire_hex));
  child.stdin.end();
  assertFailureFrame(await stdout.read(), "protocol.trailing_input");
  const exitCode = await waitForClose(child);
  assert.equal(exitCode, 1);
  assert.equal(stdout.trailing().length, 0);
  assert.match(Buffer.concat(stderr).toString(), /^fkst-local-qa-worker: protocol\.trailing_input\n$/);
});

async function writeFragmented(stream, frame) {
  for (const chunk of [frame.subarray(0, 1), frame.subarray(1, 5), frame.subarray(5)]) {
    if (!stream.write(chunk)) await new Promise((resolve) => stream.once("drain", resolve));
  }
}

function spawnWorker() {
  return spawn(process.execPath, [fileURLToPath(new URL("../dist/worker-main.js", import.meta.url))], {
    stdio: ["pipe", "pipe", "pipe"],
  });
}

async function runFailingWorker(chunks) {
  const child = spawnWorker();
  const stdout = [];
  const stderr = [];
  child.stdout.on("data", (chunk) => stdout.push(chunk));
  child.stderr.on("data", (chunk) => stderr.push(chunk));
  for (const chunk of chunks) child.stdin.write(chunk);
  child.stdin.end();
  return {
    exitCode: await waitForClose(child),
    stdout: Buffer.concat(stdout),
    stderr: Buffer.concat(stderr).toString(),
  };
}

function waitForClose(child) {
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
}

function assertFailureOutput(stdout, code) {
  assert.equal(stdout.length >= 4, true);
  const length = stdout.readUInt32BE(0);
  assert.equal(stdout.length, length + 4);
  assertFailureFrame(stdout, code);
}

function assertFailureFrame(frame, code) {
  const length = frame.readUInt32BE(0);
  assert.equal(frame.length, length + 4);
  const value = validateLocalWorkerProtocolFailure(frame.subarray(4)).value();
  assert.deepEqual(value, {
    protocol: "qa.local-worker-protocol/v1",
    kind: "protocol_failure",
    code,
  });
}

function createFrameReader(stream) {
  let buffer = Buffer.alloc(0);
  const waiters = [];
  stream.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    flush();
  });
  function flush() {
    while (waiters.length > 0 && buffer.length >= 4) {
      const length = buffer.readUInt32BE(0);
      if (buffer.length < length + 4) return;
      const frame = buffer.subarray(0, length + 4);
      buffer = buffer.subarray(length + 4);
      waiters.shift()(frame);
    }
  }
  return {
    read() {
      return new Promise((resolve) => {
        waiters.push(resolve);
        flush();
      });
    },
    trailing() {
      return buffer;
    },
  };
}
