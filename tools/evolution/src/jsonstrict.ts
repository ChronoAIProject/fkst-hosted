// Duplicate-key detection for manifest JSON (spec section 17.6).
//
// Section 17.6: "Implementations MUST reject a manifest containing duplicate
// keys rather than canonicalizing it."
//
// WHY this cannot use JSON.parse: the ECMAScript grammar resolves duplicate keys
// by last-wins, silently, before any reviver or replacer runs. By the time a
// parsed value exists the evidence is gone. A manifest carrying
// `"outputFingerprint": "<real>"` followed by `"outputFingerprint": "<forged>"`
// would parse to the forged value while a careless reader saw the real one — and
// since the manifest projection enters the output fingerprint, that is exactly
// the ambiguity an attacker would reach for.
//
// This is a VALIDATION scan, not a parser: it lexes strings (so braces and
// colons inside them are inert) and tracks object depth, then hands the text to
// JSON.parse for the actual value. It deliberately does not build a tree.

/** Thrown when a JSON document contains a duplicate key within one object. */
export class DuplicateKeyError extends Error {}

const WHITESPACE = new Set([0x20, 0x09, 0x0a, 0x0d]);

/**
 * Scan `text` for duplicate keys in any object, throwing `DuplicateKeyError`.
 *
 * Assumes `text` is otherwise well-formed JSON — callers run `JSON.parse` too,
 * which is what reports ordinary syntax errors.
 */
export function assertNoDuplicateKeys(text: string): void {
  // One key set per open object; arrays push `null` so depth stays aligned.
  const stack: (Set<string> | null)[] = [];
  let i = 0;
  // True when the next string literal is a key rather than a value: set on `{`
  // and on `,` inside an object, cleared by the `:` that follows the key.
  let expectKey = false;

  const readString = (): string => {
    // `text[i]` is the opening quote.
    let out = '';
    i += 1;
    while (i < text.length) {
      const ch = text[i];
      if (ch === '\\') {
        // Escapes are copied verbatim: two keys are duplicates only if they
        // denote the same string, and JSON.parse settles that. Preserving the
        // raw form here would make "A" and "A" look distinct, so decode.
        const next = text[i + 1];
        const simple: Record<string, string> = {
          '"': '"', '\\': '\\', '/': '/', b: '\b', f: '\f', n: '\n', r: '\r', t: '\t',
        };
        if (next === 'u') {
          out += String.fromCharCode(parseInt(text.slice(i + 2, i + 6), 16));
          i += 6;
        } else {
          out += simple[next] ?? next;
          i += 2;
        }
        continue;
      }
      if (ch === '"') {
        i += 1;
        return out;
      }
      out += ch;
      i += 1;
    }
    throw new DuplicateKeyError('unterminated string in manifest JSON');
  };

  while (i < text.length) {
    const code = text.charCodeAt(i);
    if (WHITESPACE.has(code)) {
      i += 1;
      continue;
    }
    const ch = text[i];
    if (ch === '"') {
      const value = readString();
      if (expectKey) {
        const keys = stack[stack.length - 1];
        if (keys) {
          if (keys.has(value)) {
            throw new DuplicateKeyError(`duplicate key ${JSON.stringify(value)} in manifest JSON`);
          }
          keys.add(value);
        }
        expectKey = false;
      }
      continue;
    }
    if (ch === '{') {
      stack.push(new Set());
      expectKey = true;
    } else if (ch === '[') {
      stack.push(null);
      expectKey = false;
    } else if (ch === '}' || ch === ']') {
      stack.pop();
      expectKey = false;
    } else if (ch === ',') {
      expectKey = stack[stack.length - 1] !== null && stack.length > 0;
    } else if (ch === ':') {
      expectKey = false;
    }
    i += 1;
  }
}

/** `JSON.parse` with the section 17.6 duplicate-key rejection applied first. */
export function parseStrictJson<T = unknown>(text: string): T {
  assertNoDuplicateKeys(text);
  return JSON.parse(text) as T;
}
