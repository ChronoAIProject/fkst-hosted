import type { GoalView } from '../lib/api/goals';
import type { IssuesEnvelope, CommentView } from '../lib/api/github-issues';

// Mock Hosted GoalView list
export const mockHostedGoals: GoalView[] = [
  {
    id: '214',
    title: 'Tighten consensus parser to handle nested quorum refs',
    description: 'The Lua engine needs to recursively parse nested quorum blocks to prevent cyclic dependency stalls.',
    package_names: ['github-devloop', 'consensus'],
    repo: { owner: 'ChronoAIProject', name: 'fkst-substrate' },
    status: 'running',
    owner_user_id: 'user_nyx_1',
    org_id: null,
    active_session_id: 'session-happy-123',
    created_at: '2026-06-12T10:00:00Z',
    updated_at: '2026-06-12T10:05:00Z',
  },
  {
    id: '231',
    title: 'Document DLQ retention & replay semantics for the delivery ledger',
    description: 'Formalize redb retention periods and DLQ backpressure strategy in the architecture docs.',
    package_names: ['github-devloop'],
    repo: { owner: 'ChronoAIProject', name: 'fkst-packages' },
    status: 'not_started',
    owner_user_id: 'user_nyx_1',
    org_id: null,
    active_session_id: null,
    created_at: '2026-06-12T11:00:00Z',
    updated_at: '2026-06-12T11:00:00Z',
  },
  {
    id: '205',
    title: 'Make the state label set-exclusive on PR open so labels never stack',
    description: 'Ensure fkst-dev labels are cleared before applying new status indicators on target pull requests.',
    package_names: ['github-devloop'],
    repo: null, // Test null repository
    status: 'failed',
    owner_user_id: 'user_nyx_1',
    org_id: null,
    active_session_id: null,
    created_at: '2026-06-12T09:00:00Z',
    updated_at: '2026-06-12T09:30:00Z',
  },
  {
    id: '152',
    title: 'Composed conformance suite for github-autochrono department wiring',
    description: 'Verify end-to-end event flows across consensus, proxy, and raiser modules.',
    package_names: ['github-devloop'],
    repo: { owner: 'ChronoAIProject', name: 'github-autochrono' },
    status: 'triggered',
    owner_user_id: 'user_nyx_1',
    org_id: 'org_chrono_1',
    active_session_id: 'session-happy-456',
    created_at: '2026-06-12T08:00:00Z',
    updated_at: '2026-06-12T08:01:00Z',
  },
  {
    id: '173',
    title: 'Rework dispatcher into a second coordinator for cross-pipeline locks',
    description: 'Prevent deadlocks on concurrent package updates by maintaining global coordination queues.',
    package_names: ['github-devloop'],
    repo: { owner: 'ChronoAIProject', name: 'fkst-substrate' },
    status: 'stopped',
    owner_user_id: 'user_nyx_1',
    org_id: null,
    active_session_id: null,
    created_at: '2026-06-12T07:00:00Z',
    updated_at: '2026-06-12T07:45:00Z',
  }
];

// Single Mock Hosted GoalView
export const mockHostedGoal: GoalView = mockHostedGoals[0] as GoalView;

// Single Mock Hosted GoalView with null repo (no repo connected)
export const mockHostedGoalNoRepo: GoalView = {
  ...(mockHostedGoals[2] as GoalView),
  status: 'not_started',
};


// Mock Comments for Issues detail view
export const mockIssueComments: CommentView[] = [
  {
    id: 1,
    user: 'octocat',
    body: 'I have started looking into the consensus parser issue. It seems we need to handle miter joins more gracefully.',
    html_url: 'https://github.com/ChronoAIProject/fkst-substrate/issues/214#issuecomment-1',
    created_at: '2026-06-12T10:15:00Z',
    updated_at: '2026-06-12T10:15:00Z',
  },
  {
    id: 2,
    user: 'fkst-devloop-bot',
    body: 'Automated consensus round 1 started. Analyzing dependencies...',
    html_url: 'https://github.com/ChronoAIProject/fkst-substrate/issues/214#issuecomment-2',
    created_at: '2026-06-12T10:20:00Z',
    updated_at: '2026-06-12T10:20:00Z',
  }
];

// Mock IssuesEnvelope for populated state (multiple accounts)
export const mockIssuesEnvelope: IssuesEnvelope = {
  results: [
    {
      account: 'octocat',
      page: 1,
      per_page: 10,
      has_more: false,
      rate_limit: {
        remaining: 4950,
        reset_epoch: Math.floor(Date.now() / 1000) + 3600,
      },
      issues: [
        {
          account: 'octocat',
          repository: 'ChronoAIProject/fkst-substrate',
          number: 214,
          id: 1001,
          title: 'Tighten consensus parser to handle nested quorum refs',
          body: 'The Lua engine needs to recursively parse nested quorum blocks to prevent cyclic dependency stalls.',
          state: 'open',
          labels: ['bug', 'consensus'],
          assignees: ['octocat'],
          comments: 2,
          html_url: 'https://github.com/ChronoAIProject/fkst-substrate/issues/214',
          created_at: '2026-06-12T10:00:00Z',
          updated_at: '2026-06-12T10:20:00Z',
        },
        {
          account: 'octocat',
          repository: 'ChronoAIProject/fkst-packages',
          number: 231,
          id: 1002,
          title: 'Document DLQ retention & replay semantics for the delivery ledger',
          body: 'Formalize redb retention periods and DLQ backpressure strategy in the architecture docs.',
          state: 'open',
          labels: ['documentation'],
          assignees: [],
          comments: 0,
          html_url: 'https://github.com/ChronoAIProject/fkst-packages/issues/231',
          created_at: '2026-06-12T11:00:00Z',
          updated_at: '2026-06-12T11:00:00Z',
        }
      ]
    },
    {
      account: 'chronoai-bot',
      page: 1,
      per_page: 10,
      has_more: false,
      rate_limit: {
        remaining: 4800,
        reset_epoch: Math.floor(Date.now() / 1000) + 1800,
      },
      issues: [
        {
          account: 'chronoai-bot',
          repository: 'ChronoAIProject/github-autochrono',
          number: 152,
          id: 2001,
          title: 'Composed conformance suite for github-autochrono department wiring',
          body: 'Verify end-to-end event flows across consensus, proxy, and raiser modules.',
          state: 'closed',
          labels: ['conformance', 'enhancement'],
          assignees: ['chronoai-bot'],
          comments: 0,
          html_url: 'https://github.com/ChronoAIProject/github-autochrono/issues/152',
          created_at: '2026-06-11T08:00:00Z',
          updated_at: '2026-06-12T08:00:00Z',
        }
      ]
    }
  ]
};

// Mock IssuesEnvelope for empty state
export const mockEmptyIssuesEnvelope: IssuesEnvelope = {
  results: [
    {
      account: 'octocat',
      issues: [],
      page: 1,
      per_page: 10,
      has_more: false,
      rate_limit: {
        remaining: 5000,
        reset_epoch: Math.floor(Date.now() / 1000) + 3600,
      }
    }
  ]
};

// Mock IssuesEnvelope with rate-limit and error states
export const mockErrorIssuesEnvelope: IssuesEnvelope = {
  results: [
    {
      account: 'octocat',
      issues: [],
      page: 1,
      per_page: 10,
      has_more: false,
      rate_limit: {
        remaining: 0,
        reset_epoch: Math.floor(Date.now() / 1000) + 600,
      },
      error: {
        kind: 'rate_limited',
        message: 'GitHub API rate limit exceeded. Resets in 10 minutes.',
        retry_after_secs: 600,
      }
    },
    {
      account: 'unauthorized-user',
      issues: [],
      page: 1,
      per_page: 10,
      has_more: false,
      error: {
        kind: 'unauthorized',
        message: 'Invalid GitHub credentials. Please reconnect the account.',
      }
    }
  ]
};
