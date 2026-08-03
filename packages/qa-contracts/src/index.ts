import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";

import type { ValidateFunction } from "ajv";
import { Ajv2020 } from "ajv/dist/2020.js";
import canonicalize from "canonicalize";
import {
  getNodeValue,
  parseTree,
  type Node as JsonNode,
  type ParseError,
} from "jsonc-parser";

const MAX_DEPTH = 128;
const MAX_SAFE_INTEGER = 9_007_199_254_740_991n;
const FOUNDATION_SCHEMA_NAME = "qa.contract-foundation/v1";
const FOUNDATION_TYPE_NAMES = Object.freeze([
  "ContractMeta",
  "HostScopedMeta",
  "ResourceRef",
  "ActorRef",
  "DigestBoundRef",
  "SignatureBlock",
  "ProjectionSpecimen",
  "StrictUnionSpecimen",
] as const);

export type FoundationType = (typeof FOUNDATION_TYPE_NAMES)[number];

interface RegistrySchemaEntry {
  readonly path: string;
  readonly id: string;
  readonly major: number;
}

interface RegistryTypeEntry {
  readonly schema: string;
  readonly pointer: string;
  readonly fixture_only?: boolean;
}

export interface ContractRegistry {
  readonly $schema: string;
  readonly registry_version: string;
  readonly profile: string;
  readonly schemas: Readonly<Record<string, RegistrySchemaEntry>>;
  readonly types: Readonly<Record<string, RegistryTypeEntry>>;
}

const REGISTRY_URL = resolvePackageFile("contracts/registry.json");
const REGISTRY = JSON.parse(readFileSync(REGISTRY_URL, "utf8")) as ContractRegistry;
validateRegistry(REGISTRY);
const SCHEMA_ENTRY = REGISTRY.schemas[FOUNDATION_SCHEMA_NAME]!;
const FOUNDATION_SCHEMA_URL = resolvePackageFile(SCHEMA_ENTRY.path);
const FOUNDATION_SCHEMA = JSON.parse(readFileSync(FOUNDATION_SCHEMA_URL, "utf8")) as Record<
  string,
  unknown
>;
if (FOUNDATION_SCHEMA.$id !== SCHEMA_ENTRY.id) {
  throw new Error("qa contract registry schema id does not match the referenced schema");
}

const AJV = new Ajv2020({ allErrors: true, strict: true, validateFormats: false });
const VALIDATORS = new Map<FoundationType, ValidateFunction>();
for (const type of FOUNDATION_TYPE_NAMES) {
  const schema = structuredClone(FOUNDATION_SCHEMA);
  delete schema.$id;
  schema.$ref = REGISTRY.types[type]!.pointer;
  VALIDATORS.set(type, AJV.compile(schema));
}

export interface Rejection {
  readonly category: "canonicalization" | "contract" | "validation";
  readonly code?: string;
  readonly reason: string;
  readonly path: string;
}

export class ContractError extends Error {
  readonly rejection: Rejection;

  constructor(rejection: Rejection) {
    super(rejection.reason);
    this.name = "ContractError";
    this.rejection = rejection;
  }
}

export class AdmittedJson {
  readonly #value: unknown;

  constructor(value: unknown, token: symbol) {
    if (token !== ADMISSION_TOKEN) throw new TypeError("AdmittedJson is created by admitJson");
    this.#value = immutableSnapshot(value);
  }

  value(): unknown {
    return this.#value;
  }
}

export class ValidatedValue {
  readonly #value: unknown;

  constructor(value: unknown, token: symbol) {
    if (token !== VALIDATION_TOKEN) {
      throw new TypeError("ValidatedValue is created by validateFoundation");
    }
    this.#value = immutableSnapshot(value);
  }

  value(): unknown {
    return this.#value;
  }
}

const ADMISSION_TOKEN = Symbol("admission");
const VALIDATION_TOKEN = Symbol("validation");

export function contractRegistry(): ContractRegistry {
  return immutableSnapshot(REGISTRY) as ContractRegistry;
}

export function foundationTypeNames(): readonly FoundationType[] {
  return FOUNDATION_TYPE_NAMES;
}

export function admitJson(raw: Uint8Array): AdmittedJson {
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
  } catch {
    throw canonicalError("canonicalization.invalid_utf8", "invalid_utf8");
  }
  preflightDepth(text);
  preflightNumbers(text);
  const errors: ParseError[] = [];
  const root = parseTree(text, errors, { allowTrailingComma: false, disallowComments: true });
  if (errors.length > 0 || root === undefined) {
    throw new ContractError({ category: "validation", reason: "invalid_json", path: "/" });
  }
  inspectNode(root, text, 0);
  return new AdmittedJson(getNodeValue(root), ADMISSION_TOKEN);
}

export function validateFoundation(raw: Uint8Array, type: FoundationType): ValidatedValue {
  return validateValue(admitJson(raw), type);
}

export function validateValue(admitted: AdmittedJson, type: FoundationType): ValidatedValue {
  const value = admitted.value();
  validateSpecialRules(value, type);
  const validator = VALIDATORS.get(type);
  if (validator === undefined) {
    throw new ContractError({ category: "validation", reason: "unknown_foundation_type", path: "/" });
  }
  if (!validator(value)) {
    const path = validator.errors?.[0]?.instancePath || "/";
    throw new ContractError({ category: "validation", reason: "schema_violation", path });
  }
  return new ValidatedValue(value, VALIDATION_TOKEN);
}

export function validateScalar(
  name: "ISO8601" | "Sha256" | "Base64UrlNoPad" | "UUID" | "SchemaVersion",
  value: string,
): void {
  let valid: boolean;
  switch (name) {
    case "ISO8601":
      valid = validateIso8601(value);
      break;
    case "Sha256":
      valid = /^sha256:[0-9a-f]{64}$/.test(value);
      break;
    case "Base64UrlNoPad":
      valid = validateBase64UrlNoPad(value);
      break;
    case "UUID":
      valid = validateCanonicalUuid(value);
      break;
    case "SchemaVersion":
      valid = parseSchemaMajor(value) !== undefined;
      break;
  }
  if (!valid) {
    if (name === "Base64UrlNoPad") {
      throw contractError("contract.invalid_encoding", "invalid_encoding", "/");
    }
    throw new ContractError({ category: "validation", reason: "invalid_scalar", path: "/" });
  }
}

export function canonicalBytes(value: ValidatedValue): Uint8Array {
  return canonicalizeUnknown(value.value());
}

export function canonicalAdmittedBytes(value: AdmittedJson): Uint8Array {
  return canonicalizeUnknown(value.value());
}

export function sha256Digest(bytes: Uint8Array): string {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

export function contractContentProjection(value: ValidatedValue): Uint8Array {
  const input = value.value();
  if (!isRecord(input)) {
    throw new ContractError({
      category: "validation",
      reason: "projection_requires_object",
      path: "/",
    });
  }
  const projected = structuredClone(input);
  delete projected.content_digest;
  delete projected.signature;
  return canonicalizeUnknown(projected);
}

export function contractContentDigest(value: ValidatedValue): string {
  return sha256Digest(contractContentProjection(value));
}

export function verifyContractContentDigest(value: ValidatedValue): void {
  const input = value.value();
  if (!isRecord(input) || typeof input.content_digest !== "string") {
    throw new ContractError({
      category: "validation",
      reason: "missing_content_digest",
      path: "/content_digest",
    });
  }
  if (input.content_digest !== contractContentDigest(value)) {
    throw contractError("contract.digest_mismatch", "digest_mismatch", "/content_digest");
  }
}

function resolvePackageFile(relativePath: string): URL {
  if (
    relativePath.startsWith("/") ||
    relativePath.includes("..") ||
    relativePath.includes(":")
  ) {
    throw new Error(`invalid qa contract package path: ${relativePath}`);
  }
  for (const root of [new URL("../", import.meta.url), new URL("../../", import.meta.url)]) {
    const candidate = new URL(relativePath, root);
    if (existsSync(candidate)) return candidate;
  }
  throw new Error(`qa contract package file not found: ${relativePath}`);
}

function validateRegistry(registry: ContractRegistry): void {
  if (registry.registry_version !== "qa.contract-registry/v1") {
    throw new Error("unsupported qa contract registry version");
  }
  if (registry.profile !== "local_qa_host_mvp") {
    throw new Error("qa contract registry has the wrong profile authority");
  }
  const schema = registry.schemas[FOUNDATION_SCHEMA_NAME];
  if (
    schema === undefined ||
    schema.major !== 1 ||
    schema.path !== "contracts/qa.contract-foundation/v1/schema.json" ||
    schema.id !== "urn:chronoai:fkst:qa-contracts:qa.contract-foundation:v1"
  ) {
    throw new Error("qa contract foundation registry entry is invalid");
  }
  const registeredTypes = Object.keys(registry.types).sort();
  const implementationTypes = [...FOUNDATION_TYPE_NAMES].sort();
  if (registeredTypes.join("\n") !== implementationTypes.join("\n")) {
    throw new Error("qa contract registry and TypeScript foundation types differ");
  }
  for (const type of FOUNDATION_TYPE_NAMES) {
    const entry = registry.types[type];
    if (
      entry === undefined ||
      entry.schema !== FOUNDATION_SCHEMA_NAME ||
      entry.pointer !== `#/$defs/${type}`
    ) {
      throw new Error(`qa contract registry type entry is invalid: ${type}`);
    }
    const fixtureOnly = type === "ProjectionSpecimen" || type === "StrictUnionSpecimen";
    if ((entry.fixture_only === true) !== fixtureOnly) {
      throw new Error(`qa contract fixture-only marker is invalid: ${type}`);
    }
  }
}

function inspectNode(node: JsonNode, text: string, depth: number): void {
  if (depth > MAX_DEPTH) {
    throw new ContractError({ category: "validation", reason: "depth_overflow", path: "/" });
  }
  if (node.type === "string" && typeof node.value === "string" && hasLoneSurrogate(node.value)) {
    throw canonicalError("canonicalization.invalid_unicode_scalar", "invalid_unicode_scalar");
  }
  if (node.type === "number") checkNumberToken(text.slice(node.offset, node.offset + node.length));
  if (node.type === "object") {
    const names = new Set<string>();
    for (const property of node.children ?? []) {
      const keyNode = property.children?.[0];
      const valueNode = property.children?.[1];
      if (keyNode === undefined || valueNode === undefined || typeof keyNode.value !== "string") {
        continue;
      }
      if (hasLoneSurrogate(keyNode.value)) {
        throw canonicalError("canonicalization.invalid_unicode_scalar", "invalid_unicode_scalar");
      }
      if (names.has(keyNode.value)) {
        throw canonicalError("canonicalization.duplicate_member", "duplicate_member");
      }
      names.add(keyNode.value);
      inspectNode(valueNode, text, depth + 1);
    }
    return;
  }
  for (const child of node.children ?? []) inspectNode(child, text, depth + 1);
}

function preflightDepth(text: string): void {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (const char of text) {
    if (inString) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') inString = false;
      continue;
    }
    if (char === '"') {
      inString = true;
      continue;
    }
    if (char === "{" || char === "[") {
      depth += 1;
      if (depth > MAX_DEPTH) {
        throw new ContractError({ category: "validation", reason: "depth_overflow", path: "/" });
      }
    } else if (char === "}" || char === "]") {
      depth -= 1;
    }
  }
}

function preflightNumbers(text: string): void {
  let index = 0;
  let inString = false;
  let escaped = false;
  while (index < text.length) {
    const char = text[index]!;
    if (inString) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') inString = false;
      index += 1;
      continue;
    }
    if (char === '"') {
      inString = true;
      index += 1;
      continue;
    }
    if (/[0-9+\-NI]/.test(char)) {
      const start = index;
      while (index < text.length && !/[\s,\]}:]/.test(text[index]!)) index += 1;
      checkNumberToken(text.slice(start, index));
      continue;
    }
    index += 1;
  }
}

function checkNumberToken(token: string): void {
  if (!/^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$/.test(token)) {
    throw canonicalError("canonicalization.invalid_json_number", "invalid_json_number");
  }
  if (!Number.isFinite(Number(token))) {
    throw canonicalError("canonicalization.invalid_json_number", "invalid_json_number");
  }
  if (!/[.eE]/.test(token)) {
    let magnitude: bigint;
    try {
      magnitude = BigInt(token.startsWith("-") ? token.slice(1) : token);
    } catch {
      throw canonicalError("canonicalization.invalid_json_number", "invalid_json_number");
    }
    if (magnitude > MAX_SAFE_INTEGER) {
      throw canonicalError("canonicalization.unsafe_integer", "unsafe_integer");
    }
  }
}

function validateSpecialRules(value: unknown, type: FoundationType): void {
  if (!isRecord(value)) {
    throw new ContractError({ category: "validation", reason: "expected_object", path: "/" });
  }
  const allowed = allowedFields(value, type);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw contractError("contract.forbidden_field", "unknown_field", pointer(key));
    }
  }
  if (typeof value.schema_version === "string") {
    const major = parseSchemaMajor(value.schema_version);
    if (major === undefined) {
      throw new ContractError({
        category: "validation",
        reason: "invalid_schema_version",
        path: "/schema_version",
      });
    }
    if (major !== 1n) {
      throw contractError(
        "contract.unsupported_version",
        "unsupported_version",
        "/schema_version",
      );
    }
  }
  switch (type) {
    case "ContractMeta":
      validateMetaScalars(value, true);
      break;
    case "HostScopedMeta":
      validateMetaScalars(value, false);
      break;
    case "ActorRef":
      validateClosedEnum(value, "type", ["user", "service", "device", "module"]);
      break;
    case "SignatureBlock":
      validateClosedEnum(value, "algorithm", ["ed25519", "es256"]);
      if (typeof value.value === "string") {
        validateScalarAt("Base64UrlNoPad", value.value, "/value");
      }
      break;
    case "DigestBoundRef":
      if (typeof value.content_digest === "string") {
        validateScalarAt("Sha256", value.content_digest, "/content_digest");
      }
      break;
    case "ProjectionSpecimen":
      if (typeof value.content_digest === "string") {
        validateScalarAt("Sha256", value.content_digest, "/content_digest");
      }
      break;
    case "ResourceRef":
      if (typeof value.digest === "string") {
        validateScalarAt("Sha256", value.digest, "/digest");
      }
      break;
    case "StrictUnionSpecimen":
      validateStrictUnion(value);
      break;
  }
}

function allowedFields(value: Record<string, unknown>, type: FoundationType): Set<string> {
  switch (type) {
    case "ContractMeta":
      return new Set([
        "schema_version",
        "content_digest",
        "run_id",
        "created_at",
        "producer_version",
        "correlation_id",
      ]);
    case "HostScopedMeta":
      return new Set([
        "schema_version",
        "content_digest",
        "host_instance_id",
        "created_at",
        "producer_version",
        "correlation_id",
      ]);
    case "ResourceRef":
      return new Set(["kind", "id", "digest", "version"]);
    case "ActorRef":
      return new Set(["type", "id", "display_name"]);
    case "DigestBoundRef":
      return new Set(["kind", "id", "schema_version", "content_digest", "version"]);
    case "SignatureBlock":
      return new Set(["algorithm", "key_id", "value"]);
    case "ProjectionSpecimen":
      return new Set(["schema_version", "content_digest", "signature", "payload"]);
    case "StrictUnionSpecimen":
      return strictUnionAllowedFields(value);
  }
}

function strictUnionAllowedFields(value: Record<string, unknown>): Set<string> {
  if (typeof value.kind !== "string") {
    throw contractError("contract.invalid_variant", "missing_required_field", "/kind");
  }
  if (value.kind !== "alpha" && value.kind !== "beta") {
    throw contractError("contract.invalid_variant", "unknown_discriminator", "/kind");
  }
  const other = value.kind === "alpha" ? "beta_count" : "alpha_value";
  if (other in value) {
    throw contractError("contract.forbidden_field", "mixed_variant_fields", pointer(other));
  }
  return value.kind === "alpha"
    ? new Set(["kind", "common", "alpha_value"])
    : new Set(["kind", "common", "beta_count"]);
}

function validateStrictUnion(value: Record<string, unknown>): void {
  const required = value.kind === "alpha" ? ["common", "alpha_value"] : ["common", "beta_count"];
  for (const field of required) {
    if (!(field in value)) {
      throw contractError("contract.invalid_variant", "missing_required_field", pointer(field));
    }
  }
}

function validateMetaScalars(value: Record<string, unknown>, runScoped: boolean): void {
  const scalars = [
    ["schema_version", "SchemaVersion"],
    ["content_digest", "Sha256"],
    ["created_at", "ISO8601"],
  ] as const;
  for (const [field, scalar] of scalars) {
    if (typeof value[field] === "string") {
      validateScalarAt(scalar, value[field], pointer(field));
    }
  }
  if (runScoped && typeof value.run_id === "string") {
    validateScalarAt("UUID", value.run_id, "/run_id");
  }
}

function validateScalarAt(
  name: Parameters<typeof validateScalar>[0],
  value: string,
  path: string,
): void {
  try {
    validateScalar(name, value);
  } catch (error) {
    if (error instanceof ContractError) {
      throw new ContractError({ ...error.rejection, path });
    }
    throw error;
  }
}

function validateClosedEnum(
  value: Record<string, unknown>,
  field: string,
  allowed: readonly string[],
): void {
  if (typeof value[field] === "string" && !allowed.includes(value[field])) {
    throw contractError("contract.unsupported_enum", "unsupported_enum", pointer(field));
  }
}

function validateIso8601(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d*[1-9]))?Z$/.exec(
    value,
  );
  if (match === null) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  if (month < 1 || month > 12 || hour > 23 || minute > 59 || second > 59) return false;
  const leapYear = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const daysInMonth = [31, leapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return day >= 1 && day <= daysInMonth[month - 1]!;
}

function validateBase64UrlNoPad(value: string): boolean {
  if (value.includes("=") || !/^[A-Za-z0-9_-]*$/.test(value) || value.length % 4 === 1) {
    return false;
  }
  try {
    return Buffer.from(value, "base64url").toString("base64url") === value;
  } catch {
    return false;
  }
}

function validateCanonicalUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);
}

function parseSchemaMajor(value: string): bigint | undefined {
  const match = /^qa\.[a-z0-9]+(?:-[a-z0-9]+)*\/v([1-9][0-9]*)$/.exec(value);
  if (match?.[1] === undefined) return undefined;
  try {
    return BigInt(match[1]);
  } catch {
    return undefined;
  }
}

function canonicalizeUnknown(value: unknown): Uint8Array {
  const result = canonicalize(value);
  if (result === undefined) {
    throw new ContractError({
      category: "validation",
      reason: "canonicalization_failed",
      path: "/",
    });
  }
  return new TextEncoder().encode(result);
}

function immutableSnapshot(value: unknown): unknown {
  return deepFreeze(structuredClone(value));
}

function deepFreeze(value: unknown): unknown {
  if (Array.isArray(value)) {
    for (const item of value) deepFreeze(item);
    return Object.freeze(value);
  }
  if (isRecord(value)) {
    for (const item of Object.values(value)) deepFreeze(item);
    return Object.freeze(value);
  }
  return value;
}

function hasLoneSurrogate(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return true;
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true;
    }
  }
  return false;
}

function canonicalError(code: string, reason: string): ContractError {
  return new ContractError({ category: "canonicalization", code, reason, path: "/" });
}

function contractError(code: string, reason: string, path: string): ContractError {
  return new ContractError({ category: "contract", code, reason, path });
}

function pointer(field: string): string {
  return `/${field.replaceAll("~", "~0").replaceAll("/", "~1")}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
