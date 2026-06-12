import type { Meta, StoryObj } from '@storybook/react-vite';
import { VitalsCell } from './vitals-cell';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta<typeof VitalsCell> = {
  title: 'Status/VitalsCell',
  component: VitalsCell,
  tags: ['autodocs'],
};

export default meta;
type Story = StoryObj<typeof VitalsCell>;

export const Merged: Story = {
  args: {
    value: 22,
    label: 'merged · 24h',
    tone: 'green',
  },
};

export const DeadEnded: Story = {
  args: {
    value: 3,
    label: 'dead-ended · need you',
    tone: 'red',
  },
};

export const InFlight: Story = {
  args: {
    value: 13,
    label: 'in flight now',
  },
};

export const Unknown: Story = {
  args: {
    value: 'unknown',
    label: 'deployment window rate',
  },
};
