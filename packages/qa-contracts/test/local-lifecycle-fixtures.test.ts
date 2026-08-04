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

import { canonicalBytes, sha256Digest, validateLocalState } from "../src/index.js";

interface LifecycleFixture {
  readonly schema_version: string;
  readonly valid_cases: readonly {
    readonly case_id: string;
    readonly source: string;
    readonly expected_canonical_utf8_hex: string;
    readonly expected_canonical_utf8_base64: string;
    readonly expected_sha256: string;
  }[];
}

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../../fixtures/qa/local-lifecycle-v1.json", import.meta.url),
    "utf8",
  ),
) as LifecycleFixture;

test("local lifecycle fixture metadata", () => {
  assert.equal(fixture.schema_version, "qa.local-lifecycle-fixtures/v1");
});

for (const fixtureCase of fixture.valid_cases) {
  test(fixtureCase.case_id, () => {
    console.log(`case_id=${fixtureCase.case_id}`);
    const validated = validateLocalState(Buffer.from(JSON.stringify(fixtureCase.source)));
    assert.equal(validated.value(), "accepted");
    const canonical = canonicalBytes(validated);
    assert.equal(Buffer.from(canonical).toString("hex"), fixtureCase.expected_canonical_utf8_hex);
    assert.equal(
      Buffer.from(canonical).toString("base64"),
      fixtureCase.expected_canonical_utf8_base64,
    );
    assert.equal(sha256Digest(canonical), fixtureCase.expected_sha256);
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
