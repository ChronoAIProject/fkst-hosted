import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { WindowControl } from './window-control';

const meta: Meta<typeof WindowControl> = {
  title: 'Layout/WindowControl',
  component: WindowControl,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof WindowControl>;

export const Interactive: Story = {
  render: () => {
    const [value, setValue] = useState('24h');
    return (
      <div className="flex flex-col gap-4 items-start">
        <span className="text-xs text-ghost font-mono">
          State Value: {value}
        </span>
        <WindowControl value={value} onChange={setValue} />
      </div>
    );
  },
};

export const CustomOptions: Story = {
  render: () => {
    const [value, setValue] = useState('Overview');
    return (
      <div className="flex flex-col gap-4 items-start">
        <span className="text-xs text-ghost font-mono">
          State Value: {value}
        </span>
        <WindowControl
          value={value}
          onChange={setValue}
          options={['Overview', 'Details', 'Settings']}
        />
      </div>
    );
  },
};
