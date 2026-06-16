import { request } from './client';

export interface IssueView {
  account: string;
  repository: string;
  number: number;
  id: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  assignees: string[];
  comments: number;
  html_url: string;
  created_at: string;
  updated_at: string;
}

export interface CommentView {
  id: number;
  user: string;
  body: string;
  html_url: string;
  created_at: string;
  updated_at: string;
}

export interface RateLimitView {
  remaining: number;
  reset_epoch: number;
}

export interface AccountError {
  kind: string;
  message: string;
  retry_after_secs?: number;
}

export interface AccountIssues {
  account: string;
  issues: IssueView[];
  page: number;
  per_page: number;
  has_more: boolean;
  rate_limit?: RateLimitView | null;
  error?: AccountError | null;
}

export interface IssuesEnvelope {
  results: AccountIssues[];
}

/**
 * GET /api/v1/github/issues
 */
export async function getIssuesAggregate(params?: {
  accounts?: string;
  filter?: string;
  state?: string;
  labels?: string;
  page?: number;
  per_page?: number;
}): Promise<IssuesEnvelope> {
  const searchParams = new URLSearchParams();
  if (params?.accounts) {
    searchParams.append('accounts', params.accounts);
  }
  if (params?.filter) {
    searchParams.append('filter', params.filter);
  }
  if (params?.state) {
    searchParams.append('state', params.state);
  }
  if (params?.labels) {
    searchParams.append('labels', params.labels);
  }
  if (params?.page !== undefined) {
    searchParams.append('page', params.page.toString());
  }
  if (params?.per_page !== undefined) {
    searchParams.append('per_page', params.per_page.toString());
  }
  const query = searchParams.toString();
  const path = query ? `/api/v1/github/issues?${query}` : '/api/v1/github/issues';
  return request<IssuesEnvelope>(path);
}

/**
 * GET /api/v1/github/repos/:owner/:repo/issues/:number
 */
export async function getIssue(
  owner: string,
  repo: string,
  number: number,
  account?: string
): Promise<IssueView> {
  const encodedOwner = encodeURIComponent(owner);
  const encodedRepo = encodeURIComponent(repo);
  const searchParams = new URLSearchParams();
  if (account) {
    searchParams.append('account', account);
  }
  const query = searchParams.toString();
  const path = `/api/v1/github/repos/${encodedOwner}/${encodedRepo}/issues/${number}`;
  const fullPath = query ? `${path}?${query}` : path;
  return request<IssueView>(fullPath);
}

/**
 * POST /api/v1/github/repos/:owner/:repo/issues
 */
export async function createIssue(
  owner: string,
  repo: string,
  issue: {
    title: string;
    body?: string;
    labels?: string[];
    assignees?: string[];
    account?: string;
  }
): Promise<IssueView> {
  const encodedOwner = encodeURIComponent(owner);
  const encodedRepo = encodeURIComponent(repo);
  return request<IssueView>(`/api/v1/github/repos/${encodedOwner}/${encodedRepo}/issues`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(issue),
  });
}

/**
 * PATCH /api/v1/github/repos/:owner/:repo/issues/:number
 */
export async function patchIssue(
  owner: string,
  repo: string,
  number: number,
  patch: {
    title?: string;
    body?: string;
    state?: string;
    labels?: string[];
    assignees?: string[];
    account?: string;
  }
): Promise<IssueView> {
  const encodedOwner = encodeURIComponent(owner);
  const encodedRepo = encodeURIComponent(repo);
  return request<IssueView>(`/api/v1/github/repos/${encodedOwner}/${encodedRepo}/issues/${number}`, {
    method: 'PATCH',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(patch),
  });
}

/**
 * GET /api/v1/github/repos/:owner/:repo/issues/:number/comments
 */
export async function listComments(
  owner: string,
  repo: string,
  number: number,
  params?: {
    account?: string;
    page?: number;
    per_page?: number;
  }
): Promise<CommentView[]> {
  const encodedOwner = encodeURIComponent(owner);
  const encodedRepo = encodeURIComponent(repo);
  const searchParams = new URLSearchParams();
  if (params?.account) {
    searchParams.append('account', params.account);
  }
  if (params?.page !== undefined) {
    searchParams.append('page', params.page.toString());
  }
  if (params?.per_page !== undefined) {
    searchParams.append('per_page', params.per_page.toString());
  }
  const query = searchParams.toString();
  const path = `/api/v1/github/repos/${encodedOwner}/${encodedRepo}/issues/${number}/comments`;
  const fullPath = query ? `${path}?${query}` : path;
  return request<CommentView[]>(fullPath);
}

/**
 * POST /api/v1/github/repos/:owner/:repo/issues/:number/comments
 */
export async function createComment(
  owner: string,
  repo: string,
  number: number,
  body: string,
  account?: string
): Promise<CommentView> {
  const encodedOwner = encodeURIComponent(owner);
  const encodedRepo = encodeURIComponent(repo);
  return request<CommentView>(`/api/v1/github/repos/${encodedOwner}/${encodedRepo}/issues/${number}/comments`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ body, account }),
  });
}
