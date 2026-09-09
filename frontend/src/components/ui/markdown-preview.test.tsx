import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { MarkdownPreview } from './markdown-preview';

describe('MarkdownPreview', () => {
  it('renders the supported block and inline Markdown subset', () => {
    render(
      <MarkdownPreview
        ariaLabel="Markdown preview"
        markdown={[
          '# Delivery plan',
          '',
          'Use **care**, *tests*, and `cargo test` with [the docs](https://example.com/docs).',
          '',
          '- first task',
          '- second task',
          '',
          '1. prepare',
          '2. ship',
          '',
          '```rust',
          'let ready = true;',
          '```',
        ].join('\n')}
      />
    );

    expect(screen.getByRole('heading', { level: 1, name: 'Delivery plan' })).toBeInTheDocument();
    expect(screen.getByText('care').tagName).toBe('STRONG');
    expect(screen.getByText('tests').tagName).toBe('EM');
    expect(screen.getByText('cargo test').tagName).toBe('CODE');
    expect(screen.getByRole('link', { name: 'the docs' })).toHaveAttribute(
      'href',
      'https://example.com/docs'
    );
    const lists = screen.getAllByRole('list');
    expect(within(lists[0]!).getByText('second task')).toBeInTheDocument();
    expect(within(lists[1]!).getByText('ship')).toBeInTheDocument();
    const fenced = screen.getByText('let ready = true;');
    expect(fenced.tagName).toBe('CODE');
    expect(fenced).toHaveAttribute('data-language', 'rust');
    expect(fenced.closest('pre')).not.toBeNull();
  });

  it('keeps raw HTML as text and refuses executable link schemes', () => {
    const markdown =
      '<img src=x onerror=alert(1)> <script>alert(2)</script> [unsafe](javascript:alert(3))';
    const { container } = render(
      <MarkdownPreview ariaLabel="Markdown preview" markdown={markdown} />
    );

    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('script')).toBeNull();
    expect(screen.queryByRole('link', { name: 'unsafe' })).not.toBeInTheDocument();
    expect(container).toHaveTextContent('<img src=x onerror=alert(1)>');
    expect(container).toHaveTextContent('[unsafe](javascript:alert(3))');
  });

  it('renders a pipe table as a real table', () => {
    // Observed live: the Orchestrator answered with a Markdown table and it rendered as
    // raw pipes, which reads as broken output.
    const markdown = [
      '| Profile | Status | Secrets |',
      '|---|:--:|---|',
      '| video-studio | ready | 1 |',
      '| docs | failed | 0 |',
    ].join('\n');
    const { container } = render(
      <MarkdownPreview ariaLabel="Markdown preview" markdown={markdown} />
    );

    const table = container.querySelector('table');
    expect(table).not.toBeNull();
    expect(container.querySelectorAll('thead th')).toHaveLength(3);
    expect(container.querySelectorAll('tbody tr')).toHaveLength(2);
    expect(screen.getByText('video-studio')).toBeInTheDocument();
    // No stray pipes left behind.
    expect(table!.textContent).not.toContain('|');
  });

  it('pads a short row so the columns stay aligned', () => {
    const markdown = ['| A | B | C |', '|---|---|---|', '| only-one |'].join('\n');
    const { container } = render(
      <MarkdownPreview ariaLabel="Markdown preview" markdown={markdown} />
    );
    // Indexed by the HEADER width, so a ragged row cannot shift the columns.
    expect(container.querySelectorAll('tbody td')).toHaveLength(3);
  });

  it('leaves pipes alone when there is no delimiter row', () => {
    // A sentence containing pipes is not a table.
    const markdown = 'run `a | b | c` to pipe them';
    const { container } = render(
      <MarkdownPreview ariaLabel="Markdown preview" markdown={markdown} />
    );
    expect(container.querySelector('table')).toBeNull();
    expect(container).toHaveTextContent('a | b | c');
  });

  it('renders flow variant without the boxed height cap', () => {
    const { container: boxed } = render(
      <MarkdownPreview ariaLabel="p" markdown="hi" variant="boxed" />
    );
    const { container: flow } = render(
      <MarkdownPreview ariaLabel="p" markdown="hi" variant="flow" />
    );
    // The boxed preview reserves a minimum height and caps its own scroll; the flow
    // variant must do neither, or a chat answer is cut off mid-sentence.
    expect(boxed.firstElementChild!.className).toContain('max-h-64');
    expect(flow.firstElementChild!.className).not.toContain('max-h-64');
    expect(flow.firstElementChild!.className).not.toContain('min-h-');
  });
});
