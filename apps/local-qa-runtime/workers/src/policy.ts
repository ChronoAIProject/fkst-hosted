import { parseStrictJson, type ParsedJson } from "./json.js";
import { BrowserSmokeWorkerError, type WorkerErrorCode } from "./worker-error.js";

const REQUEST_VERSION = "local-qa-browser-smoke/request-v1";
const RESULT_VERSION = "local-qa-browser-smoke/result-v1";
const SELECTOR = '[data-local-qa="status"]';
const EXPECTED_TEXT = "READY";
const TIMEOUT_MS = 5000;
const RUNNER_LOG = utf8Bytes("navigation accepted\nassertion passed\n");
const REQUEST_FIELDS = new Set([
  "version",
  "fixtureUrl",
  "selector",
  "expectedText",
  "timeoutMs",
]);
const REFERENCE_FIELDS = new Set([
  "kind",
  "id",
  "schema_version",
  "content_digest",
  "version",
]);

export type BrowserSmokeRequest = {
  readonly version: typeof REQUEST_VERSION;
  readonly fixtureUrl: string;
  readonly selector: typeof SELECTOR;
  readonly expectedText: typeof EXPECTED_TEXT;
  readonly timeoutMs: typeof TIMEOUT_MS;
};

export type DigestBoundRef<TSchema extends string> = {
  readonly kind: string;
  readonly id: string;
  readonly schema_version: TSchema;
  readonly content_digest: string;
  readonly version?: string;
};

export interface BrowserSessionPort {
  run(request: BrowserSmokeRequest): Promise<{
    finalUrl: string;
    observedText: string;
    sanitizedObservationRef: DigestBoundRef<"qa.sanitized-observation/v1">;
    screenshotArtifactRef: DigestBoundRef<"qa.artifact-pointer/v1">;
  }>;
  close(): Promise<void>;
}

export interface EvidenceStagingPort {
  stageGeneratedLog(input: {
    name: "runner.log";
    mediaType: "text/plain; charset=utf-8";
    bytes: Uint8Array;
  }): Promise<DigestBoundRef<"qa.artifact-pointer/v1">>;
}

export interface ClockPort {
  now(): string;
  monotonicMs(): number;
}

export interface BrowserSmokeResult {
  readonly version: typeof RESULT_VERSION;
  readonly outcome: "passed";
  readonly observation: {
    readonly fixtureUrl: string;
    readonly finalUrl: string;
    readonly selector: typeof SELECTOR;
    readonly expectedText: typeof EXPECTED_TEXT;
    readonly observedText: typeof EXPECTED_TEXT;
    readonly sanitizedObservationRef: DigestBoundRef<"qa.sanitized-observation/v1">;
  };
  readonly startedAt: string;
  readonly finishedAt: string;
  readonly durationMs: number;
  readonly evidence: readonly [
    {
      readonly objectId: "evidence/0";
      readonly role: "screenshot";
      readonly artifactRef: DigestBoundRef<"qa.artifact-pointer/v1">;
    },
    {
      readonly objectId: "evidence/1";
      readonly role: "runner-log";
      readonly artifactRef: DigestBoundRef<"qa.artifact-pointer/v1">;
    },
  ];
}

export interface BrowserSmokeBundle {
  readonly result: BrowserSmokeResult;
}

export { BrowserSmokeWorkerError, type WorkerErrorCode };

export function parseBrowserSmokeRequest(source: string): BrowserSmokeRequest {
  const parsed = parseStrictJson(source);
  if (!(parsed.value instanceof Map)) {
    throw new BrowserSmokeWorkerError("request.root_not_object");
  }
  const members = parsed.value;
  for (const field of members.keys()) {
    if (!REQUEST_FIELDS.has(field)) {
      throw new BrowserSmokeWorkerError("request.unknown_field");
    }
  }
  for (const field of REQUEST_FIELDS) {
    if (!members.has(field)) {
      throw new BrowserSmokeWorkerError("request.missing_field");
    }
  }

  const version = stringField(members.get("version"));
  const fixtureUrl = stringField(members.get("fixtureUrl"));
  const selector = stringField(members.get("selector"));
  const expectedText = stringField(members.get("expectedText"));
  const timeout = members.get("timeoutMs");
  if (typeof timeout?.value !== "number") {
    throw new BrowserSmokeWorkerError("request.wrong_type");
  }
  if (
    version !== REQUEST_VERSION ||
    !isFixedFixtureUrl(fixtureUrl) ||
    selector !== SELECTOR ||
    expectedText !== EXPECTED_TEXT ||
    timeout.raw !== "5000"
  ) {
    throw new BrowserSmokeWorkerError("request.unsupported_value");
  }

  return {
    version: REQUEST_VERSION,
    fixtureUrl,
    selector: SELECTOR,
    expectedText: EXPECTED_TEXT,
    timeoutMs: TIMEOUT_MS,
  };
}

export async function runBrowserSmoke(
  source: string,
  ports: {
    readonly session: BrowserSessionPort;
    readonly evidence: EvidenceStagingPort;
    readonly clock: ClockPort;
  },
): Promise<BrowserSmokeBundle> {
  const request = parseBrowserSmokeRequest(source);
  const startedAt = clockNow(ports.clock);
  const startedMonotonicMs = monotonicNow(ports.clock);
  let validatedSession: ReturnType<typeof validateSessionResult>;
  let runnerLogArtifactRef: DigestBoundRef<"qa.artifact-pointer/v1">;

  try {
    let sessionResult: Awaited<ReturnType<BrowserSessionPort["run"]>>;
    try {
      sessionResult = await ports.session.run(request);
    } catch {
      throw new BrowserSmokeWorkerError("session.run_failed");
    }
    try {
      validatedSession = validateSessionResult(sessionResult);
    } catch (error) {
      if (error instanceof BrowserSmokeWorkerError) {
        throw error;
      }
      throw new BrowserSmokeWorkerError("session.invalid_response");
    }
    if (validatedSession.finalUrl !== request.fixtureUrl) {
      throw new BrowserSmokeWorkerError("policy.final_url_rejected");
    }
    if (validatedSession.observedText !== EXPECTED_TEXT) {
      throw new BrowserSmokeWorkerError("policy.assertion_failed");
    }
    try {
      runnerLogArtifactRef = validateReference(
        await ports.evidence.stageGeneratedLog({
          name: "runner.log",
          mediaType: "text/plain; charset=utf-8",
          bytes: RUNNER_LOG.slice(),
        }),
        "artifact-pointer",
        "qa.artifact-pointer/v1",
      );
    } catch (error) {
      if (error instanceof BrowserSmokeWorkerError) {
        throw error;
      }
      throw new BrowserSmokeWorkerError("evidence.staging_failed");
    }
  } finally {
    try {
      await ports.session.close();
    } catch {
      throw new BrowserSmokeWorkerError("session.finalization_failed");
    }
  }

  const finishedAt = clockNow(ports.clock);
  const finishedMonotonicMs = monotonicNow(ports.clock);
  const durationMs = finishedMonotonicMs - startedMonotonicMs;
  if (!Number.isSafeInteger(durationMs) || durationMs < 0) {
    throw new BrowserSmokeWorkerError("clock.invalid_value");
  }

  return {
    result: {
      version: RESULT_VERSION,
      outcome: "passed",
      observation: {
        fixtureUrl: request.fixtureUrl,
        finalUrl: validatedSession.finalUrl,
        selector: SELECTOR,
        expectedText: EXPECTED_TEXT,
        observedText: EXPECTED_TEXT,
        sanitizedObservationRef: validatedSession.sanitizedObservationRef,
      },
      startedAt,
      finishedAt,
      durationMs,
      evidence: [
        {
          objectId: "evidence/0",
          role: "screenshot",
          artifactRef: validatedSession.screenshotArtifactRef,
        },
        {
          objectId: "evidence/1",
          role: "runner-log",
          artifactRef: runnerLogArtifactRef,
        },
      ],
    },
  };
}

export function serializeBrowserSmokeResult(result: BrowserSmokeResult): Uint8Array {
  const canonicalResult = {
    version: result.version,
    outcome: result.outcome,
    observation: {
      fixtureUrl: result.observation.fixtureUrl,
      finalUrl: result.observation.finalUrl,
      selector: result.observation.selector,
      expectedText: result.observation.expectedText,
      observedText: result.observation.observedText,
      sanitizedObservationRef: canonicalReference(result.observation.sanitizedObservationRef),
    },
    startedAt: result.startedAt,
    finishedAt: result.finishedAt,
    durationMs: result.durationMs,
    evidence: result.evidence.map((entry) => ({
      objectId: entry.objectId,
      role: entry.role,
      artifactRef: canonicalReference(entry.artifactRef),
    })),
  };
  return utf8Bytes(JSON.stringify(canonicalResult));
}

function utf8Bytes(value: string): Uint8Array {
  const bytes: number[] = [];
  for (let index = 0; index < value.length; index += 1) {
    let codePoint = value.codePointAt(index);
    if (codePoint === undefined) {
      break;
    }
    if (codePoint >= 0xd800 && codePoint <= 0xdfff) {
      codePoint = 0xfffd;
    } else if (codePoint > 0xffff) {
      index += 1;
    }
    if (codePoint <= 0x7f) {
      bytes.push(codePoint);
    } else if (codePoint <= 0x7ff) {
      bytes.push(0xc0 | (codePoint >> 6), 0x80 | (codePoint & 0x3f));
    } else if (codePoint <= 0xffff) {
      bytes.push(
        0xe0 | (codePoint >> 12),
        0x80 | ((codePoint >> 6) & 0x3f),
        0x80 | (codePoint & 0x3f),
      );
    } else {
      bytes.push(
        0xf0 | (codePoint >> 18),
        0x80 | ((codePoint >> 12) & 0x3f),
        0x80 | ((codePoint >> 6) & 0x3f),
        0x80 | (codePoint & 0x3f),
      );
    }
  }
  return Uint8Array.from(bytes);
}

function stringField(field: ParsedJson | undefined): string {
  if (typeof field?.value !== "string") {
    throw new BrowserSmokeWorkerError("request.wrong_type");
  }
  return field.value;
}

function isFixedFixtureUrl(value: string): boolean {
  const match = /^http:\/\/127\.0\.0\.1:([0-9]+)\/fixed-page\.html$/.exec(value);
  if (match === null) {
    return false;
  }
  const port = Number(match[1]);
  return Number.isSafeInteger(port) && port >= 1 && port <= 65535;
}

function validateSessionResult(value: unknown): {
  readonly finalUrl: string;
  readonly observedText: string;
  readonly sanitizedObservationRef: DigestBoundRef<"qa.sanitized-observation/v1">;
  readonly screenshotArtifactRef: DigestBoundRef<"qa.artifact-pointer/v1">;
} {
  if (!isExactRecord(value, [
    "finalUrl",
    "observedText",
    "sanitizedObservationRef",
    "screenshotArtifactRef",
  ])) {
    throw new BrowserSmokeWorkerError("session.invalid_response");
  }
  if (typeof value.finalUrl !== "string" || typeof value.observedText !== "string") {
    throw new BrowserSmokeWorkerError("session.invalid_response");
  }
  return {
    finalUrl: value.finalUrl,
    observedText: value.observedText,
    sanitizedObservationRef: validateReference(
      value.sanitizedObservationRef,
      "sanitized-observation",
      "qa.sanitized-observation/v1",
    ),
    screenshotArtifactRef: validateReference(
      value.screenshotArtifactRef,
      "artifact-pointer",
      "qa.artifact-pointer/v1",
    ),
  };
}

function validateReference<TSchema extends string>(
  value: unknown,
  expectedKind: string,
  expectedSchema: TSchema,
): DigestBoundRef<TSchema> {
  if (!isRecord(value)) {
    throw new BrowserSmokeWorkerError("evidence.invalid_reference");
  }
  const keys = ownStringKeys(value);
  if (
    keys.some((key) => !REFERENCE_FIELDS.has(key)) ||
    !["kind", "id", "schema_version", "content_digest"].every((key) => keys.includes(key)) ||
    keys.length > 5
  ) {
    throw new BrowserSmokeWorkerError("evidence.invalid_reference");
  }
  if (
    value.kind !== expectedKind ||
    typeof value.id !== "string" ||
    value.schema_version !== expectedSchema ||
    typeof value.content_digest !== "string" ||
    !/^[0-9a-f]{64}$/.test(value.content_digest) ||
    ("version" in value && typeof value.version !== "string")
  ) {
    throw new BrowserSmokeWorkerError("evidence.invalid_reference");
  }
  return {
    kind: expectedKind,
    id: value.id,
    schema_version: expectedSchema,
    content_digest: value.content_digest,
    ...(typeof value.version === "string" ? { version: value.version } : {}),
  };
}

function canonicalReference<TSchema extends string>(
  reference: DigestBoundRef<TSchema>,
): DigestBoundRef<TSchema> {
  return {
    kind: reference.kind,
    id: reference.id,
    schema_version: reference.schema_version,
    content_digest: reference.content_digest,
    ...(reference.version === undefined ? {} : { version: reference.version }),
  };
}

function clockNow(clock: ClockPort): string {
  let value: string;
  try {
    value = clock.now();
  } catch {
    throw new BrowserSmokeWorkerError("clock.failed");
  }
  if (typeof value !== "string") {
    throw new BrowserSmokeWorkerError("clock.invalid_value");
  }
  return value;
}

function monotonicNow(clock: ClockPort): number {
  let value: number;
  try {
    value = clock.monotonicMs();
  } catch {
    throw new BrowserSmokeWorkerError("clock.failed");
  }
  if (!Number.isSafeInteger(value)) {
    throw new BrowserSmokeWorkerError("clock.invalid_value");
  }
  return value;
}

function isExactRecord(value: unknown, expectedKeys: readonly string[]): value is Record<string, unknown> {
  if (!isRecord(value)) {
    return false;
  }
  const keys = ownStringKeys(value);
  return keys.length === expectedKeys.length &&
    expectedKeys.every((key) => Object.prototype.hasOwnProperty.call(value, key));
}

function ownStringKeys(value: object): string[] {
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key !== "string")) {
    throw new BrowserSmokeWorkerError("evidence.invalid_reference");
  }
  return keys as string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
