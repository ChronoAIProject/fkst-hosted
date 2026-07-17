import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useVisibilityPoll } from './use-visibility-poll';

function setHidden(hidden: boolean) {
  Object.defineProperty(document, 'hidden', { configurable: true, value: hidden });
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('useVisibilityPoll', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
  });

  it('ticks on the interval while enabled and visible', () => {
    const cb = vi.fn();
    renderHook(() => useVisibilityPoll(cb, 15000, true));
    expect(cb).not.toHaveBeenCalled();
    vi.advanceTimersByTime(45000);
    expect(cb).toHaveBeenCalledTimes(3);
  });

  it('does nothing when disabled', () => {
    const cb = vi.fn();
    renderHook(() => useVisibilityPoll(cb, 15000, false));
    vi.advanceTimersByTime(60000);
    expect(cb).not.toHaveBeenCalled();
  });

  it('pauses while the document is hidden and resumes with an immediate tick', () => {
    const cb = vi.fn();
    renderHook(() => useVisibilityPoll(cb, 15000, true));

    setHidden(true);
    vi.advanceTimersByTime(60000);
    expect(cb).not.toHaveBeenCalled(); // paused

    setHidden(false);
    expect(cb).toHaveBeenCalledTimes(1); // stale-on-return refresh
    vi.advanceTimersByTime(15000);
    expect(cb).toHaveBeenCalledTimes(2); // interval resumed
  });

  it('stops ticking after unmount', () => {
    const cb = vi.fn();
    const { unmount } = renderHook(() => useVisibilityPoll(cb, 15000, true));
    unmount();
    vi.advanceTimersByTime(60000);
    expect(cb).not.toHaveBeenCalled();
  });
});
