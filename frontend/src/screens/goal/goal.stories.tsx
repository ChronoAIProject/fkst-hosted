import { MemoryRouter } from 'react-router-dom';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { Goal } from './goal';

const meta: Meta<typeof Goal> = {
  title: 'Screens/GoalPage',
  component: Goal,
  decorators: [
    (Story) => (
      <MemoryRouter>
        <div className="bg-bg text-fg p-6 min-h-screen">
          <Story />
        </div>
      </MemoryRouter>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Goal>;

// 1. Honest Empty State (Storybook Default)
export const Default: Story = {
  args: {
    goalId: '—',
  },
};

// 2. Populated State (for reference/Wave-3 verification)
export const Populated: Story = {
  args: {
    goalId: '152',
    title: 'Composed conformance for github-autochrono',
    state: 'merge-ready',
    version: '2026-06-11T00-12Z',
    headSha: '3c1a9f',
    branch: 'fkst/cand-152-…',
    blocksGoalId: '242',
    lifecycleEvents: [
      { name: 'thinking', timestamp: '00:41Z · 41m ago', body: 'intake: fkst-dev:enabled → re-derived issue → consensus.proposal raised' },
      { name: 'converge · round 1', timestamp: '00:46Z', body: 'converge mode (no reject): 2 approve / 1 abstain · consensus_converge → meta-judge narrowed the question → re-asked at round 2', type: 'converge' },
      { name: 'ready', timestamp: '00:52Z', body: 'consensus reached · devloop_ready', marker: '<!-- fkst:github-devloop:state:v1 proposal="…/152" state="ready" version="…00-52Z" stage_rank="500" -->', trustedBy: 'fkst-devloop-bot', type: 'approve' },
      { name: 'implementing → pr-open', timestamp: '00:58Z', body: 'setup_worktree devloop-152 · codex committed · PR #29 opened · pr-origin:v1 backpointer' },
      { name: 'reviewing · review-loop 2', timestamp: '01:08Z', body: 'PR-diff review consensus over head 3c1a9f · round 1 converged, round 2 reached approve', type: 'converge' },
      { name: 'merge-ready', timestamp: 'just now', body: 'review-result:v1 approve → merge-ready:v1 (head-bound) · devloop_merge_ready delivery raised → merges into integration on next dispatch (~seconds)', marker: '<!-- fkst:github-devloop:merge-ready:v1 proposal="…/152" pr="29" head_sha="3c1a9f" review_proposal="…" -->', trustedBy: 'fkst-devloop-bot', isCurrent: true },
    ],
    deliveries: [
      { status: 'ACK', name: 'review_result', gen: 5, state: 'done' },
      { status: 'LEASED', name: 'merge', gen: 1, state: '0:51 left', timeLeft: '0:51', sourceRef: 'fkst-substrate#pr/29' },
    ],
    runs: [
      { exitCode: 0, action: 'review angle×3', duration: '44s', permits: 3 },
      { exitCode: 0, action: 'meta-judge', duration: '17s' },
    ],
  },
};
