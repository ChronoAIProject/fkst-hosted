import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  LocalWorkerFrameDecoder,
  canonicalBytes,
  contractRegistry,
  encodeLocalWorkerFrame,
  validateLocalWorkerFrame,
} from "../src/index.js";

type FixtureFrame = {
  readonly value: unknown;
  readonly canonical_utf8: string;
  readonly wire_hex: string;
};

type Fixture = { readonly frames: readonly FixtureFrame[] };

const fixture = JSON.parse(
  readFileSync(new URL("../../fixtures/qa.local-worker-protocol/v1/happy-path.json", import.meta.url), "utf8"),
) as Fixture;
const encoder = new TextEncoder();

function fromHex(value: string): Uint8Array {
  return Uint8Array.from(value.match(/../g)?.map((pair) => Number.parseInt(pair, 16)) ?? []);
}

test("registers and round-trips the shared worker transcript", () => {
  assert.equal(contractRegistry().schemas["qa.local-worker-protocol/v1"]?.major, 1);
  for (const frame of fixture.frames) {
    const validated = validateLocalWorkerFrame(encoder.encode(JSON.stringify(frame.value)));
    assert.equal(new TextDecoder().decode(canonicalBytes(validated)), frame.canonical_utf8);
    assert.deepEqual(encodeLocalWorkerFrame(validated), fromHex(frame.wire_hex));
  }
});

test("decodes coalesced and fragmented binary frames without newline semantics", () => {
  const wires = fixture.frames.map((frame) => fromHex(frame.wire_hex));
  const combined = new Uint8Array(wires.reduce((total, wire) => total + wire.length, 0));
  let offset = 0;
  for (const wire of wires) {
    combined.set(wire, offset);
    offset += wire.length;
  }
  const coalesced = new LocalWorkerFrameDecoder();
  assert.equal(coalesced.push(combined).length, fixture.frames.length);
  coalesced.finish();

  const fragmented = new LocalWorkerFrameDecoder();
  const decoded = [
    ...fragmented.push(combined.slice(0, 2)),
    ...fragmented.push(combined.slice(2, 9)),
    ...fragmented.push(combined.slice(9)),
  ];
  assert.equal(decoded.length, fixture.frames.length);
  fragmented.finish();
  assert.equal(fixture.frames[0]?.canonical_utf8.includes("\\n"), false);
});
