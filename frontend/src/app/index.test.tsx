import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { App } from './index';

describe('App Smoke Test', () => {
  it('redirects to /overview and renders the Overview screen pending text', async () => {
    render(<App />);
    
    await waitFor(() => {
      expect(screen.getByText('Overview')).toBeInTheDocument();
      expect(screen.getByText('screen pending')).toBeInTheDocument();
      expect(window.location.pathname).toBe('/overview');
    });
  });
});
