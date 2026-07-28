import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { useToast } from '@/components/ui/toast';
import { createTrigger, createWorkItem, stopTrigger } from '@/lib/api/canvas';
import { mapDraftToRequest, parseActionProposal } from './action-types';
import type { ActionProposal } from './action-types';
import { mockEchoTransport } from './transport';
import type { ChatTransport, ChatTurnMessage, SessionRef } from './transport';

/** One tool the assistant used during a turn, as the UI shows it. */
export interface ChatToolEvent {
  id: string;
  name: string;
  /** `undefined` while the call is in flight. */
  status?: number;
  truncated?: boolean;
}

/** How far a proposal's execution has got.
 *
 *  `failed` covers both a rejected mutation and the unknowable case: a transcript
 *  restored while a request was in flight, where the outcome cannot be recovered. */
export type ProposalExecState = 'idle' | 'executing' | 'succeeded' | 'failed';

/** A confirm-gated proposal as the UI tracks it. */
export interface ChatProposal {
  /** Client-assigned: the wire union carries no id. */
  id: string;
  proposal: ActionProposal;
  state: ProposalExecState;
  /** Server message on failure, or the restored-mid-flight note. */
  error?: string;
  /** The created issue's URL on success. */
  issueUrl?: string;
  /** The created issue's number on success — the dashboard deep link needs it. */
  issueNumber?: number;
}

/** One entry in the visible transcript.
 *
 *  `system-note` is a LOCAL message (an error, an outcome) that never goes on the
 *  wire — keeping it in the same list means the user reads one chronological
 *  thread instead of a transcript plus a separate notification area. */
export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system-note';
  content: string;
  /** True while the assistant message is still growing. */
  pending?: boolean;
  /** Tool activity attributed to this assistant message. */
  toolEvents?: ChatToolEvent[];
  /** Sessions the turn identified, for deep-linking cards. */
  sessionRefs?: SessionRef[];
  /** Confirm-gated action proposals drafted during this turn. */
  proposals?: ChatProposal[];
  /** A `system-note` that should read as a warning rather than information. */
  tone?: 'info' | 'warn';
}

interface ChatContextValue {
  open: boolean;
  messages: ChatMessage[];
  streaming: boolean;
  openPanel: () => void;
  closePanel: () => void;
  toggle: () => void;
  sendMessage: (text: string) => void;
  stopStreaming: () => void;
  clearTranscript: () => void;
  /** Execute a reviewed proposal under the user's own token. Nothing ever runs
   *  without a deliberate call to this. */
  executeProposal: (id: string) => Promise<void>;
  /** Drop a proposal the user does not want. */
  dismissProposal: (id: string) => void;
  /** Record that a proposal's mutation ALREADY ran elsewhere and succeeded.
   *
   *  Exists for exactly one caller: the stop path, where `ConfirmDialog` owns the
   *  mutation by contract. Calling `executeProposal` after the dialog succeeded
   *  would close the trigger a second time. */
  markProposalSucceeded: (id: string) => void;
}

const ChatContext = createContext<ChatContextValue | null>(null);

/** Per-tab transcript storage. `sessionStorage`, not `localStorage`: a chat
 *  transcript is a working conversation, not a saved document, and per-tab scope
 *  means two tabs do not fight over one thread. */
const STORAGE_KEY = 'fkst-chat-transcript';

/** Transcript cap. Bounded because a long conversation would otherwise grow the
 *  stored payload without limit; the oldest messages are the least useful. */
const MAX_STORED_MESSAGES = 100;

let messageSeq = 0;
/** Monotonic ids. A counter, not a timestamp: two messages appended in the same
 *  millisecond must not collide as React keys. */
function nextId(prefix: string): string {
  messageSeq += 1;
  return `${prefix}-${messageSeq}`;
}

/** Resolve user-facing error copy from a stable code.
 *
 *  `rate_limited` gets a variant naming the retry delay when the server sent one,
 *  because "try again in 5s" is actionable where "try again" is not. */
function errorCopy(
  s: ReturnType<typeof useContent>['chat'],
  code: string,
  fallback: string,
  retryAfterSeconds?: number
): string {
  if (code === 'rate_limited' && retryAfterSeconds != null) {
    return s.errors.rate_limited_after!.replace('{seconds}', String(retryAfterSeconds));
  }
  return s.errors[code] ?? fallback ?? s.errors.unknown!;
}

/** The thread note recording what a confirmed proposal actually created. */
function outcomeNote(
  s: ReturnType<typeof useContent>['chat'],
  proposal: ActionProposal,
  issueNumber?: number
): string {
  const repo = `${proposal.owner}/${proposal.name}`;
  const template =
    proposal.kind === 'stop_session'
      ? s.outcomeStopped
      : proposal.kind === 'create_session'
        ? s.outcomeSession
        : s.outcomeWorkItem;
  const number =
    issueNumber ?? ('trigger_issue_number' in proposal ? proposal.trigger_issue_number : 0);
  return template.replace('{number}', String(number)).replace('{repo}', repo);
}

/** Read the stored transcript, tolerating anything. A corrupt or foreign value
 *  must degrade to an empty transcript, never break the panel. */
function readStored(): ChatMessage[] {
  try {
    const raw = window.sessionStorage?.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (entry): entry is ChatMessage =>
          typeof entry === 'object' &&
          entry != null &&
          typeof (entry as ChatMessage).id === 'string' &&
          typeof (entry as ChatMessage).content === 'string'
      )
      .map(restoreProposals);
  } catch {
    return [];
  }
}

/** Rehydrate a restored message's proposals.
 *
 *  A proposal stored as `executing` is unknowable after a reload: the request may
 *  have succeeded, failed, or never left. It becomes `failed` with a note telling
 *  the user to check the dashboard — because the one thing that must NEVER happen
 *  is re-executing it silently on restore.
 *
 *  `RESTORED_UNKNOWN` is a sentinel, not copy: the component localizes it. */
export const RESTORED_UNKNOWN = 'restored-unknown';

function restoreProposals(message: ChatMessage): ChatMessage {
  if (message.proposals == null || message.proposals.length === 0) return message;
  return {
    ...message,
    proposals: message.proposals.map((entry) =>
      entry.state === 'executing'
        ? { ...entry, state: 'failed' as const, error: RESTORED_UNKNOWN }
        : entry
    ),
  };
}

function writeStored(messages: ChatMessage[]) {
  try {
    // An EMPTY transcript removes the key rather than storing `[]`. That makes this
    // effect the single writer: a clear (or a sign-out) does not need to also call
    // removeItem, which the very next persist would undo anyway.
    if (messages.length === 0) {
      window.sessionStorage?.removeItem(STORAGE_KEY);
      return;
    }
    // A restored `pending` message would show a caret that never resolves, so the
    // flag is dropped on the way out.
    const storable = messages.slice(-MAX_STORED_MESSAGES).map((message) => {
      const stored: ChatMessage = { ...message };
      delete stored.pending;
      return stored;
    });
    window.sessionStorage?.setItem(STORAGE_KEY, JSON.stringify(storable));
  } catch {
    // Storage can be full or blocked; the transcript still works in memory.
  }
}

/**
 * Owns the concierge's client state: panel visibility, the transcript, and the
 * one in-flight turn.
 *
 * The transport is injected rather than imported so the shell can supply the real
 * SSE client while tests and this milestone's review supply the mock — the
 * provider itself never knows which it has.
 */
export function ChatProvider({
  children,
  transport = mockEchoTransport,
}: {
  children: ReactNode;
  transport?: ChatTransport;
}) {
  const { isAuthenticated, apiFetch } = useAuth();
  const s = useContent().chat;
  const { show: showToast } = useToast();
  const [open, setOpen] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>(() => readStored());
  const [streaming, setStreaming] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  // Persist on every change so a refresh (or an accidental tab switch) keeps the
  // conversation the user is reading.
  useEffect(() => {
    writeStored(messages);
  }, [messages]);

  // Sign-out (or an expiry) must not leave one user's conversation on screen for
  // the next person on a shared machine. Only a true→false transition clears, so
  // a visitor who was never signed in keeps whatever they were reading.
  const wasAuthenticated = useRef(isAuthenticated);
  useEffect(() => {
    if (wasAuthenticated.current && !isAuthenticated) {
      abortRef.current?.abort();
      setMessages([]);
      setStreaming(false);
    }
    wasAuthenticated.current = isAuthenticated;
  }, [isAuthenticated]);

  // A turn outlives the panel being closed only if we let it; aborting on unmount
  // stops a dead component's callbacks from firing.
  useEffect(() => () => abortRef.current?.abort(), []);

  const stopStreaming = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setStreaming(false);
    setMessages((current) =>
      current.map((message) => (message.pending ? { ...message, pending: false } : message))
    );
  }, []);

  /** Apply an update to the assistant message with `id`. */
  const patch = useCallback((id: string, update: (message: ChatMessage) => ChatMessage) => {
    setMessages((current) =>
      current.map((message) => (message.id === id ? update(message) : message))
    );
  }, []);

  const sendMessage = useCallback(
    (text: string) => {
      const content = text.trim();
      if (!content || streaming) return;

      const userMessage: ChatMessage = { id: nextId('u'), role: 'user', content };
      const assistantId = nextId('a');
      const assistantMessage: ChatMessage = {
        id: assistantId,
        role: 'assistant',
        content: '',
        pending: true,
        toolEvents: [],
      };

      // The wire history is built from what is on screen PLUS this message, and
      // carries only user/assistant content — never a local notice, and never the
      // empty placeholder we are about to add.
      const history: ChatTurnMessage[] = [...messages, userMessage]
        .filter(
          (message): message is ChatMessage & { role: 'user' | 'assistant' } =>
            (message.role === 'user' || message.role === 'assistant') &&
            message.content.trim().length > 0
        )
        .map((message) => ({ role: message.role, content: message.content }));

      setMessages((current) => [...current, userMessage, assistantMessage]);
      setStreaming(true);

      const controller = new AbortController();
      abortRef.current = controller;

      transport.send(
        history,
        {
          onDelta: (delta) =>
            patch(assistantId, (message) => ({ ...message, content: message.content + delta })),
          onToolCall: ({ id, name }) =>
            patch(assistantId, (message) => ({
              ...message,
              toolEvents: [...(message.toolEvents ?? []), { id, name }],
            })),
          onToolResult: ({ id, name, status, truncated }) =>
            patch(assistantId, (message) => {
              const events = message.toolEvents ?? [];
              const known = events.some((event) => event.id === id);
              return {
                ...message,
                toolEvents: known
                  ? events.map((event) =>
                      event.id === id ? { ...event, status, truncated } : event
                    )
                  : // A result with no matching call still gets shown rather than
                    // dropped — silence would hide real activity.
                    [...events, { id, name, status, truncated }],
              };
            }),
          onActionProposal: (raw) => {
            const proposal = parseActionProposal(raw);
            if (proposal == null) {
              // An unreadable or unrecognized draft becomes a note, NOT an error
              // toast and never a card: the turn is fine, only this draft is not,
              // and a card the SPA cannot execute is worse than saying so.
              setMessages((current) => [
                ...current,
                {
                  id: nextId('n'),
                  role: 'system-note',
                  content: s.unreadableProposal,
                  tone: 'info',
                },
              ]);
              return;
            }
            patch(assistantId, (message) => ({
              ...message,
              proposals: [
                ...(message.proposals ?? []),
                { id: nextId('p'), proposal, state: 'idle' as const },
              ],
            }));
          },
          onDone: ({ sessionRefs }) => {
            patch(assistantId, (message) => ({ ...message, pending: false, sessionRefs }));
            setStreaming(false);
            abortRef.current = null;
          },
          onError: ({ code, message, retryAfterSeconds }) => {
            // Copy comes from the stable CODE, not the server's prose: the prose is
            // for the log, and a user-facing string must be translatable. The raw
            // message is kept as a fallback for a code we do not recognize yet.
            const text = errorCopy(s, code, message, retryAfterSeconds);
            patch(assistantId, (current) => ({ ...current, pending: false }));
            setMessages((current) => [
              ...current,
              { id: nextId('n'), role: 'system-note', content: text, tone: 'warn' },
            ]);
            // The note explains it in place; the toast makes sure it is noticed even
            // if the panel is scrolled away from the bottom.
            showToast({ kind: 'error', message: text });
            setStreaming(false);
            abortRef.current = null;
          },
        },
        controller.signal
      );
    },
    [messages, patch, showToast, s, streaming, transport]
  );

  /** Update one proposal by id, wherever in the transcript it lives. */
  const patchProposal = useCallback((id: string, update: (entry: ChatProposal) => ChatProposal) => {
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
  }, []);

  /** Find a proposal by id across the transcript. */
  const findProposal = useCallback(
    (id: string): ChatProposal | undefined =>
      messages.flatMap((message) => message.proposals ?? []).find((entry) => entry.id === id),
    [messages]
  );

  const executeProposal = useCallback(
    async (id: string) => {
      const entry = findProposal(id);
      if (entry == null) return;
      // Double-submit guard: a `succeeded` proposal never re-runs, and an
      // `executing` one is already in flight.
      if (entry.state === 'executing' || entry.state === 'succeeded') return;

      patchProposal(id, (current) => ({ ...current, state: 'executing', error: undefined }));
      const { proposal } = entry;
      try {
        // Only these three whitelisted, typed functions — the exact ones the
        // dashboard's own buttons call. There is deliberately no generic
        // method/path executor, so `target` can never drive a request.
        const result =
          proposal.kind === 'create_session'
            ? await createTrigger(
                apiFetch,
                proposal.owner,
                proposal.name,
                mapDraftToRequest(proposal.request)
              )
            : proposal.kind === 'create_work_item'
              ? await createWorkItem(
                  apiFetch,
                  proposal.owner,
                  proposal.name,
                  proposal.trigger_issue_number,
                  {
                    title: proposal.title,
                    ...(proposal.label ? { label: proposal.label } : {}),
                    body: proposal.body,
                  }
                )
              : await stopTrigger(
                  apiFetch,
                  proposal.owner,
                  proposal.name,
                  proposal.trigger_issue_number
                );

        if (!result.ok) {
          patchProposal(id, (current) => ({
            ...current,
            state: 'failed',
            error: result.message ?? s.executeFailed,
          }));
          showToast({ kind: 'error', message: result.message ?? s.executeFailed });
          return;
        }

        // A created issue carries its own number and URL; a stop returns nothing.
        const created = result.data as { issue_number?: number; html_url?: string } | null;
        patchProposal(id, (current) => ({
          ...current,
          state: 'succeeded',
          error: undefined,
          ...(created?.html_url ? { issueUrl: created.html_url } : {}),
          ...(created?.issue_number ? { issueNumber: created.issue_number } : {}),
        }));
        // A note in the thread so the outcome is part of the conversation, not just
        // a card the user might scroll past.
        setMessages((current) => [
          ...current,
          {
            id: nextId('n'),
            role: 'system-note',
            content: outcomeNote(s, proposal, created?.issue_number),
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
    [apiFetch, findProposal, patchProposal, s, showToast]
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
          content: outcomeNote(s, entry.proposal),
          tone: 'info',
        },
      ]);
    },
    [findProposal, patchProposal, s]
  );

  const dismissProposal = useCallback((id: string) => {
    setMessages((current) =>
      current.map((message) =>
        message.proposals?.some((entry) => entry.id === id)
          ? { ...message, proposals: message.proposals.filter((entry) => entry.id !== id) }
          : message
      )
    );
  }, []);

  const clearTranscript = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setStreaming(false);
    setMessages([]);
  }, []);

  const value = useMemo(
    () => ({
      open,
      messages,
      streaming,
      openPanel: () => setOpen(true),
      closePanel: () => setOpen(false),
      toggle: () => setOpen((current) => !current),
      sendMessage,
      stopStreaming,
      clearTranscript,
      executeProposal,
      dismissProposal,
      markProposalSucceeded,
    }),
    [
      open,
      messages,
      streaming,
      sendMessage,
      stopStreaming,
      clearTranscript,
      executeProposal,
      dismissProposal,
      markProposalSucceeded,
    ]
  );

  return <ChatContext.Provider value={value}>{children}</ChatContext.Provider>;
}

/** Read the chat state. Throws outside a provider, because a silent no-op
 *  context would make a mis-mounted panel look merely broken. */
export function useChat(): ChatContextValue {
  const value = useContext(ChatContext);
  if (value == null) throw new Error('useChat must be used inside a ChatProvider');
  return value;
}
