import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { GetStarted, scrollToAnchor } from './get-started';

function renderGetStarted() {
  return render(
    <MemoryRouter>
      <GetStarted />
    </MemoryRouter>
  );
}

describe('GetStarted', () => {
  it('renders the page title and the install step', () => {
    renderGetStarted();
    expect(
      screen.getByRole('heading', { level: 1, name: /Drive fkst-hosted with GitHub issues/i })
    ).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Install the GitHub App/i })).toBeInTheDocument();
  });

  it('documents the required trigger parameters', () => {
    renderGetStarted();
    // Each heading appears at least in the field reference (some are also
    // referenced in prose), so assert presence rather than uniqueness.
    expect(screen.getAllByText('### Session Name').length).toBeGreaterThan(0);
    expect(screen.getAllByText('### Packages').length).toBeGreaterThan(0);
    expect(screen.getAllByText('### Work Label').length).toBeGreaterThan(0);
  });

  it('explains the package-reference grammar and log access', () => {
    renderGetStarted();
    expect(screen.getAllByText(/owner\/repo@ref:path/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/\/api\/v1\/logs/).length).toBeGreaterThan(0);
  });
});

describe('GetStarted anchor navigation', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    // Clear any hash a prior test left so the mount deep-link effect stays inert.
    window.history.replaceState(null, '', window.location.pathname);
  });

  it('renders in-page index links whose targets carry a scroll offset', () => {
    const { container } = renderGetStarted();
    // The index link and the section it targets must agree, and the section
    // must keep the scroll-margin so its heading clears the topbar.
    const link = container.querySelector('a[href="#status"]');
    expect(link).not.toBeNull();
    const section = document.getElementById('status');
    expect(section).not.toBeNull();
    expect(section).toHaveClass('scroll-mt-[80px]');
    expect(section?.tagName.toLowerCase()).toBe('section');
  });

  it('drives scrollIntoView on the target section (not the window) when clicked', () => {
    // jsdom stubs scrollIntoView as a no-op; spy on the prototype so any
    // target element is covered, and assert the click targets it explicitly
    // rather than leaving the nested-container jump to the browser.
    const spy = vi
      .spyOn(HTMLElement.prototype, 'scrollIntoView')
      .mockImplementation(() => undefined);
    const { container } = renderGetStarted();

    const link = container.querySelector('a[href="#logs"]') as HTMLAnchorElement;
    fireEvent.click(link);

    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith({ behavior: 'smooth', block: 'start' });
    // The hash is preserved for shareable deep-links.
    expect(window.location.hash).toBe('#logs');
  });

  it('scrollToAnchor is a no-op for an unknown id and never throws', () => {
    const spy = vi
      .spyOn(HTMLElement.prototype, 'scrollIntoView')
      .mockImplementation(() => undefined);
    renderGetStarted();

    expect(() => scrollToAnchor('does-not-exist')).not.toThrow();
    expect(spy).not.toHaveBeenCalled();
  });

  it('falls back to an instant jump under reduced motion', () => {
    const spy = vi
      .spyOn(HTMLElement.prototype, 'scrollIntoView')
      .mockImplementation(() => undefined);
    const mql = { matches: true } as MediaQueryList;
    vi.spyOn(window, 'matchMedia').mockReturnValue(mql);
    renderGetStarted();

    scrollToAnchor('install');
    expect(spy).toHaveBeenCalledWith({ behavior: 'auto', block: 'start' });
  });
});
