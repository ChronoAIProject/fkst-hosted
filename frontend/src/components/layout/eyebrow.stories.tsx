import type { Meta, StoryObj } from '@storybook/react-vite';
import { Eyebrow } from './eyebrow';

const meta: Meta<typeof Eyebrow> = {
  title: 'Layout/Eyebrow',
  component: Eyebrow,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Eyebrow>;

export const Default: Story = {
  args: {
    children: 'deployment-configuration',
  },
};
