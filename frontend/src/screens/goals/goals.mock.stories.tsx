import type { Meta, StoryObj } from '@storybook/react-vite';
import { Goals } from './goals';
import { mockGoals, mockRuns, mockVitals } from '../../fixtures/goals';

const meta: Meta<typeof Goals> = {
  title: 'Mock / Goals',
  component: Goals,
  decorators: [
    (Story) => (
      <div className="relative pt-8">
        {/* Thin gold-tinted strip banner */}
        <div className="absolute top-0 left-0 right-0 h-8 bg-amber/10 border-b border-amber/20 flex items-center px-4 select-none">
          <div className="flex items-center gap-2 text-[10.5px] font-mono text-amber-ink/90 font-medium">
            <span className="w-1.5 h-1.5 rounded-full bg-amber" />
            <span>Mock Data Mode</span>
          </div>
        </div>
        <div className="bg-bg text-fg p-6 min-h-screen">
          <Story />
        </div>
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Goals>;

/**
 * IssuesPopulated story displaying the transcribed issue list from goals.html mockup.
 *
 * Note: This story uses mock data.
 * The GitHub plane operations are simulated post-NyxID.
 *
 * GAP NOTE: The Goals component does not expose any filter or search props (e.g. search query, stage filter, repository filter, state filter).
 * The search box and filter dropdowns in the toolbar are hardcoded as disabled.
 */
export const IssuesPopulated: Story = {
  args: {
    view: 'issues',
    goals: mockGoals,
  },
  play: async ({ canvasElement }) => {
    const textContent = canvasElement.textContent || '';
    if (!textContent.includes('Tighten consensus parser to handle nested quorum refs')) {
      throw new Error('Smoke test failed: Mockup goal title not found');
    }
    if (!textContent.includes('Document DLQ retention & replay semantics for the delivery ledger')) {
      throw new Error('Smoke test failed: Mockup ready/gated goal not found');
    }
  },
};

/**
 * ActivityPopulated story displaying the transcribed runs and vitals from goals.html mockup.
 *
 * Note: This story uses mock data.
 * The GitHub plane operations are simulated post-NyxID.
 */
export const ActivityPopulated: Story = {
  args: {
    view: 'activity',
    vitals: mockVitals,
    runs: mockRuns,
  },
  play: async ({ canvasElement }) => {
    const textContent = canvasElement.textContent || '';
    if (!textContent.includes('~94%')) {
      throw new Error('Smoke test failed: Mockup success rate vital not found');
    }
    if (!textContent.includes('Make the state label set-exclusive on PR open')) {
      throw new Error('Smoke test failed: Mockup running run title not found');
    }
  },
};
