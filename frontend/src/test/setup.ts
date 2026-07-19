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

// jsdom's window.localStorage is unreliable across Node versions: absent in
// some environments, present-but-broken (methods missing) in others. Install
// the in-memory shim whenever a WORKING Storage is not there, so the suite
// passes identically everywhere.
if (
  typeof window !== 'undefined' &&
  (!window.localStorage || typeof window.localStorage.clear !== 'function')
) {
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

// ---- Canvas-stack shims -----------------------------------------------------
// @xyflow/react (React Flow 12) and recharts measure themselves with browser
// APIs jsdom does not implement. These are the mocks React Flow's own testing
// guide prescribes: no-op ResizeObserver, a DOMMatrixReadOnly that only knows
// the zoom scale, element sizes, and SVG getBBox. framer-motion additionally
// queries matchMedia for prefers-reduced-motion.

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

class DOMMatrixReadOnlyMock {
  m22: number;
  constructor(transform?: string) {
    const scale = transform?.match(/scale\(([1-9.])\)/)?.[1];
    this.m22 = scale !== undefined ? +scale : 1;
  }
}

globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;
globalThis.DOMMatrixReadOnly = DOMMatrixReadOnlyMock as unknown as typeof DOMMatrixReadOnly;

Object.defineProperties(globalThis.HTMLElement.prototype, {
  offsetHeight: {
    configurable: true,
    get() {
      return parseFloat(this.style.height) || 1;
    },
  },
  offsetWidth: {
    configurable: true,
    get() {
      return parseFloat(this.style.width) || 1;
    },
  },
});

(globalThis.SVGElement.prototype as unknown as { getBBox: () => DOMRect }).getBBox = () =>
  ({ x: 0, y: 0, width: 0, height: 0 }) as DOMRect;

// jsdom does not implement Element.scrollIntoView; the in-page anchor nav
// (get-started) calls it and its tests spy on it. Install a no-op so both the
// call and the spy work — same "browser API jsdom omits" pattern as above.
if (typeof globalThis.HTMLElement !== 'undefined' && !globalThis.HTMLElement.prototype.scrollIntoView) {
  globalThis.HTMLElement.prototype.scrollIntoView = function scrollIntoView() {};
}

// jsdom's Blob does not implement the async reader methods (`text()`,
// `arrayBuffer()`). The outcome-file preview reads committed text via
// `blob.text()`, so polyfill it through jsdom's FileReader — the same
// "browser API jsdom omits" pattern as the shims above.
if (typeof Blob !== 'undefined' && typeof Blob.prototype.text !== 'function') {
  Blob.prototype.text = function (this: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsText(this);
    });
  };
}

if (typeof window !== 'undefined' && typeof window.matchMedia !== 'function') {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

afterEach(() => {
  cleanup();
});
