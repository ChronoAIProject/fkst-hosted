import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

const fixture = JSON.parse(await readFile(new URL("../../../../packages/qa-contracts/fixtures/qa.local-worker-protocol/v1/happy-path.json", import.meta.url), "utf8"));
const fromHex = (value) => Buffer.from(value, "hex");

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
  assert.deepEqual(await stdout.read(), fromHex(fixture.frames[15].wire_hex));
  child.stdin.end();
  const exitCode = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  assert.equal(exitCode, 0);
  assert.equal(Buffer.concat(stderr).length, 0);
  assert.equal(stdout.trailing().length, 0);
});

async function writeFragmented(stream, frame) {
  for (const chunk of [frame.subarray(0, 1), frame.subarray(1, 5), frame.subarray(5)]) {
    if (!stream.write(chunk)) await new Promise((resolve) => stream.once("drain", resolve));
  }
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
