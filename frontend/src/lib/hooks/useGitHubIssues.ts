import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  getIssuesAggregate,
  getIssue,
  createIssue,
  patchIssue,
  listComments,
  createComment,
} from '../api/github-issues';

/**
 * Hook to retrieve aggregated GitHub issues across accounts.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): GitHub issues are read-heavy and cached for 30s to keep UI snappy.
 * - retry (false): Fail fast. If the proxy or GitHub returns an error (403, 404, 429), surface it immediately.
 */
export function useGitHubIssues(params?: {
  accounts?: string;
  filter?: string;
  state?: string;
  labels?: string;
  page?: number;
  per_page?: number;
}) {
  return useQuery({
    queryKey: ['github-issues', params],
    queryFn: () => getIssuesAggregate(params),
    staleTime: 30000,
    retry: false,
  });
}

/**
 * Hook to retrieve details for a specific GitHub issue.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Cache issue details for 30s.
 * - retry (false): Fail fast.
 * - enabled: Only executes if owner, repo, and number are provided.
 */
export function useIssue(
  owner: string | undefined,
  repo: string | undefined,
  number: number | undefined,
  account?: string
) {
  const isEnabled = !!owner && !!repo && number !== undefined;
  return useQuery({
    queryKey: ['github-issue', owner, repo, number, account],
    queryFn: () => getIssue(owner!, repo!, number!, account),
    staleTime: 30000,
    retry: false,
    enabled: isEnabled,
  });
}

/**
 * Hook to create a new GitHub issue.
 * Invalidates the aggregate issues list query.
 */
export function useCreateIssue() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      owner,
      repo,
      issue,
    }: {
      owner: string;
      repo: string;
      issue: {
        title: string;
        body?: string;
        labels?: string[];
        assignees?: string[];
        account?: string;
      };
    }) => createIssue(owner, repo, issue),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['github-issues'] });
    },
  });
}

/**
 * Hook to update a GitHub issue.
 * Invalidates the aggregate issues list and specific issue detail queries.
 */
export function usePatchIssue() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      owner,
      repo,
      number,
      patch,
    }: {
      owner: string;
      repo: string;
      number: number;
      patch: {
        title?: string;
        body?: string;
        state?: string;
        labels?: string[];
        assignees?: string[];
        account?: string;
      };
    }) => patchIssue(owner, repo, number, patch),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['github-issues'] });
      queryClient.invalidateQueries({
        queryKey: ['github-issue', variables.owner, variables.repo, variables.number],
      });
    },
  });
}

/**
 * Hook to list comments for a GitHub issue.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Cache comments for 30s.
 * - retry (false): Fail fast.
 * - enabled: Only executes if owner, repo, and number are provided.
 */
export function useComments(
  owner: string | undefined,
  repo: string | undefined,
  number: number | undefined,
  params?: {
    account?: string;
    page?: number;
    per_page?: number;
  }
) {
  const isEnabled = !!owner && !!repo && number !== undefined;
  return useQuery({
    queryKey: ['github-comments', owner, repo, number, params],
    queryFn: () => listComments(owner!, repo!, number!, params),
    staleTime: 30000,
    retry: false,
    enabled: isEnabled,
  });
}

/**
 * Hook to add a comment to a GitHub issue.
 * Invalidates the comments query and specific issue detail query (to update comment count).
 */
export function useCreateComment() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      owner,
      repo,
      number,
      body,
      account,
    }: {
      owner: string;
      repo: string;
      number: number;
      body: string;
      account?: string;
    }) => createComment(owner, repo, number, body, account),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({
        queryKey: ['github-comments', variables.owner, variables.repo, variables.number],
      });
      queryClient.invalidateQueries({
        queryKey: ['github-issue', variables.owner, variables.repo, variables.number],
      });
    },
  });
}
