import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';
import '@testing-library/jest-dom';

// React Router + JSDOM AbortSignal / Request conflict in Node 18+: modern Node
// fetch/Request expects native AbortSignal instances, but JSDOM overrides the
// globals. Restore the native Node classes so router data APIs behave.
import util from 'node:util';

const nativeAC = util.transferableAbortController();
const NodeAbortController = nativeAC.constructor;
const NodeAbortSignal = nativeAC.signal.constructor;

globalThis.AbortController = NodeAbortController as typeof AbortController;
globalThis.AbortSignal = NodeAbortSignal as typeof AbortSignal;

afterEach(() => {
  cleanup();
});
