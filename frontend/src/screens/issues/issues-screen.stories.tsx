import type { Meta, StoryObj } from '@storybook/react-vite';
import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { userEvent, within, expect } from 'storybook/test';
import { MemoryRouter } from 'react-router-dom';
import IssuesScreen from './issues-screen';
import {
  mockIssuesEnvelope,
  mockEmptyIssuesEnvelope,
  mockErrorIssuesEnvelope,
  mockIssueComments,
} from '../../fixtures/hosted';

const meta: Meta<typeof IssuesScreen> = {
  title: 'Screens/IssuesScreen',
  component: IssuesScreen,
  decorators: [
    (Story) => (
      <div className="bg-bg text-fg p-6 min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof IssuesScreen>;

// --- Helper to create QueryClient with mocked fetch ---
const createQueryDecorator = (fetchMock: (url: string, init?: RequestInit) => Promise<Response>) => {
  return (Story: React.ComponentType) => {
    React.useEffect(() => {
      const originalFetch = globalThis.fetch;
      globalThis.fetch = fetchMock as typeof globalThis.fetch;
      return () => {
        globalThis.fetch = originalFetch;
      };
    }, []);

    const queryClient = new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
          staleTime: 0,
          gcTime: 0,
        },
      },
    });

    return (
      <MemoryRouter>
        <QueryClientProvider client={queryClient}>
          <Story />
        </QueryClientProvider>
      </MemoryRouter>
    );
  };
};

// 1. No Accounts connected state (degrades to ConnectGitHub empty state)
export const NoAccounts: Story = {
  decorators: [
    createQueryDecorator(async (url) => {
      if (url.includes('/api/v1/github/accounts')) {
        return new Response(JSON.stringify([]), { status: 200 });
      }
      return Promise.reject(new Error('Unknown URL: ' + url));
    }),
  ],
};

// 2. Loading state
export const Loading: Story = {
  decorators: [
    createQueryDecorator(() => {
      return new Promise<Response>(() => {}); // Never resolves to keep loading state
    }),
  ],
};

// 3. Populated Aggregate state (multiple connected accounts)
export const PopulatedAggregate: Story = {
  decorators: [
    createQueryDecorator(async (url) => {
      if (url.includes('/api/v1/github/accounts')) {
        return new Response(
          JSON.stringify([
            { connection_id: 'c1', login: 'octocat', primary: true },
            { connection_id: 'c2', login: 'chronoai-bot', primary: false },
          ]),
          { status: 200 }
        );
      }
      if (url.includes('/api/v1/github/issues')) {
        return new Response(JSON.stringify(mockIssuesEnvelope), { status: 200 });
      }
      return Promise.reject(new Error('Unknown URL: ' + url));
    }),
  ],
};

// 4. Per-account error/rate-limit state
export const PerAccountErrorRateLimit: Story = {
  decorators: [
    createQueryDecorator(async (url) => {
      if (url.includes('/api/v1/github/accounts')) {
        return new Response(
          JSON.stringify([
            { connection_id: 'c1', login: 'octocat', primary: true },
            { connection_id: 'c3', login: 'unauthorized-user', primary: false },
          ]),
          { status: 200 }
        );
      }
      if (url.includes('/api/v1/github/issues')) {
        return new Response(JSON.stringify(mockErrorIssuesEnvelope), { status: 200 });
      }
      return Promise.reject(new Error('Unknown URL: ' + url));
    }),
  ],
};

// 5. Empty issues state
export const EmptyIssues: Story = {
  decorators: [
    createQueryDecorator(async (url) => {
      if (url.includes('/api/v1/github/accounts')) {
        return new Response(
          JSON.stringify([{ connection_id: 'c1', login: 'octocat', primary: true }]),
          { status: 200 }
        );
      }
      if (url.includes('/api/v1/github/issues')) {
        return new Response(JSON.stringify(mockEmptyIssuesEnvelope), { status: 200 });
      }
      return Promise.reject(new Error('Unknown URL: ' + url));
    }),
  ],
};

// 6. Issue detail drawer state (with comments)
export const IssueDetailDrawer: Story = {
  decorators: [
    createQueryDecorator(async (url) => {
      if (url.includes('/api/v1/github/accounts')) {
        return new Response(
          JSON.stringify([{ connection_id: 'c1', login: 'octocat', primary: true }]),
          { status: 200 }
        );
      }
      if (url.includes('/api/v1/github/issues')) {
        return new Response(JSON.stringify(mockIssuesEnvelope), { status: 200 });
      }
      if (url.includes('/issues/214/comments')) {
        return new Response(JSON.stringify(mockIssueComments), { status: 200 });
      }
      if (url.includes('/issues/214')) {
        const issue = mockIssuesEnvelope.results[0]!.issues[0]!;
        return new Response(JSON.stringify(issue), { status: 200 });
      }

      return Promise.reject(new Error('Unknown URL: ' + url));
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    
    // Wait for the issue card containing #214 to appear in the list
    const hashEl = await canvas.findByText(/#214/);
    const targetCard = hashEl.closest('[role="button"]') as HTMLElement | null;
    if (!targetCard) {
      throw new Error('Could not find issue card container with role="button" for issue #214');
    }

    // Click the card using userEvent
    await userEvent.click(targetCard);

    // Assert that the dialog/drawer is open by querying the portaled body
    const body = within(canvasElement.ownerDocument.body);
    const dialogTitle = await body.findByText('Tighten consensus parser to handle nested quorum refs');
    expect(dialogTitle).toBeInTheDocument();

    const commentBody = await body.findByText(/I have started looking into the consensus parser issue/);
    expect(commentBody).toBeInTheDocument();
  },
};
