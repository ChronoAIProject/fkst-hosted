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
  constructor(kind: 'manifest' | 'file', status: number) {
    super(`log ${kind} failed: ${status}`);
    this.name = 'LogError';
    this.status = status;
  }
}

/** GET /api/v1/logs/{session_id}/manifest — the bundle's file listing. */
export async function getLogManifest(
  apiFetch: ApiFetch,
  sessionId: string
): Promise<LogManifest> {
  const res = await apiFetch(`/api/v1/logs/${encodeURIComponent(sessionId)}/manifest`);
  if (!res.ok) throw new LogError('manifest', res.status);
  const body = (await res.json()) as LogManifest;
  assertShape(Array.isArray(body?.files), 'log manifest');
  return body;
}

/** GET /api/v1/logs/{session_id}/file — one decompressed file's UTF-8 text.
 *  `path` must match a manifest entry exactly (the backend rejects traversal /
 *  unknown paths with 404). When `tailBytes` is set, only the last N bytes are
 *  returned (snapped to a line boundary) and `truncated` is true. */
export async function getLogFile(
  apiFetch: ApiFetch,
  sessionId: string,
  path: string,
  tailBytes?: number
): Promise<LogFileContent> {
  const query = new URLSearchParams({ path });
  if (tailBytes != null) query.set('tail_bytes', String(tailBytes));
  const res = await apiFetch(
    `/api/v1/logs/${encodeURIComponent(sessionId)}/file?${query.toString()}`
  );
  if (!res.ok) throw new LogError('file', res.status);
  const body = (await res.json()) as LogFileContent;
  assertShape(typeof body?.content === 'string', 'log file');
  return body;
}
