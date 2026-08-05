// Fetch layer for the scheduled-workflow endpoints. Like the sibling clients it
// takes the caller's `apiFetch` (the token-bearing fetch from useAuth) as a
// dependency, so components stay testable with a plain stub and this module
// never imports auth state itself.
//
// One rule runs through the whole surface: NO cadence arithmetic here. Every
// firing time — `nextDue`, `upcoming` — arrives from the API, which computes it
// with the same code the control plane's clock uses. A second implementation in
// TypeScript would eventually drift, and the symptom would be a dashboard
// confidently showing a firing time the schedule does not honour.

import type { ApiFetch, MutationResult } from './canvas';
import { assertShape, readErrorMessage } from './canvas';

/** A schedule's lifecycle as the API states it. */
export type ScheduleLifecycle = 'idle' | 'running' | 'paused' | 'invalid';

/** The `fkst-cron-run:v1` status vocabulary. */
export type RunStatus = 'dispatched' | 'ok' | 'failed' | 'timeout' | 'skipped-overlap';

export type StepStatus = 'ok' | 'failed' | 'skipped';

export interface RunSummary {
  slot: string;
  manual: boolean;
  status: RunStatus;
  startedAt: string;
  endedAt: string | null;
  durationS: number | null;
  issue: number | null;
  detail: string | null;
}

export interface RunStepView {
  index: number;
  id: string;
  status: StepStatus;
  durationS: number | null;
}

export interface ScheduleSummary {
  scheduleIssue: number;
  title: string;
  htmlUrl: string;
  workflowId: string;
  runMode: string;
  cadence: string;
  state: ScheduleLifecycle;
  nextDue: string | null;
  lastRun: RunSummary | null;
  successRate30d: number | null;
  invalidDetail: string | null;
}

export interface ScheduleDetail {
  summary: ScheduleSummary;
  upcoming: string[];
  arguments: Record<string, string>;
  runs: RunSummary[];
}

export interface ScheduleRunDetail {
  run: RunSummary;
  steps: RunStepView[];
  runIssue: number | null;
}

export interface RepoSchedulesResponse {
  owner: string;
  name: string;
  installed: boolean;
  schedules: ScheduleSummary[];
}

const base = (owner: string, name: string) =>
  `/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/schedules`;

/** GET a repository's scheduled workflows. */
export async function listRepoSchedules(
  apiFetch: ApiFetch,
  owner: string,
  name: string
): Promise<RepoSchedulesResponse> {
  const res = await apiFetch(base(owner, name));
  if (!res.ok) throw new Error(`schedules ${res.status}`);
  const body = (await res.json()) as RepoSchedulesResponse;
  assertShape(Array.isArray(body?.schedules), 'schedules');
  return body;
}

/** GET one schedule with its upcoming firings and run history. */
export async function getSchedule(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  scheduleIssue: number
): Promise<ScheduleDetail> {
  const res = await apiFetch(`${base(owner, name)}/${scheduleIssue}`);
  if (!res.ok) throw new Error(`schedule ${res.status}`);
  const body = (await res.json()) as ScheduleDetail;
  assertShape(Array.isArray(body?.runs) && !!body?.summary, 'schedule detail');
  return body;
}

/** GET one run's per-step outcomes. The slot is a path segment, so it is
 *  encoded — an RFC 3339 instant carries a `+` in a non-UTC offset. */
export async function getScheduleRun(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  scheduleIssue: number,
  slot: string
): Promise<ScheduleRunDetail> {
  const res = await apiFetch(
    `${base(owner, name)}/${scheduleIssue}/runs/${encodeURIComponent(slot)}`
  );
  if (!res.ok) throw new Error(`schedule run ${res.status}`);
  const body = (await res.json()) as ScheduleRunDetail;
  assertShape(Array.isArray(body?.steps) && !!body?.run, 'schedule run');
  return body;
}

/** POST one of the three durable state changes.
 *
 *  The server's envelope message rides along on failure so the UI can surface it
 *  verbatim — a 409 explaining that a run is already in flight is far more useful
 *  than a generic "that did not work".
 */
async function mutate<T>(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  scheduleIssue: number,
  action: 'run' | 'pause' | 'resume',
  parse: (res: Response) => Promise<T>
): Promise<MutationResult<T>> {
  const res = await apiFetch(`${base(owner, name)}/${scheduleIssue}/${action}`, {
    method: 'POST',
  });
  if (!res.ok) return { ok: false, message: await readErrorMessage(res) };
  return { ok: true, data: await parse(res) };
}

/** POST …/run — dispatch a manual run, returning the created run issue. */
export function runScheduleNow(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  scheduleIssue: number
): Promise<MutationResult<number>> {
  return mutate(apiFetch, owner, name, scheduleIssue, 'run', (res) => res.json() as Promise<number>);
}

/** POST …/pause — idempotent. */
export function pauseSchedule(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  scheduleIssue: number
): Promise<MutationResult<null>> {
  return mutate(apiFetch, owner, name, scheduleIssue, 'pause', async () => null);
}

/** POST …/resume — idempotent. */
export function resumeSchedule(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  scheduleIssue: number
): Promise<MutationResult<null>> {
  return mutate(apiFetch, owner, name, scheduleIssue, 'resume', async () => null);
}
