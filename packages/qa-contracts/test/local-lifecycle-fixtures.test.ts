import assert from "node:assert/strict";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  canonicalBytes,
  contractRegistry,
  ContractError,
  type Rejection,
  sha256Digest,
  validateCancelDisposition,
  validateEventCursor,
  validateEventSequence,
  validateExecutionOutcome,
  validateLocalState,
  ValidatedValue,
} from "../src/index.js";

type LifecycleType =
  | "LocalState"
  | "ExecutionOutcome"
  | "CancelDisposition"
  | "EventSequence"
  | "EventCursor";

interface LifecycleCase {
  readonly case_id: string;
  readonly source: unknown;
}

interface LifecycleFixture {
  readonly schema_version: string;
  readonly valid_cases: readonly (LifecycleCase & {
    readonly lifecycle_type: LifecycleType;
    readonly source: string;
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  })[];
  readonly cancel_disposition_valid_cases: readonly (LifecycleCase & {
    readonly source: string;
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  })[];
  readonly event_sequence_valid_cases: readonly (LifecycleCase & {
    readonly source: number;
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  })[];
  readonly event_cursor_valid_cases: readonly (LifecycleCase & {
    readonly source: number;
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  })[];
  readonly cancel_disposition_invalid_cases: readonly (LifecycleCase & {
    readonly expected: Rejection;
  })[];
  readonly event_sequence_invalid_cases: readonly (LifecycleCase & {
    readonly expected: Rejection;
  })[];
  readonly event_cursor_invalid_cases: readonly (LifecycleCase & {
    readonly expected: Rejection;
  })[];
  readonly raw_invalid_cases: readonly {
    readonly case_id: string;
    readonly lifecycle_type: "CancelDisposition" | "EventSequence" | "EventCursor";
    readonly raw_utf8_hex: string;
    readonly expected: Rejection;
  }[];
  readonly invalid_cases: readonly (LifecycleCase & {
    readonly lifecycle_type: LifecycleType;
    readonly expected: Rejection;
  })[];
}

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../../fixtures/qa/local-lifecycle-v1.json", import.meta.url),
    "utf8",
  ),
) as LifecycleFixture;

test("local lifecycle fixture metadata", () => {
  assert.equal(fixture.schema_version, "qa.local-lifecycle-fixtures/v1");
  assert.deepEqual(
    fixture.valid_cases.map(({ lifecycle_type, source }) => [lifecycle_type, source]),
    [
      ["LocalState", "accepted"],
      ["LocalState", "preparing"],
      ["LocalState", "ready"],
      ["LocalState", "executing"],
      ["LocalState", "staging_evidence"],
      ["LocalState", "cleaning_up_execution"],
      ["LocalState", "uploading"],
      ["LocalState", "finalizing_local"],
      ["LocalState", "terminal"],
      ["ExecutionOutcome", "passed"],
      ["ExecutionOutcome", "failed"],
      ["ExecutionOutcome", "cancelled"],
      ["ExecutionOutcome", "timed_out"],
      ["ExecutionOutcome", "lost"],
      ["ExecutionOutcome", "blocked"],
    ],
  );

  const executionOutcome = contractRegistry().types.ExecutionOutcome;
  assert.deepEqual(executionOutcome, {
    schema: "qa.local-lifecycle/v1",
    pointer: "#/$defs/ExecutionOutcome",
  });
  assert.deepEqual(contractRegistry().types.CancelDisposition, {
    schema: "qa.local-lifecycle/v1",
    pointer: "#/$defs/CancelDisposition",
  });
  assert.deepEqual(contractRegistry().types.EventSequence, {
    schema: "qa.local-lifecycle/v1",
    pointer: "#/$defs/EventSequence",
  });
  assert.deepEqual(contractRegistry().types.EventCursor, {
    schema: "qa.local-lifecycle/v1",
    pointer: "#/$defs/EventCursor",
  });
});

for (const fixtureCase of fixture.valid_cases) {
  test(fixtureCase.case_id, () => {
    assertValidCase(fixtureCase, fixtureCase.lifecycle_type);
  });
}

for (const fixtureCase of fixture.cancel_disposition_valid_cases) {
  test(fixtureCase.case_id, () => {
    assertValidCase(fixtureCase, "CancelDisposition");
  });
}

for (const fixtureCase of fixture.event_sequence_valid_cases) {
  test(fixtureCase.case_id, () => {
    assertValidCase(fixtureCase, "EventSequence");
  });
}

for (const fixtureCase of fixture.event_cursor_valid_cases) {
  test(fixtureCase.case_id, () => {
    assertValidCase(fixtureCase, "EventCursor");
  });
}

for (const [lifecycleType, invalidCases] of [
  ["CancelDisposition", fixture.cancel_disposition_invalid_cases],
  ["EventSequence", fixture.event_sequence_invalid_cases],
  ["EventCursor", fixture.event_cursor_invalid_cases],
] as const) {
  for (const fixtureCase of invalidCases) {
    test(fixtureCase.case_id, () => {
      assertInvalidCase(fixtureCase, lifecycleType);
    });
  }
}

for (const fixtureCase of fixture.raw_invalid_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    assert.throws(
      () =>
        validateLifecycleCase(
          fixtureCase.lifecycle_type,
          Buffer.from(fixtureCase.raw_utf8_hex, "hex"),
        ),
      (error) => rejectionMatches(error, fixtureCase.expected, fixtureCase.case_id),
    );
  });
}

for (const fixtureCase of fixture.invalid_cases) {
  test(fixtureCase.case_id, () => {
    assertInvalidCase(fixtureCase, fixtureCase.lifecycle_type);
  });
}

const preExistingRawInvalidCases = [
  {
    case_id: "local-state-malformed-json",
    lifecycle_type: "LocalState",
    raw: Buffer.from([0x22]),
    expected: { category: "validation", reason: "invalid_json", path: "/" },
  },
  {
    case_id: "local-state-invalid-utf8",
    lifecycle_type: "LocalState",
    raw: Buffer.from([0xff]),
    expected: {
      category: "canonicalization",
      code: "canonicalization.invalid_utf8",
      reason: "invalid_utf8",
      path: "/",
    },
  },
  {
    case_id: "execution-outcome-malformed-json",
    lifecycle_type: "ExecutionOutcome",
    raw: Buffer.from([0x22]),
    expected: { category: "validation", reason: "invalid_json", path: "/" },
  },
  {
    case_id: "execution-outcome-invalid-utf8",
    lifecycle_type: "ExecutionOutcome",
    raw: Buffer.from([0xff]),
    expected: {
      category: "canonicalization",
      code: "canonicalization.invalid_utf8",
      reason: "invalid_utf8",
      path: "/",
    },
  },
] as const satisfies readonly {
  readonly case_id: string;
  readonly lifecycle_type: LifecycleType;
  readonly raw: Uint8Array;
  readonly expected: Rejection;
}[];

for (const admissionCase of preExistingRawInvalidCases) {
  test(admissionCase.case_id, () => {
    console.log(`case_id=${admissionCase.case_id}`);
    assert.throws(
      () => validateLifecycleCase(admissionCase.lifecycle_type, admissionCase.raw),
      (error) => rejectionMatches(error, admissionCase.expected, admissionCase.case_id),
    );
  });
}

test("unknown lifecycle fixture type fails closed", () => {
  assert.throws(
    () => validateLifecycleCase("Unknown" as LifecycleType, Buffer.from('"passed"')),
    /unsupported lifecycle fixture type: Unknown/,
  );
});

const registryFailures: readonly (readonly [string, string, PackageMutation])[] = [
  ["mismatched schema id", "invalid_embedded_schema", (registry: RegistryJson) => {
    registry.schemas["qa.local-lifecycle/v1"]!.id = "urn:example:mismatch";
  }],
  ["unsupported schema major", "unsupported_schema_major", (registry: RegistryJson) => {
    registry.schemas["qa.local-lifecycle/v1"]!.major = 2;
  }],
  ["unknown registered ExecutionOutcome type", "unknown_registered_type", (registry: RegistryJson) => {
    delete registry.types.ExecutionOutcome;
  }],
  ["unresolved registered ExecutionOutcome pointer", "unresolved_registered_pointer", (registry: RegistryJson) => {
    registry.types.ExecutionOutcome!.pointer = "#/$defs/Missing";
  }],
  ["invalid registered ExecutionOutcome schema", "invalid_embedded_schema", (_registry, lifecycleSchema) => {
    lifecycleSchema.$defs.ExecutionOutcome = { type: "not-a-json-schema-type" };
  }],
  ["unknown registered CancelDisposition type", "unknown_registered_type", (registry: RegistryJson) => {
    delete registry.types.CancelDisposition;
  }],
  ["unresolved registered CancelDisposition pointer", "unresolved_registered_pointer", (registry: RegistryJson) => {
    registry.types.CancelDisposition!.pointer = "#/$defs/Missing";
  }],
  ["fixture-only registered CancelDisposition type", "invalid_embedded_registry", (registry: RegistryJson) => {
    registry.types.CancelDisposition!.fixture_only = true;
  }],
  ["unknown registered EventSequence type", "unknown_registered_type", (registry: RegistryJson) => {
    delete registry.types.EventSequence;
  }],
  ["unresolved registered EventSequence pointer", "unresolved_registered_pointer", (registry: RegistryJson) => {
    registry.types.EventSequence!.pointer = "#/$defs/Missing";
  }],
  ["fixture-only registered EventSequence type", "invalid_embedded_registry", (registry: RegistryJson) => {
    registry.types.EventSequence!.fixture_only = true;
  }],
  ["unknown registered EventCursor type", "unknown_registered_type", (registry: RegistryJson) => {
    delete registry.types.EventCursor;
  }],
  ["unresolved registered EventCursor pointer", "unresolved_registered_pointer", (registry: RegistryJson) => {
    registry.types.EventCursor!.pointer = "#/$defs/Missing";
  }],
  ["fixture-only registered EventCursor type", "invalid_embedded_registry", (registry: RegistryJson) => {
    registry.types.EventCursor!.fixture_only = true;
  }],
  ["escaping registered path", "invalid_embedded_schema_path", (registry: RegistryJson) => {
    registry.schemas["qa.local-lifecycle/v1"]!.path = "../schema.json";
  }],
];

for (const [name, expectedReason, mutate] of registryFailures) {
  test(`registry fails closed for ${name}`, async () => {
    await assert.rejects(importWithRegistryMutation(mutate), (error: unknown) => {
      const rejection = (error as { rejection?: { reason?: unknown } }).rejection;
      assert.equal(rejection?.reason, expectedReason);
      return true;
    });
  });
}

interface RegistryJson {
  schemas: Record<string, { path: string; id: string; major: number }>;
  types: Record<string, { schema: string; pointer: string; fixture_only?: boolean }>;
}

interface LifecycleSchemaJson {
  $defs: Record<string, unknown>;
}

type PackageMutation = (
  registry: RegistryJson,
  lifecycleSchema: LifecycleSchemaJson,
) => void;

async function importWithRegistryMutation(mutate: PackageMutation): Promise<void> {
  const packageRoot = dirname(dirname(dirname(fileURLToPath(import.meta.url))));
  const temporaryRoot = mkdtempSync(join(tmpdir(), "qa-contracts-registry-"));
  try {
    const moduleDirectory = join(temporaryRoot, "dist-test", "src");
    mkdirSync(moduleDirectory, { recursive: true });
    cpSync(join(packageRoot, "dist-test", "src", "index.js"), join(moduleDirectory, "index.js"));
    cpSync(join(packageRoot, "contracts"), join(temporaryRoot, "contracts"), { recursive: true });
    symlinkSync(join(packageRoot, "node_modules"), join(temporaryRoot, "node_modules"), "dir");
    writeFileSync(join(temporaryRoot, "package.json"), '{"type":"module"}\n');
    const registryPath = join(temporaryRoot, "contracts", "registry.json");
    const registry = JSON.parse(readFileSync(registryPath, "utf8")) as RegistryJson;
    const lifecycleSchemaPath = join(
      temporaryRoot,
      "contracts",
      "qa.local-lifecycle",
      "v1",
      "schema.json",
    );
    const lifecycleSchema = JSON.parse(
      readFileSync(lifecycleSchemaPath, "utf8"),
    ) as LifecycleSchemaJson;
    mutate(registry, lifecycleSchema);
    writeFileSync(registryPath, `${JSON.stringify(registry, null, 2)}\n`);
    writeFileSync(lifecycleSchemaPath, `${JSON.stringify(lifecycleSchema, null, 2)}\n`);
    await import(`${pathToFileURL(join(moduleDirectory, "index.js")).href}?case=${Date.now()}`);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function validateLifecycleCase(lifecycleType: LifecycleType, raw: Uint8Array) {
  switch (lifecycleType) {
    case "LocalState":
      return validateLocalState(raw);
    case "ExecutionOutcome":
      return validateExecutionOutcome(raw);
    case "CancelDisposition":
      return validateCancelDisposition(raw);
    case "EventSequence":
      return validateEventSequence(raw);
    case "EventCursor":
      return validateEventCursor(raw);
    default: {
      const unsupportedType: never = lifecycleType;
      throw new Error(`unsupported lifecycle fixture type: ${String(unsupportedType)}`);
    }
  }
}

function assertValidCase(
  fixtureCase: LifecycleCase & {
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  },
  lifecycleType: LifecycleType,
): void {
  console.log(`case_id=${fixtureCase.case_id}`);
  const validated = validateLifecycleCase(
    lifecycleType,
    Buffer.from(JSON.stringify(fixtureCase.source)),
  );
  assert.ok(validated instanceof ValidatedValue);
  assert.ok(Object.isFrozen(validated.value()));
  assert.deepEqual(validated.value(), fixtureCase.source);
  const canonical = canonicalBytes(validated);
  assert.equal(Buffer.from(canonical).toString("hex"), fixtureCase.expected_canonical_utf8_hex);
  assert.equal(
    Buffer.from(canonical).toString("base64"),
    fixtureCase.expected_canonical_utf8_base64,
  );
  assert.equal(sha256Digest(canonical), fixtureCase.expected_sha256);
}

function assertInvalidCase(
  fixtureCase: LifecycleCase & { readonly expected: Rejection },
  lifecycleType: LifecycleType,
): void {
  console.log(`case_id=${fixtureCase.case_id}`);
  assert.throws(
    () => validateLifecycleCase(lifecycleType, Buffer.from(JSON.stringify(fixtureCase.source))),
    (error) => rejectionMatches(error, fixtureCase.expected, fixtureCase.case_id),
  );
}

function rejectionMatches(error: unknown, expected: Rejection, caseId: string): boolean {
  assert.ok(error instanceof ContractError, `${caseId}: expected ContractError`);
  assert.deepEqual(error.rejection, expected, `${caseId}: rejection`);
  return true;
}
