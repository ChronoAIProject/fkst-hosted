import { describe, expect, it, vi } from 'vitest';
import {
  getSchedule,
  getScheduleRun,
  listRepoSchedules,
  pauseSchedule,
  resumeSchedule,
  runScheduleNow,
} from './schedules';

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

const summary = {
  scheduleIssue: 50,
  title: 'nightly',
  htmlUrl: 'https://github.com/acme/site/issues/50',
  workflowId: 'sourcing',
  runMode: 'cron: 0 1 * * 1-5',
  cadence: 'weekdays at 01:00 UTC',
  state: 'idle',
  nextDue: '2026-08-03T01:00:00Z',
  lastRun: null,
  successRate30d: null,
  invalidDetail: null,
};

describe('schedules client', () => {
  it('lists a repository’s schedules from the repo-scoped path', async () => {
    const apiFetch = vi.fn().mockResolvedValue(
      json({ owner: 'acme', name: 'site', installed: true, schedules: [summary] })
    );
    const response = await listRepoSchedules(apiFetch, 'acme', 'site');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/acme/site/schedules');
    expect(response.schedules).toHaveLength(1);
  });

  it('encodes path segments so an unusual owner or name cannot escape the path', async () => {
    const apiFetch = vi
      .fn()
      .mockResolvedValue(json({ owner: 'a', name: 'b', installed: true, schedules: [] }));
    await listRepoSchedules(apiFetch, 'a/b', 'c d');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/a%2Fb/c%20d/schedules');
  });

  it('encodes the slot, which carries characters a path segment cannot hold raw', async () => {
    const apiFetch = vi
      .fn()
      .mockResolvedValue(json({ run: { slot: 'x' }, steps: [], runIssue: null }));
    await getScheduleRun(apiFetch, 'acme', 'site', 50, '2026-08-03T01:00:00+02:00');
    expect(apiFetch).toHaveBeenCalledWith(
      '/api/v1/repos/acme/site/schedules/50/runs/2026-08-03T01%3A00%3A00%2B02%3A00'
    );
  });

  it('rejects a malformed payload at the boundary rather than deep inside a component', async () => {
    const apiFetch = vi.fn().mockResolvedValue(json({ owner: 'acme', name: 'site' }));
    await expect(listRepoSchedules(apiFetch, 'acme', 'site')).rejects.toThrow(/malformed/);
  });

  it('surfaces a non-2xx read as an error carrying its status', async () => {
    const apiFetch = vi.fn().mockResolvedValue(json({ message: 'nope' }, 404));
    await expect(getSchedule(apiFetch, 'acme', 'site', 50)).rejects.toThrow(/404/);
  });

  it('returns the created run issue on a successful run-now', async () => {
    const apiFetch = vi.fn().mockResolvedValue(json(4242, 202));
    const result = await runScheduleNow(apiFetch, 'acme', 'site', 50);
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/acme/site/schedules/50/run', {
      method: 'POST',
    });
    expect(result).toEqual({ ok: true, data: 4242 });
  });

  it('carries the server’s own message on a refused mutation', async () => {
    // A 409 explaining that a run is already in flight is far more useful to the
    // operator than a generic failure line, so it must survive to the UI.
    const apiFetch = vi
      .fn()
      .mockResolvedValue(json({ message: 'a run is already in flight for this schedule' }, 409));
    const result = await runScheduleNow(apiFetch, 'acme', 'site', 50);
    expect(result).toEqual({
      ok: false,
      message: 'a run is already in flight for this schedule',
    });
  });

  it('treats pause and resume as no-content mutations', async () => {
    const apiFetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    await expect(pauseSchedule(apiFetch, 'acme', 'site', 50)).resolves.toEqual({
      ok: true,
      data: null,
    });
    await expect(resumeSchedule(apiFetch, 'acme', 'site', 50)).resolves.toEqual({
      ok: true,
      data: null,
    });
    expect(apiFetch).toHaveBeenNthCalledWith(1, '/api/v1/repos/acme/site/schedules/50/pause', {
      method: 'POST',
    });
    expect(apiFetch).toHaveBeenNthCalledWith(2, '/api/v1/repos/acme/site/schedules/50/resume', {
      method: 'POST',
    });
  });
});
