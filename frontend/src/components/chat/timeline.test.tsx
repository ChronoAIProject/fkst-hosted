import { describe, it, expect } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { LanguageProvider } from '@/i18n';
import { Timeline } from './timeline';
import type { ChatStep } from './steps';

function renderTimeline(steps: ChatStep[], level: 'clean' | 'verbose') {
  return render(
    <LanguageProvider>
      <Timeline steps={steps} level={level} />
    </LanguageProvider>
  );
}

const STEPS: ChatStep[] = [
  { kind: 'round', index: 0, toolsOffered: 17, finishReason: 'tool_calls', toolCalls: 1 },
  {
    kind: 'tool',
    id: 't1',
    name: 'get_overview',
    argsPreview: '{}',
    args: '{"account":"acme"}',
    status: 200,
    truncated: false,
    response: '{"repos":["a"]}',
    bytes: 15,
  },
];

describe('Timeline', () => {
  it('renders nothing when the turn used no machinery', () => {
    const { container } = renderTimeline([], 'verbose');
    expect(container).toBeEmptyDOMElement();
  });

  it('collapses to a single count in CLEAN', () => {
    renderTimeline(STEPS, 'clean');
    expect(screen.getByTestId('chat-timeline-summary')).toHaveTextContent('1 step');
    expect(screen.queryByTestId('chat-timeline')).not.toBeInTheDocument();
  });

  it('shows nothing in CLEAN when a turn ran no tools at all', () => {
    // A round marker alone is machinery, not a step worth counting.
    const { container } = renderTimeline([STEPS[0]!], 'clean');
    expect(container).toBeEmptyDOMElement();
  });

  it('gives every tool call its own row in VERBOSE', () => {
    renderTimeline(
      [...STEPS, { kind: 'tool', id: 't2', name: 'list_repos', argsPreview: '{}', status: 200 }],
      'verbose'
    );
    expect(screen.getAllByTestId('chat-step-tool')).toHaveLength(2);
    expect(screen.getByTestId('chat-step-round')).toBeInTheDocument();
  });

  it('hides the detail until the row is activated, then shows both payloads', () => {
    renderTimeline(STEPS, 'verbose');
    expect(screen.queryByText(/"account": "acme"/)).not.toBeInTheDocument();

    const row = screen.getByTestId('chat-step-tool');
    expect(row).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(row);

    expect(row).toHaveAttribute('aria-expanded', 'true');
    // The FULL arguments, pretty-printed — not the truncated preview.
    expect(screen.getByText(/"account": "acme"/)).toBeInTheDocument();
    expect(screen.getByText(/"repos"/)).toBeInTheDocument();
  });

  it('reports the true size even when the payload was capped', () => {
    renderTimeline(
      [
        {
          kind: 'tool',
          id: 't1',
          name: 'get_overview',
          argsPreview: '{}',
          status: 200,
          response: '{"partial"',
          responseTruncated: true,
          bytes: 14173,
        },
      ],
      'verbose'
    );
    // The row states how much there really was, not how much survived the cap.
    expect(screen.getByText('13.8 KB')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('chat-step-tool'));
    expect(screen.getByText(/truncated/i)).toBeInTheDocument();
  });

  it('marks a round still in flight differently from a closed one', () => {
    renderTimeline([{ kind: 'round', index: 0, toolsOffered: 3 }], 'verbose');
    expect(screen.getByTestId('chat-step-round')).toHaveTextContent(/working/i);
  });

  it('renders a running call without a status as RUNNING, not as an error', () => {
    renderTimeline(
      [{ kind: 'tool', id: 't1', name: 'get_overview', argsPreview: '{}' }],
      'verbose'
    );
    expect(screen.getByTestId('chat-step-tool')).toHaveTextContent(/RUNNING/i);
  });

  it('reads a denial as denied rather than as a failure', () => {
    renderTimeline(
      [{ kind: 'tool', id: 't1', name: 'observe_session', argsPreview: '{}', status: 403 }],
      'verbose'
    );
    expect(screen.getByTestId('chat-step-tool')).toHaveTextContent(/DENIED/i);
  });

  it('reads an absent thing as absent, not as a denial', () => {
    // A session with no logs yet must not tell the user they lack access.
    renderTimeline(
      [{ kind: 'tool', id: 't1', name: 'tail_log_file', argsPreview: '{}', status: 404 }],
      'verbose'
    );
    const row = screen.getByTestId('chat-step-tool');
    expect(row).not.toHaveTextContent(/DENIED/i);
  });
});
