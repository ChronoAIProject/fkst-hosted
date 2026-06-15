import { request, requestVoid, ApiError } from './client';
import { RepoRef } from './types';

export type GoalStatus =
  | 'not_started'
  | 'triggered'
  | 'running'
  | 'stopped'
  | 'failed';

export interface GoalView {
  id: string;
  title: string;
  description: string;
  package_names: string[];
  repo: RepoRef | null;
  status: GoalStatus;
  owner_user_id: string;
  org_id: string | null;
  active_session_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateGoalRequest {
  title: string;
  description: string;
  package_names: string[];
  repo?: RepoRef | null;
  org_id?: string | null;
}

export interface UpdateGoalRequest {
  title?: string;
  description?: string;
  package_names?: string[];
  repo?: RepoRef | null;
  clear_repo?: boolean | null;
}

export interface TriggerRequest {
  repo?: RepoRef | null;
  repo_mode?: 'existing' | 'create_new';
  create?: {
    name: string;
    private?: boolean;
    description?: string | null;
    org_login?: string | null;
  } | null;
}

export interface TriggerResponse {
  goal_id: string;
  session_id: string;
  goal_status: GoalStatus;
  session_status: string;
}

/**
 * GET /api/v1/goals
 */
export async function listGoals(params?: {
  status?: string;
  limit?: number;
  offset?: number;
}): Promise<GoalView[]> {
  const searchParams = new URLSearchParams();
  if (params?.status) {
    searchParams.append('status', params.status);
  }
  if (params?.limit !== undefined) {
    searchParams.append('limit', params.limit.toString());
  }
  if (params?.offset !== undefined) {
    searchParams.append('offset', params.offset.toString());
  }
  const query = searchParams.toString();
  const path = query ? `/api/v1/goals?${query}` : '/api/v1/goals';
  return request<GoalView[]>(path);
}

/**
 * GET /api/v1/goals/:id
 */
export async function getGoal(id: string): Promise<GoalView> {
  const encodedId = encodeURIComponent(id);
  return request<GoalView>(`/api/v1/goals/${encodedId}`);
}

/**
 * POST /api/v1/goals
 */
export async function createGoal(req: CreateGoalRequest): Promise<GoalView> {
  return request<GoalView>('/api/v1/goals', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(req),
  });
}

/**
 * PATCH /api/v1/goals/:id
 */
export async function updateGoal(id: string, req: UpdateGoalRequest): Promise<GoalView> {
  const encodedId = encodeURIComponent(id);
  return request<GoalView>(`/api/v1/goals/${encodedId}`, {
    method: 'PATCH',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(req),
  });
}

/**
 * DELETE /api/v1/goals/:id
 */
export async function deleteGoal(id: string): Promise<void> {
  const encodedId = encodeURIComponent(id);
  return requestVoid(`/api/v1/goals/${encodedId}`, {
    method: 'DELETE',
  });
}

/**
 * POST /api/v1/goals/:id/trigger
 */
export async function triggerGoal(id: string, req: TriggerRequest): Promise<TriggerResponse> {
  const encodedId = encodeURIComponent(id);
  return request<TriggerResponse>(`/api/v1/goals/${encodedId}/trigger`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(req),
  });
}

/**
 * Maps create/trigger errors into honest user-facing copies.
 */
export function mapRepoTargetError(err: unknown, context: 'trigger' | 'issues'): string {
  if (err instanceof ApiError) {
    if (err.status === 404 || err.status === 403) {
      return 'no access to that repo';
    }
    if (err.status === 422) {
      if (context === 'trigger') {
        const message = err.body && 'message' in err.body ? String(err.body.message) : '';
        if (message && message.toLowerCase().includes('github app not installed')) {
          return message;
        }
        return 'GitHub App not installed on owner/repo';
      } else {
        return 'account selection / validation error';
      }
    }
    const message = err.body && 'message' in err.body ? String(err.body.message) : err.message;
    return message || 'An unexpected error occurred';
  }
  return err instanceof Error ? err.message : 'An unexpected error occurred';
}

