import type { Meta, StoryObj } from '@storybook/react-vite';
import { PostureChip } from './posture-chip';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta<typeof PostureChip> = {
  title: 'Status/PostureChip',
  component: PostureChip,
  tags: ['autodocs'],
};

export default meta;
type Story = StoryObj<typeof PostureChip>;

export const Default: Story = {
  args: {},
};
