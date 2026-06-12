import type { Meta, StoryObj } from '@storybook/react-vite';
import { SectionHeading } from './section-heading';

const meta: Meta<typeof SectionHeading> = {
  title: 'Layout/SectionHeading',
  component: SectionHeading,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof SectionHeading>;

export const Default: Story = {
  args: {
    children: 'Active Packages',
  },
};

export const WithCount: Story = {
  args: {
    children: 'Queued Goals',
    count: 24,
  },
};
