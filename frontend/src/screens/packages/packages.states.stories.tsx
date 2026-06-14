import type { Meta, StoryObj } from '@storybook/react-vite';
import React from 'react';
import { SessionRegistryProvider, useSessionRegistry } from '../../lib/hooks/session-registry';
import { MemoryRouter } from 'react-router-dom';
import PackagesScreen from './packages-screen';
import {
  mockSuccessFetch,
  mockLoadingFetch,
  mockUnreachableFetch,
  mockEmptyFetch,
  mockCreateConflictFetch,
  createQueryDecorator,
} from '../../fixtures/api';
import { userEvent, within, expect } from 'storybook/test';

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

const meta: Meta<typeof PackagesScreen> = {
  title: 'Packages/States',
  component: PackagesScreen,
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
type Story = StoryObj<typeof PackagesScreen>;

export const Loading: Story = {
  decorators: [createQueryDecorator(mockLoadingFetch)],
};

export const GenuineEmpty: Story = {
  decorators: [createQueryDecorator(mockEmptyFetch)],
};

export const UnreachableUnknown: Story = {
  decorators: [createQueryDecorator(mockUnreachableFetch)],
};

export const Populated: Story = {
  decorators: [createQueryDecorator(mockSuccessFetch)],
  render: () => (
    <RegistryInit packageName="github-devloop" sessionId="session-happy-456">
      <PackagesScreen />
    </RegistryInit>
  ),
};

export const Create409: Story = {
  decorators: [createQueryDecorator(mockCreateConflictFetch)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // Scope the button search to avoid ambiguity with "+ Add package root"
    const addBtn = await canvas.findByRole('button', { name: /^\+ Add package$/ });
    await userEvent.click(addBtn);

    const body = within(canvasElement.ownerDocument.body);
    const nameInput = await body.findByLabelText(/Name · unique, create-only/i);
    const filesTextarea = await body.findByLabelText(/Files · the package root, inline/i);
    const submitBtn = await body.findByRole('button', { name: /Create package/i });

    await userEvent.type(nameInput, 'duplicate-pkg');
    await userEvent.type(
      filesTextarea,
      '--- path: departments/my-dept/main.lua\n-- some Lua code\n'
    );
    await userEvent.click(submitBtn);

    // Assert the inline 409 copy to verify the story works
    const errorText = await body.findByText('name already exists (a revision is a new name)');
    expect(errorText).toBeInTheDocument();
  },
};
