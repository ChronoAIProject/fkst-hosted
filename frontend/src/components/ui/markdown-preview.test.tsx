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
});
