// The Phase 1 synthetic-baseline replay (spec section 37, Phase 1).
//
// WHY IT EXISTS. Every Phase 1 repository is a real repository with no Evolution
// history and therefore no manifest, so convergence conditions 1, 2, 3, 4 and 6
// have no operand and a live oracle would emit "baseline required" for every
// repository for a week — measuring nothing. The phase instead picks a
// historical revision, replays the branch forward commit by commit, and reports
// per commit whether it was product-relevant or coverage-only, and at which
// commits a cycle would have been admitted.
//
// WHAT IT ANSWERS. Open question 40.16: what belongs in `source.productRelevant`,
// and whether a defensible default exists at all. The spec deliberately ships no
// default because the failure modes are asymmetric — a too-broad set is merely
// expensive and visibly so, while a too-narrow one fails silently. This turns
// the question from a guess into an observation, and it can score several
// candidate selectors over the same history so they are directly comparable.
//
// METHOD. Admission is decided by intersecting each commit's changed-path set
// with the selector, which is the same method section 17.7 condition 5
// prescribes. It is deliberately not a per-commit tree hash: that would cost a
// full tree read per commit per candidate, and the spec explicitly forbids
// enumerating and hashing every commit in a range on every reconcile.
//
// KNOWN IMPRECISION, stated rather than hidden: a change followed by its exact
// revert touches product-relevant paths twice while leaving the fingerprint
// equal to where it started. Path intersection counts two admissions where
// fingerprint comparison would count zero. It therefore OVER-reports admission
// slightly, which is the safe direction for sizing.

import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { log } from './log.ts';
import { coverageMatcher, productRelevantMatcher, type PathSelector } from './selector.ts';

const execFileAsync = promisify(execFile);

/** One named candidate `source.productRelevant` set to score. */
export interface Candidate {
  name: string;
  selector: PathSelector;
}

export interface CommitOutcome {
  sha: string;
  date: string;
  subject: string;
  changedPaths: number;
  /** Candidate name -> would this commit have admitted a cycle. */
  admits: Record<string, boolean>;
  /** True when the commit touched nothing outside the reserved prefixes. */
  reservedOnly: boolean;
}

export interface CandidateScore {
  name: string;
  admitted: number;
  coverageOnly: number;
  admissionRate: number;
  /** Directories that most often drove an admission, most frequent first. */
  topDrivers: { path: string; commits: number }[];
}

export interface ReplayReport {
  repository: string;
  fromCommit: string;
  toCommit: string;
  commits: number;
  /** Commits that touched only reserved prefixes — never admissible. */
  reservedOnlyCommits: number;
  candidates: CandidateScore[];
  timeline: CommitOutcome[];
}

async function git(repoRoot: string, args: string[]): Promise<string> {
  const { stdout } = await execFileAsync('git', args, {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
  });
  return stdout;
}

/** First-parent commit list, oldest first, so a merge is one product event. */
async function commitList(repoRoot: string, range: string): Promise<{ sha: string; date: string; subject: string }[]> {
  // `--first-parent`: section 15.5 asks that merge topology not present
  // implementation merge commits as independent user-facing releases. On a
  // squash-merge repository the first-parent walk IS the sequence of merged
  // pull requests, which is the unit a cycle would actually observe.
  const out = await git(repoRoot, [
    'log', '--first-parent', '--reverse', '--format=%H%x1f%ad%x1f%s', '--date=short', range,
  ]);
  return out
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [sha, date, subject] = line.split('\x1f');
      return { sha, date, subject };
    });
}

async function changedPaths(repoRoot: string, sha: string): Promise<string[]> {
  // `-m --first-parent` makes a merge report its net effect against the branch
  // it landed on, rather than an empty diff.
  const out = await git(repoRoot, [
    'diff-tree', '--no-commit-id', '--name-only', '-r', '-m', '--first-parent', sha,
  ]);
  return [...new Set(out.split('\n').filter(Boolean))];
}

/** The directory a path is attributed to when reporting admission drivers. */
function driverKey(path: string): string {
  const parts = path.split('/');
  // Two levels is the useful granularity here: `backend/src` and `frontend/src`
  // are the decisions an owner actually makes, while a full path produces a
  // histogram with one entry per file and no signal.
  return parts.length <= 2 ? path : `${parts[0]}/${parts[1]}`;
}

/**
 * Replay a commit range and score each candidate selector.
 *
 * `coverage` is used only to identify commits that touched nothing but reserved
 * prefixes — those can never admit a cycle under any candidate, and counting
 * them separately keeps the admission rates honest.
 */
export async function replay(
  repoRoot: string,
  repository: string,
  range: string,
  candidates: Candidate[],
  coverage: PathSelector
): Promise<ReplayReport> {
  const commits = await commitList(repoRoot, range);
  if (commits.length === 0) throw new Error(`no commits in range ${range}`);

  const matchers = candidates.map((c) => ({ name: c.name, match: productRelevantMatcher(c.selector) }));
  const inCoverage = coverageMatcher(coverage);
  const drivers = new Map<string, Map<string, number>>(
    candidates.map((c) => [c.name, new Map<string, number>()])
  );

  const timeline: CommitOutcome[] = [];
  let reservedOnlyCommits = 0;

  for (const commit of commits) {
    const paths = await changedPaths(repoRoot, commit.sha);
    const reservedOnly = paths.length > 0 && !paths.some((p) => inCoverage(p));
    if (reservedOnly) reservedOnlyCommits += 1;

    const admits: Record<string, boolean> = {};
    for (const { name, match } of matchers) {
      const hits = paths.filter(match);
      admits[name] = hits.length > 0;
      if (hits.length > 0) {
        const counted = new Set(hits.map(driverKey));
        const bucket = drivers.get(name)!;
        for (const key of counted) bucket.set(key, (bucket.get(key) ?? 0) + 1);
      }
    }
    timeline.push({
      sha: commit.sha,
      date: commit.date,
      subject: commit.subject,
      changedPaths: paths.length,
      admits,
      reservedOnly,
    });
  }

  const scores: CandidateScore[] = candidates.map((c) => {
    const admitted = timeline.filter((t) => t.admits[c.name]).length;
    return {
      name: c.name,
      admitted,
      coverageOnly: timeline.length - admitted,
      admissionRate: Number((admitted / timeline.length).toFixed(4)),
      topDrivers: [...drivers.get(c.name)!.entries()]
        .map(([path, count]) => ({ path, commits: count }))
        .sort((a, b) => b.commits - a.commits)
        .slice(0, 8),
    };
  });

  log.info('replay complete', {
    commits: timeline.length,
    candidates: candidates.length,
    reservedOnly: reservedOnlyCommits,
  });

  return {
    repository,
    fromCommit: commits[0].sha,
    toCommit: commits[commits.length - 1].sha,
    commits: timeline.length,
    reservedOnlyCommits,
    candidates: scores,
    timeline,
  };
}

/** Render a replay report as an operator-readable summary. */
export function formatReport(report: ReplayReport): string {
  const lines: string[] = [];
  lines.push(`repository        ${report.repository}`);
  lines.push(`range             ${report.fromCommit.slice(0, 12)} .. ${report.toCommit.slice(0, 12)}`);
  lines.push(`commits replayed  ${report.commits} (first-parent)`);
  lines.push(`reserved-only     ${report.reservedOnlyCommits} (never admissible under any candidate)`);
  lines.push('');
  lines.push('candidate                       admits  coverage-only   rate');
  lines.push('------------------------------  ------  -------------  -----');
  for (const c of report.candidates) {
    lines.push(
      `${c.name.padEnd(30)}  ${String(c.admitted).padStart(6)}  ${String(c.coverageOnly).padStart(13)}  ${(c.admissionRate * 100).toFixed(1).padStart(4)}%`
    );
  }
  for (const c of report.candidates) {
    lines.push('');
    lines.push(`drivers — ${c.name}`);
    for (const d of c.topDrivers) {
      lines.push(`  ${String(d.commits).padStart(5)}  ${d.path}`);
    }
  }
  return `${lines.join('\n')}\n`;
}
