import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { ViewSwitch } from './view-switch';

const meta: Meta<typeof ViewSwitch> = {
  title: 'Layout/ViewSwitch',
  component: ViewSwitch,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof ViewSwitch>;

export const Interactive: Story = {
  render: () => {
    const [view, setView] = useState<'pipeline' | 'board'>('pipeline');
    return (
      <div className="flex flex-col gap-4 items-start">
        <span className="text-xs text-ghost font-mono">
          Active View: {view}
        </span>
        <ViewSwitch value={view} onChange={setView} />
      </div>
    );
  },
};
