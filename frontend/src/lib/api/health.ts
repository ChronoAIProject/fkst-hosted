// Fetch layer for a session's business-aware health reports. Each function takes
// the caller's `apiFetch` (the token-bearing fetch from useAuth) as a dependency,
// so components stay testable with a plain stub. Both endpoints are gated by the
// same grant as the log endpoints; the frontend just surfaces the result.

import { assertShape, type ApiFetch } from './canvas';

/** The v1 report status taxonomy. `unknown` is also what the backend maps any
 *  status string it does not recognize to — the raw string is preserved in
 *  `status_raw`, so a newer producer's vocabulary stays displayable. */
export type HealthStatus =
  | 'working'
  | 'idle'
  | 'blocked'
  | 'stalled'
  | 'failing'
  | 'unknown';

/** The heartbeat verdict.
 *
 *  `not_running` is NOT a fault: reports stop when a pod is reaped, which is the
 *  normal end of a session's work. Only `stale` — the runtime is live and its
 *  reports stopped — indicates something is wrong. */
export type StalenessState = 'not_running' | 'never_reported' | 'fresh' | 'stale';

export interface HealthStaleness {
  state: StalenessState;
  /** The producer's own declared cadence; absent until a report has been seen. */
  expected_interval_secs?: number | null;
  /** Seconds since the newest report; absent when there is none. */
  age_secs?: number | null;
}

/** One report as listed — everything a badge or a history row needs. */
export interface HealthReportSummary {
  id: string;
  generated_at: string;
  status: HealthStatus;
  status_raw: string;
  headline: string;
  producer: string;
}

export interface SessionHealth {
  session_id: string;
  /** Newest first. Empty is a normal state, not an error. */
  reports: HealthReportSummary[];
  latest?: HealthReportSummary | null;
  staleness: HealthStaleness;
}

export interface HealthEvidence {
  key: string;
  value: string;
}

export interface HealthWorkItem {
  number: number;
  state: string;
  progress: string;
}

export interface HealthReport {
  session_id: string;
  id: string;
  generated_at: string;
  window_start?: string | null;
  status: HealthStatus;
  status_raw: string;
  headline: string;
  producer: string;
  confidence?: string | null;
  expected_interval_secs: number;
  evidence: HealthEvidence[];
  work_items: HealthWorkItem[];
  /** The producer's narrative, verbatim.
   *
   *  UNTRUSTED: authored by an LLM inside a session pod. Render it only through
   *  `MarkdownPreview`, which emits React elements (never raw HTML) and
   *  protocol-allowlists links. */
  body_markdown: string;
}

/** Error carrying the HTTP status of a failed health request, so a caller can
 *  tell 503 (health reporting not configured for this deployment) from a
 *  transient failure without string-matching. Mirrors `LogError`. */
export class HealthError extends Error {
  readonly status: number;
  constructor(kind: 'health' | 'report', status: number) {
    super(`session ${kind} failed: ${status}`);
    this.name = 'HealthError';
    this.status = status;
  }
}

/** GET /api/v1/sessions/{session_id}/health — the report listing (newest first)
 *  plus the heartbeat verdict. An empty list is a 200, not a 404: the first
 *  report is simply not in yet. */
export async function getSessionHealth(
  apiFetch: ApiFetch,
  sessionId: string
): Promise<SessionHealth> {
  const res = await apiFetch(`/api/v1/sessions/${encodeURIComponent(sessionId)}/health`);
  if (!res.ok) throw new HealthError('health', res.status);
  const body = (await res.json()) as SessionHealth;
  assertShape(Array.isArray(body?.reports) && body?.staleness != null, 'session health');
  return body;
}

/** GET /api/v1/sessions/{session_id}/health/{report_id} — one report in full.
 *  `reportId` must match an id from the listing exactly (the backend validates it
 *  against the index and 404s anything else without touching storage). */
export async function getHealthReport(
  apiFetch: ApiFetch,
  sessionId: string,
  reportId: string
): Promise<HealthReport> {
  const res = await apiFetch(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/health/${encodeURIComponent(reportId)}`
  );
  if (!res.ok) throw new HealthError('report', res.status);
  const body = (await res.json()) as HealthReport;
  assertShape(typeof body?.body_markdown === 'string', 'health report');
  return body;
}
