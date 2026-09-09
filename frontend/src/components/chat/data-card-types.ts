/**
 * The wire `DataCard` union — a structured rendering of one tool result.
 *
 * These arrive DURING a turn, projected by the backend from the tool's own response.
 * The model chose which tool to call; it did not author what the card says. That is the
 * whole point: a data-heavy answer (install commands, pull requests, log files) is
 * rendered from the data instead of described in generated prose, so what the user reads
 * is verifiable.
 *
 * Parsing is strict and total: an unrecognized or malformed card is dropped rather than
 * rendered half-empty, because a card with blank fields reads as "there is nothing there"
 * when the truth is "this build does not understand that payload".
 */

export interface CardVariable {
  key: string;
  value: string;
}

export interface EnvironmentSummaryCard {
  name: string;
  status: string;
  validated_at: string;
  install_command_count: number;
  variable_count: number;
  secret_count: number;
}

export interface PullRequestCard {
  number: number;
  title: string;
  html_url: string;
  state: string;
  merged: boolean;
  work_issue: number | null;
  files_changed: number;
}

export interface LogRunCard {
  run_id: string;
  started_at: string;
  ended_at?: string | null;
}

export interface LogFileCard {
  path: string;
  size_bytes: number;
}

export type DataCard =
  | { kind: 'environments'; profiles: EnvironmentSummaryCard[]; omitted: number }
  | {
      kind: 'environment_detail';
      name: string;
      status: string;
      validated_at: string;
      install: string[];
      variables: CardVariable[];
      /** Secret NAMES only — the endpoint never returns a value, and this union has
       *  no field that could hold one. */
      secret_keys: string[];
    }
  | {
      kind: 'outcomes';
      owner: string;
      name: string;
      trigger_issue: number;
      pull_requests: PullRequestCard[];
      merged: number;
      omitted: number;
    }
  | { kind: 'log_runs'; session_id: string; runs: LogRunCard[]; omitted: number }
  | {
      kind: 'log_manifest';
      session_id: string;
      run?: string | null;
      files: LogFileCard[];
      omitted: number;
    };

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value != null;

const isString = (value: unknown): value is string => typeof value === 'string';
const isNumber = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);
const isStringArray = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every(isString);

/** A count field. Absent or malformed counts as zero rather than failing the card:
 *  the rows are the payload, and a missing "omitted" only costs a footnote. */
const count = (value: unknown): number => (isNumber(value) && value >= 0 ? value : 0);

function parseRows<T>(value: unknown, row: (entry: Record<string, unknown>) => T | null): T[] | null {
  if (!Array.isArray(value)) return null;
  const rows: T[] = [];
  for (const entry of value) {
    if (!isRecord(entry)) return null;
    const parsed = row(entry);
    if (parsed == null) return null;
    rows.push(parsed);
  }
  return rows;
}

/**
 * Structurally validate a card from the stream.
 *
 * Returns `null` for anything malformed OR for an unrecognized `kind`, so a newer
 * server adding a card kind degrades to "no card" on an older client rather than
 * rendering something it cannot describe.
 */
export function parseDataCard(value: unknown): DataCard | null {
  if (!isRecord(value)) return null;

  switch (value.kind) {
    case 'environments': {
      const profiles = parseRows(value.profiles, (entry) =>
        isString(entry.name)
          ? {
              name: entry.name,
              status: isString(entry.status) ? entry.status : '',
              validated_at: isString(entry.validated_at) ? entry.validated_at : '',
              install_command_count: count(entry.install_command_count),
              variable_count: count(entry.variable_count),
              secret_count: count(entry.secret_count),
            }
          : null
      );
      return profiles == null
        ? null
        : { kind: 'environments', profiles, omitted: count(value.omitted) };
    }

    case 'environment_detail': {
      if (!isString(value.name)) return null;
      if (!isStringArray(value.install) || !isStringArray(value.secret_keys)) return null;
      const variables = parseRows(value.variables, (entry) =>
        isString(entry.key) && isString(entry.value)
          ? { key: entry.key, value: entry.value }
          : null
      );
      if (variables == null) return null;
      return {
        kind: 'environment_detail',
        name: value.name,
        status: isString(value.status) ? value.status : '',
        validated_at: isString(value.validated_at) ? value.validated_at : '',
        install: value.install,
        variables,
        secret_keys: value.secret_keys,
      };
    }

    case 'outcomes': {
      if (!isString(value.owner) || !isString(value.name) || !isNumber(value.trigger_issue)) {
        return null;
      }
      const pull_requests = parseRows(value.pull_requests, (entry) =>
        isNumber(entry.number) && isString(entry.title)
          ? {
              number: entry.number,
              title: entry.title,
              html_url: isString(entry.html_url) ? entry.html_url : '',
              state: isString(entry.state) ? entry.state : '',
              merged: entry.merged === true,
              work_issue: isNumber(entry.work_issue) ? entry.work_issue : null,
              files_changed: count(entry.files_changed),
            }
          : null
      );
      return pull_requests == null
        ? null
        : {
            kind: 'outcomes',
            owner: value.owner,
            name: value.name,
            trigger_issue: value.trigger_issue,
            pull_requests,
            merged: count(value.merged),
            omitted: count(value.omitted),
          };
    }

    case 'log_runs': {
      if (!isString(value.session_id)) return null;
      const runs = parseRows(value.runs, (entry) =>
        isString(entry.run_id)
          ? {
              run_id: entry.run_id,
              started_at: isString(entry.started_at) ? entry.started_at : '',
              // A live run has no end time; null must survive so the card can say
              // "running" rather than showing a blank column.
              ended_at: isString(entry.ended_at) ? entry.ended_at : null,
            }
          : null
      );
      return runs == null
        ? null
        : { kind: 'log_runs', session_id: value.session_id, runs, omitted: count(value.omitted) };
    }

    case 'log_manifest': {
      if (!isString(value.session_id)) return null;
      const files = parseRows(value.files, (entry) =>
        isString(entry.path) ? { path: entry.path, size_bytes: count(entry.size_bytes) } : null
      );
      return files == null
        ? null
        : {
            kind: 'log_manifest',
            session_id: value.session_id,
            run: isString(value.run) ? value.run : null,
            files,
            omitted: count(value.omitted),
          };
    }

    default:
      return null;
  }
}
