import type { Meta, StoryObj } from '@storybook/react-vite';
import { StateBadge } from './state-badge';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta<typeof StateBadge> = {
  title: 'Status/StateBadge',
  component: StateBadge,
  tags: ['autodocs'],
};

export default meta;
type Story = StoryObj<typeof StateBadge>;

export const Thinking: Story = {
  args: {
    state: 'thinking',
  },
};

export const Ready: Story = {
  args: {
    state: 'ready',
  },
};

export const ReadyGated: Story = {
  args: {
    state: 'ready',
    gated: true,
  },
};

export const Implementing: Story = {
  args: {
    state: 'implementing',
  },
};

export const PrOpen: Story = {
  args: {
    state: 'pr-open',
  },
};

export const Reviewing: Story = {
  args: {
    state: 'reviewing',
  },
};

export const ReviewingPressure: Story = {
  args: {
    state: 'reviewing',
    pressure: true,
  },
};

export const MergeReady: Story = {
  args: {
    state: 'merge-ready',
  },
};

export const Merging: Story = {
  args: {
    state: 'merging',
  },
};

export const Fixing: Story = {
  args: {
    state: 'fixing',
  },
};

export const ReviewMeta: Story = {
  args: {
    state: 'review-meta',
  },
};

export const ReviewMetaPressure: Story = {
  args: {
    state: 'review-meta',
    pressure: true,
  },
};

export const ImplFailed: Story = {
  args: {
    state: 'impl-failed',
  },
};

export const Blocked: Story = {
  args: {
    state: 'blocked',
  },
};

export const Merged: Story = {
  args: {
    state: 'merged',
  },
};
