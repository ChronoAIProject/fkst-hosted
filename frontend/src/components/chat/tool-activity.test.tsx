import { describe, it, expect } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ToolActivity } from './tool-activity';
import type { ChatToolEvent } from './chat-context';

const event = (over: Partial<ChatToolEvent> & Pick<ChatToolEvent, 'id'>): ChatToolEvent => ({
  name: 'get_overview',
  ...over,
});

describe('ToolActivity', () => {
  it('renders nothing without events', () => {
    const { container } = render(<ToolActivity events={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('shows a running call, then its result, as TEXT', () => {
    // Every state carries a word: colour is reinforcement, never the signal.
    const { rerender } = render(<ToolActivity events={[event({ id: 't1' })]} />);
    expect(screen.getByText(/RUNNING/)).toBeInTheDocument();

    rerender(<ToolActivity events={[event({ id: 't1', status: 200, truncated: false })]} />);
    expect(screen.queryByText(/RUNNING/)).not.toBeInTheDocument();
    expect(screen.getByText(/OK 200/)).toBeInTheDocument();
  });

  it('distinguishes a denial from a failure', () => {
    // A 403 is an answer about the user's access; a 500 is a fault. Reading the
    // same would mislead.
    render(
      <ToolActivity
        events={[
          event({ id: 'a', name: 'tail_log_file', status: 403 }),
          event({ id: 'b', name: 'observe_session', status: 500 }),
        ]}
      />
    );
    expect(screen.getByText(/DENIED 403/)).toBeInTheDocument();
    expect(screen.getByText(/ERR 500/)).toBeInTheDocument();
  });

  it('reads 404 as a denial too', () => {
    // The backend returns 404 for "no runtime, or you cannot see it" — from the
    // user's side that is the same class of answer as a 403.
    render(<ToolActivity events={[event({ id: 'a', name: 'observe_session', status: 404 })]} />);
    expect(screen.getByText(/DENIED 404/)).toBeInTheDocument();
  });

  it('marks a truncated result', () => {
    render(
      <ToolActivity
        events={[event({ id: 'a', name: 'tail_log_file', status: 200, truncated: true })]}
      />
    );
    expect(screen.getByText(/TRUNCATED/)).toBeInTheDocument();
  });

  it('humanizes known tool names and shows unknown ones raw', () => {
    render(
      <ToolActivity
        events={[
          event({ id: 'a', name: 'get_overview' }),
          event({ id: 'b', name: 'brand_new_tool' }),
        ]}
      />
    );
    expect(screen.getByText('accounts & repos')).toBeInTheDocument();
    // A newer backend tool still renders legibly rather than showing nothing.
    expect(screen.getByText('brand_new_tool')).toBeInTheDocument();
  });

  it('shows every row up to three without a disclosure', () => {
    render(
      <ToolActivity
        events={[
          event({ id: 'a' }),
          event({ id: 'b', name: 'list_log_runs' }),
          event({ id: 'c', name: 'search_manual' }),
        ]}
      />
    );
    expect(screen.queryByTestId('chat-activity-toggle')).not.toBeInTheDocument();
    expect(screen.getByText('accounts & repos')).toBeInTheDocument();
    expect(screen.getByText('log runs')).toBeInTheDocument();
    expect(screen.getByText('manual')).toBeInTheDocument();
  });

  it('collapses beyond three rows behind a disclosure', () => {
    // Showing the machine's work builds trust; burying the answer under it does not.
    render(
      <ToolActivity
        events={[
          event({ id: 'a', name: 'get_overview' }),
          event({ id: 'b', name: 'list_repo_sessions' }),
          event({ id: 'c', name: 'list_log_runs' }),
          event({ id: 'd', name: 'tail_log_file' }),
          event({ id: 'e', name: 'search_manual' }),
        ]}
      />
    );
    expect(screen.queryByText('log tail')).not.toBeInTheDocument();

    const toggle = screen.getByTestId('chat-activity-toggle');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    // The count tells the user what is hidden.
    expect(toggle).toHaveTextContent('(5)');

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('log tail')).toBeInTheDocument();
    expect(screen.getByText('manual')).toBeInTheDocument();

    fireEvent.click(toggle);
    expect(screen.queryByText('log tail')).not.toBeInTheDocument();
  });
});
