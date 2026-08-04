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
  ContractError,
  type Rejection,
  sha256Digest,
  validateLocalState,
} from "../src/index.js";

interface LifecycleCase {
  readonly case_id: string;
  readonly source: unknown;
}

interface LifecycleFixture {
  readonly schema_version: string;
  readonly valid_cases: readonly (LifecycleCase & {
    readonly source: string;
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  })[];
  readonly invalid_cases: readonly (LifecycleCase & {
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
    fixture.valid_cases.map((fixtureCase) => fixtureCase.source),
    [
      "accepted",
      "preparing",
      "ready",
      "executing",
      "staging_evidence",
      "cleaning_up_execution",
      "uploading",
      "finalizing_local",
      "terminal",
    ],
  );
});

for (const fixtureCase of fixture.valid_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    const validated = validateLocalState(Buffer.from(JSON.stringify(fixtureCase.source)));
    assert.equal(validated.value(), fixtureCase.source);
    const canonical = canonicalBytes(validated);
    assert.equal(Buffer.from(canonical).toString("hex"), fixtureCase.expected_canonical_utf8_hex);
    assert.equal(
      Buffer.from(canonical).toString("base64"),
      fixtureCase.expected_canonical_utf8_base64,
    );
    assert.equal(sha256Digest(canonical), fixtureCase.expected_sha256);
  });
}

for (const fixtureCase of fixture.invalid_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    assert.throws(
      () => validateLocalState(Buffer.from(JSON.stringify(fixtureCase.source))),
      (error) => rejectionMatches(error, fixtureCase.expected, fixtureCase.case_id),
    );
  });
}

const admissionCases = [
  {
    case_id: "local-state-malformed-json",
    raw: Buffer.from([0x22]),
    expected: { category: "validation", reason: "invalid_json", path: "/" },
  },
  {
    case_id: "local-state-invalid-utf8",
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
  readonly raw: Uint8Array;
  readonly expected: Rejection;
}[];

for (const admissionCase of admissionCases) {
  test(admissionCase.case_id, () => {
    console.log(`case_id=${admissionCase.case_id}`);
    assert.throws(
      () => validateLocalState(admissionCase.raw),
      (error) => rejectionMatches(error, admissionCase.expected, admissionCase.case_id),
    );
  });
}

const registryFailures = [
  ["mismatched schema id", (registry: RegistryJson) => {
    registry.schemas["qa.local-lifecycle/v1"]!.id = "urn:example:mismatch";
  }],
  ["unsupported schema major", (registry: RegistryJson) => {
    registry.schemas["qa.local-lifecycle/v1"]!.major = 2;
  }],
  ["unknown registered type", (registry: RegistryJson) => {
    delete registry.types.LocalState;
  }],
  ["unresolved registered pointer", (registry: RegistryJson) => {
    registry.types.LocalState!.pointer = "#/$defs/Missing";
  }],
  ["escaping registered path", (registry: RegistryJson) => {
    registry.schemas["qa.local-lifecycle/v1"]!.path = "../schema.json";
  }],
] as const;

for (const [name, mutate] of registryFailures) {
  test(`registry fails closed for ${name}`, async () => {
    await assert.rejects(importWithRegistryMutation(mutate));
  });
}

interface RegistryJson {
  schemas: Record<string, { path: string; id: string; major: number }>;
  types: Record<string, { schema: string; pointer: string; fixture_only?: boolean }>;
}

async function importWithRegistryMutation(mutate: (registry: RegistryJson) => void): Promise<void> {
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
    mutate(registry);
    writeFileSync(registryPath, `${JSON.stringify(registry, null, 2)}\n`);
    await import(`${pathToFileURL(join(moduleDirectory, "index.js")).href}?case=${Date.now()}`);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function rejectionMatches(error: unknown, expected: Rejection, caseId: string): boolean {
  assert.ok(error instanceof ContractError, `${caseId}: expected ContractError`);
  assert.deepEqual(error.rejection, expected, `${caseId}: rejection`);
  return true;
}
