import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { PassThrough, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  encodeLocalWorkerFrame,
  validateLocalWorkerAbort,
  validateLocalWorkerCancelAck,
  validateLocalWorkerControlFailure,
  validateLocalWorkerProtocolFailure,
} from "../../../../packages/qa-contracts/dist/index.js";
import { ProtocolPeer, WorkerControlState } from "../dist/protocol-worker.js";

const fixture = JSON.parse(await readFile(new URL("../../../../packages/qa-contracts/fixtures/qa.local-worker-protocol/v1/happy-path.json", import.meta.url), "utf8"));
const fromHex = (value) => Buffer.from(value, "hex");
const controlEncoder = new TextEncoder();
const workerCloseTimeoutMs = 5_000;
const workerCleanupTimeoutMs = 1_000;
const workerCloseStates = new WeakMap();
const abort = {
  protocol: "qa.local-worker-control/v1",
  kind: "abort",
  control_id: "00000000-0000-0000-0000-000000000001",
  invocation_id: "invocation/0",
  deadline_utc: "2099-09-02T12:00:00Z",
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
          JSON.stringify({ ...abort, deadline_utc: "2099-09-02T12:00:01Z" }),
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

test("input teardown is idempotent and waits for queued output", async () => {
  const input = new PassThrough();
  const events = [];
  let finishWrite;
  let resolveWriteStarted;
  const writeStarted = new Promise((resolve) => {
    resolveWriteStarted = resolve;
  });
  const output = new Writable({
    write(_chunk, _encoding, callback) {
      finishWrite = () => {
        events.push("write-completed");
        callback();
      };
      resolveWriteStarted();
    },
  });
  let finishReturn;
  const returnFinished = new Promise((resolve) => {
    finishReturn = resolve;
  });
  input[Symbol.asyncIterator] = () => ({
    return() {
      events.push("return-started");
      return returnFinished;
    },
  });
  let destroyCalls = 0;
  const destroy = input.destroy.bind(input);
  input.destroy = (error) => {
    destroyCalls += 1;
    events.push("input-destroyed");
    finishReturn({ done: true, value: undefined });
    return destroy(error);
  };
  const peer = new ProtocolPeer(input, output);
  const write = peer.write(
    {
      protocol: "qa.local-worker-control/v1",
      kind: "cancel_ack",
      control_id: abort.control_id,
      invocation_id: abort.invocation_id,
      status: "accepted",
    },
    validateLocalWorkerCancelAck,
  );
  const firstRelease = peer.releaseInput();
  const secondRelease = peer.releaseInput();

  assert.equal(firstRelease, secondRelease);
  await writeStarted;
  assert.equal(input.destroyed, false);
  finishWrite();
  await write;
  await firstRelease;
  assert.equal(input.destroyed, true);
  assert.deepEqual(events, ["write-completed", "return-started", "input-destroyed"]);

  const completedDestroyCalls = destroyCalls;
  await peer.releaseInput();
  assert.equal(destroyCalls, completedDestroyCalls);
});

test("walks one fragmented invocation through the fixed worker process", async (context) => {
  const child = spawnWorker(context);
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
  const exitCode = await waitForClose(child);
  assert.equal(exitCode, 0);
  assert.equal(Buffer.concat(stderr).length, 0);
  assert.equal(stdout.trailing().length, 0);
});

test("acknowledges a fragmented abort while awaiting a capability result", async (context) => {
  const child = spawnWorker(context);
  const stdout = createFrameReader(child.stdout);
  const stderr = [];
  child.stderr.on("data", (chunk) => stderr.push(chunk));
  child.stdin.write(fromHex(fixture.frames[0].wire_hex));
  assert.deepEqual(await stdout.read(), fromHex(fixture.frames[1].wire_hex));

  await writeFragmented(child.stdin, controlFrame(abort));
  const acknowledgement = validateLocalWorkerCancelAck((await stdout.read()).subarray(4)).value();
  assert.deepEqual(acknowledgement, {
    protocol: "qa.local-worker-control/v1",
    kind: "cancel_ack",
    control_id: abort.control_id,
    invocation_id: abort.invocation_id,
    status: "accepted",
  });
  assert.equal(child.stdin.writableEnded, false);
  assert.equal(await waitForClose(child), 0);
  assert.equal(stdout.trailing().length, 0);
  assert.equal(Buffer.concat(stderr).length, 0);
});

test("acknowledges coalesced identical aborts idempotently", async (context) => {
  const child = spawnWorker(context);
  const stdout = createFrameReader(child.stdout);
  child.stdin.write(fromHex(fixture.frames[0].wire_hex));
  assert.deepEqual(await stdout.read(), fromHex(fixture.frames[1].wire_hex));

  const frame = controlFrame(abort);
  child.stdin.write(Buffer.concat([frame, frame]));
  for (let index = 0; index < 2; index += 1) {
    const acknowledgement = validateLocalWorkerCancelAck((await stdout.read()).subarray(4)).value();
    assert.equal(acknowledgement.status, "accepted");
    assert.equal(acknowledgement.control_id, abort.control_id);
  }
  assert.equal(child.stdin.writableEnded, false);
  assert.equal(await waitForClose(child), 0);
  assert.equal(stdout.trailing().length, 0);
});

test("prioritizes a coalesced abort over its pending capability result", async (context) => {
  const child = spawnWorker(context);
  const stdout = createFrameReader(child.stdout);
  child.stdin.write(fromHex(fixture.frames[0].wire_hex));
  assert.deepEqual(await stdout.read(), fromHex(fixture.frames[1].wire_hex));

  child.stdin.write(Buffer.concat([controlFrame(abort), fromHex(fixture.frames[2].wire_hex)]));
  const acknowledgement = validateLocalWorkerCancelAck((await stdout.read()).subarray(4)).value();
  assert.equal(acknowledgement.status, "accepted");
  assert.equal(child.stdin.writableEnded, false);
  assert.equal(await waitForClose(child), 0);
  assert.equal(stdout.trailing().length, 0);
});

test("emits a typed failure for a conflicting invocation", async (context) => {
  const child = spawnWorker(context);
  const stdout = createFrameReader(child.stdout);
  child.stdin.write(fromHex(fixture.frames[0].wire_hex));
  assert.deepEqual(await stdout.read(), fromHex(fixture.frames[1].wire_hex));

  child.stdin.write(controlFrame({ ...abort, invocation_id: "invocation/conflict" }));
  const failure = validateLocalWorkerControlFailure((await stdout.read()).subarray(4)).value();
  assert.deepEqual(failure, {
    protocol: "qa.local-worker-control/v1",
    kind: "control_failure",
    control_id: abort.control_id,
    code: "control.invalid_invocation",
  });
  assert.equal(child.stdin.writableEnded, false);
  assert.equal(await waitForClose(child), 0);
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

test("rejects trailing input instead of publishing a success terminal", async (context) => {
  const child = spawnWorker(context);
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

function controlFrame(value) {
  return Buffer.from(encodeLocalWorkerFrame(validateLocalWorkerAbort(controlEncoder.encode(JSON.stringify(value)))));
}

async function writeFragmented(stream, frame) {
  for (const chunk of [frame.subarray(0, 1), frame.subarray(1, 5), frame.subarray(5)]) {
    if (!stream.write(chunk)) await new Promise((resolve) => stream.once("drain", resolve));
  }
}

function spawnWorker(context) {
  const child = spawn(process.execPath, [fileURLToPath(new URL("../dist/worker-main.js", import.meta.url))], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const state = { closed: false, promise: undefined };
  state.promise = new Promise((resolve) => {
    child.once("error", (error) => {
      state.closed = true;
      resolve({ error });
    });
    child.once("close", (exitCode) => {
      state.closed = true;
      resolve({ exitCode });
    });
  });
  workerCloseStates.set(child, state);
  context?.after(() => stopWorker(child));
  return child;
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

async function waitForClose(child) {
  const state = workerCloseStates.get(child);
  assert.notEqual(state, undefined);
  const outcome = await waitBounded(state.promise, workerCloseTimeoutMs);
  if (outcome === undefined) {
    await stopWorker(child);
    throw new Error(`Worker did not exit within ${workerCloseTimeoutMs}ms`);
  }
  if (outcome.error !== undefined) throw outcome.error;
  return outcome.exitCode;
}

async function stopWorker(child) {
  const state = workerCloseStates.get(child);
  assert.notEqual(state, undefined);
  if (state.closed) return;
  child.kill("SIGKILL");
  const outcome = await waitBounded(state.promise, workerCleanupTimeoutMs);
  if (outcome === undefined) throw new Error("Worker did not exit after test cleanup");
}

function waitBounded(promise, timeoutMs) {
  return new Promise((resolve) => {
    const timeout = setTimeout(() => resolve(undefined), timeoutMs);
    promise.then((value) => {
      clearTimeout(timeout);
      resolve(value);
    });
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
