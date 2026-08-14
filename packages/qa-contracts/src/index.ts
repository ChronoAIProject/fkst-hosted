import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { basename, dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

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
const SUPPORTED_SCHEMA_MAJOR = 1;
export const LOCAL_WORKER_MAX_FRAME_BYTES = 65_536;
const LOCAL_STATE_TYPE_NAME = "LocalState";
const EXECUTION_OUTCOME_TYPE_NAME = "ExecutionOutcome";
const CANCEL_DISPOSITION_TYPE_NAME = "CancelDisposition";
const EVENT_SEQUENCE_TYPE_NAME = "EventSequence";
const EVENT_CURSOR_TYPE_NAME = "EventCursor";
const LIFECYCLE_TYPE_NAMES = Object.freeze([
  LOCAL_STATE_TYPE_NAME,
  EXECUTION_OUTCOME_TYPE_NAME,
  CANCEL_DISPOSITION_TYPE_NAME,
  EVENT_SEQUENCE_TYPE_NAME,
  EVENT_CURSOR_TYPE_NAME,
] as const);
const LOCAL_SANITIZED_OBSERVATION_TYPE_NAME = "LocalSanitizedObservation";
const LOCAL_EVIDENCE_OBJECT_TYPE_NAME = "LocalEvidenceObject";
const LOCAL_SANITIZED_OBSERVATION_REF_TYPE_NAME = "LocalSanitizedObservationRef";
const LOCAL_EVIDENCE_OBJECT_REF_TYPE_NAME = "LocalEvidenceObjectRef";
const LOCAL_EVIDENCE_TYPE_NAMES = Object.freeze([
  LOCAL_SANITIZED_OBSERVATION_TYPE_NAME,
  LOCAL_EVIDENCE_OBJECT_TYPE_NAME,
  LOCAL_SANITIZED_OBSERVATION_REF_TYPE_NAME,
  LOCAL_EVIDENCE_OBJECT_REF_TYPE_NAME,
] as const);
const LOCAL_WORKER_TYPE_NAMES = Object.freeze([
  "LocalWorkerFrame",
  "LocalWorkerInvocation",
  "LocalWorkerCapabilityRequest",
  "LocalWorkerCapabilityResult",
  "LocalWorkerTerminalResult",
  "LocalWorkerProtocolFailure",
] as const);
const LOCAL_QA_RUN_REQUEST_TYPE_NAME = "LocalQARunRequest";
const RUN_ACCEPTANCE_TYPE_NAME = "RunAcceptance";
const LOCAL_RUN_ADMISSION_TYPE_NAMES = Object.freeze([
  LOCAL_QA_RUN_REQUEST_TYPE_NAME,
  RUN_ACCEPTANCE_TYPE_NAME,
] as const);
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

export interface LocalSanitizedObservation {
  readonly schema_version: "qa.local-evidence/v1";
  readonly run_id: string;
  readonly attempt: number;
  readonly fixture_url: string;
  readonly final_url: string;
  readonly selector: '[data-local-qa="status"]';
  readonly expected_text: "READY";
  readonly observed_text: "READY";
}

export interface LocalEvidenceObject {
  readonly schema_version: "qa.local-evidence/v1";
  readonly run_id: string;
  readonly attempt: number;
  readonly object_id: string;
  readonly role: "browser-screenshot" | "runner-log";
  readonly media_type: "image/png" | "text/plain; charset=utf-8";
  readonly byte_length: number;
  readonly sha256: string;
  readonly ownership: "local-only:not-uploadable";
}

export interface LocalSanitizedObservationRef {
  readonly kind: "local-sanitized-observation";
  readonly id: string;
  readonly schema_version: "qa.local-evidence/v1";
  readonly content_digest: string;
  readonly version?: string;
}

export interface LocalEvidenceObjectRef {
  readonly kind: "local-evidence-object";
  readonly id: string;
  readonly schema_version: "qa.local-evidence/v1";
  readonly content_digest: string;
  readonly version?: string;
}

export interface DigestBoundIdentity {
  readonly kind: string;
  readonly id: string;
  readonly schema_version: string;
  readonly content_digest: string;
}

export interface LocalQARunRequest {
  readonly schema_version: "qa.local-run-admission/v1";
  readonly content_digest: string;
  readonly run_id: string;
  readonly created_at: string;
  readonly producer_version: string;
  readonly idempotency_key: string;
  readonly nonce: string;
  readonly expires_at: string;
  readonly profile: "local_qa_agent_mvp";
  readonly source: DigestBoundIdentity & { readonly kind: "source" };
  readonly structured_plan: DigestBoundIdentity & { readonly kind: "structured-plan" };
  readonly environment_profile: DigestBoundIdentity & { readonly kind: "environment-profile" };
  readonly device: DigestBoundIdentity & { readonly kind: "device" };
  readonly nyxid_node: DigestBoundIdentity & { readonly kind: "nyxid-node" };
  readonly host_installation: DigestBoundIdentity & { readonly kind: "host-installation" };
  readonly authorization: DigestBoundIdentity & {
    readonly kind: "local-credential-authorization";
  };
}

export interface RunAcceptance {
  readonly schema_version: "qa.local-run-admission/v1";
  readonly content_digest: string;
  readonly run_id: string;
  readonly created_at: string;
  readonly producer_version: string;
  readonly request_digest: string;
  readonly idempotency_key: string;
  readonly state: "accepted";
  readonly accepted_at: string;
}

const PACKAGE_ROOT = realpathSync(resolvePackageRoot());

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

const REGISTRY_URL = resolvePackageFile("contracts/registry.json");
const REGISTRY = JSON.parse(readFileSync(REGISTRY_URL, "utf8")) as ContractRegistry;
validateRegistry(REGISTRY);

const AJV = new Ajv2020({ allErrors: true, strict: true, validateFormats: false });
const VALIDATORS = new Map<string, ValidateFunction>();
for (const type of FOUNDATION_TYPE_NAMES) {
  VALIDATORS.set(type, compileRegisteredValidator(REGISTRY, type));
}
for (const type of LIFECYCLE_TYPE_NAMES) {
  VALIDATORS.set(type, compileRegisteredValidator(REGISTRY, type));
}
for (const type of LOCAL_EVIDENCE_TYPE_NAMES) {
  VALIDATORS.set(type, compileRegisteredValidator(REGISTRY, type));
}
for (const type of LOCAL_WORKER_TYPE_NAMES) {
  VALIDATORS.set(type, compileRegisteredValidator(REGISTRY, type));
}
for (const type of LOCAL_RUN_ADMISSION_TYPE_NAMES) {
  VALIDATORS.set(type, compileRegisteredValidator(REGISTRY, type));
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
      throw new TypeError("ValidatedValue is created by a contract validator");
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
  if (raw.length >= 3 && raw[0] === 0xef && raw[1] === 0xbb && raw[2] === 0xbf) {
    throw validationError("invalid_json", "/");
  }
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
  } catch {
    throw canonicalError("canonicalization.invalid_utf8", "invalid_utf8");
  }
  if (/^\s|\s$/u.test(text)) {
    throw validationError("invalid_json", "/");
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

export function validateLocalState(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), LOCAL_STATE_TYPE_NAME);
}

export function validateExecutionOutcome(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), EXECUTION_OUTCOME_TYPE_NAME);
}

export function validateCancelDisposition(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), CANCEL_DISPOSITION_TYPE_NAME);
}

export function validateEventSequence(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), EVENT_SEQUENCE_TYPE_NAME);
}

export function validateEventCursor(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), EVENT_CURSOR_TYPE_NAME);
}

export function validateLocalSanitizedObservation(raw: Uint8Array): ValidatedValue {
  const validated = validateRegisteredValue(admitJson(raw), LOCAL_SANITIZED_OBSERVATION_TYPE_NAME);
  validateLocalSanitizedObservationRules(validated.value() as LocalSanitizedObservation);
  return validated;
}

export function validateLocalEvidenceObject(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), LOCAL_EVIDENCE_OBJECT_TYPE_NAME);
}

export function validateLocalSanitizedObservationRef(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), LOCAL_SANITIZED_OBSERVATION_REF_TYPE_NAME);
}

export function validateLocalEvidenceObjectRef(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), LOCAL_EVIDENCE_OBJECT_REF_TYPE_NAME);
}

export function validateLocalWorkerFrame(raw: Uint8Array): ValidatedValue {
  const validated = validateLocalWorkerRegistered(raw, "LocalWorkerFrame");
  validateLocalWorkerRules(validated.value());
  return validated;
}

export function validateLocalWorkerInvocation(raw: Uint8Array): ValidatedValue {
  const validated = validateLocalWorkerRegistered(raw, "LocalWorkerInvocation");
  validateLocalWorkerRules(validated.value());
  return validated;
}

export function validateLocalWorkerCapabilityRequest(raw: Uint8Array): ValidatedValue {
  const validated = validateLocalWorkerRegistered(raw, "LocalWorkerCapabilityRequest");
  validateLocalWorkerRules(validated.value());
  return validated;
}

export function validateLocalWorkerCapabilityResult(raw: Uint8Array): ValidatedValue {
  const validated = validateLocalWorkerRegistered(raw, "LocalWorkerCapabilityResult");
  validateLocalWorkerRules(validated.value());
  return validated;
}

export function validateLocalWorkerTerminalResult(raw: Uint8Array): ValidatedValue {
  const validated = validateLocalWorkerRegistered(raw, "LocalWorkerTerminalResult");
  validateLocalWorkerRules(validated.value());
  return validated;
}

export function validateLocalWorkerProtocolFailure(raw: Uint8Array): ValidatedValue {
  return validateRegisteredValue(admitJson(raw), "LocalWorkerProtocolFailure");
}

export function validateLocalQARunRequest(raw: Uint8Array): ValidatedValue {
  return validateLocalQARunRequestValue(admitJson(raw));
}

export function validateRunAcceptance(raw: Uint8Array): ValidatedValue {
  return validateRunAcceptanceValue(admitJson(raw));
}

export function buildInitialRunAcceptance(
  request: ValidatedValue,
  acceptedAt: string,
  producerVersion: string,
): ValidatedValue {
  const validatedRequest = validateLocalQARunRequestValue(
    new AdmittedJson(request.value(), ADMISSION_TOKEN),
  );
  const requestValue = validatedRequest.value() as LocalQARunRequest;
  if (!validateIso8601(acceptedAt)) {
    throw validationError("schema_violation", "/accepted_at");
  }
  if (producerVersion.length === 0) {
    throw validationError("schema_violation", "/producer_version");
  }
  if (
    compareIso8601(acceptedAt, requestValue.created_at) < 0 ||
    compareIso8601(acceptedAt, requestValue.expires_at) >= 0
  ) {
    throw contractError("contract.invalid_relation", "accepted_at_out_of_window", "/accepted_at");
  }
  const acceptance: Record<string, unknown> = {
    schema_version: "qa.local-run-admission/v1",
    run_id: requestValue.run_id,
    created_at: acceptedAt,
    producer_version: producerVersion,
    request_digest: requestValue.content_digest,
    idempotency_key: requestValue.idempotency_key,
    state: "accepted",
    accepted_at: acceptedAt,
  };
  const projected = new ValidatedValue(acceptance, VALIDATION_TOKEN);
  acceptance.content_digest = contractContentDigest(projected);
  return validateRunAcceptance(canonicalizeUnknown(acceptance));
}

export function encodeLocalWorkerFrame(value: ValidatedValue): Uint8Array {
  const payload = canonicalBytes(value);
  if (payload.length < 1 || payload.length > LOCAL_WORKER_MAX_FRAME_BYTES) {
    throw new ContractError({ category: "validation", reason: "frame_length_out_of_range", path: "/" });
  }
  const frame = new Uint8Array(4 + payload.length);
  new DataView(frame.buffer).setUint32(0, payload.length, false);
  frame.set(payload, 4);
  return frame;
}

const LOCAL_WORKER_CAPABILITY_SEQUENCE = Object.freeze([
  "clock.now/v1",
  "clock.monotonic-ms/v1",
  "browser-session.run/v1",
  "evidence.stage-fixed-runner-log/v1",
  "browser-session.close/v1",
  "clock.now/v1",
  "clock.monotonic-ms/v1",
] as const);

export class LocalWorkerInputSequence {
  #invocationId: string | undefined;
  #nextFrame = 0;

  accept(frame: ValidatedValue): void {
    const value = frame.value();
    if (!isRecord(value)) {
      throw new ContractError({ category: "validation", reason: "invalid_sequence", path: "/" });
    }
    if (this.#nextFrame === 0) {
      if (value.kind !== "invocation" || typeof value.invocation_id !== "string") {
        throw new ContractError({ category: "validation", reason: "invalid_sequence", path: "/kind" });
      }
      this.#invocationId = value.invocation_id;
      this.#nextFrame = 1;
      return;
    }
    if (this.#nextFrame > LOCAL_WORKER_CAPABILITY_SEQUENCE.length) {
      throw new ContractError({ category: "validation", reason: "trailing_input", path: "/" });
    }
    if (value.kind !== "capability_result") {
      throw new ContractError({ category: "validation", reason: "invalid_sequence", path: "/kind" });
    }
    const index = this.#nextFrame - 1;
    const expectedCapability = LOCAL_WORKER_CAPABILITY_SEQUENCE[index];
    if (
      value.invocation_id !== this.#invocationId ||
      value.request_id !== `capability/${index}` ||
      value.capability !== expectedCapability
    ) {
      throw contractError("contract.invalid_relation", "capability_mismatch", "/request_id");
    }
    this.#nextFrame += 1;
  }

  finish(): void {
    if (this.#nextFrame !== LOCAL_WORKER_CAPABILITY_SEQUENCE.length + 1) {
      throw new ContractError({ category: "validation", reason: "unexpected_eof", path: "/" });
    }
  }
}

export class LocalWorkerFrameDecoder {
  #buffer = new Uint8Array(0);

  bufferedBytes(): number {
    return this.#buffer.length;
  }

  push(chunk: Uint8Array): readonly ValidatedValue[] {
    if (chunk.length > 0) {
      const combined = new Uint8Array(this.#buffer.length + chunk.length);
      combined.set(this.#buffer);
      combined.set(chunk, this.#buffer.length);
      this.#buffer = combined;
    }
    const frames: ValidatedValue[] = [];
    let offset = 0;
    while (this.#buffer.length - offset >= 4) {
      const length = new DataView(this.#buffer.buffer, this.#buffer.byteOffset + offset, 4).getUint32(0, false);
      if (length < 1 || length > LOCAL_WORKER_MAX_FRAME_BYTES) {
        throw new ContractError({ category: "validation", reason: "frame_length_out_of_range", path: "/" });
      }
      if (this.#buffer.length - offset - 4 < length) break;
      frames.push(validateLocalWorkerFrame(this.#buffer.slice(offset + 4, offset + 4 + length)));
      offset += 4 + length;
    }
    if (offset > 0) this.#buffer = this.#buffer.slice(offset);
    return frames;
  }

  finish(): void {
    if (this.#buffer.length !== 0) {
      throw new ContractError({ category: "validation", reason: "truncated_frame", path: "/" });
    }
  }
}

export function validateValue(admitted: AdmittedJson, type: FoundationType): ValidatedValue {
  const value = admitted.value();
  validateSpecialRules(value, type);
  return validateRegisteredValue(admitted, type);
}

function validateRegisteredValue(admitted: AdmittedJson, typeName: string): ValidatedValue {
  const value = admitted.value();
  const validator = VALIDATORS.get(typeName);
  if (validator === undefined) {
    throw new ContractError({ category: "validation", reason: "unknown_registered_type", path: "/" });
  }
  if (!validator(value)) {
    const path = validator.errors?.[0]?.instancePath || "/";
    throw new ContractError({ category: "validation", reason: "schema_violation", path });
  }
  return new ValidatedValue(value, VALIDATION_TOKEN);
}

function validateLocalQARunRequestValue(admitted: AdmittedJson): ValidatedValue {
  const admittedValue = admitted.value();
  let validated: ValidatedValue;
  try {
    validated = validateRegisteredValue(admitted, LOCAL_QA_RUN_REQUEST_TYPE_NAME);
  } catch (error) {
    if (
      error instanceof ContractError &&
      error.rejection.reason === "schema_violation" &&
      isRecord(admittedValue) &&
      typeof admittedValue.nonce === "string" &&
      !validateBase64UrlNoPad(admittedValue.nonce)
    ) {
      throw contractError("contract.invalid_encoding", "invalid_encoding", "/nonce");
    }
    throw error;
  }
  const value = validated.value() as LocalQARunRequest;
  if (!validateIso8601(value.created_at)) {
    throw validationError("schema_violation", "/created_at");
  }
  if (!validateIso8601(value.expires_at)) {
    throw validationError("schema_violation", "/expires_at");
  }
  if (!validateBase64UrlNoPad(value.nonce)) {
    throw contractError("contract.invalid_encoding", "invalid_encoding", "/nonce");
  }
  if (compareIso8601(value.created_at, value.expires_at) >= 0) {
    throw contractError("contract.invalid_relation", "invalid_request_window", "/expires_at");
  }
  verifyContractContentDigest(validated);
  return validated;
}

function validateRunAcceptanceValue(admitted: AdmittedJson): ValidatedValue {
  const validated = validateRegisteredValue(admitted, RUN_ACCEPTANCE_TYPE_NAME);
  const value = validated.value() as RunAcceptance;
  if (!validateIso8601(value.created_at)) {
    throw validationError("schema_violation", "/created_at");
  }
  if (!validateIso8601(value.accepted_at)) {
    throw validationError("schema_violation", "/accepted_at");
  }
  if (value.created_at !== value.accepted_at) {
    throw contractError("contract.invalid_relation", "accepted_at_mismatch", "/created_at");
  }
  verifyContractContentDigest(validated);
  return validated;
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

function resolvePackageRoot(): string {
  const moduleDirectory = dirname(fileURLToPath(import.meta.url));
  if (basename(moduleDirectory) === "dist") {
    return dirname(moduleDirectory);
  }
  const testOutputDirectory = dirname(moduleDirectory);
  if (basename(moduleDirectory) === "src" && basename(testOutputDirectory) === "dist-test") {
    return dirname(testOutputDirectory);
  }
  throw new Error(`unsupported qa contract module layout: ${moduleDirectory}`);
}

function resolvePackageFile(relativePath: string): URL {
  if (relativePath.length === 0 || isAbsolute(relativePath) || relativePath.includes(":")) {
    throw new Error(`invalid qa contract package path: ${relativePath}`);
  }
  const unresolvedCandidate = resolve(PACKAGE_ROOT, relativePath);
  if (!existsSync(unresolvedCandidate)) {
    throw new Error(`qa contract package file not found: ${relativePath}`);
  }
  const candidate = realpathSync(unresolvedCandidate);
  const containedPath = relative(PACKAGE_ROOT, candidate);
  if (
    containedPath.length === 0 ||
    containedPath === ".." ||
    containedPath.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) ||
    isAbsolute(containedPath)
  ) {
    throw new Error(`qa contract package file not found: ${relativePath}`);
  }
  return pathToFileURL(candidate);
}

function validateRegistry(registry: ContractRegistry): void {
  if (registry.registry_version !== "qa.contract-registry/v1") {
    throw new Error("unsupported qa contract registry version");
  }
  if (registry.profile !== "local_qa_host_mvp") {
    throw new Error("qa contract registry has the wrong profile authority");
  }
  for (const type of FOUNDATION_TYPE_NAMES) {
    const entry = registry.types[type];
    const fixtureOnly = type === "ProjectionSpecimen" || type === "StrictUnionSpecimen";
    if (entry !== undefined && (entry.fixture_only === true) !== fixtureOnly) {
      throw new Error(`qa contract fixture-only marker is invalid: ${type}`);
    }
  }
  for (const type of LIFECYCLE_TYPE_NAMES) {
    const entry = registry.types[type];
    if (entry?.fixture_only !== undefined) {
      throw validationError("invalid_embedded_registry", `/types/${type}`);
    }
  }
  for (const type of LOCAL_EVIDENCE_TYPE_NAMES) {
    const entry = registry.types[type];
    if (entry?.fixture_only !== undefined) {
      throw new Error(`qa contract fixture-only marker is invalid: ${type}`);
    }
  }
  for (const type of LOCAL_WORKER_TYPE_NAMES) {
    const entry = registry.types[type];
    if (entry?.fixture_only !== undefined) {
      throw new Error(`qa contract fixture-only marker is invalid: ${type}`);
    }
  }
  for (const type of LOCAL_RUN_ADMISSION_TYPE_NAMES) {
    const entry = registry.types[type];
    if (entry?.fixture_only !== undefined) {
      throw new Error(`qa contract fixture-only marker is invalid: ${type}`);
    }
  }
}


function validateLocalWorkerRegistered(raw: Uint8Array, typeName: string): ValidatedValue {
  try {
    return validateRegisteredValue(admitJson(raw), typeName);
  } catch (error) {
    if (error instanceof ContractError && error.rejection.reason === "schema_violation") {
      throw new ContractError({ ...error.rejection, path: "/" });
    }
    throw error;
  }
}

function validateLocalWorkerRules(value: unknown): void {
  validateLocalWorkerUrls(value);
  if (!isRecord(value)) return;

  if (value.kind === "capability_result" && isRecord(value.output)) {
    if (value.capability === "clock.now/v1") {
      const timestamp = value.output.value;
      if (typeof timestamp !== "string" || !validateWorkerTimestamp(timestamp)) {
        throw new ContractError({ category: "validation", reason: "invalid_timestamp", path: "/output/value" });
      }
    }
    if (value.capability === "browser-session.run/v1") {
      validateExpectedReference(value.output.sanitizedObservationRef, "observation/0", "/output/sanitizedObservationRef/id");
      validateExpectedReference(value.output.screenshotEvidenceRef, "evidence/0", "/output/screenshotEvidenceRef/id");
    }
    if (value.capability === "evidence.stage-fixed-runner-log/v1") {
      validateExpectedReference(value.output.runnerLogEvidenceRef, "evidence/1", "/output/runnerLogEvidenceRef/id");
    }
  }

  if (value.kind === "terminal_result" && isRecord(value.result)) {
    validateTerminalResultRelations(value.result);
  }
}

function validateLocalWorkerUrls(value: unknown): void {
  if (Array.isArray(value)) {
    for (const item of value) validateLocalWorkerUrls(item);
    return;
  }
  if (typeof value !== "object" || value === null) return;
  for (const [key, child] of Object.entries(value)) {
    if ((key === "fixtureUrl" || key === "finalUrl") &&
        (typeof child !== "string" || !isFixedFixtureUrl(child))) {
      throw new ContractError({ category: "validation", reason: "schema_violation", path: `/${key}` });
    }
    validateLocalWorkerUrls(child);
  }
}

function validateExpectedReference(value: unknown, expectedId: string, path: string): void {
  if (!isRecord(value) || value.id !== expectedId) {
    throw contractError("contract.invalid_relation", "reference_id_mismatch", path);
  }
}

function validateTerminalResultRelations(result: Record<string, unknown>): void {
  const startedAt = result.startedAt;
  const finishedAt = result.finishedAt;
  const durationMs = result.durationMs;
  if (typeof startedAt !== "string" || !validateWorkerTimestamp(startedAt)) {
    throw new ContractError({ category: "validation", reason: "invalid_timestamp", path: "/result/startedAt" });
  }
  if (typeof finishedAt !== "string" || !validateWorkerTimestamp(finishedAt)) {
    throw new ContractError({ category: "validation", reason: "invalid_timestamp", path: "/result/finishedAt" });
  }
  const startedMs = Date.parse(startedAt);
  const finishedMs = Date.parse(finishedAt);
  if (startedMs > finishedMs) {
    throw contractError("contract.invalid_relation", "finished_before_started", "/result/finishedAt");
  }
  if (typeof durationMs !== "number" || finishedMs - startedMs !== durationMs) {
    throw contractError("contract.invalid_relation", "duration_mismatch", "/result/durationMs");
  }
  if (!isRecord(result.observation)) {
    throw new ContractError({ category: "validation", reason: "schema_violation", path: "/result/observation" });
  }
  validateExpectedReference(
    result.observation.sanitizedObservationRef,
    "observation/0",
    "/result/observation/sanitizedObservationRef/id",
  );
  if (!Array.isArray(result.evidence) || result.evidence.length !== 2) {
    throw new ContractError({ category: "validation", reason: "schema_violation", path: "/result/evidence" });
  }
  for (const [index, expectedId] of ["evidence/0", "evidence/1"].entries()) {
    const entry = result.evidence[index];
    if (!isRecord(entry) || entry.objectId !== expectedId) {
      throw contractError("contract.invalid_relation", "object_id_mismatch", `/result/evidence/${index}/objectId`);
    }
    validateExpectedReference(entry.artifactRef, expectedId, `/result/evidence/${index}/artifactRef/id`);
  }
}

function validateWorkerTimestamp(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(value)) return false;
  const millis = Date.parse(value);
  return Number.isFinite(millis) && new Date(millis).toISOString() === value;
}

function validateLocalSanitizedObservationRules(value: LocalSanitizedObservation): void {
  if (!isFixedFixtureUrl(value.fixture_url)) {
    throw new ContractError({
      category: "validation",
      reason: "schema_violation",
      path: "/fixture_url",
    });
  }
  if (!isFixedFixtureUrl(value.final_url)) {
    throw new ContractError({
      category: "validation",
      reason: "schema_violation",
      path: "/final_url",
    });
  }
  if (value.final_url !== value.fixture_url) {
    throw contractError("contract.invalid_relation", "fixture_url_mismatch", "/final_url");
  }
}

function isFixedFixtureUrl(value: string): boolean {
  const match = /^http:\/\/127\.0\.0\.1:([0-9]+)\/fixed-page\.html$/.exec(value);
  const portText = match?.[1];
  if (match === null || portText === undefined || match[0] !== value) return false;
  const port = Number(portText);
  return Number.isSafeInteger(port) && port >= 1 && port <= 65_535;
}

function compileRegisteredValidator(registry: ContractRegistry, typeName: string): ValidateFunction {
  const typeEntry = registry.types[typeName];
  if (typeEntry === undefined) {
    throw validationError("unknown_registered_type", "/types");
  }
  const schemaEntry = registry.schemas[typeEntry.schema];
  if (schemaEntry === undefined) {
    throw validationError("unknown_registered_schema", "/schemas");
  }
  if (schemaEntry.major !== SUPPORTED_SCHEMA_MAJOR) {
    throw validationError("unsupported_schema_major", `/schemas/${typeEntry.schema}/major`);
  }
  let schemaUrl: URL;
  try {
    schemaUrl = resolvePackageFile(schemaEntry.path);
  } catch {
    throw validationError("invalid_embedded_schema_path", `/schemas/${typeEntry.schema}/path`);
  }
  const schema = JSON.parse(readFileSync(schemaUrl, "utf8")) as Record<string, unknown>;
  if (schema.$id !== schemaEntry.id) {
    throw validationError("invalid_embedded_schema", "/$id");
  }
  const resolvedSchema = resolveRegisteredReferences(schema, registry);
  if (resolveJsonPointer(schema, typeEntry.pointer) === undefined) {
    throw validationError("unresolved_registered_pointer", "/types");
  }
  const validatorSchema = structuredClone(resolvedSchema);
  delete validatorSchema.$id;
  validatorSchema.$ref = typeEntry.pointer;
  try {
    return AJV.compile(validatorSchema);
  } catch {
    throw validationError("invalid_embedded_schema", "/");
  }
}

function resolveJsonPointer(root: unknown, pointerValue: string): unknown {
  if (!pointerValue.startsWith("#/")) return undefined;
  let current = root;
  for (const encodedToken of pointerValue.slice(2).split("/")) {
    if (/~(?![01])/u.test(encodedToken)) return undefined;
    const token = encodedToken.replace(/~1/g, "/").replace(/~0/g, "~");
    if (!isRecord(current) || !(token in current)) return undefined;
    current = current[token];
  }
  return current;
}

function resolveRegisteredReferences(
  value: unknown,
  registry: ContractRegistry,
): Record<string, unknown> {
  const resolved = resolveRegisteredReferenceValue(value, registry);
  if (!isRecord(resolved)) {
    throw validationError("invalid_embedded_schema", "/");
  }
  return resolved;
}

function resolveRegisteredReferenceValue(value: unknown, registry: ContractRegistry): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => resolveRegisteredReferenceValue(item, registry));
  }
  if (!isRecord(value)) return value;
  if (typeof value.$ref === "string" && !value.$ref.startsWith("#")) {
    const imported = importRegisteredReference(value.$ref, registry);
    const siblings = Object.fromEntries(Object.entries(value).filter(([key]) => key !== "$ref"));
    if (Object.keys(siblings).length === 0) return imported;
    return {
      allOf: [
        imported,
        resolveRegisteredReferenceValue(siblings, registry),
      ],
    };
  }
  return Object.fromEntries(
    Object.entries(value).map(([key, child]) => [
      key,
      resolveRegisteredReferenceValue(child, registry),
    ]),
  );
}

function importRegisteredReference(reference: string, registry: ContractRegistry): unknown {
  const match = Object.entries(registry.schemas).find(([, entry]) =>
    reference === entry.id || reference.startsWith(`${entry.id}#`),
  );
  if (match === undefined) {
    throw validationError("external_schema_reference", "/$ref");
  }
  const [schemaName, schemaEntry] = match;
  let schemaUrl: URL;
  try {
    schemaUrl = resolvePackageFile(schemaEntry.path);
  } catch {
    throw validationError("invalid_embedded_schema_path", `/schemas/${schemaName}/path`);
  }
  const schema = JSON.parse(readFileSync(schemaUrl, "utf8")) as Record<string, unknown>;
  if (schema.$id !== schemaEntry.id) {
    throw validationError("invalid_embedded_schema", "/$id");
  }
  const fragment = reference.slice(schemaEntry.id.length);
  const target = fragment.length === 0 ? schema : resolveJsonPointer(schema, fragment);
  if (target === undefined || containsSchemaReference(target)) {
    throw validationError("invalid_embedded_schema", "/$ref");
  }
  return structuredClone(target);
}

function containsSchemaReference(value: unknown): boolean {
  if (Array.isArray(value)) return value.some(containsSchemaReference);
  if (!isRecord(value)) return false;
  return typeof value.$ref === "string" || Object.values(value).some(containsSchemaReference);
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
  const integerMagnitude = exactIntegerMagnitude(token);
  const plainIntegerToken = !/[.eE]/.test(token);
  const rendersAsPlainInteger = Math.abs(Number(token)) < 1e21;
  if (
    integerMagnitude !== undefined &&
    integerMagnitude > MAX_SAFE_INTEGER &&
    (plainIntegerToken || rendersAsPlainInteger)
  ) {
    throw canonicalError("canonicalization.unsafe_integer", "unsafe_integer");
  }
}

function exactIntegerMagnitude(token: string): bigint | undefined {
  const match = /^(-?)(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?$/.exec(token);
  if (match === null) return undefined;
  const integerDigits = match[2]!;
  const fractionDigits = match[3] ?? "";
  const digits = `${integerDigits}${fractionDigits}`.replace(/^0+/, "");
  if (digits.length === 0) return 0n;
  const exponent = Number(match[4] ?? "0");
  const scale = fractionDigits.length - exponent;
  if (!Number.isSafeInteger(scale)) return undefined;
  if (scale <= 0) {
    return BigInt(digits) * 10n ** BigInt(-scale);
  }
  if (scale >= digits.length) return undefined;
  const fractionalTail = digits.slice(digits.length - scale);
  if (!/^0+$/.test(fractionalTail)) return undefined;
  return BigInt(digits.slice(0, digits.length - scale));
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
      validateSignatureBlock(value, "");
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
      if (isRecord(value.signature)) {
        validateSignatureBlock(value.signature, "/signature");
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

function validateSignatureBlock(value: Record<string, unknown>, pathPrefix: string): void {
  const allowedFields = new Set(["algorithm", "key_id", "value"]);
  for (const key of Object.keys(value)) {
    if (!allowedFields.has(key)) {
      throw contractError(
        "contract.forbidden_field",
        "unknown_field",
        `${pathPrefix}${pointer(key)}`,
      );
    }
  }
  validateClosedEnum(value, "algorithm", ["ed25519", "es256"], pathPrefix);
  if (typeof value.value === "string") {
    validateScalarAt("Base64UrlNoPad", value.value, `${pathPrefix}/value`);
  }
}

function validateClosedEnum(
  value: Record<string, unknown>,
  field: string,
  allowed: readonly string[],
  pathPrefix = "",
): void {
  if (typeof value[field] === "string" && !allowed.includes(value[field])) {
    throw contractError(
      "contract.unsupported_enum",
      "unsupported_enum",
      `${pathPrefix}${pointer(field)}`,
    );
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

function compareIso8601(left: string, right: string): number {
  const leftSecond = left.slice(0, 19);
  const rightSecond = right.slice(0, 19);
  if (leftSecond !== rightSecond) return leftSecond < rightSecond ? -1 : 1;
  const leftFraction = iso8601Fraction(left);
  const rightFraction = iso8601Fraction(right);
  const length = Math.max(leftFraction.length, rightFraction.length);
  for (let index = 0; index < length; index += 1) {
    const leftDigit = leftFraction.charCodeAt(index) || 48;
    const rightDigit = rightFraction.charCodeAt(index) || 48;
    if (leftDigit !== rightDigit) return leftDigit < rightDigit ? -1 : 1;
  }
  return 0;
}

function iso8601Fraction(value: string): string {
  return value[19] === "." ? value.slice(20, -1) : "";
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

function validationError(reason: string, path: string): ContractError {
  return new ContractError({ category: "validation", reason, path });
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
