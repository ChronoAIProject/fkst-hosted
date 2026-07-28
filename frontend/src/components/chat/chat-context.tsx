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
import { useAuth } from '@/lib/auth/github-auth';
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

/** Read the stored transcript, tolerating anything. A corrupt or foreign value
 *  must degrade to an empty transcript, never break the panel. */
function readStored(): ChatMessage[] {
  try {
    const raw = window.sessionStorage?.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (entry): entry is ChatMessage =>
        typeof entry === 'object' &&
        entry != null &&
        typeof (entry as ChatMessage).id === 'string' &&
        typeof (entry as ChatMessage).content === 'string'
    );
  } catch {
    return [];
  }
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
  const { isAuthenticated } = useAuth();
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
          // Proposals are the confirm-UI milestone's job; ignoring them here keeps
          // this surface honest about what it can do.
          onActionProposal: () => {},
          onDone: ({ sessionRefs }) => {
            patch(assistantId, (message) => ({ ...message, pending: false, sessionRefs }));
            setStreaming(false);
            abortRef.current = null;
          },
          onError: ({ message }) => {
            patch(assistantId, (current) => ({ ...current, pending: false }));
            setMessages((current) => [
              ...current,
              { id: nextId('n'), role: 'system-note', content: message, tone: 'warn' },
            ]);
            setStreaming(false);
            abortRef.current = null;
          },
        },
        controller.signal
      );
    },
    [messages, patch, streaming, transport]
  );

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
    }),
    [open, messages, streaming, sendMessage, stopStreaming, clearTranscript]
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
