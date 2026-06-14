import type { OverviewProps, OverviewGoal } from '@/screens/overview/overview';

// Mock goals optimized with short display titles to match overview.html pipeline rows exactly.
export const mockGoalsPipeline: OverviewGoal[] = [
  // Design Stage
  {
    id: '214',
    title: 'Tighten consensus parser',
    stage: 'Design',
    state: 'thinking',
    age: '4m',
    repo: 'fkst-substrate',
  },
  {
    id: '231',
    title: 'Document DLQ retention',
    stage: 'Design',
    state: 'ready',
    age: '11m',
    repo: 'fkst-packages',
  },
  {
    id: '240',
    title: 'Validate cron source cadence',
    stage: 'Design',
    state: 'thinking',
    age: '18m',
    repo: 'fkst-substrate',
  },
  // Build Stage
  {
    id: '205',
    title: 'Set-exclusive state label',
    stage: 'Build',
    state: 'implementing',
    age: '1m',
    repo: 'fkst-substrate',
  },
  {
    id: '238',
    title: 'Reflect dead-letters in board',
    stage: 'Build',
    state: 'implementing',
    age: '7m',
    repo: 'fkst-packages',
  },
  {
    id: '242',
    title: 'Extract fanout spawn',
    stage: 'Build',
    state: 'pr-open',
    age: '12m',
    repo: 'fkst-substrate',
  },
  // Review Stage
  {
    id: '187',
    title: 'source_ref propagation',
    stage: 'Review',
    state: 'reviewing',
    age: '14m',
    repo: 'fkst-substrate',
    pr: '41',
    pressure: true,
  },
  {
    id: '161',
    title: 'ack on RAISED fail',
    stage: 'Review',
    state: 'reviewing',
    age: '31m',
    repo: 'fkst-substrate',
    pr: '38',
  },
  {
    id: '158',
    title: 'Reconcile lease_generation',
    stage: 'Review',
    state: 'fixing',
    age: '44m',
    repo: 'fkst-packages',
    pr: '36',
  },
  // Ship Stage
  {
    id: '152',
    title: 'composed conformance',
    stage: 'Ship',
    state: 'merging',
    age: '3m',
    repo: 'github-autochrono',
    pr: '29',
    pressure: true,
  },
  {
    id: '149',
    title: 'cache_get/set helpers',
    stage: 'Ship',
    state: 'merge-ready', // green badge in mockup, merging renders red
    age: '6m',
    repo: 'fkst-substrate',
    pr: '27',
  },
  // Merged Stage
  {
    id: '118',
    title: 'Durable redb cutover for the delivery ledger',
    stage: 'Merged',
    state: 'merged',
    age: '2h',
    repo: 'fkst-substrate',
    pr: '25',
  },
  {
    id: '117',
    title: 'Lease fencing on ack / retry / dead',
    stage: 'Merged',
    state: 'merged',
    age: '3h',
    repo: 'fkst-substrate',
    pr: '24',
  },
];

// Mock goals optimized with full display titles to match overview.html board cards exactly.
// Note: We avoid overloading semantic props (like age/repo/pr) with presentation strings.
// Gaps between mockup metadata strings and screen props are annotated and reported.
export const mockGoalsBoard: OverviewGoal[] = [
  // Design Stage
  {
    id: '214',
    title: 'Tighten consensus parser to handle nested quorum refs',
    stage: 'Design',
    state: 'thinking',
    age: '4m',
    repo: 'fkst-substrate',
  },
  {
    id: '231',
    title: 'Document DLQ retention & replay semantics',
    stage: 'Design',
    state: 'ready',
    age: '11m',
    repo: 'fkst-packages',
  },
  {
    id: '240',
    title: 'Validate cron source cadence on cold start',
    stage: 'Design',
    state: 'thinking',
    age: '18m',
    repo: 'fkst-substrate',
  },
  // Build Stage
  {
    id: '205',
    title: 'Make the state label set-exclusive on PR open',
    stage: 'Build',
    state: 'implementing',
    age: '1m',
    repo: 'fkst-substrate',
  },
  {
    id: '238',
    title: 'Reflect dead-letters back into the goal board',
    stage: 'Build',
    state: 'implementing',
    age: '7m',
    repo: 'fkst-packages',
  },
  {
    id: '242',
    title: 'Extract fanout spawn into its own module',
    stage: 'Build',
    state: 'pr-open',
    age: '12m',
    repo: 'fkst-substrate',
  },
  // Review Stage
  {
    id: '187',
    title: 'Propagate source_ref through the fanout spawn path',
    stage: 'Review',
    state: 'reviewing',
    age: '14m',
    repo: 'fkst-substrate',
    pr: '41',
    pressure: true,
  },
  {
    id: '161',
    title: 'ack on RAISED publish failure before retry',
    stage: 'Review',
    state: 'reviewing',
    age: '31m',
    repo: 'fkst-substrate',
    pr: '38',
  },
  {
    id: '158',
    title: 'Reconcile lease_generation on requeue',
    stage: 'Review',
    state: 'fixing',
    age: '44m',
    repo: 'fkst-packages',
    pr: '36',
  },
  // Ship Stage
  {
    id: '152',
    title: 'Composed conformance suite for github-autochrono',
    stage: 'Ship',
    state: 'merging',
    age: '3m',
    repo: 'github-autochrono',
    pr: '29',
    pressure: true,
  },
  {
    id: '149',
    title: 'Extract cache_get / cache_set helper functions',
    stage: 'Ship',
    state: 'merge-ready', // green badge in mockup, merging renders red
    age: '6m',
    repo: 'fkst-substrate',
    pr: '27',
  },
  // Merged Stage
  {
    id: '118',
    title: 'Durable redb cutover for the delivery ledger',
    stage: 'Merged',
    state: 'merged',
    age: '2h',
    repo: 'fkst-substrate',
    pr: '25',
  },
  {
    id: '117',
    title: 'Lease fencing on ack / retry / dead',
    stage: 'Merged',
    state: 'merged',
    age: '3h',
    repo: 'fkst-substrate',
    pr: '24',
  },
];

export const mockVitals: OverviewProps['vitals'] = {
  inFlight: 13,
  merged24h: 22,
  deadEnded: 3,
  throughput: '~5/h',
  medianReviewTime: '19m',
  windowStart: '12:00 Jun 10',
  windowEnd: '12:00 Jun 11',
};

export const mockNeedsYou: OverviewProps['needsYou'] = [
  {
    lead: 'Merging',
    leadTone: 'red',
    title: 'composed conformance for github-autochrono',
    id: '152',
    pr: '29',
    why: 'WRITE: REAL · merges autonomously into the integration branch (seconds) · CI green · to stop autonomous merges, flip the global write posture to DRY-RUN or close the PR on GitHub',
    actionLabel: 'Write posture →',
    actionTone: 'red',
  },
  {
    lead: 'Blocked',
    leadTone: 'red',
    title: 'Rework dispatcher into 2nd coordinator',
    id: '173',
    why: 'true-stall reconcile · engine gave up this framing after 3 rounds · terminal — re-engage with a fresh GitHub issue, not a re-open',
    actionLabel: 'New issue from this',
  },
];

export const mockStageIo: OverviewProps['stageIo'] = {
  Design: { inCount: 4, outCount: 2 },
  Build: { inCount: 2, outCount: 3 },
  Review: { inCount: 1, outCount: 1 },
  Ship: { inCount: 1, outCount: 22 },
};

export const mockStageMore: OverviewProps['stageMore'] = {
  Design: '+ 3 in consensus (converge)',
  Build: '+ 1 PR opening',
  Review: '+ 2 more',
  Merged: '+ 20 more · 24h',
};

export const mockIntakeDetails = ['github-proxy', 'cron · 5m', '↑ raised 2m ago'];

// Keeping the '· 24h' suffix for the Merged cap out via mergedDetails as best the props allow.
// The first index '—' is replaced by mergedGoals.length, and the second line will render as '· 24h'.
export const mockMergedDetails = ['—', '· 24h', 'terminal'];

// Type the populated fixtures as a whole via satisfies OverviewProps.
export const overviewPipelineFixture = {
  goals: mockGoalsPipeline,
  vitals: mockVitals,
  needsYou: mockNeedsYou,
  stageIo: mockStageIo,
  stageMore: mockStageMore,
  intakeDetails: mockIntakeDetails,
  mergedDetails: mockMergedDetails,
  reviewPressureLabel: 'bottleneck · 3 stalled ≥14m',
  shipTag: 'REAL · #152 → integration',
} satisfies OverviewProps;

export const overviewBoardFixture = {
  goals: mockGoalsBoard,
  vitals: mockVitals,
  needsYou: mockNeedsYou,
  stageIo: mockStageIo,
  stageMore: mockStageMore,
  intakeDetails: mockIntakeDetails,
  mergedDetails: mockMergedDetails,
  reviewPressureLabel: 'bottleneck · 3 stalled ≥14m',
  shipTag: 'REAL · #152 → integration',
} satisfies OverviewProps;
