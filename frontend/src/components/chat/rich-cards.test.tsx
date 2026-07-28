import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { labelTone, RichCards } from './rich-cards';
import type { SessionRef } from './transport';

const ref = (over: Partial<SessionRef> = {}): SessionRef => ({
  owner: 'acme',
  name: 'site',
  session_id: 'sess-1',
  trigger_number: 7,
  title: 'nightly',
  status_label: 'fkst-substrate-active',
  ...over,
});

function renderCards(refs: SessionRef[]) {
  return render(
    <MemoryRouter>
      <RichCards refs={refs} />
    </MemoryRouter>
  );
}

describe('RichCards', () => {
  it('renders no strip without refs', () => {
    const { container } = renderCards([]);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders a card with the session, repo and status label', () => {
    renderCards([ref()]);
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('acme/site')).toBeInTheDocument();
    // The raw label IS the chip text, so meaning never rides on colour.
    expect(screen.getByText('fkst-substrate-active')).toBeInTheDocument();
  });

  it('falls back to the trigger number when a session has no name', () => {
    renderCards([ref({ title: undefined })]);
    expect(screen.getByText('trigger #7')).toBeInTheDocument();
  });

  it('labels a session with no status at all', () => {
    renderCards([ref({ status_label: undefined })]);
    expect(screen.getByText('SESSION')).toBeInTheDocument();
  });

  it('deep-links to the dashboard by session id', () => {
    renderCards([ref()]);
    expect(screen.getByTestId('chat-card-dashboard-link')).toHaveAttribute(
      'href',
      '/dashboard?owner=acme&repo=site&session=sess-1'
    );
  });

  it('deep-links by the trigger alias when there is no session id yet', () => {
    // This is the only key a card can carry before the session starts, and the
    // workspace accepts both forms — so the link keeps working afterwards.
    renderCards([ref({ session_id: undefined })]);
    expect(screen.getByTestId('chat-card-dashboard-link')).toHaveAttribute(
      'href',
      '/dashboard?owner=acme&repo=site&session=trigger-7'
    );
  });

  it('encodes owner, repo and session in the deep link', () => {
    renderCards([ref({ owner: 'a c', name: 'b&d', session_id: 'x/y' })]);
    expect(screen.getByTestId('chat-card-dashboard-link')).toHaveAttribute(
      'href',
      '/dashboard?owner=a%20c&repo=b%26d&session=x%2Fy'
    );
  });

  it('links the trigger issue on github over https, safely', () => {
    renderCards([ref()]);
    const link = screen.getByTestId('chat-card-trigger-link');
    expect(link).toHaveAttribute('href', 'https://github.com/acme/site/issues/7');
    expect(link).toHaveAttribute('target', '_blank');
    // Without noopener the opened page gets a handle on this window.
    expect(link).toHaveAttribute('rel', 'noopener noreferrer');
  });

  it('caps the number of cards', () => {
    const many = Array.from({ length: 10 }, (_, index) =>
      ref({ trigger_number: index + 1, session_id: `s-${index}` })
    );
    renderCards(many);
    expect(screen.getAllByTestId('chat-session-card')).toHaveLength(6);
  });
});

describe('labelTone', () => {
  it('reads a healthy session as green', () => {
    expect(labelTone('fkst-substrate-active')).toBe('green');
    expect(labelTone('fkst-picked-up')).toBe('green');
  });

  it('reads every problem label as red', () => {
    for (const label of [
      'fkst-degraded',
      'fkst-substrate-invalid',
      'fkst-config-rejected',
      'fkst-trigger-unauthorized',
      'fkst-unrouted',
      'fkst-unauthorized',
    ]) {
      expect(labelTone(label)).toBe('red');
    }
  });

  it('reads a retired session as neutral, because it is a fact not a problem', () => {
    expect(labelTone('fkst-session-retired')).toBe('neutral');
  });

  it('reads an unknown or absent label as neutral', () => {
    // Guessing a tone for something we do not recognize would mislead.
    expect(labelTone('fkst-something-new')).toBe('neutral');
    expect(labelTone(undefined)).toBe('neutral');
  });
});
