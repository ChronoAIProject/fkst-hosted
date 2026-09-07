// Canonical hashing primitives for FKST Evolution fingerprints (spec section 17.2).
//
// The spec fixes the SHAPE of a leaf:
//
//     leaf = SHA256(path_length || path || mode || content_length || content_bytes)
//
// but calls it "conceptual" and defers the exact serialization to
// implementations, requiring only that it be documented and covered by test
// vectors (sections 17.2, 17.5). This module IS that documentation. Every
// encoding decision below is load-bearing: two implementations that disagree on
// a single byte produce different fingerprints for the same tree, which the
// control plane reads as permanent non-convergence.
//
// The concrete encoding:
//
//   * lengths are unsigned 64-bit BIG-ENDIAN — fixed width, so a length can
//     never be confused with the field it delimits, and endianness is pinned
//     rather than inherited from the host;
//   * paths are slash-separated repository-relative UTF-8, hashed as raw UTF-8
//     bytes with no normalization (NFC vs NFD is a real difference in a Git
//     tree, so imposing one here would hide it);
//   * mode is the 6-byte zero-padded ASCII octal Git mode ("100644", "100755",
//     "120000", "160000") — fixed width, so it needs no length prefix;
//   * content is raw blob bytes, no line-ending conversion.
//
// WHY domain separation: three different hash families live in one fingerprint
// graph (file leaves, named scalar leaves, and trees). Without a distinct domain
// prefix per family, a crafted file path/content pair could produce the same
// digest as a named generator leaf, letting repository content forge a
// generator input. Each family therefore begins with its own ASCII tag.

import { createHash } from 'node:crypto';

/** Domain tag for section 17.2 file leaves. */
export const DOMAIN_FILE_LEAF = 'fkst-evolution-file-leaf-v1';
/** Domain tag for named scalar leaves (section 17.4's `leaf("name", value)`). */
export const DOMAIN_NAMED_LEAF = 'fkst-evolution-leaf-v1';
/** Domain tag for an ordered set of leaves hashed into a tree fingerprint. */
export const DOMAIN_TREE = 'fkst-evolution-tree-v1';

/** A single fingerprinted file, as read from a Git tree. */
export interface FileEntry {
  /** Slash-separated repository-relative path. */
  path: string;
  /** 6-digit octal Git mode, e.g. `100644`. */
  mode: string;
  /** Raw blob bytes. For a gitlink (mode 160000) this is the ASCII commit id. */
  content: Buffer;
}

/** Unsigned 64-bit big-endian length prefix. */
function u64be(value: number): Buffer {
  const buf = Buffer.allocUnsafe(8);
  buf.writeBigUInt64BE(BigInt(value));
  return buf;
}

function sha256(parts: Buffer[]): Buffer {
  const h = createHash('sha256');
  for (const part of parts) h.update(part);
  return h.digest();
}

/** Render a digest in the `sha256:<hex>` form the manifest uses. */
export function formatDigest(digest: Buffer): string {
  return `sha256:${digest.toString('hex')}`;
}

/** Normalize a Git mode to the 6-byte zero-padded octal form. */
export function normalizeMode(mode: string): string {
  const trimmed = mode.trim();
  if (!/^[0-7]{5,6}$/.test(trimmed)) {
    throw new Error(`invalid git mode: ${JSON.stringify(mode)}`);
  }
  return trimmed.padStart(6, '0');
}

/**
 * Section 17.2 file leaf.
 *
 * The mode is NOT length-prefixed because `normalizeMode` guarantees exactly six
 * bytes; a fixed-width field is already unambiguous, and adding a prefix would
 * only invite disagreement about whether it is there.
 */
export function fileLeaf(entry: FileEntry): Buffer {
  const path = Buffer.from(entry.path, 'utf8');
  const mode = Buffer.from(normalizeMode(entry.mode), 'ascii');
  return sha256([
    Buffer.from(DOMAIN_FILE_LEAF, 'ascii'),
    u64be(path.length),
    path,
    mode,
    u64be(entry.content.length),
    entry.content,
  ]);
}

/**
 * A named scalar leaf, used by section 17.4's `leaf("manifestRef", …)` notation.
 * Both name and value are length-delimited so `("ab", "c")` and `("a", "bc")`
 * cannot collide.
 */
export function namedLeaf(name: string, value: string | Buffer): Buffer {
  const nameBytes = Buffer.from(name, 'utf8');
  const valueBytes = typeof value === 'string' ? Buffer.from(value, 'utf8') : value;
  return sha256([
    Buffer.from(DOMAIN_NAMED_LEAF, 'ascii'),
    u64be(nameBytes.length),
    nameBytes,
    u64be(valueBytes.length),
    valueBytes,
  ]);
}

/**
 * Hash an ordered list of 32-byte leaf digests into one fingerprint.
 *
 * The entry count is hashed before the leaves so that concatenating two trees
 * cannot equal a third: length-delimiting the SET, not just each element.
 */
export function hashLeaves(leaves: Buffer[]): Buffer {
  const parts: Buffer[] = [Buffer.from(DOMAIN_TREE, 'ascii'), u64be(leaves.length)];
  for (const leaf of leaves) {
    if (leaf.length !== 32) throw new Error(`leaf digest must be 32 bytes, got ${leaf.length}`);
    parts.push(u64be(leaf.length), leaf);
  }
  return sha256(parts);
}

/**
 * Section 17.2 tree fingerprint: leaves sorted by path BYTE order, then hashed.
 *
 * Byte order — not locale collation and not UTF-16 code-unit order, which is
 * what a bare `Array.prototype.sort()` on JS strings would give. Those differ
 * for any path containing a character outside the BMP, so the comparison is done
 * on the UTF-8 buffers.
 */
export function treeFingerprint(entries: FileEntry[]): Buffer {
  const sorted = [...entries].sort((a, b) =>
    Buffer.compare(Buffer.from(a.path, 'utf8'), Buffer.from(b.path, 'utf8'))
  );
  for (let i = 1; i < sorted.length; i += 1) {
    if (sorted[i].path === sorted[i - 1].path) {
      throw new Error(`duplicate path in fingerprint input: ${sorted[i].path}`);
    }
  }
  return hashLeaves(sorted.map(fileLeaf));
}

/** SHA-256 of raw bytes — the `contentHash` of an artifact or Release asset. */
export function contentHash(bytes: Buffer): string {
  return formatDigest(createHash('sha256').update(bytes).digest());
}

/**
 * Hash an ASCII domain tag followed by fixed-width digests, concatenated.
 *
 * This is the construction sections 17.4, 17.5 and 17.6 write literally, e.g.
 *
 *     inputFingerprint = SHA256("fkst-evolution-input-v2" ||
 *                               productRelevantFingerprint ||
 *                               generatorPinnedFingerprint)
 *
 * No length prefixes are added. That is deliberate rather than an oversight:
 * every operand is a 32-byte digest, so the concatenation is already
 * unambiguous, and a second implementation reading the spec text would produce
 * exactly these bytes. Adding a prefix here would be silently incompatible with
 * the document while looking more rigorous.
 */
export function domainConcat(domain: string, digests: Buffer[]): Buffer {
  for (const d of digests) {
    if (d.length !== 32) throw new Error(`operand must be a 32-byte digest, got ${d.length}`);
  }
  return sha256([Buffer.from(domain, 'ascii'), ...digests]);
}
