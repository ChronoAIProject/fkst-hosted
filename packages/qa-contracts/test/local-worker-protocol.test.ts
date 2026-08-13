import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  ContractError,
  LocalWorkerFrameDecoder,
  LocalWorkerInputSequence,
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
type ExpectedRejection = {
  readonly category: string;
  readonly code?: string;
  readonly reason: string;
  readonly path: string;
};
type Mutation = { readonly path: readonly (string | number)[]; readonly value: unknown };
type FrameCase = {
  readonly case_id: string;
  readonly happy_index?: number;
  readonly source_utf8?: string;
  readonly source_hex?: string;
  readonly replacement?: unknown;
  readonly mutation?: Mutation;
  readonly expected: ExpectedRejection;
};
type FramingCase = {
  readonly case_id: string;
  readonly wire_hex?: string;
  readonly happy_index?: number;
  readonly suffix_hex?: string;
  readonly phase: "push" | "finish";
  readonly expected: ExpectedRejection;
};
type SequenceFrame = { readonly happy_index: number; readonly mutation?: Mutation };
type SequenceCase = {
  readonly case_id: string;
  readonly frames: readonly SequenceFrame[];
  readonly finish?: boolean;
  readonly expected: ExpectedRejection;
};
type NegativeFixture = {
  readonly frame_cases: readonly FrameCase[];
  readonly framing_cases: readonly FramingCase[];
  readonly sequence_cases: readonly SequenceCase[];
};

const fixture = JSON.parse(
  readFileSync(new URL("../../fixtures/qa.local-worker-protocol/v1/happy-path.json", import.meta.url), "utf8"),
) as Fixture;
const negativeFixture = JSON.parse(
  readFileSync(new URL("../../fixtures/qa.local-worker-protocol/v1/negative.json", import.meta.url), "utf8"),
) as NegativeFixture;
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

for (const fixtureCase of negativeFixture.frame_cases) {
  test(fixtureCase.case_id, () => {
    const raw = frameCaseBytes(fixtureCase);
    assert.throws(
      () => validateLocalWorkerFrame(raw),
      (error: unknown) => rejectionMatches(error, fixtureCase.expected),
    );
  });
}

for (const fixtureCase of negativeFixture.framing_cases) {
  test(fixtureCase.case_id, () => {
    const decoder = new LocalWorkerFrameDecoder();
    const wire = framingCaseBytes(fixtureCase);
    assert.throws(
      () => {
        decoder.push(wire);
        if (fixtureCase.phase === "finish") decoder.finish();
      },
      (error: unknown) => rejectionMatches(error, fixtureCase.expected),
    );
  });
}

for (const fixtureCase of negativeFixture.sequence_cases) {
  test(fixtureCase.case_id, () => {
    const sequence = new LocalWorkerInputSequence();
    assert.throws(
      () => {
        for (const frame of fixtureCase.frames) {
          sequence.accept(validatedFixtureFrame(frame.happy_index, frame.mutation));
        }
        if (fixtureCase.finish === true) sequence.finish();
      },
      (error: unknown) => rejectionMatches(error, fixtureCase.expected),
    );
  });
}

test("accepts the exact inbound sequence and clean EOF", () => {
  const sequence = new LocalWorkerInputSequence();
  for (const index of [0, 2, 4, 6, 8, 10, 12, 14]) {
    sequence.accept(validatedFixtureFrame(index));
  }
  sequence.finish();
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

function frameCaseBytes(fixtureCase: FrameCase): Uint8Array {
  if (fixtureCase.source_hex !== undefined) return fromHex(fixtureCase.source_hex);
  if (fixtureCase.source_utf8 !== undefined) return encoder.encode(fixtureCase.source_utf8);
  const value = fixtureCase.replacement ?? mutatedFixtureValue(fixtureCase.happy_index!, fixtureCase.mutation);
  return encoder.encode(JSON.stringify(value));
}

function framingCaseBytes(fixtureCase: FramingCase): Uint8Array {
  if (fixtureCase.wire_hex !== undefined) return fromHex(fixtureCase.wire_hex);
  const wire = fromHex(fixture.frames[fixtureCase.happy_index!]?.wire_hex ?? "");
  const suffix = fromHex(fixtureCase.suffix_hex ?? "");
  const combined = new Uint8Array(wire.length + suffix.length);
  combined.set(wire);
  combined.set(suffix, wire.length);
  return combined;
}

function validatedFixtureFrame(index: number, mutation?: Mutation) {
  return validateLocalWorkerFrame(encoder.encode(JSON.stringify(mutatedFixtureValue(index, mutation))));
}

function mutatedFixtureValue(index: number, mutation?: Mutation): unknown {
  const source = fixture.frames[index]?.value;
  assert.notEqual(source, undefined);
  const value = structuredClone(source);
  if (mutation !== undefined) setPath(value, mutation.path, mutation.value);
  return value;
}

function setPath(root: unknown, path: readonly (string | number)[], value: unknown): void {
  let current = root as Record<string | number, unknown>;
  for (const token of path.slice(0, -1)) current = current[token]! as Record<string | number, unknown>;
  current[path[path.length - 1]!] = value;
}

function rejectionMatches(error: unknown, expected: ExpectedRejection): boolean {
  assert.ok(error instanceof ContractError);
  assert.deepEqual(error.rejection, expected);
  return true;
}
