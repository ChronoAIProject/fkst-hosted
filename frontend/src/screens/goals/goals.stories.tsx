import type { Meta, StoryObj } from '@storybook/react-vite';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { NyxIDProvider } from '../../lib/auth';
import { Goals } from './goals';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,
      refetchOnWindowFocus: false,
    },
  },
});

const meta: Meta<typeof Goals> = {
  title: 'Screens/Goals',
  component: Goals,
  decorators: [
    (Story) => (
      <QueryClientProvider client={queryClient}>
        <NyxIDProvider baseUrl="" clientId="" redirectUri="">
          <div className="bg-bg text-fg p-6 min-h-screen">
            <Story />
          </div>
        </NyxIDProvider>
      </QueryClientProvider>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Goals>;

// 1. Honest Empty Issues View (Storybook Default)
export const DefaultIssues: Story = {
  args: {
    view: 'issues',
  },
};

// 2. Honest Empty Activity View
export const DefaultActivity: Story = {
  args: {
    view: 'activity',
  },
};

// 3. Populated Issues State (for reference/Wave-3 verification)
export const PopulatedIssues: Story = {
  args: {
    view: 'issues',
    authSessionOverride: { isAuthenticated: true },
    accountsOverride: [{ connection_id: 'c1', login: 'octocat', primary: true }],
    goals: [
      { id: '214', title: 'Tighten consensus parser to handle nested quorum refs', stage: 'Design', state: 'thinking', age: '4m', repo: 'fkst-substrate', pr: '', ci: 'unknown' },
      { id: '231', title: 'Document DLQ retention & replay semantics for the delivery ledger', stage: 'Design', state: 'ready', age: '11m', repo: 'fkst-packages', pr: '', ci: 'unknown', gated: true },
      { id: '205', title: 'Make the state label set-exclusive on PR open so labels never stack', stage: 'Build', state: 'implementing', age: '1m', repo: 'fkst-substrate', pr: '', ci: 'unknown' },
      { id: '187', title: 'Propagate source_ref through the fanout spawn path end to end', stage: 'Review', state: 'reviewing', age: '14m', repo: 'fkst-substrate', pr: '#41', ci: 'passing' },
      { id: '152', title: 'Composed conformance suite for github-autochrono department wiring', stage: 'Ship', state: 'merging', age: '3m', repo: 'fkst-substrate', pr: '#29', ci: 'passing' },
      { id: '118', title: 'Durable redb cutover for the delivery ledger with lease fencing', stage: 'Merged', state: 'merged', age: '2h', repo: 'fkst-substrate', pr: '#21', ci: 'passing' },
    ],
  },
};

// 4. Populated Activity State (for reference/Wave-3 verification)
export const PopulatedActivity: Story = {
  args: {
    view: 'activity',
    vitals: {
      runsDispatched: '~9/h',
      successRate: '~94%',
      medianDuration: '~48s',
      inDlq: 'unknown',
    },
    runs: [
      {
        id: 'run_8f3a91',
        goalId: '205',
        goalTitle: 'Make the state label set-exclusive on PR open',
        action: 'implement',
        attempt: '1/5',
        duration: 'running · 38s',
        exitCode: null,
        when: 'leased 38s ago',
        lease: '7',
        status: 'running',
      },
      {
        id: 'run_7c1d44',
        goalId: '214',
        goalTitle: 'Tighten consensus parser to handle nested quorum refs',
        action: 'consensus',
        attempt: '1/3',
        duration: '44s',
        exitCode: 0,
        when: '2m ago',
        lease: '3',
        status: 'ok',
      },
    ],
  },
};
