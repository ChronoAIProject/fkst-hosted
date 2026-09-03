import {
  ContractError,
  LOCAL_WORKER_MAX_FRAME_BYTES,
  encodeLocalWorkerFrame,
  validateLocalWorkerCapabilityRequest,
  validateLocalWorkerControlFailure,
  validateLocalWorkerFrame,
  validateLocalWorkerAbort,
  validateLocalWorkerCancelAck,
  validateLocalWorkerProtocolFailure,
  validateLocalWorkerTerminalResult,
  type ValidatedValue,
} from "@chronoai/fkst-qa-contracts";
import type { Readable, Writable } from "node:stream";

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

class WorkerCancelled extends Error {}

class WireFrameDecoder {
  #buffer = new Uint8Array(0);

  bufferedBytes(): number {
    return this.#buffer.length;
  }

  push(chunk: Uint8Array): readonly Uint8Array[] {
    const combined = new Uint8Array(this.#buffer.length + chunk.length);
    combined.set(this.#buffer);
    combined.set(chunk, this.#buffer.length);
    this.#buffer = combined;
    const frames: Uint8Array[] = [];
    let offset = 0;
    while (this.#buffer.length - offset >= 4) {
      const length = new DataView(
        this.#buffer.buffer,
        this.#buffer.byteOffset + offset,
        4,
      ).getUint32(0, false);
      if (length < 1 || length > LOCAL_WORKER_MAX_FRAME_BYTES) {
        throw new Error("invalid frame length");
      }
      if (this.#buffer.length - offset - 4 < length) break;
      frames.push(this.#buffer.slice(offset + 4, offset + 4 + length));
      offset += 4 + length;
    }
    if (offset > 0) this.#buffer = this.#buffer.slice(offset);
    return frames;
  }

  finish(): void {
    if (this.#buffer.length !== 0) throw new Error("truncated frame");
  }
}

class ProtocolFailure extends Error {
  readonly code: ProtocolFailureCode;

  constructor(code: ProtocolFailureCode) {
    super(code);
    this.name = "ProtocolFailure";
    this.code = code;
  }
}

export class ProtocolPeer {
  readonly #decoder = new WireFrameDecoder();
  readonly #input: Readable;
  readonly #output: Writable;
  readonly #iterator: AsyncIterator<Buffer>;
  readonly #pending: Uint8Array[] = [];
  #controlState: WorkerControlState | undefined;
  #cancelled = false;
  #writeChain = Promise.resolve();
  #terminal = false;
  #recordedFailure: ProtocolFailureCode | undefined;
  #inputRelease: Promise<void> | undefined;

  constructor(input: Readable = process.stdin, output: Writable = process.stdout) {
    this.#input = input;
    this.#output = output;
    this.#iterator = input[Symbol.asyncIterator]();
  }

  attachControl(state: WorkerControlState): void {
    if (this.#controlState !== undefined) throw new ProtocolFailure("protocol.invalid_sequence");
    this.#controlState = state;
  }

  async read(finalInboundFrame = false): Promise<FrameRecord> {
    if (this.#terminal) throw new ProtocolFailure("protocol.trailing_input");
    for (;;) {
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
        let frames: readonly Uint8Array[];
        try {
          frames = this.#decoder.push(next.value);
        } catch {
          throw new ProtocolFailure("protocol.invalid_frame");
        }
        const executionFrames = frames.filter((frame) => frameProtocol(frame) !== "qa.local-worker-control/v1");
        if (
          (this.#controlState === undefined && frames.length + this.#pending.length > MAX_PENDING_FRAMES) ||
          executionFrames.length > MAX_PENDING_FRAMES ||
          (frames.length > 0 && this.#decoder.bufferedBytes() > 0)
        ) {
          throw new ProtocolFailure(finalInboundFrame ? "protocol.trailing_input" : "protocol.invalid_sequence");
        }
        this.#pending.push(...frames);
      }
      const raw = this.#pending.shift();
      if (raw === undefined) throw new ProtocolFailure("protocol.invalid_frame");
      if (frameProtocol(raw) === "qa.local-worker-control/v1") {
        await this.#handleControl(raw);
        if (this.#cancelled && this.#pending.length === 0) throw new WorkerCancelled();
        continue;
      }
      if (this.#cancelled) throw new WorkerCancelled();
      let value: unknown;
      try {
        value = validateLocalWorkerFrame(raw).value();
      } catch {
        throw new ProtocolFailure("protocol.invalid_frame");
      }
      if (!isRecord(value)) throw new ProtocolFailure("protocol.invalid_frame");
      return value;
    }
  }

  async #handleControl(raw: Uint8Array): Promise<void> {
    let abort: AbortFrame;
    try {
      abort = validateLocalWorkerAbort(raw).value() as AbortFrame;
    } catch {
      const controlId = frameString(raw, "control_id");
      if (controlId === undefined) throw new ProtocolFailure("protocol.invalid_frame");
      await this.#enqueueWrite(
        {
          protocol: "qa.local-worker-control/v1",
          kind: "control_failure",
          control_id: controlId,
          code: "control.invalid_frame",
        },
        validateLocalWorkerControlFailure,
      );
      throw new WorkerCancelled();
    }
    try {
      if (this.#controlState === undefined) throw new Error("control.invalid_invocation");
      const acknowledgement = this.#controlState.acceptAbort(raw);
      await this.#enqueueWrite(acknowledgement, validateLocalWorkerCancelAck);
      if (acknowledgement.status === "accepted") this.#cancelled = true;
    } catch (error) {
      if (error instanceof WorkerCancelled) throw error;
      const code = controlFailureCode(error);
      await this.#enqueueWrite(
        {
          protocol: "qa.local-worker-control/v1",
          kind: "control_failure",
          control_id: abort.control_id,
          code,
        },
        validateLocalWorkerControlFailure,
      );
      this.#cancelled = true;
    }
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

  releaseInput(): Promise<void> {
    this.#inputRelease ??= (async () => {
      await this.#writeChain;
      try {
        await this.#iterator.return?.();
      } finally {
        this.#input.destroy();
      }
    })();
    return this.#inputRelease;
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
    for (;;) {
      while (this.#pending.length > 0) {
        const raw = this.#pending.shift();
        if (raw === undefined || frameProtocol(raw) !== "qa.local-worker-control/v1") {
          throw new ProtocolFailure("protocol.trailing_input");
        }
        await this.#handleControl(raw);
      }
      if (this.#cancelled) throw new WorkerCancelled();
      const next = await this.#iterator.next();
      if (next.done === true) {
        try {
          this.#decoder.finish();
        } catch {
          throw new ProtocolFailure("protocol.trailing_input");
        }
        return;
      }
      let frames: readonly Uint8Array[];
      try {
        frames = this.#decoder.push(next.value);
      } catch {
        throw new ProtocolFailure("protocol.trailing_input");
      }
      if (frames.length > 0 && this.#decoder.bufferedBytes() > 0) {
        throw new ProtocolFailure("protocol.trailing_input");
      }
      this.#pending.push(...frames);
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
    const write = this.#writeChain.then(() => writeStdout(this.#output, frame));
    this.#writeChain = write.catch(() => undefined);
    await write;
  }
}

class CapabilityClient {
  readonly #peer: ProtocolPeer;
  readonly #invocationId: string;
  readonly #control: WorkerControlState;
  #nextRequest = 0;
  #outstanding = false;

  constructor(peer: ProtocolPeer, invocationId: string, control: WorkerControlState) {
    this.#peer = peer;
    this.#invocationId = invocationId;
    this.#control = control;
  }

  async request(capability: string, input: unknown): Promise<FrameRecord> {
    if (this.#outstanding || this.#nextRequest >= 7) {
      throw new ProtocolFailure("protocol.invalid_sequence");
    }
    this.#control.assertCapabilityAllowed();
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
      this.#control.assertCapabilityAllowed();
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
    const control = new WorkerControlState(invocationId);
    peer.attachControl(control);
    const capabilities = new CapabilityClient(peer, invocationId, control);
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
    control.markTerminal();
    await peer.writeTerminal({
      protocol: PROTOCOL,
      kind: "terminal_result",
      invocation_id: invocationId,
      outcome: "passed",
      result: bundle.result,
    });
  } catch (error) {
    if (error instanceof WorkerCancelled) {
      await peer.releaseInput();
      return;
    }
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

function writeStdout(output: Writable, frame: Uint8Array): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let callbackDone = false;
    let drainDone = false;
    const cleanup = () => {
      output.off("error", onError);
      output.off("drain", onDrain);
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
    output.once("error", onError);
    const accepted = output.write(frame, (error) => {
      if (error !== undefined && error !== null) {
        fail();
        return;
      }
      callbackDone = true;
      complete();
    });
    drainDone = accepted;
    if (!accepted) output.once("drain", onDrain);
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

type AbortFrame = Readonly<{
  protocol: "qa.local-worker-control/v1";
  kind: "abort";
  control_id: string;
  invocation_id: string;
  deadline_utc: string;
}>;

export type CancelAck = Readonly<{
  protocol: "qa.local-worker-control/v1";
  kind: "cancel_ack";
  control_id: string;
  invocation_id: string;
  status: "accepted" | "too_late";
}>;

export class WorkerControlState {
  readonly #invocationId: string;
  #terminal = false;
  #accepted: AbortFrame | undefined;

  constructor(invocationId: string) {
    if (invocationId.length === 0) throw new Error("control.invalid_invocation");
    this.#invocationId = invocationId;
  }

  acceptAbort(raw: Uint8Array): CancelAck {
    const value = validateLocalWorkerAbort(raw).value() as AbortFrame;
    if (value.invocation_id !== this.#invocationId) {
      throw new Error("control.invalid_invocation");
    }
    if (this.#accepted !== undefined) {
      if (!sameAbort(this.#accepted, value)) throw new Error("control.conflict");
      return this.#ack(value, "accepted");
    }
    if (this.#terminal) return this.#ack(value, "too_late");
    if (Date.parse(value.deadline_utc) <= Date.now()) throw new Error("control.deadline_elapsed");
    this.#accepted = value;
    return this.#ack(value, "accepted");
  }

  markTerminal(): void {
    this.#terminal = true;
  }

  assertCapabilityAllowed(): void {
    if (this.#accepted !== undefined) throw new Error("control.cancelled");
  }

  cancelled(): boolean {
    return this.#accepted !== undefined;
  }

  #ack(abort: AbortFrame, status: "accepted" | "too_late"): CancelAck {
    const ack = {
      protocol: "qa.local-worker-control/v1",
      kind: "cancel_ack",
      control_id: abort.control_id,
      invocation_id: abort.invocation_id,
      status,
    } as const;
    return validateLocalWorkerCancelAck(
      new TextEncoder().encode(JSON.stringify(ack)),
    ).value() as CancelAck;
  }
}

function sameAbort(left: AbortFrame, right: AbortFrame): boolean {
  return (
    left.control_id === right.control_id &&
    left.invocation_id === right.invocation_id &&
    left.deadline_utc === right.deadline_utc
  );
}

function frameProtocol(raw: Uint8Array): string | undefined {
  try {
    const value = JSON.parse(new TextDecoder().decode(raw)) as unknown;
    return isRecord(value) && typeof value.protocol === "string" ? value.protocol : undefined;
  } catch {
    return undefined;
  }
}

function frameString(raw: Uint8Array, field: string): string | undefined {
  try {
    const value = JSON.parse(new TextDecoder().decode(raw)) as unknown;
    return isRecord(value) && typeof value[field] === "string" ? value[field] : undefined;
  } catch {
    return undefined;
  }
}

function controlFailureCode(
  error: unknown,
): "control.conflict" | "control.invalid_invocation" | "control.invalid_frame" | "control.deadline_elapsed" {
  if (error instanceof Error && error.message === "control.conflict") return "control.conflict";
  if (error instanceof Error && error.message === "control.invalid_invocation") {
    return "control.invalid_invocation";
  }
  if (error instanceof Error && error.message === "control.deadline_elapsed") {
    return "control.deadline_elapsed";
  }
  return "control.invalid_frame";
}
