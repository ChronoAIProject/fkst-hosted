import React from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import IssuesScreen from './issues-screen';
import { useGitHubAccounts } from '@/lib/hooks/useGitHubAccounts';
import {
  useGitHubIssues,
  useCreateIssue,
  usePatchIssue,
  useIssue,
  useComments,
  useCreateComment,
} from '@/lib/hooks/useGitHubIssues';

// Mock all hooks
vi.mock('@/lib/hooks/useGitHubAccounts');
vi.mock('@/lib/hooks/useGitHubIssues');

function renderWithProviders(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>
  );
}

describe('IssuesScreen', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubEnv('VITE_NYXID_CONNECT_GITHUB_URL', 'https://example.com/connect');

    vi.mocked(useGitHubIssues).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: false,
    } as any);

    vi.mocked(useCreateIssue).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    vi.mocked(usePatchIssue).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    vi.mocked(useIssue).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: false,
    } as any);

    vi.mocked(useComments).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: false,
    } as any);

    vi.mocked(useCreateComment).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);
  });

  it('renders loading state when accounts are loading', () => {
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: undefined,
      isLoading: true,
      isError: false,
    } as any);

    renderWithProviders(<IssuesScreen />);
    expect(screen.getByText('loading GitHub accounts...')).toBeInTheDocument();
  });

  it('renders ConnectGitHub empty state when no accounts are connected', () => {
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
    } as any);

    renderWithProviders(<IssuesScreen />);
    expect(screen.getByText('no GitHub accounts connected')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Connect GitHub/i })).toBeInTheDocument();
  });

  it('renders ConnectGitHub empty state on account check failure', () => {
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
    } as any);

    renderWithProviders(<IssuesScreen />);
    expect(screen.getByText(/GitHub status unknown/i)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Connect GitHub/i })).toBeInTheDocument();
  });

  it('renders issues list and groups per account when accounts exist', () => {
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [
        { connection_id: '1', login: 'user1', primary: true },
        { connection_id: '2', login: 'user2', primary: false },
      ],
      isLoading: false,
      isError: false,
    } as any);

    vi.mocked(useGitHubIssues).mockReturnValue({
      data: {
        results: [
          {
            account: 'user1',
            issues: [
              {
                id: 101,
                number: 1,
                repository: 'user1/repo1',
                title: 'Issue 1 on repo1',
                body: 'body 1',
                state: 'open',
                labels: ['bug'],
                assignees: [],
                comments: 2,
                html_url: 'https://github.com/user1/repo1/issues/1',
                created_at: '2026-06-15T00:00:00Z',
                updated_at: '2026-06-15T01:00:00Z',
              },
            ],
            page: 1,
            per_page: 30,
            has_more: false,
            rate_limit: { remaining: 4900, reset_epoch: 12345 },
            error: null,
          },
          {
            account: 'user2',
            issues: [],
            page: 1,
            per_page: 30,
            has_more: false,
            rate_limit: null,
            error: { kind: 'rate_limited', message: 'Rate limit hit' },
          },
        ],
      },
      isLoading: false,
      isError: false,
    } as any);

    // Mock other detail hooks returning idle states to avoid crashes
    vi.mocked(useIssue).mockReturnValue({ data: undefined, isLoading: false, isError: false } as any);
    vi.mocked(useComments).mockReturnValue({ data: undefined, isLoading: false, isError: false } as any);

    const mockCreateComment = { mutate: vi.fn(), isPending: false } as any;
    vi.mocked(useCreateComment).mockReturnValue(mockCreateComment);

    const mockPatchIssue = { mutate: vi.fn(), isPending: false } as any;
    vi.mocked(usePatchIssue).mockReturnValue(mockPatchIssue);

    const mockCreateIssue = { mutate: vi.fn(), isPending: false } as any;
    vi.mocked(useCreateIssue).mockReturnValue(mockCreateIssue);

    renderWithProviders(<IssuesScreen />);

    // Assert account headers exist
    expect(screen.getByText('@user1')).toBeInTheDocument();
    expect(screen.getByText('@user2')).toBeInTheDocument();

    // Assert rate limit indicator exists
    expect(screen.getByText('(remaining rate limit: 4900)')).toBeInTheDocument();

    // Assert individual account error exists honestly
    expect(screen.getByText(/Error: Rate limit hit/i)).toBeInTheDocument();

    // Assert issue exists
    expect(screen.getByText('Issue 1 on repo1')).toBeInTheDocument();
    expect(screen.getByText('user1/repo1 · #1')).toBeInTheDocument();
  });
});
