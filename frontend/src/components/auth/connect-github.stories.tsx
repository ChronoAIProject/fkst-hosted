import type { Meta, StoryObj } from '@storybook/react-vite';
import React from 'react';
import { ConnectGitHub } from './connect-github';

const meta: Meta<typeof ConnectGitHub> = {
  title: 'Components/ConnectGitHub',
  component: ConnectGitHub,
  decorators: [
    (Story) => (
      <div className="bg-bg text-fg p-6 min-h-screen flex items-center justify-center">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof ConnectGitHub>;

// 1. Enabled CTA state (env URL present)
export const Enabled: Story = {
  decorators: [
    (Story) => {
      // Temporarily override the environment variable dynamically in the browser
      const originalValue = import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL;
      Object.defineProperty(import.meta.env, 'VITE_NYXID_CONNECT_GITHUB_URL', {
        value: 'https://nyx.chrono-ai.fun/api/v1/github/connect',
        writable: true,
        configurable: true,
      });

      React.useEffect(() => {
        return () => {
          Object.defineProperty(import.meta.env, 'VITE_NYXID_CONNECT_GITHUB_URL', {
            value: originalValue,
            writable: true,
            configurable: true,
          });
        };
      }, [originalValue]);

      return <Story />;
    },
  ],
};

// 2. Disabled + honest-note state (env unset)
export const Disabled: Story = {
  decorators: [
    (Story) => {
      // Temporarily override the environment variable dynamically in the browser to empty/unset
      const originalValue = import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL;
      Object.defineProperty(import.meta.env, 'VITE_NYXID_CONNECT_GITHUB_URL', {
        value: '',
        writable: true,
        configurable: true,
      });

      React.useEffect(() => {
        return () => {
          Object.defineProperty(import.meta.env, 'VITE_NYXID_CONNECT_GITHUB_URL', {
            value: originalValue,
            writable: true,
            configurable: true,
          });
        };
      }, [originalValue]);

      return <Story />;
    },
  ],
};
