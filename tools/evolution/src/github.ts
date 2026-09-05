// A `GitHubPort` backed by the `gh` CLI.
//
// WHY `gh` and not a raw HTTP client with a token: CLAUDE.md mandates that all
// GitHub operations go through `git` and the `gh` CLI, and `gh` already holds
// the operator's authenticated identity. Nothing in this module ever reads,
// stores, or logs a token.
//
// This is the local-operator implementation. The control plane will supply its
// own `GitHubPort` driven by an installation token with the section 25.5
// permission split; the convergence logic is unchanged by that swap, which is
// the point of the interface.

import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { log } from './log.ts';
import type { GitHubPort } from './converge.ts';

const execFileAsync = promisify(execFile);

/** Appendix A.3 sync-PR marker. */
const PR_MARKER = /<!--\s*fkst-evolution-pr:v1\s*(\{[\s\S]*?\})\s*-->/;

async function gh(args: string[], encoding: 'utf8' | 'buffer' = 'utf8'): Promise<string | Buffer> {
  log.debug('gh', { args: args.join(' ') });
  const { stdout } = await execFileAsync('gh', args, {
    encoding: encoding === 'buffer' ? 'buffer' : 'utf8',
    // Release assets and check-run payloads can be large; the default 1 MiB
    // buffer would truncate an MP4 into a wrong-but-plausible hash.
    maxBuffer: 256 * 1024 * 1024,
  });
  return stdout as string | Buffer;
}

export class GhCliGitHubPort implements GitHubPort {
  async fetchReleaseAsset(repository: string, tag: string, asset: string): Promise<Buffer | null> {
    let apiUrl: string;
    try {
      const out = (await gh([
        'release', 'view', tag, '--repo', repository, '--json', 'assets',
        '--jq', `.assets[] | select(.name == "${asset}") | .apiUrl`,
      ])) as string;
      apiUrl = out.trim();
    } catch (error) {
      log.warn('release not retrievable', { repository, tag, error: String(error) });
      return null;
    }
    if (!apiUrl) {
      log.warn('release asset not found', { repository, tag, asset });
      return null;
    }
    const bytes = (await gh(
      ['api', apiUrl, '-H', 'Accept: application/octet-stream'],
      'buffer'
    )) as Buffer;
    return bytes;
  }

  async getCheckRun(
    repository: string,
    id: number
  ): Promise<{ appId: number | null; conclusion: string | null; outputText: string } | null> {
    try {
      const out = (await gh(['api', `repos/${repository}/check-runs/${id}`])) as string;
      const run = JSON.parse(out) as {
        app?: { id?: number };
        conclusion?: string | null;
        output?: { title?: string; summary?: string; text?: string };
      };
      const output = run.output ?? {};
      return {
        appId: run.app?.id ?? null,
        conclusion: run.conclusion ?? null,
        // All three output fields are concatenated because section 17.7.1 only
        // requires that the run "records an input fingerprint", without pinning
        // which field carries it.
        outputText: [output.title, output.summary, output.text].filter(Boolean).join('\n'),
      };
    } catch (error) {
      log.warn('check run not retrievable', { repository, id, error: String(error) });
      return null;
    }
  }

  async openSyncPullRequestInputs(repository: string): Promise<string[]> {
    const out = (await gh([
      'pr', 'list', '--repo', repository, '--state', 'open', '--limit', '100', '--json', 'body,number',
    ])) as string;
    const prs = JSON.parse(out) as { body: string | null; number: number }[];
    const inputs: string[] = [];
    for (const pr of prs) {
      const match = PR_MARKER.exec(pr.body ?? '');
      if (!match) continue;
      try {
        const marker = JSON.parse(match[1]) as { input?: string };
        // Appendix A: marker text alone never grants authority. It is used here
        // only to answer "does an open sync PR describe a different input",
        // which is a comparison, not a grant.
        if (typeof marker.input === 'string') inputs.push(marker.input);
      } catch {
        log.warn('sync PR carries an unparseable marker', { repository, pr: pr.number });
      }
    }
    return inputs;
  }

  async configuredAppId(): Promise<number | null> {
    const raw = process.env.FKST_EVOLUTION_APP_ID;
    if (!raw) {
      // Not fatal: condition 4 treats a missing App id as "cannot verify the
      // actor" and still checks existence, conclusion and fingerprint. Silently
      // accepting any actor would be the unsafe reading, so this is logged.
      log.warn('FKST_EVOLUTION_APP_ID unset — check-run actor cannot be verified');
      return null;
    }
    const id = Number(raw);
    if (!Number.isSafeInteger(id)) throw new Error('FKST_EVOLUTION_APP_ID must be an integer');
    return id;
  }
}
