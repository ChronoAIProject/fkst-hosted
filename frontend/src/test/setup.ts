import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';
import '@testing-library/jest-dom';

// Fix for React Router + JSDOM AbortSignal / Request conflict in Node 18+
// Modern Node fetch/Request expects native AbortSignal instances.
// JSDOM overrides globalThis.AbortSignal / AbortController, causing mismatches.
// We can retrieve the native Node classes from util.transferableAbortController() and restore them.
import util from 'node:util';

const nativeAC = util.transferableAbortController();
const NodeAbortController = nativeAC.constructor;
const NodeAbortSignal = nativeAC.signal.constructor;

globalThis.AbortController = NodeAbortController as typeof AbortController;
globalThis.AbortSignal = NodeAbortSignal as typeof AbortSignal;

afterEach(() => {
  cleanup();
});

import { vi } from 'vitest';

vi.mock('@/lib/auth', () => ({
  authRequired: vi.fn(() => false),
  useAuthSession: vi.fn(() => ({
    isAuthenticated: false,
    accessToken: null,
    login: vi.fn(),
    logout: vi.fn(),
    handleRedirectCallback: vi.fn(),
    getUserInfo: vi.fn(),
  })),
}));

vi.mock('../lib/auth', () => ({
  authRequired: vi.fn(() => false),
  useAuthSession: vi.fn(() => ({
    isAuthenticated: false,
    accessToken: null,
    login: vi.fn(),
    logout: vi.fn(),
    handleRedirectCallback: vi.fn(),
    getUserInfo: vi.fn(),
  })),
}));
