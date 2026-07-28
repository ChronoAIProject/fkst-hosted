import { render } from '@testing-library/react';
import type { ReactNode } from 'react';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider } from '@/components/ui/toast';
import { ChatProvider } from './chat-context';
import type { ChatTransport, ChatTransportHandlers, ChatTurnMessage } from './transport';

/** A transport a test drives step by step.
 *
 *  The mock echo transport is right for reviewing the surface; a scripted one is
 *  right for asserting behaviour, because a test that waits on timers is a test
 *  that flakes. */
export function scriptedTransport() {
  const sent: ChatTurnMessage[][] = [];
  let handlers: ChatTransportHandlers | null = null;
  let aborted = false;

  const transport: ChatTransport = {
    send(history, next, signal) {
      sent.push(history);
      handlers = next;
      signal.addEventListener('abort', () => {
        aborted = true;
      });
    },
  };

  return {
    transport,
    /** Every history the provider has sent, in order. */
    sent,
    /** The live handler set. Throws if nothing has been sent, so a mis-ordered
     *  test fails loudly instead of silently doing nothing. */
    handlers: () => {
      if (handlers == null) throw new Error('the transport has not been called yet');
      return handlers;
    },
    aborted: () => aborted,
  };
}

/** Make `prefers-reduced-motion` answer `matches`, for the whole document.
 *
 *  jsdom ships no `matchMedia`, so without this the provider cannot ask — and a test
 *  asserting WHAT the transcript holds would otherwise have to drive the typewriter's
 *  interval, which is exactly the timer dependence this kit avoids. */
export function stubReducedMotion(matches: boolean) {
  window.matchMedia = ((query: string) => ({
    matches,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

/** Mount `children` with the providers the chat surface needs.
 *
 *  `reducedMotion` defaults to TRUE so the typewriter reveals each delta synchronously.
 *  That keeps every behavioural test ("what does the transcript contain") free of timers;
 *  the reveal ANIMATION is covered by `typewriter.test.ts` and by the one provider test
 *  that opts out with `reducedMotion: false`. */
export function renderChat(
  children: ReactNode,
  {
    transport,
    signedIn = true,
    reducedMotion = true,
  }: { transport?: ChatTransport; signedIn?: boolean; reducedMotion?: boolean } = {}
) {
  if (signedIn) window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  stubReducedMotion(reducedMotion);
  return render(
    <ToastProvider>
      <AuthProvider>
        <ChatProvider transport={transport}>{children}</ChatProvider>
      </AuthProvider>
    </ToastProvider>
  );
}
