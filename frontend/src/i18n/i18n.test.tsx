import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { LanguageProvider, useContent } from './index';
import { LanguageToggle } from '@/components/layout/language-toggle';
import { Rich } from '@/components/content/rich';

function Probe() {
  const c = useContent();
  return <div>{c.nav.introduction}</div>;
}

describe('i18n', () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it('defaults to English and switches to 简体中文 via the toggle', async () => {
    const user = userEvent.setup();
    render(
      <LanguageProvider>
        <LanguageToggle />
        <Probe />
      </LanguageProvider>
    );

    expect(screen.getByText('Introduction')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '中文' }));

    expect(screen.getByText('介绍')).toBeInTheDocument();
    expect(document.documentElement.lang).toBe('zh');
    expect(window.localStorage.getItem('fkst-lang')).toBe('zh');
  });

  it('useContent works without a provider (English default)', () => {
    render(<Probe />);
    expect(screen.getByText('Introduction')).toBeInTheDocument();
  });
});

describe('Rich', () => {
  it('renders `code` as <code> and **bold** as emphasis', () => {
    render(<Rich>{'run `gh` with **care**'}</Rich>);
    const code = screen.getByText('gh');
    expect(code.tagName).toBe('CODE');
    expect(screen.getByText('care')).toBeInTheDocument();
  });
});
