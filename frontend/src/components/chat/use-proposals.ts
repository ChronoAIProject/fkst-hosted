import { useCallback } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import { useContent } from '@/i18n';
import { useToast } from '@/components/ui/toast';
import type { ApiFetch } from '@/lib/api/canvas';
import { outcomeNote } from './proposal-meta';
import { runProposal } from './proposal-exec';
import type { ProposalExecutionInput } from './proposal-exec';
import type { ChatMessage, ChatProposal } from './chat-context';

/**
 * Confirm-gated action proposals: finding them, running them, recording what
 * happened.
 *
 * Split out of the chat context because it is a genuinely separate concern from
 * streaming a turn — it owns the double-submit guard, the whitelisted execution
 * path, and the outcome notes, and it touches the transcript only to update the
 * proposal it owns.
 */
export function useProposals({
  messages,
  setMessages,
  apiFetch,
  nextId,
}: {
  messages: ChatMessage[];
  setMessages: Dispatch<SetStateAction<ChatMessage[]>>;
  apiFetch: ApiFetch;
  nextId: (prefix: string) => string;
}) {
  const s = useContent().chat;
  const { show: showToast } = useToast();

  /** Update one proposal by id, wherever in the transcript it lives. */
  const patchProposal = useCallback(
    (id: string, update: (entry: ChatProposal) => ChatProposal) => {
      setMessages((current) =>
        current.map((message) =>
          message.proposals?.some((entry) => entry.id === id)
            ? {
                ...message,
                proposals: message.proposals.map((entry) =>
                  entry.id === id ? update(entry) : entry
                ),
              }
            : message
        )
      );
    },
    [setMessages]
  );

  /** Find a proposal by id across the transcript. */
  const findProposal = useCallback(
    (id: string): ChatProposal | undefined =>
      messages.flatMap((message) => message.proposals ?? []).find((entry) => entry.id === id),
    [messages]
  );

  const executeProposal = useCallback(
    async (id: string, input: ProposalExecutionInput = {}) => {
      const entry = findProposal(id);
      if (entry == null) return;
      // Double-submit guard: a `succeeded` proposal never re-runs, and an
      // `executing` one is already in flight.
      if (entry.state === 'executing' || entry.state === 'succeeded') return;

      patchProposal(id, (current) => ({ ...current, state: 'executing', error: undefined }));
      const { proposal } = entry;
      try {
        // `runProposal` maps each kind to ONE whitelisted, typed API function — the
        // exact ones the dashboard's own buttons call. There is deliberately no
        // generic method/path executor, so `target` can never drive a request.
        const result = await runProposal(apiFetch, proposal, input);

        if (!result.ok) {
          patchProposal(id, (current) => ({
            ...current,
            state: 'failed',
            error: result.message ?? s.executeFailed,
          }));
          showToast({ kind: 'error', message: result.message ?? s.executeFailed });
          return;
        }

        patchProposal(id, (current) => ({
          ...current,
          state: 'succeeded',
          error: undefined,
          ...(result.issueUrl ? { issueUrl: result.issueUrl } : {}),
          ...(result.issueNumber ? { issueNumber: result.issueNumber } : {}),
        }));
        // A note in the thread so the outcome is part of the conversation, not just
        // a card the user might scroll past.
        setMessages((current) => [
          ...current,
          {
            id: nextId('n'),
            role: 'system-note',
            content: outcomeNote(proposal, s, result.issueNumber),
            tone: 'info',
          },
        ]);
      } catch {
        patchProposal(id, (current) => ({
          ...current,
          state: 'failed',
          error: s.executeFailed,
        }));
        showToast({ kind: 'error', message: s.executeFailed });
      }
    },
    [apiFetch, findProposal, nextId, patchProposal, s, setMessages, showToast]
  );

  const markProposalSucceeded = useCallback(
    (id: string) => {
      const entry = findProposal(id);
      if (entry == null) return;
      patchProposal(id, (current) => ({ ...current, state: 'succeeded', error: undefined }));
      setMessages((current) => [
        ...current,
        {
          id: nextId('n'),
          role: 'system-note',
          content: outcomeNote(entry.proposal, s),
          tone: 'info',
        },
      ]);
    },
    [findProposal, nextId, patchProposal, s, setMessages]
  );

  const dismissProposal = useCallback(
    (id: string) => {
      setMessages((current) =>
        current.map((message) =>
          message.proposals?.some((entry) => entry.id === id)
            ? { ...message, proposals: message.proposals.filter((entry) => entry.id !== id) }
            : message
        )
      );
    },
    [setMessages]
  );

  return { executeProposal, markProposalSucceeded, dismissProposal };
}
