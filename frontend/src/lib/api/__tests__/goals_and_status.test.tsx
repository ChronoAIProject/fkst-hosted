import React from 'react';
import { describe, it, expect, beforeAll, afterEach, afterAll } from 'vitest';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ApiError } from '../client';

import { goalStatusPresentation } from '../goal-status';
import {
  listGoals,
  getGoal,
  createGoal,
  updateGoal,
  deleteGoal,
  triggerGoal,
  mapRepoTargetError,
} from '../goals';
import {
  getIssuesAggregate,
  getIssue,
  createIssue,
  patchIssue,
  listComments,
  createComment,
} from '../github-issues';
import {
  updatePackage,
  deletePackage,
  archiveCreate,
  archiveReplace,
  generatePackage,
  listShares,
  createShare,
  deleteShare,
} from '../packages-extra';
import { useGoalsList, useGoal } from '../../hooks/useGoals';

interface MockRequestBody {
  title?: string;
  description?: string;
  package_names?: string[];
  repo?: unknown;
  org_id?: string;
  account?: string;
  body?: string;
  labels?: string[];
  assignees?: string[];
  state?: string;
  files?: unknown[];
  composed_deps?: string[];
  name?: string;
  save?: boolean;
  grantee_kind?: 'user' | 'org';
  grantee_id?: string;
  level?: 'read' | 'use';
}

// MSW Setup
const handlers = [
  // Goals
  http.get('*/api/v1/goals', () => {
    return HttpResponse.json([
      {
        id: 'goal-1',
        title: 'Goal One',
        description: 'First goal',
        package_names: ['pkg1'],
        repo: null,
        status: 'not_started',
        owner_user_id: 'user-1',
        org_id: null,
        active_session_id: null,
        created_at: '2026-06-15T00:00:00Z',
        updated_at: '2026-06-15T00:00:00Z',
      },
    ]);
  }),
  http.get('*/api/v1/goals/:id', ({ params }) => {
    return HttpResponse.json({
      id: params.id,
      title: 'Goal One',
      description: 'First goal',
      package_names: ['pkg1'],
      repo: null,
      status: 'not_started',
      owner_user_id: 'user-1',
      org_id: null,
      active_session_id: null,
      created_at: '2026-06-15T00:00:00Z',
      updated_at: '2026-06-15T00:00:00Z',
    });
  }),
  http.post('*/api/v1/goals', async ({ request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      id: 'goal-new',
      title: body.title,
      description: body.description,
      package_names: body.package_names,
      repo: body.repo || null,
      status: 'not_started',
      owner_user_id: 'user-1',
      org_id: body.org_id || null,
      active_session_id: null,
      created_at: '2026-06-15T00:00:00Z',
      updated_at: '2026-06-15T00:00:00Z',
    }, { status: 201 });
  }),
  http.patch('*/api/v1/goals/:id', async ({ params, request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      id: params.id,
      title: body.title || 'Goal One',
      description: body.description || 'First goal',
      package_names: body.package_names || ['pkg1'],
      repo: body.repo || null,
      status: 'not_started',
      owner_user_id: 'user-1',
      org_id: null,
      active_session_id: null,
      created_at: '2026-06-15T00:00:00Z',
      updated_at: '2026-06-15T00:00:00Z',
    });
  }),
  http.delete('*/api/v1/goals/:id', () => {
    return new HttpResponse(null, { status: 204 });
  }),
  http.post('*/api/v1/goals/:id/trigger', ({ params }) => {
    return HttpResponse.json({
      goal_id: params.id,
      session_id: 'session-123',
      goal_status: 'triggered',
      session_status: 'pending',
    }, { status: 202 });
  }),

  // GitHub Issues
  http.get('*/api/v1/github/issues', () => {
    return HttpResponse.json({
      results: [
        {
          account: 'octocat',
          issues: [],
          page: 1,
          per_page: 30,
          has_more: false,
        },
      ],
    });
  }),
  http.get('*/api/v1/github/repos/:owner/:repo/issues', ({ params }) => {
    if (params.owner === 'fail') {
      return new HttpResponse(null, { status: 404 });
    }
    if (params.owner === 'uninstalled') {
      return new HttpResponse(null, { status: 422 });
    }
    return HttpResponse.json([{ number: 1, title: 'Issue 1' }]);
  }),
  http.get('*/api/v1/github/repos/:owner/:repo/issues/:number', ({ params }) => {
    return HttpResponse.json({
      account: 'octocat',
      repository: `${params.owner}/${params.repo}`,
      number: Number(params.number),
      id: 12345,
      title: 'Fix issue',
      body: 'Detailed body',
      state: 'open',
      labels: [],
      assignees: [],
      comments: 0,
      html_url: '',
      created_at: '',
      updated_at: '',
    });
  }),
  http.post('*/api/v1/github/repos/:owner/:repo/issues', async ({ params, request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      account: body.account || 'octocat',
      repository: `${params.owner}/${params.repo}`,
      number: 42,
      id: 12345,
      title: body.title,
      body: body.body || null,
      state: 'open',
      labels: body.labels || [],
      assignees: body.assignees || [],
      comments: 0,
      html_url: '',
      created_at: '',
      updated_at: '',
    }, { status: 201 });
  }),
  http.patch('*/api/v1/github/repos/:owner/:repo/issues/:number', async ({ params, request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      account: 'octocat',
      repository: `${params.owner}/${params.repo}`,
      number: Number(params.number),
      id: 12345,
      title: body.title || 'Updated title',
      body: body.body || null,
      state: body.state || 'open',
      labels: body.labels || [],
      assignees: body.assignees || [],
      comments: 0,
      html_url: '',
      created_at: '',
      updated_at: '',
    });
  }),
  http.get('*/api/v1/github/repos/:owner/:repo/issues/:number/comments', () => {
    return HttpResponse.json([
      {
        id: 1,
        user: 'octocat',
        body: 'Comment body',
        html_url: '',
        created_at: '',
        updated_at: '',
      },
    ]);
  }),
  http.post('*/api/v1/github/repos/:owner/:repo/issues/:number/comments', async ({ request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      id: 2,
      user: body.account || 'octocat',
      body: body.body,
      html_url: '',
      created_at: '',
      updated_at: '',
    }, { status: 201 });
  }),

  // Packages Extra
  http.put('*/api/v1/packages/:name', async ({ params, request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      name: params.name,
      files: body.files || [],
      composed_deps: body.composed_deps || [],
      owner_user_id: 'user-1',
      org_id: null,
      created_at: '2026-06-15T00:00:00Z',
      updated_at: '2026-06-15T00:00:00Z',
    });
  }),
  http.delete('*/api/v1/packages/:name', () => {
    return new HttpResponse(null, { status: 204 });
  }),
  http.post('*/api/v1/packages/:name/archive', ({ params }) => {
    return HttpResponse.json({ name: params.name }, { status: 201 });
  }),
  http.put('*/api/v1/packages/:name/archive', ({ params }) => {
    return HttpResponse.json({
      name: params.name,
      files: [],
      composed_deps: [],
      owner_user_id: 'user-1',
      org_id: null,
      created_at: '2026-06-15T00:00:00Z',
      updated_at: '2026-06-15T00:00:00Z',
    });
  }),
  http.post('*/api/v1/packages/generate', async ({ request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      package: {
        name: body.name || 'gen-123',
        files: [],
        composed_deps: [],
      },
      validation: { ok: true, errors: [] },
      conformance: { status: 'ok', errors: [], skipped_reason: null },
      saved: body.save || false,
      save_error: null,
      attempts: 1,
    });
  }),
  http.get('*/api/v1/packages/:name/shares', ({ params }) => {
    return HttpResponse.json([
      {
        id: 'share-1',
        package_name: params.name,
        grantee_kind: 'user',
        grantee_id: 'user-2',
        level: 'use',
        granted_by: 'user-1',
        created_at: '2026-06-15T00:00:00Z',
      },
    ]);
  }),
  http.post('*/api/v1/packages/:name/shares', async ({ params, request }) => {
    const body = (await request.json()) as MockRequestBody;
    return HttpResponse.json({
      id: 'share-new',
      package_name: params.name,
      grantee_kind: body.grantee_kind,
      grantee_id: body.grantee_id,
      level: body.level,
      granted_by: 'user-1',
      created_at: '2026-06-15T00:00:00Z',
    }, { status: 201 });
  }),
  http.delete('*/api/v1/packages/:name/shares/:shareId', () => {
    return new HttpResponse(null, { status: 204 });
  }),
];

const server = setupServer(...handlers);

beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

function createTestWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

describe('Foundation W0 Client & Hooks Tests', () => {
  describe('goalStatusPresentation Helper', () => {
    it('correctly maps tones and labels for all GoalStatus values', () => {
      expect(goalStatusPresentation('not_started')).toEqual({ label: 'Not Started', tone: 'neutral' });
      expect(goalStatusPresentation('triggered')).toEqual({ label: 'Triggered', tone: 'gold' });
      expect(goalStatusPresentation('running')).toEqual({ label: 'Running', tone: 'green' });
      expect(goalStatusPresentation('stopped')).toEqual({ label: 'Stopped', tone: 'amber' });
      expect(goalStatusPresentation('failed')).toEqual({ label: 'Failed', tone: 'red' });
    });
  });

  describe('Goals Client API', () => {
    it('lists goals', async () => {
      const goals = await listGoals();
      expect(goals).toHaveLength(1);
      expect(goals[0]!.title).toBe('Goal One');
    });

    it('gets goal by id', async () => {
      const goal = await getGoal('goal-1');
      expect(goal.id).toBe('goal-1');
      expect(goal.title).toBe('Goal One');
    });

    it('creates a goal', async () => {
      const newGoal = await createGoal({
        title: 'New Title',
        description: 'New Description',
        package_names: ['pkg-a'],
      });
      expect(newGoal.id).toBe('goal-new');
      expect(newGoal.title).toBe('New Title');
    });

    it('updates a goal', async () => {
      const updated = await updateGoal('goal-1', { title: 'Updated Title' });
      expect(updated.title).toBe('Updated Title');
    });

    it('deletes a goal', async () => {
      await expect(deleteGoal('goal-1')).resolves.toBeUndefined();
    });

    it('triggers a goal', async () => {
      const res = await triggerGoal('goal-1', { repo_mode: 'existing' });
      expect(res.goal_status).toBe('triggered');
      expect(res.session_id).toBe('session-123');
    });
  });

  describe('GitHub Issues Client API', () => {
    it('aggregates issues', async () => {
      const env = await getIssuesAggregate();
      expect(env.results).toHaveLength(1);
      expect(env.results[0]!.account).toBe('octocat');
    });

    it('gets a single issue', async () => {
      const issue = await getIssue('owner', 'repo', 42);
      expect(issue.number).toBe(42);
      expect(issue.repository).toBe('owner/repo');
    });

    it('creates an issue', async () => {
      const issue = await createIssue('owner', 'repo', { title: 'Bug report' });
      expect(issue.title).toBe('Bug report');
      expect(issue.number).toBe(42);
    });

    it('patches an issue', async () => {
      const issue = await patchIssue('owner', 'repo', 42, { state: 'closed' });
      expect(issue.state).toBe('closed');
    });

    it('lists comments', async () => {
      const comments = await listComments('owner', 'repo', 42);
      expect(comments).toHaveLength(1);
      expect(comments[0]!.user).toBe('octocat');
    });

    it('creates a comment', async () => {
      const comment = await createComment('owner', 'repo', 42, 'new comment');
      expect(comment.body).toBe('new comment');
    });
  });

  describe('Packages Extra Client API', () => {
    it('updates a package', async () => {
      const pkg = await updatePackage('pkg1', { files: [] });
      expect(pkg.name).toBe('pkg1');
    });

    it('deletes a package', async () => {
      await expect(deletePackage('pkg1')).resolves.toBeUndefined();
    });

    it('archive create', async () => {
      const res = await archiveCreate('pkg1', new Uint8Array());
      expect(res.name).toBe('pkg1');
    });

    it('archive replace', async () => {
      const pkg = await archiveReplace('pkg1', new Uint8Array());
      expect(pkg.name).toBe('pkg1');
    });

    it('generates a package', async () => {
      const report = await generatePackage({ description: 'test gen', save: true });
      expect(report.saved).toBe(true);
      expect(report.package.name).toBe('gen-123');
    });

    it('lists shares', async () => {
      const shares = await listShares('pkg1');
      expect(shares).toHaveLength(1);
      expect(shares[0]!.grantee_id).toBe('user-2');
    });

    it('creates a share', async () => {
      const share = await createShare('pkg1', {
        grantee_kind: 'org',
        grantee_id: 'org-1',
        level: 'read',
      });
      expect(share.id).toBe('share-new');
      expect(share.grantee_id).toBe('org-1');
    });

    it('deletes a share', async () => {
      await expect(deleteShare('pkg1', 'share-1')).resolves.toBeUndefined();
    });
  });

  describe('Hooks', () => {
    it('useGoalsList retrieves list of goals', async () => {
      const { result } = renderHook(() => useGoalsList(), { wrapper: createTestWrapper() });
      await waitFor(() => expect(result.current.isSuccess).toBe(true));
      expect(result.current.data).toHaveLength(1);
      expect(result.current.data?.[0]?.title).toBe('Goal One');
    });

    it('useGoal retrieves specific goal details', async () => {
      const { result } = renderHook(() => useGoal('goal-1'), { wrapper: createTestWrapper() });
      await waitFor(() => expect(result.current.isSuccess).toBe(true));
      expect(result.current.data?.id).toBe('goal-1');
    });

    it('mapRepoTargetError maps errors correctly', () => {
      const err404 = new ApiError(404, null, 'Not found');
      expect(mapRepoTargetError(err404, 'trigger')).toBe('no access to that repo');
      expect(mapRepoTargetError(err404, 'issues')).toBe('no access to that repo');

      const err403 = new ApiError(403, null, 'Forbidden');
      expect(mapRepoTargetError(err403, 'trigger')).toBe('no access to that repo');
      expect(mapRepoTargetError(err403, 'issues')).toBe('no access to that repo');

      const err422Trigger = new ApiError(422, { error: 'unprocessable', message: 'github app not installed on foo/bar (https://github.com/apps/install)' }, 'Unprocessable');
      expect(mapRepoTargetError(err422Trigger, 'trigger')).toBe('github app not installed on foo/bar (https://github.com/apps/install)');

      const err422TriggerNoMsg = new ApiError(422, null, 'Unprocessable');
      expect(mapRepoTargetError(err422TriggerNoMsg, 'trigger')).toBe('GitHub App not installed on owner/repo');

      const err422Issues = new ApiError(422, null, 'Unprocessable');
      expect(mapRepoTargetError(err422Issues, 'issues')).toBe('account selection / validation error');

      const stdErr = new Error('Standard Error');
      expect(mapRepoTargetError(stdErr, 'trigger')).toBe('Standard Error');
    });
  });
});
