import type { Meta, StoryObj } from '@storybook/react-vite';
import React from 'react';
import { SessionRegistryProvider, useSessionRegistry } from '../../lib/hooks/session-registry';
import { MemoryRouter } from 'react-router-dom';
import { SettingsScreen } from './settings-screen';
import {
  mockSuccessFetch,
  mockUnreachableFetch,
  mockDegradedFetch,
  createQueryDecorator,
} from '../../fixtures/api';

// Registry initializer for the known session story
function RegistryInit({
  packageName,
  sessionId,
  children,
}: {
  packageName: string;
  sessionId: string;
  children: React.ReactNode;
}) {
  const { registerSession } = useSessionRegistry();
  React.useEffect(() => {
    registerSession(packageName, sessionId);
  }, [packageName, sessionId, registerSession]);
  return <>{children}</>;
}

const meta: Meta<typeof SettingsScreen> = {
  title: 'Settings/States',
  component: SettingsScreen,
  decorators: [
    (Story) => (
      <MemoryRouter>
        <SessionRegistryProvider>
          <Story />
        </SessionRegistryProvider>
      </MemoryRouter>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof SettingsScreen>;

export const EngineHealthyWithKnownSession: Story = {
  decorators: [createQueryDecorator(mockSuccessFetch)],
  render: () => (
    <RegistryInit packageName="github-devloop" sessionId="session-happy-456">
      <SettingsScreen />
    </RegistryInit>
  ),
};

export const Degraded: Story = {
  decorators: [createQueryDecorator(mockDegradedFetch)],
};

export const UnreachableUnknown: Story = {
  decorators: [createQueryDecorator(mockUnreachableFetch)],
};

export const NoKnownSessionGap: Story = {
  decorators: [createQueryDecorator(mockSuccessFetch)],
};
