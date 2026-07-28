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

/** Mount `children` with the providers the chat surface needs. */
export function renderChat(
  children: ReactNode,
  { transport, signedIn = true }: { transport?: ChatTransport; signedIn?: boolean } = {}
) {
  if (signedIn) window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  return render(
    <ToastProvider>
      <AuthProvider>
        <ChatProvider transport={transport}>{children}</ChatProvider>
      </AuthProvider>
    </ToastProvider>
  );
}
