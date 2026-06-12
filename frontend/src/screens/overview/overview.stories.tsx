import type { Meta, StoryObj } from '@storybook/react-vite';
import { Overview } from './overview';

const meta: Meta<typeof Overview> = {
  title: 'Screens/Overview',
  component: Overview,
  decorators: [
    (Story) => (
      <div className="bg-bg text-fg p-6 min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Overview>;

// 1. Honest Empty State (Storybook Default)
export const Default: Story = {};

// 2. Viewport 1440px (Desktop Full Width)
export const Viewport1440: Story = {
  name: 'Viewport 1440px (Desktop)',
  decorators: [
    (Story) => (
      <div className="w-[1392px] mx-auto border border-line p-4 bg-bg rounded-panel overflow-hidden">
        <Story />
      </div>
    ),
  ],
};

// 3. Viewport 980px (Rail Scrolls Horizontally)
export const Viewport980: Story = {
  name: 'Viewport 980px (Rail Scrolls)',
  decorators: [
    (Story) => (
      <div className="w-[932px] mx-auto border border-line p-4 bg-bg rounded-panel overflow-hidden">
        <Story />
      </div>
    ),
  ],
};

// 4. Viewport 780px (Toolbar Wraps, Vitals Reflows)
export const Viewport780: Story = {
  name: 'Viewport 780px (Toolbar Wrap)',
  decorators: [
    (Story) => (
      <div className="w-[732px] mx-auto border border-line p-4 bg-bg rounded-panel overflow-hidden">
        <Story />
      </div>
    ),
  ],
};

// 5. Viewport 480px (Mobile, Rail Stacks Vertically)
export const Viewport480: Story = {
  name: 'Viewport 480px (Mobile Stacks)',
  decorators: [
    (Story) => (
      <div className="w-[448px] mx-auto border border-line p-4 bg-bg rounded-panel overflow-hidden">
        <Story />
      </div>
    ),
  ],
};

// 6. Populated Fixture State (for reference/Wave-3 verification)
export const Populated: Story = {
  args: {
    goals: [
      { id: '214', title: 'Tighten consensus parser to handle nested quorum refs', stage: 'Design', state: 'thinking', age: '4m' },
      { id: '231', title: 'Document DLQ retention & replay semantics for the delivery ledger', stage: 'Design', state: 'ready', age: '11m', gated: true },
      { id: '240', title: 'Validate cron source cadence on cold start before first faucet tick', stage: 'Design', state: 'thinking', age: '18m' },
      { id: '205', title: 'Make the state label set-exclusive on PR open so labels never stack', stage: 'Build', state: 'implementing', age: '1m' },
      { id: '238', title: 'Reflect dead-letters back into the goal board as a surfaced state', stage: 'Build', state: 'implementing', age: '7m' },
      { id: '242', title: 'Extract fanout spawn into its own module to drop the cycle', stage: 'Build', state: 'pr-open', age: '12m' },
      { id: '187', title: 'Propagate source_ref through the fanout spawn path end to end', stage: 'Review', state: 'reviewing', age: '14m', pressure: true },
      { id: '161', title: 'ack on RAISED publish failure before the retry reschedules', stage: 'Review', state: 'reviewing', age: '31m', pr: '#38' },
      { id: '158', title: 'Reconcile lease_generation on requeue to stop double leases', stage: 'Review', state: 'fixing', age: '44m', pr: '#36' },
      { id: '152', title: 'Composed conformance suite for github-autochrono department wiring', stage: 'Ship', state: 'merging', age: '3m', pr: '#29', ci: 'passing', pressure: true },
      { id: '149', title: 'Extract cache_get / cache_set helper functions from the scratch KV', stage: 'Ship', state: 'merge-ready', age: '6m', pr: '#27', ci: 'passing' },
      { id: '118', title: 'Durable redb cutover for the delivery ledger with lease fencing', stage: 'Merged', state: 'merged', age: '2h' },
      { id: '117', title: 'Lease fencing on ack / retry / dead via lease_generation match', stage: 'Merged', state: 'merged', age: '3h' },
    ],
    vitals: {
      inFlight: 13,
      merged24h: 22,
      deadEnded: 3,
      throughput: '~5/h',
      medianReviewTime: '19m',
      windowStart: '12:00 Jun 10',
      windowEnd: '12:00 Jun 11',
    },
    needsYou: [
      {
        lead: 'Merging',
        leadTone: 'red',
        title: 'composed conformance for github-autochrono',
        id: '152',
        pr: '#29',
        why: 'WRITE: REAL · merges autonomously into the integration branch (seconds) · CI green · to stop autonomous merges, flip the global write posture to DRY-RUN or close the PR on GitHub',
        actionLabel: 'Write posture →',
        actionTone: 'red',
      },
      {
        lead: 'Blocked',
        leadTone: 'red',
        title: 'Rework dispatcher into 2nd coordinator',
        id: '173',
        why: 'true-stall reconcile · engine gave up this framing after 3 rounds · terminal — re-engage with a fresh GitHub issue, not a re-open',
        actionLabel: 'New issue from this',
      },
    ],
  },
};
