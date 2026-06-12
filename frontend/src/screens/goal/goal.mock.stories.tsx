import { MemoryRouter } from 'react-router-dom';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { Goal } from './goal';
import { mockLifecyclePopulated, mockTerminalBlocked } from '../../fixtures/goal';

const meta: Meta<typeof Goal> = {
  title: 'Mock / Goal',
  component: Goal,
  decorators: [
    (Story) => (
      <MemoryRouter>
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
      </MemoryRouter>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Goal>;

/**
 * LifecyclePopulated story displaying a fully populated goal lifecycle and diagnostics.
 *
 * Note: This story uses mock data.
 * The GitHub plane operations are simulated post-NyxID.
 *
 * Check Status: The posture check in the merge gate displays 'posture unknown (deploy-time)' and renders as '—' honestly.
 */
export const LifecyclePopulated: Story = {
  args: mockLifecyclePopulated,
  play: async ({ canvasElement }) => {
    const textContent = canvasElement.textContent || '';
    if (!textContent.includes('Composed conformance for github-autochrono')) {
      throw new Error('Smoke test failed: Mockup goal title not found');
    }
    if (!textContent.includes('#152')) {
      throw new Error('Smoke test failed: Mockup goal ID not found');
    }
    if (!textContent.includes('merge-ready')) {
      throw new Error('Smoke test failed: Mockup state not found');
    }
  },
};

/**
 * LifecycleRealPosture story representing a goal with autonomous REAL posture.
 *
 * Note: This story uses mock data.
 * Displays the Decide box and sets the write posture check to OK (REAL).
 */
export const LifecycleRealPosture: Story = {
  args: {
    ...mockLifecyclePopulated,
    isReal: true,
    mergeGate: {
      ...mockLifecyclePopulated.mergeGate,
      posture: 'ok',
    },
  },
  play: async ({ canvasElement }) => {
    const textContent = canvasElement.textContent || '';
    if (!textContent.includes('Real · autonomous')) {
      throw new Error('Smoke test failed: Real autonomous banner not found');
    }
    if (!textContent.includes('Merges PR into the integration branch')) {
      throw new Error('Smoke test failed: Autonomous merge text not found');
    }
  },
};

/**
 * TerminalBlocked story representing a terminal blocked goal state.
 *
 * Note: This story uses mock data.
 * The GitHub plane operations are simulated post-NyxID.
 *
 * GAP NOTE: The prop surface does not support a "New issue from this" action or context button.
 * The decision header reflects terminality honestly via the 'blocked' state badge, but lacks contextual action triggers.
 * Additionally, there is a layout gap: the "PR-diff review" panel showing details of individual review angles is missing from the Goal page layout entirely.
 */
export const TerminalBlocked: Story = {
  args: mockTerminalBlocked,
  play: async ({ canvasElement }) => {
    const textContent = canvasElement.textContent || '';
    if (!textContent.includes('Rework dispatcher into a second coordinator for cross-pipeline locks')) {
      throw new Error('Smoke test failed: Mockup blocked goal title not found');
    }
    if (!textContent.includes('#173')) {
      throw new Error('Smoke test failed: Mockup blocked goal ID not found');
    }
    if (!textContent.includes('blocked')) {
      throw new Error('Smoke test failed: Mockup blocked state not found');
    }
  },
};
