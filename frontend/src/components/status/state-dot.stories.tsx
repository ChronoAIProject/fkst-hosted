import type { Meta, StoryObj } from '@storybook/react-vite';
import { StateDot } from './state-dot';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta<typeof StateDot> = {
  title: 'Status/StateDot',
  component: StateDot,
  tags: ['autodocs'],
};

export default meta;
type Story = StoryObj<typeof StateDot>;

export const Green: Story = {
  args: {
    tone: 'green',
    label: 'Healthy / Active',
  },
};

export const Red: Story = {
  args: {
    tone: 'red',
    label: 'Blocked / Failure',
  },
};

export const Gold: Story = {
  args: {
    tone: 'gold',
    label: 'Pressure / Warning',
  },
};

export const Neutral: Story = {
  args: {
    tone: 'neutral',
    label: 'In Progress / Thinking',
  },
};
