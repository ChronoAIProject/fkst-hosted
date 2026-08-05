import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { Ajv2020 } from "ajv/dist/2020.js";

import {
  contractRegistry,
  validateLocalEvidenceObject,
  validateLocalEvidenceObjectRef,
  validateLocalSanitizedObservationRef,
} from "../src/index.js";

const encoder = new TextEncoder();
const zeroDigest = "0".repeat(64);
const patternedDigest = "abcdef".repeat(10) + "abcd";

const screenshotObject = Object.freeze({
  schema_version: "qa.local-evidence/v1",
  run_id: "run_1",
  attempt: 1,
  object_id: "evidence/1",
  role: "browser-screenshot",
  media_type: "image/png",
  byte_length: 0,
  sha256: zeroDigest,
  ownership: "local-only:not-uploadable",
});
const runnerLogObject = Object.freeze({
  ...screenshotObject,
  object_id: "evidence/2",
  role: "runner-log",
  media_type: "text/plain; charset=utf-8",
  byte_length: 1_048_576,
  sha256: patternedDigest,
});
const observationRef = Object.freeze({
  kind: "local-sanitized-observation",
  id: "observation/1",
  schema_version: "qa.local-evidence/v1",
  content_digest: `sha256:${zeroDigest}`,
});
const versionedObservationRef = Object.freeze({
  ...observationRef,
  id: "observation/2",
  content_digest: `sha256:${patternedDigest}`,
  version: "v1",
});
const evidenceRef = Object.freeze({
  kind: "local-evidence-object",
  id: "evidence/1",
  schema_version: "qa.local-evidence/v1",
  content_digest: `sha256:${zeroDigest}`,
});
const versionedEvidenceRef = Object.freeze({
  ...evidenceRef,
  id: "evidence/2",
  content_digest: `sha256:${patternedDigest}`,
  version: "v1",
});

const duplicateEvidenceObject =
  `{"schema_version":"qa.local-evidence/v1","run_id":"run_1","attempt":1,"object_id":"evidence/1","object_id":"evidence/2","role":"browser-screenshot","media_type":"image/png","byte_length":0,"sha256":"${zeroDigest}","ownership":"local-only:not-uploadable"}`;
const duplicateObservationRef =
  `{"kind":"local-sanitized-observation","id":"observation/1","id":"observation/2","schema_version":"qa.local-evidence/v1","content_digest":"sha256:${zeroDigest}"}`;
const duplicateEvidenceRef =
  `{"kind":"local-evidence-object","id":"evidence/1","id":"evidence/2","schema_version":"qa.local-evidence/v1","content_digest":"sha256:${zeroDigest}"}`;

const validators = {
  LocalEvidenceObject: validateLocalEvidenceObject,
  LocalSanitizedObservationRef: validateLocalSanitizedObservationRef,
  LocalEvidenceObjectRef: validateLocalEvidenceObjectRef,
} as const;

const fixtures = {
  LocalEvidenceObject: screenshotObject,
  LocalSanitizedObservationRef: observationRef,
  LocalEvidenceObjectRef: evidenceRef,
} as const;

test("local evidence registry exposes exactly the approved public definitions", () => {
  const registry = contractRegistry();
  for (const typeName of [
    "LocalSanitizedObservation",
    "LocalEvidenceObject",
    "LocalSanitizedObservationRef",
    "LocalEvidenceObjectRef",
  ] as const) {
    assert.deepEqual(registry.types[typeName], {
      schema: "qa.local-evidence/v1",
      pointer: `#/$defs/${typeName}`,
    });
  }
  for (const forbiddenAlias of [
    "SanitizedObservation",
    "ArtifactPointer",
    "qa.sanitized-observation/v1",
    "qa.artifact-pointer/v1",
  ]) {
    assert.equal(registry.types[forbiddenAlias], undefined);
  }
});

test("all exact local evidence fixtures validate as immutable values", () => {
  for (const [validator, fixture] of [
    [validateLocalEvidenceObject, screenshotObject],
    [validateLocalEvidenceObject, runnerLogObject],
    [validateLocalSanitizedObservationRef, observationRef],
    [validateLocalSanitizedObservationRef, versionedObservationRef],
    [validateLocalEvidenceObjectRef, evidenceRef],
    [validateLocalEvidenceObjectRef, versionedEvidenceRef],
  ] as const) {
    const validated = validator(raw(fixture));
    assert.deepEqual(validated.value(), fixture);
    assert.ok(Object.isFrozen(validated.value()));
  }
});

test("new validators reject exact duplicate-key fixtures and unknown fields", () => {
  for (const [validator, duplicate] of [
    [validateLocalEvidenceObject, duplicateEvidenceObject],
    [validateLocalSanitizedObservationRef, duplicateObservationRef],
    [validateLocalEvidenceObjectRef, duplicateEvidenceRef],
  ] as const) {
    assert.throws(() => validator(encoder.encode(duplicate)));
  }
  for (const typeName of Object.keys(validators) as Array<keyof typeof validators>) {
    assert.throws(() => validators[typeName](raw({ ...fixtures[typeName], uploadable: true })));
  }
});

test("new validators reject missing required members and wrong schema versions", () => {
  for (const typeName of Object.keys(validators) as Array<keyof typeof validators>) {
    assert.throws(() => validators[typeName](raw(without(fixtures[typeName], "schema_version"))));
    assert.throws(() =>
      validators[typeName](raw({ ...fixtures[typeName], schema_version: "qa.local-evidence/v2" })),
    );
  }
  for (const field of [
    "run_id",
    "attempt",
    "object_id",
    "role",
    "media_type",
    "byte_length",
    "sha256",
    "ownership",
  ]) {
    assert.throws(() => validateLocalEvidenceObject(raw(without(screenshotObject, field))));
  }
  for (const [validator, fixture] of [
    [validateLocalSanitizedObservationRef, observationRef],
    [validateLocalEvidenceObjectRef, evidenceRef],
  ] as const) {
    for (const field of ["kind", "id", "content_digest"]) {
      assert.throws(() => validator(raw(without(fixture, field))));
    }
  }
});

test("LocalEvidenceObject enforces identifier and numeric boundaries", () => {
  for (const replacement of [
    { run_id: "A" },
    { run_id: "A".repeat(64) },
    { attempt: 1 },
    { attempt: 9_007_199_254_740_991 },
    { object_id: "evidence/0" },
    { object_id: `evidence/${"1".repeat(55)}` },
    { byte_length: 0 },
    { byte_length: 1_048_576 },
  ]) {
    validateLocalEvidenceObject(raw({ ...screenshotObject, ...replacement }));
  }
  for (const replacement of [
    { run_id: "" },
    { run_id: "-run" },
    { run_id: "A".repeat(65) },
    { attempt: 0 },
    { attempt: 9_007_199_254_740_992 },
    { attempt: 1.5 },
    { object_id: "evidence/" },
    { object_id: "evidence/a" },
    { object_id: "observation/1" },
    { object_id: `evidence/${"1".repeat(56)}` },
    { byte_length: -1 },
    { byte_length: 1_048_577 },
    { byte_length: 1.5 },
  ]) {
    assert.throws(() => validateLocalEvidenceObject(raw({ ...screenshotObject, ...replacement })));
  }
});

test("LocalEvidenceObject enforces digests, ownership, literals, and paired media", () => {
  for (const fixture of [screenshotObject, runnerLogObject]) {
    validateLocalEvidenceObject(raw(fixture));
  }
  for (const replacement of [
    { sha256: "A".repeat(64) },
    { sha256: "g".repeat(64) },
    { sha256: "0".repeat(63) },
    { sha256: "0".repeat(65) },
    { sha256: `sha256:${zeroDigest}` },
    { ownership: "uploadable" },
    { role: "browser-video" },
    { media_type: "application/octet-stream" },
    { role: "browser-screenshot", media_type: "text/plain; charset=utf-8" },
    { role: "runner-log", media_type: "image/png" },
  ]) {
    assert.throws(() => validateLocalEvidenceObject(raw({ ...screenshotObject, ...replacement })));
  }
});

test("registered LocalEvidenceObject schema directly owns role and media pairing", () => {
  const registry = JSON.parse(
    readFileSync(new URL("../../contracts/registry.json", import.meta.url), "utf8"),
  ) as { types: Record<string, { pointer: string }> };
  const schema = JSON.parse(
    readFileSync(new URL("../../contracts/qa.local-evidence/v1/schema.json", import.meta.url), "utf8"),
  ) as Record<string, unknown>;
  const validatorSchema = structuredClone(schema);
  delete validatorSchema.$id;
  validatorSchema.$ref = registry.types.LocalEvidenceObject?.pointer;
  assert.equal(validatorSchema.$ref, "#/$defs/LocalEvidenceObject");
  const validate = new Ajv2020({ strict: true }).compile(validatorSchema);

  assert.equal(validate(screenshotObject), true);
  assert.equal(validate(runnerLogObject), true);
  assert.equal(validate({ ...screenshotObject, media_type: "text/plain; charset=utf-8" }), false);
  assert.equal(validate({ ...runnerLogObject, media_type: "image/png" }), false);
});

test("reference validators enforce IDs, kinds, digests, and optional versions", () => {
  const cases = [
    {
      validator: validateLocalSanitizedObservationRef,
      fixture: observationRef,
      prefix: "observation/",
      otherKind: "local-evidence-object",
      wrongPrefix: "evidence/1",
      prohibited: [
        "/observation/1",
        "http://127.0.0.1/observation/1",
        "observation\\1",
        "observation/../1",
        "observation/%2F1",
        "observation/%5C1",
      ],
    },
    {
      validator: validateLocalEvidenceObjectRef,
      fixture: evidenceRef,
      prefix: "evidence/",
      otherKind: "local-sanitized-observation",
      wrongPrefix: "observation/1",
      prohibited: [
        "/evidence/1",
        "https://example.test/evidence/1",
        "evidence\\1",
        "evidence/../1",
        "evidence/%2F1",
        "evidence/%5C1",
      ],
    },
  ] as const;

  for (const { validator, fixture, prefix, otherKind, wrongPrefix, prohibited } of cases) {
    for (const replacement of [
      { id: `${prefix}0` },
      { id: `${prefix}${"1".repeat(64 - prefix.length)}` },
      { version: "v" },
      { version: "v".repeat(64) },
    ]) {
      validator(raw({ ...fixture, ...replacement }));
    }
    for (const replacement of [
      { id: prefix },
      { id: `${prefix}x` },
      { id: wrongPrefix },
      { id: `${prefix}${"1".repeat(65 - prefix.length)}` },
      ...prohibited.map((id) => ({ id })),
      { kind: otherKind },
      { kind: "unknown" },
      { content_digest: zeroDigest },
      { content_digest: `sha256:${"A".repeat(64)}` },
      { content_digest: `sha256:${"g".repeat(64)}` },
      { content_digest: `sha256:${"0".repeat(63)}` },
      { content_digest: `sha256:${"0".repeat(65)}` },
      { version: "" },
      { version: "v".repeat(65) },
    ]) {
      assert.throws(() => validator(raw({ ...fixture, ...replacement })));
    }
  }
});

function raw(value: unknown): Uint8Array {
  return encoder.encode(JSON.stringify(value));
}

function without(value: object, field: string): Record<string, unknown> {
  const copy = { ...value } as Record<string, unknown>;
  delete copy[field];
  return copy;
}
