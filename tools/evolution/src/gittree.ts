// Read a Git tree at an exact revision, for fingerprinting.
//
// Section 17.2 rule 1 requires the Git tree at the exact revision rather than
// filesystem state: a dirty working tree, a stale editor buffer, or a checkout
// with different line endings would otherwise change a fingerprint that is
// supposed to describe a commit.
//
// Blob contents are read through ONE `git cat-file --batch` process rather than
// one process per file. On this repository that is ~550 blobs; a process per
// blob costs seconds per fingerprint and turns the verifier into something
// nobody runs.

import { spawn } from 'node:child_process';
import { log } from './log.ts';
import type { FileEntry } from './hash.ts';

/** A tree entry before its content has been read. */
interface TreeRef {
  mode: string;
  type: string;
  oid: string;
  path: string;
}

function run(cmd: string, args: string[], cwd: string, stdin?: Buffer): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { cwd, stdio: ['pipe', 'pipe', 'pipe'] });
    const out: Buffer[] = [];
    const err: Buffer[] = [];
    child.stdout.on('data', (c: Buffer) => out.push(c));
    child.stderr.on('data', (c: Buffer) => err.push(c));
    child.on('error', reject);
    child.on('close', (code) => {
      if (code === 0) return resolve(Buffer.concat(out));
      const message = Buffer.concat(err).toString('utf8').trim();
      reject(new Error(`${cmd} ${args.join(' ')} exited ${code}: ${message}`));
    });
    if (stdin) child.stdin.end(stdin);
    else child.stdin.end();
  });
}

/** Resolve a revision (branch, tag, `HEAD`) to a full commit SHA. */
export async function resolveRevision(repoRoot: string, revision: string): Promise<string> {
  const out = await run('git', ['rev-parse', `${revision}^{commit}`], repoRoot);
  return out.toString('utf8').trim();
}

/** List every entry of a tree, recursively, without reading blob contents. */
export async function listTree(repoRoot: string, revision: string): Promise<TreeRef[]> {
  const out = await run('git', ['ls-tree', '-r', '-z', revision], repoRoot);
  const refs: TreeRef[] = [];
  for (const record of out.toString('utf8').split('\0')) {
    if (!record) continue;
    // `<mode> SP <type> SP <oid> TAB <path>`
    const tab = record.indexOf('\t');
    if (tab < 0) throw new Error(`unparseable ls-tree record: ${JSON.stringify(record)}`);
    const [mode, type, oid] = record.slice(0, tab).split(' ');
    refs.push({ mode, type, oid, path: record.slice(tab + 1) });
  }
  log.debug('listed git tree', { revision, entries: refs.length });
  return refs;
}

/**
 * Read blob contents for the given refs through one batched `git cat-file`.
 *
 * A gitlink (mode 160000, type `commit`) is NOT read: the submodule's objects
 * live in another repository and are not fetchable here. Section 17.3 defines
 * submodule identity as "the recorded submodule commit", so the commit id itself
 * is the content — which is also what makes the fingerprint independent of
 * mutable remote content.
 */
async function readBlobs(repoRoot: string, refs: TreeRef[]): Promise<Map<string, Buffer>> {
  const blobRefs = refs.filter((r) => r.type === 'blob');
  const contents = new Map<string, Buffer>();
  if (blobRefs.length === 0) return contents;

  // De-duplicate: identical files share one oid, and asking twice wastes I/O.
  const oids = [...new Set(blobRefs.map((r) => r.oid))];
  const stdin = Buffer.from(oids.map((o) => `${o}\n`).join(''), 'ascii');
  const raw = await run('git', ['cat-file', '--batch'], repoRoot, stdin);

  let offset = 0;
  for (let i = 0; i < oids.length; i += 1) {
    const newline = raw.indexOf(0x0a, offset);
    if (newline < 0) throw new Error(`cat-file output truncated before header ${i}`);
    const header = raw.subarray(offset, newline).toString('ascii');
    const parts = header.split(' ');
    if (parts.length !== 3) {
      throw new Error(`unexpected cat-file header: ${JSON.stringify(header)}`);
    }
    const [oid, , sizeText] = parts;
    const size = Number(sizeText);
    if (!Number.isSafeInteger(size) || size < 0) {
      throw new Error(`unexpected cat-file size for ${oid}: ${sizeText}`);
    }
    const start = newline + 1;
    const end = start + size;
    if (end > raw.length) throw new Error(`cat-file output truncated in body of ${oid}`);
    contents.set(oid, raw.subarray(start, end));
    offset = end + 1; // trailing newline after each object body
  }
  log.debug('read blobs', { unique: contents.size, referenced: blobRefs.length });
  return contents;
}

/**
 * Read a full tree as fingerprintable entries.
 *
 * `paths` restricts the read to the entries actually being fingerprinted. It
 * matters for more than speed: reading every blob of the repository to hash a
 * dozen of them would make the tool's cost scale with the repository rather than
 * with the selector.
 */
export async function readTree(
  repoRoot: string,
  revision: string,
  keep?: (path: string) => boolean
): Promise<FileEntry[]> {
  const refs = await listTree(repoRoot, revision);
  const selected = keep ? refs.filter((r) => keep(r.path)) : refs;
  const contents = await readBlobs(repoRoot, selected);

  return selected.map((ref) => {
    if (ref.type === 'commit') {
      return { path: ref.path, mode: ref.mode, content: Buffer.from(ref.oid, 'ascii') };
    }
    const content = contents.get(ref.oid);
    if (!content) throw new Error(`missing blob content for ${ref.path} (${ref.oid})`);
    return { path: ref.path, mode: ref.mode, content };
  });
}

/**
 * The most recent commit, at or before `revision`, that touched `path`.
 *
 * This is what "resolved commit" means for a package reference (section 28.4).
 * The distinction is invisible when packages live in their own repository —
 * there, the branch head IS the package's commit — but it is load-bearing when a
 * package lives inside the source repository, as this proof's generator does.
 * Substituting the branch head there would move `generatorPinnedFingerprint` on
 * EVERY commit, including the commit that writes the manifest, so the input
 * fingerprint could never match the one just recorded and the post-merge no-op
 * that the whole self-trigger design rests on could never hold.
 */
export async function lastCommitTouching(
  repoRoot: string,
  revision: string,
  path: string
): Promise<string> {
  const out = await run('git', ['log', '-1', '--format=%H', revision, '--', path], repoRoot);
  const sha = out.toString('utf8').trim();
  if (!/^[0-9a-f]{40}$/.test(sha)) {
    throw new Error(`no commit found for ${path} at ${revision}`);
  }
  return sha;
}

/** Paths changed between two revisions — the section 17.7 condition 5 `treeDiff`. */
export async function changedPaths(
  repoRoot: string,
  base: string,
  head: string
): Promise<string[]> {
  const out = await run('git', ['diff', '--name-only', '-z', `${base}..${head}`], repoRoot);
  return out.toString('utf8').split('\0').filter(Boolean);
}
