// Fetch layer for the in-bundle log viewer (spec B4). Each function takes the
// caller's `apiFetch` (the token-bearing fetch from useAuth) as a dependency,
// so components stay testable with a plain stub. Both endpoints are identity +
// three-tier authz gated on the backend; the frontend just surfaces the result.

import { assertShape, type ApiFetch } from './canvas';
import type { LogFileContent, LogManifest } from './types';

/** Default tail window for a log file: the last ~200 KiB is plenty for a live
 *  read and keeps the response (and the DOM) bounded on multi-MB logs. */
export const DEFAULT_LOG_TAIL_BYTES = 200 * 1024;

/** Error carrying the HTTP status of a failed log request so a caller can tell
 *  503 (log storage not configured for this deployment) from a transient
 *  failure without string-matching. */
export class LogError extends Error {
  readonly status: number;
  constructor(kind: 'manifest' | 'file' | 'runs', status: number) {
    super(`log ${kind} failed: ${status}`);
    this.name = 'LogError';
    this.status = status;
  }
}

/** One pod incarnation ("run") that served the session, as returned by the
 *  runs endpoint (NEWEST FIRST). `started_at`/`ended_at` are RFC3339 UTC;
 *  `started_at` MAY be empty for a legacy session's single synthetic run
 *  (`run_id: "latest"`), and `ended_at` is absent for the current, still-running
 *  incarnation. */
export interface LogRun {
  run_id: string;
  started_at: string;
  ended_at?: string;
}

/** A run id worth sending as an explicit `run` query param. The latest bundle
 *  is the backend default, so an absent / empty / `"latest"` id is omitted,
 *  keeping the request byte-identical to the pre-runs call (and the existing
 *  latest-only call sites unchanged). */
function runParam(run?: string): string | undefined {
  return run && run !== 'latest' ? run : undefined;
}

/** GET /api/v1/logs/{session_id}/runs — the session's pod incarnations, newest
 *  first. Throws the shared typed {@link LogError} (carrying the HTTP status)
 *  on a non-2xx so a 503 (log storage not configured) is distinguishable from a
 *  transient failure, mirroring {@link getLogManifest}. */
export async function getLogRuns(apiFetch: ApiFetch, sessionId: string): Promise<LogRun[]> {
  const res = await apiFetch(`/api/v1/logs/${encodeURIComponent(sessionId)}/runs`);
  if (!res.ok) throw new LogError('runs', res.status);
  const body = (await res.json()) as LogRun[];
  assertShape(Array.isArray(body), 'log runs');
  return body;
}

/** GET /api/v1/logs/{session_id}/manifest — the bundle's file listing. `run`
 *  selects a specific incarnation's bundle; absent / `"latest"` is the latest
 *  bundle (unchanged). */
export async function getLogManifest(
  apiFetch: ApiFetch,
  sessionId: string,
  run?: string
): Promise<LogManifest> {
  const query = new URLSearchParams();
  const r = runParam(run);
  if (r) query.set('run', r);
  const qs = query.toString();
  const res = await apiFetch(
    `/api/v1/logs/${encodeURIComponent(sessionId)}/manifest${qs ? `?${qs}` : ''}`
  );
  if (!res.ok) throw new LogError('manifest', res.status);
  const body = (await res.json()) as LogManifest;
  assertShape(Array.isArray(body?.files), 'log manifest');
  return body;
}

/** GET /api/v1/logs/{session_id}/file — one decompressed file's UTF-8 text.
 *  `path` must match a manifest entry exactly (the backend rejects traversal /
 *  unknown paths with 404). When `tailBytes` is set, only the last N bytes are
 *  returned (snapped to a line boundary) and `truncated` is true. `run` selects
 *  a specific incarnation's bundle; absent / `"latest"` is the latest bundle. */
export async function getLogFile(
  apiFetch: ApiFetch,
  sessionId: string,
  path: string,
  tailBytes?: number,
  run?: string
): Promise<LogFileContent> {
  const query = new URLSearchParams({ path });
  if (tailBytes != null) query.set('tail_bytes', String(tailBytes));
  const r = runParam(run);
  if (r) query.set('run', r);
  const res = await apiFetch(
    `/api/v1/logs/${encodeURIComponent(sessionId)}/file?${query.toString()}`
  );
  if (!res.ok) throw new LogError('file', res.status);
  const body = (await res.json()) as LogFileContent;
  assertShape(typeof body?.content === 'string', 'log file');
  return body;
}
