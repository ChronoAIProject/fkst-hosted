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

// jsdom in this environment doesn't expose window.localStorage; provide a
// minimal in-memory shim so the LanguageProvider's persistence is testable.
if (typeof window !== 'undefined' && !window.localStorage) {
  const store = new Map<string, string>();
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => {
        store.set(k, String(v));
      },
      removeItem: (k: string) => {
        store.delete(k);
      },
      clear: () => store.clear(),
      key: (i: number) => Array.from(store.keys())[i] ?? null,
      get length() {
        return store.size;
      },
    },
  });
}

afterEach(() => {
  cleanup();
});
