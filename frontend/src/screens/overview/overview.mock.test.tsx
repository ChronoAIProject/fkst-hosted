import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Overview } from './overview';
import {
  overviewPipelineFixture,
  mockGoalsPipeline,
  mockVitals,
  mockNeedsYou,
} from '@/fixtures/overview';

describe('Overview Mock Fixture and Smoke Tests', () => {
  it('asserts fixture data is type-valid and populated', () => {
    expect(mockGoalsPipeline).toBeDefined();
    expect(mockGoalsPipeline.length).toBe(13);
    
    const designGoals = mockGoalsPipeline.filter(g => g.stage === 'Design');
    expect(designGoals.length).toBe(3);
    expect(designGoals[0]?.id).toBe('214');
    expect(designGoals[0]?.title).toBe('Tighten consensus parser');
    expect(designGoals[0]?.state).toBe('thinking');
    
    expect(mockVitals?.inFlight).toBe(13);
    expect(mockVitals?.throughput).toBe('~5/h');
    
    expect(mockNeedsYou).toBeDefined();
    expect(mockNeedsYou?.length).toBe(2);
    expect(mockNeedsYou?.[0]?.lead).toBe('Merging');
    expect(mockNeedsYou?.[0]?.id).toBe('152');
  });

  it('renders PipelinePopulated via story args and asserts mockup-derived strings', () => {
    render(<Overview {...overviewPipelineFixture} />);

    // Assert short display titles in pipeline view render
    expect(screen.getByText('Tighten consensus parser')).toBeInTheDocument();
    expect(screen.getByText('Document DLQ retention')).toBeInTheDocument();

    // Assert vitals render
    expect(screen.getByText('13')).toBeInTheDocument(); // in flight
    expect(screen.getByText('~5/h')).toBeInTheDocument(); // throughput
    expect(screen.getByText('19m')).toBeInTheDocument(); // median time-in-review

    // Assert needs you items render
    expect(screen.getByText('composed conformance for github-autochrono')).toBeInTheDocument();
    expect(screen.getByText('Rework dispatcher into 2nd coordinator')).toBeInTheDocument();
  });

  it('renders the "Needs-you unavailable" fallback when needsYou is undefined', () => {
    render(<Overview needsYou={undefined} />);
    expect(screen.getByText(/Needs-you unavailable — requires GitHub plane/i)).toBeInTheDocument();
    expect(screen.queryByText(/Nothing needs you/i)).toBeNull();
  });

  it('renders the quiet "Nothing needs you" status when needsYou is empty array', () => {
    render(<Overview needsYou={[]} />);
    expect(screen.getByText('Nothing needs you')).toBeInTheDocument();
    expect(screen.queryByText(/Needs-you unavailable/i)).toBeNull();
  });
});
