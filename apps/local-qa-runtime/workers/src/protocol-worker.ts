import {
  LocalWorkerFrameDecoder,
  encodeLocalWorkerFrame,
  validateLocalWorkerCapabilityRequest,
  validateLocalWorkerTerminalResult,
  type ValidatedValue,
} from "@chronoai/fkst-qa-contracts";

import {
  runBrowserSmoke,
  type BrowserSmokeRequest,
  type DigestBoundRef,
} from "./policy.js";

const PROTOCOL = "qa.local-worker-protocol/v1";
const FIXED_RUNNER_LOG = new TextEncoder().encode("navigation accepted\nassertion passed\n");

type FrameRecord = Readonly<Record<string, unknown>>;

class ProtocolPeer {
  readonly #decoder = new LocalWorkerFrameDecoder();
  readonly #iterator = process.stdin[Symbol.asyncIterator]();
  readonly #pending: ValidatedValue[] = [];

  async read(): Promise<FrameRecord> {
    while (this.#pending.length === 0) {
      const next = await this.#iterator.next();
      if (next.done === true) {
        this.#decoder.finish();
        throw new Error("protocol input ended before the expected frame");
      }
      this.#pending.push(...this.#decoder.push(next.value));
    }
    const value = this.#pending.shift()?.value();
    if (!isRecord(value)) throw new Error("validated protocol frame is not an object");
    return value;
  }

  write(value: unknown, validate: (raw: Uint8Array) => ValidatedValue): void {
    const raw = new TextEncoder().encode(JSON.stringify(value));
    process.stdout.write(encodeLocalWorkerFrame(validate(raw)));
  }
}

class CapabilityClient {
  readonly #peer: ProtocolPeer;
  readonly #invocationId: string;
  #nextRequest = 0;

  constructor(peer: ProtocolPeer, invocationId: string) {
    this.#peer = peer;
    this.#invocationId = invocationId;
  }

  async request(capability: string, input: unknown): Promise<FrameRecord> {
    const requestId = `capability/${this.#nextRequest}`;
    this.#nextRequest += 1;
    this.#peer.write(
      {
        protocol: PROTOCOL,
        kind: "capability_request",
        invocation_id: this.#invocationId,
        request_id: requestId,
        capability,
        input,
      },
      validateLocalWorkerCapabilityRequest,
    );
    const result = await this.#peer.read();
    if (
      result.protocol !== PROTOCOL ||
      result.kind !== "capability_result" ||
      result.invocation_id !== this.#invocationId ||
      result.request_id !== requestId ||
      result.capability !== capability ||
      !isRecord(result.output)
    ) {
      throw new Error("capability result does not match the outstanding request");
    }
    return result.output;
  }
}

export async function runProtocolWorker(): Promise<void> {
  const peer = new ProtocolPeer();
  const invocation = await peer.read();
  if (
    invocation.protocol !== PROTOCOL ||
    invocation.kind !== "invocation" ||
    typeof invocation.invocation_id !== "string" ||
    invocation.operation !== "browser-smoke" ||
    !isRecord(invocation.input)
  ) {
    throw new Error("expected one browser-smoke invocation");
  }
  const invocationId = invocation.invocation_id;
  const request = invocation.input as BrowserSmokeRequest;
  const capabilities = new CapabilityClient(peer, invocationId);
  const bundle = await runBrowserSmoke(JSON.stringify(request), {
    clock: {
      async now() {
        return stringValue(await capabilities.request("clock.now/v1", {}), "value");
      },
      async monotonicMs() {
        return numberValue(await capabilities.request("clock.monotonic-ms/v1", {}), "value");
      },
    },
    session: {
      async run(input) {
        const output = await capabilities.request("browser-session.run/v1", input);
        return {
          finalUrl: stringValue(output, "finalUrl"),
          observedText: stringValue(output, "observedText"),
          sanitizedObservationRef: referenceValue(output, "sanitizedObservationRef"),
          screenshotEvidenceRef: referenceValue(output, "screenshotEvidenceRef"),
        };
      },
      async close() {
        const output = await capabilities.request("browser-session.close/v1", {});
        if (Reflect.ownKeys(output).length !== 0) throw new Error("browser close output is not empty");
      },
    },
    evidence: {
      async stageGeneratedLog(input) {
        if (
          input.name !== "runner.log" ||
          input.mediaType !== "text/plain; charset=utf-8" ||
          !equalBytes(input.bytes, FIXED_RUNNER_LOG)
        ) {
          throw new Error("policy runner log does not match the fixed template");
        }
        const output = await capabilities.request("evidence.stage-fixed-runner-log/v1", {
          name: input.name,
          mediaType: input.mediaType,
          template: "fixed-browser-smoke-runner-log/v1",
        });
        return referenceValue(output, "runnerLogEvidenceRef");
      },
    },
  });
  peer.write(
    {
      protocol: PROTOCOL,
      kind: "terminal_result",
      invocation_id: invocationId,
      outcome: "passed",
      result: bundle.result,
    },
    validateLocalWorkerTerminalResult,
  );
}

function referenceValue(value: FrameRecord, field: string): DigestBoundRef<"qa.local-evidence/v1"> {
  const reference = value[field];
  if (!isRecord(reference)) throw new Error(`${field} is not an object`);
  const { kind, id, schema_version, content_digest, version } = reference;
  if (
    typeof kind !== "string" ||
    typeof id !== "string" ||
    schema_version !== "qa.local-evidence/v1" ||
    typeof content_digest !== "string" ||
    (version !== undefined && typeof version !== "string")
  ) {
    throw new Error(`${field} is not a digest-bound Local Evidence reference`);
  }
  return { kind, id, schema_version, content_digest, ...(version === undefined ? {} : { version }) };
}

function stringValue(value: FrameRecord, field: string): string {
  const member = value[field];
  if (typeof member !== "string") throw new Error(`${field} is not a string`);
  return member;
}

function numberValue(value: FrameRecord, field: string): number {
  const member = value[field];
  if (typeof member !== "number") throw new Error(`${field} is not a number`);
  return member;
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
