import type { Meta, StoryObj } from '@storybook/react-vite';
import { FreshnessChip } from './freshness-chip';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta<typeof FreshnessChip> = {
  title: 'Status/FreshnessChip',
  component: FreshnessChip,
  tags: ['autodocs'],
};

export default meta;
type Story = StoryObj<typeof FreshnessChip>;

export const Fresh: Story = {
  args: {
    source: 'github',
    asOf: '30s ago',
    state: 'fresh',
  },
};

export const Syncing: Story = {
  args: {
    source: 'github',
    state: 'syncing',
  },
};

export const StaleWarn: Story = {
  args: {
    source: 'github',
    asOf: '5m ago',
    state: 'stale-warn',
  },
};

export const StaleCritical: Story = {
  args: {
    source: 'github',
    asOf: '15m ago',
    state: 'stale-critical',
  },
};

export const Unknown: Story = {
  args: {
    source: 'github',
    state: 'unknown',
  },
};
