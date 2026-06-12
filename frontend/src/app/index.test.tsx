import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { App } from './index';
import { nextCondensed } from './shell';

describe('App Smoke Test', () => {
  it('redirects to /overview and renders the Overview screen pending text', async () => {
    render(<App />);
    
    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Overview' })).toBeInTheDocument();
      expect(screen.getByText('screen pending')).toBeInTheDocument();
      expect(window.location.pathname).toBe('/overview');
    });
  });

  it('renders topbar logo and nav links correctly, excluding Settings from primary nav', async () => {
    render(<App />);

    await waitFor(() => {
      // 1. Logo text "FKST"
      const logoLink = screen.getByRole('link', { name: (name) => name.replace(/\s+/g, '') === 'FKST' });
      expect(logoLink).toBeInTheDocument();
      expect(logoLink.getAttribute('href')).toBe('/overview');

      // 2. Primary nav links exist
      const overviewLink = screen.getByRole('link', { name: 'Overview' });
      const goalsLink = screen.getByRole('link', { name: 'Goals' });
      const packagesLink = screen.getByRole('link', { name: 'Packages' });
      
      expect(overviewLink).toBeInTheDocument();
      expect(goalsLink).toBeInTheDocument();
      expect(packagesLink).toBeInTheDocument();

      // 3. Settings is NOT in the primary nav list (nav role)
      const navElement = screen.getByRole('navigation');
      const settingsInNav = navElement.querySelector('a[href="/settings"]');
      expect(settingsInNav).toBeNull();

      // 4. Avatar (outside nav) links to settings
      const avatarLink = screen.getByRole('link', { name: /sign-in pending/i });
      expect(avatarLink).toBeInTheDocument();
      expect(avatarLink.getAttribute('href')).toBe('/settings');
    });
  });
});

describe('Hysteresis Unit Tests (nextCondensed)', () => {
  it('handles standard transition triggers', () => {
    // y = 0 -> false
    expect(nextCondensed(false, 0)).toBe(false);
    expect(nextCondensed(true, 0)).toBe(false);

    // y = 100 from false -> false
    expect(nextCondensed(false, 100)).toBe(false);

    // y = 141 -> true
    expect(nextCondensed(false, 141)).toBe(true);
    expect(nextCondensed(true, 141)).toBe(true);

    // y = 100 from true -> true (remains true due to hysteresis)
    expect(nextCondensed(true, 100)).toBe(true);

    // y = 39 -> false
    expect(nextCondensed(true, 39)).toBe(false);
    expect(nextCondensed(false, 39)).toBe(false);
  });

  it('handles boundary values 40/140 exactly per semantics', () => {
    // 140 is NOT > 140, so state remains unchanged
    expect(nextCondensed(false, 140)).toBe(false);
    expect(nextCondensed(true, 140)).toBe(true);

    // 40 is NOT < 40, so state remains unchanged
    expect(nextCondensed(true, 40)).toBe(true);
    expect(nextCondensed(false, 40)).toBe(false);
  });
});
