import {
  ContractError,
  LocalWorkerFrameDecoder,
  encodeLocalWorkerFrame,
  validateLocalWorkerCapabilityRequest,
  validateLocalWorkerProtocolFailure,
  validateLocalWorkerTerminalResult,
  type ValidatedValue,
} from "@chronoai/fkst-qa-contracts";

import {
  BrowserSmokeWorkerError,
  runBrowserSmoke,
  type BrowserSmokeRequest,
  type DigestBoundRef,
} from "./policy.js";

const PROTOCOL = "qa.local-worker-protocol/v1";
const FIXED_RUNNER_LOG = new TextEncoder().encode("navigation accepted\nassertion passed\n");
const MAX_PENDING_FRAMES = 1;

export type ProtocolFailureCode =
  | "protocol.invalid_frame"
  | "protocol.invalid_sequence"
  | "protocol.unexpected_eof"
  | "protocol.trailing_input"
  | "protocol.capability_mismatch"
  | "worker.execution_failed"
  | "worker.internal_failure"
  | "io.stdout_failed";

type FrameRecord = Readonly<Record<string, unknown>>;
type FrameValidator = (raw: Uint8Array) => ValidatedValue;

class ProtocolFailure extends Error {
  readonly code: ProtocolFailureCode;

  constructor(code: ProtocolFailureCode) {
    super(code);
    this.name = "ProtocolFailure";
    this.code = code;
  }
}

class ProtocolPeer {
  readonly #decoder = new LocalWorkerFrameDecoder();
  readonly #iterator = process.stdin[Symbol.asyncIterator]();
  readonly #pending: ValidatedValue[] = [];
  #writeChain = Promise.resolve();
  #terminal = false;
  #recordedFailure: ProtocolFailureCode | undefined;

  async read(finalInboundFrame = false): Promise<FrameRecord> {
    if (this.#terminal) throw new ProtocolFailure("protocol.trailing_input");
    while (this.#pending.length === 0) {
      const next = await this.#iterator.next();
      if (next.done === true) {
        try {
          this.#decoder.finish();
        } catch {
          throw new ProtocolFailure("protocol.invalid_frame");
        }
        throw new ProtocolFailure("protocol.unexpected_eof");
      }
      let frames: readonly ValidatedValue[];
      try {
        frames = this.#decoder.push(next.value);
      } catch {
        throw new ProtocolFailure("protocol.invalid_frame");
      }
      if (
        frames.length + this.#pending.length > MAX_PENDING_FRAMES ||
        (frames.length > 0 && this.#decoder.bufferedBytes() > 0)
      ) {
        throw new ProtocolFailure(finalInboundFrame ? "protocol.trailing_input" : "protocol.invalid_sequence");
      }
      this.#pending.push(...frames);
    }
    const value = this.#pending.shift()?.value();
    if (!isRecord(value)) throw new ProtocolFailure("protocol.invalid_frame");
    return value;
  }

  async write(value: unknown, validate: FrameValidator): Promise<void> {
    if (this.#terminal) throw new ProtocolFailure("protocol.invalid_sequence");
    await this.#enqueueWrite(value, validate);
  }

  recordFailure(error: unknown): void {
    if (this.#recordedFailure === undefined && error instanceof ProtocolFailure) {
      this.#recordedFailure = error.code;
    }
  }

  recordedFailure(): ProtocolFailureCode | undefined {
    return this.#recordedFailure;
  }

  async writeFailure(code: ProtocolFailureCode): Promise<void> {
    if (this.#terminal) return;
    this.#terminal = true;
    await this.#enqueueWrite({ protocol: PROTOCOL, kind: "protocol_failure", code }, validateLocalWorkerProtocolFailure);
  }

  async writeTerminal(value: unknown): Promise<void> {
    if (this.#terminal) throw new ProtocolFailure("protocol.invalid_sequence");
    this.#terminal = true;
    await this.#enqueueWrite(value, validateLocalWorkerTerminalResult);
  }

  async expectCleanEof(): Promise<void> {
    if (this.#terminal) throw new ProtocolFailure("protocol.invalid_sequence");
    if (this.#pending.length !== 0) throw new ProtocolFailure("protocol.trailing_input");
    const next = await this.#iterator.next();
    if (next.done !== true) throw new ProtocolFailure("protocol.trailing_input");
    try {
      this.#decoder.finish();
    } catch {
      throw new ProtocolFailure("protocol.trailing_input");
    }
  }

  async #enqueueWrite(value: unknown, validate: FrameValidator): Promise<void> {
    let frame: Uint8Array;
    try {
      const raw = new TextEncoder().encode(JSON.stringify(value));
      frame = encodeLocalWorkerFrame(validate(raw));
    } catch (error) {
      if (error instanceof ProtocolFailure) throw error;
      throw new ProtocolFailure("worker.internal_failure");
    }
    const write = this.#writeChain.then(() => writeStdout(frame));
    this.#writeChain = write.catch(() => undefined);
    await write;
  }
}

class CapabilityClient {
  readonly #peer: ProtocolPeer;
  readonly #invocationId: string;
  #nextRequest = 0;
  #outstanding = false;

  constructor(peer: ProtocolPeer, invocationId: string) {
    this.#peer = peer;
    this.#invocationId = invocationId;
  }

  async request(capability: string, input: unknown): Promise<FrameRecord> {
    if (this.#outstanding || this.#nextRequest >= 7) {
      throw new ProtocolFailure("protocol.invalid_sequence");
    }
    this.#outstanding = true;
    const requestId = `capability/${this.#nextRequest}`;
    this.#nextRequest += 1;
    try {
      await this.#peer.write(
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
      const result = await this.#peer.read(this.#nextRequest === 7);
      if (
        result.protocol !== PROTOCOL ||
        result.kind !== "capability_result" ||
        result.invocation_id !== this.#invocationId ||
        result.request_id !== requestId ||
        result.capability !== capability ||
        !isRecord(result.output)
      ) {
        throw new ProtocolFailure("protocol.capability_mismatch");
      }
      return result.output;
    } catch (error) {
      this.#peer.recordFailure(error);
      throw error;
    } finally {
      this.#outstanding = false;
    }
  }

  assertComplete(): void {
    if (this.#outstanding || this.#nextRequest !== 7) {
      throw new ProtocolFailure("protocol.invalid_sequence");
    }
  }
}

export async function runProtocolWorker(): Promise<void> {
  const peer = new ProtocolPeer();
  try {
    const invocation = await peer.read();
    if (
      invocation.protocol !== PROTOCOL ||
      invocation.kind !== "invocation" ||
      typeof invocation.invocation_id !== "string" ||
      invocation.operation !== "browser-smoke" ||
      !isRecord(invocation.input)
    ) {
      throw new ProtocolFailure("protocol.invalid_sequence");
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
          if (Reflect.ownKeys(output).length !== 0) {
            throw new ProtocolFailure("protocol.capability_mismatch");
          }
        },
      },
      evidence: {
        async stageGeneratedLog(input) {
          if (
            input.name !== "runner.log" ||
            input.mediaType !== "text/plain; charset=utf-8" ||
            !equalBytes(input.bytes, FIXED_RUNNER_LOG)
          ) {
            throw new ProtocolFailure("worker.internal_failure");
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
    capabilities.assertComplete();
    await peer.expectCleanEof();
    await peer.writeTerminal({
      protocol: PROTOCOL,
      kind: "terminal_result",
      invocation_id: invocationId,
      outcome: "passed",
      result: bundle.result,
    });
  } catch (error) {
    const code = peer.recordedFailure() ?? classifyFailure(error);
    try {
      await peer.writeFailure(code);
    } catch {
      writeSanitizedStderr("io.stdout_failed");
      process.exitCode = 1;
      return;
    }
    writeSanitizedStderr(code);
    process.exitCode = 1;
  }
}

function classifyFailure(error: unknown): ProtocolFailureCode {
  if (error instanceof ProtocolFailure) return error.code;
  if (error instanceof BrowserSmokeWorkerError) return "worker.execution_failed";
  if (error instanceof ContractError) return "protocol.invalid_frame";
  return "worker.internal_failure";
}

function writeStdout(frame: Uint8Array): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let callbackDone = false;
    let drainDone = false;
    const cleanup = () => {
      process.stdout.off("error", onError);
      process.stdout.off("drain", onDrain);
    };
    const fail = () => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(new ProtocolFailure("io.stdout_failed"));
    };
    const complete = () => {
      if (settled || !callbackDone || !drainDone) return;
      settled = true;
      cleanup();
      resolve();
    };
    const onError = () => fail();
    const onDrain = () => {
      drainDone = true;
      complete();
    };
    process.stdout.once("error", onError);
    const accepted = process.stdout.write(frame, (error) => {
      if (error !== undefined && error !== null) {
        fail();
        return;
      }
      callbackDone = true;
      complete();
    });
    drainDone = accepted;
    if (!accepted) process.stdout.once("drain", onDrain);
  });
}

function writeSanitizedStderr(code: ProtocolFailureCode): void {
  const message = `fkst-local-qa-worker: ${code}\n`;
  try {
    process.stderr.write(message.slice(0, 128));
  } catch {
    // The worker has no remaining reporting channel.
  }
}

function referenceValue(value: FrameRecord, field: string): DigestBoundRef<"qa.local-evidence/v1"> {
  const reference = value[field];
  if (!isRecord(reference)) throw new ProtocolFailure("protocol.capability_mismatch");
  const { kind, id, schema_version, content_digest, version } = reference;
  if (
    typeof kind !== "string" ||
    typeof id !== "string" ||
    schema_version !== "qa.local-evidence/v1" ||
    typeof content_digest !== "string" ||
    (version !== undefined && typeof version !== "string")
  ) {
    throw new ProtocolFailure("protocol.capability_mismatch");
  }
  return { kind, id, schema_version, content_digest, ...(version === undefined ? {} : { version }) };
}

function stringValue(value: FrameRecord, field: string): string {
  const member = value[field];
  if (typeof member !== "string") throw new ProtocolFailure("protocol.capability_mismatch");
  return member;
}

function numberValue(value: FrameRecord, field: string): number {
  const member = value[field];
  if (typeof member !== "number") throw new ProtocolFailure("protocol.capability_mismatch");
  return member;
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
