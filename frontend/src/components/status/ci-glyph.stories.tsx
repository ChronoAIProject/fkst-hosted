import type { Meta, StoryObj } from '@storybook/react-vite';
import { CiGlyph } from './ci-glyph';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta<typeof CiGlyph> = {
  title: 'Status/CiGlyph',
  component: CiGlyph,
  tags: ['autodocs'],
};

export default meta;
type Story = StoryObj<typeof CiGlyph>;

export const Passing: Story = {
  args: {
    status: 'passing',
  },
};

export const Unknown: Story = {
  args: {
    status: 'unknown',
  },
};

export const Failing: Story = {
  args: {
    status: 'failing',
  },
};
