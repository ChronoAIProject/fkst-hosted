import { BrowserSmokeWorkerError } from "./worker-error.js";

export interface ParsedJson {
  readonly value: JsonValue;
  readonly raw: string;
}

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly ParsedJson[]
  | ReadonlyMap<string, ParsedJson>;

export function parseStrictJson(source: string): ParsedJson {
  return new StrictJsonParser(source).parse();
}

class StrictJsonParser {
  private offset = 0;

  constructor(private readonly source: string) {}

  parse(): ParsedJson {
    this.skipWhitespace();
    const parsed = this.parseValue();
    this.skipWhitespace();
    if (this.offset !== this.source.length) {
      throw new BrowserSmokeWorkerError("request.trailing_data");
    }
    return parsed;
  }

  private parseValue(): ParsedJson {
    const start = this.offset;
    const character = this.source[this.offset];
    let value: JsonValue;

    if (character === "{") {
      value = this.parseObject();
    } else if (character === "[") {
      value = this.parseArray();
    } else if (character === '"') {
      value = this.parseString();
    } else if (character === "t") {
      this.consumeLiteral("true");
      value = true;
    } else if (character === "f") {
      this.consumeLiteral("false");
      value = false;
    } else if (character === "n") {
      this.consumeLiteral("null");
      value = null;
    } else if (character === "-" || isDigit(character)) {
      value = this.parseNumber();
    } else {
      throw new BrowserSmokeWorkerError("request.invalid_json");
    }

    return { value, raw: this.source.slice(start, this.offset) };
  }

  private parseObject(): ReadonlyMap<string, ParsedJson> {
    const members = new Map<string, ParsedJson>();
    this.offset += 1;
    this.skipWhitespace();
    if (this.source[this.offset] === "}") {
      this.offset += 1;
      return members;
    }

    while (this.offset < this.source.length) {
      if (this.source[this.offset] !== '"') {
        throw new BrowserSmokeWorkerError("request.invalid_json");
      }
      const key = this.parseString();
      if (members.has(key)) {
        throw new BrowserSmokeWorkerError("request.duplicate_key");
      }
      this.skipWhitespace();
      if (this.source[this.offset] !== ":") {
        throw new BrowserSmokeWorkerError("request.invalid_json");
      }
      this.offset += 1;
      this.skipWhitespace();
      members.set(key, this.parseValue());
      this.skipWhitespace();
      const delimiter = this.source[this.offset];
      if (delimiter === "}") {
        this.offset += 1;
        return members;
      }
      if (delimiter !== ",") {
        throw new BrowserSmokeWorkerError("request.invalid_json");
      }
      this.offset += 1;
      this.skipWhitespace();
    }

    throw new BrowserSmokeWorkerError("request.invalid_json");
  }

  private parseArray(): readonly ParsedJson[] {
    const values: ParsedJson[] = [];
    this.offset += 1;
    this.skipWhitespace();
    if (this.source[this.offset] === "]") {
      this.offset += 1;
      return values;
    }

    while (this.offset < this.source.length) {
      values.push(this.parseValue());
      this.skipWhitespace();
      const delimiter = this.source[this.offset];
      if (delimiter === "]") {
        this.offset += 1;
        return values;
      }
      if (delimiter !== ",") {
        throw new BrowserSmokeWorkerError("request.invalid_json");
      }
      this.offset += 1;
      this.skipWhitespace();
    }

    throw new BrowserSmokeWorkerError("request.invalid_json");
  }

  private parseString(): string {
    const start = this.offset;
    this.offset += 1;
    let escaped = false;

    while (this.offset < this.source.length) {
      const character = this.source[this.offset];
      this.offset += 1;
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        try {
          return JSON.parse(this.source.slice(start, this.offset)) as string;
        } catch {
          throw new BrowserSmokeWorkerError("request.invalid_json");
        }
      }
    }

    throw new BrowserSmokeWorkerError("request.invalid_json");
  }

  private parseNumber(): number {
    const match = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(
      this.source.slice(this.offset),
    );
    if (match === null) {
      throw new BrowserSmokeWorkerError("request.invalid_json");
    }
    this.offset += match[0].length;
    const value = Number(match[0]);
    if (!Number.isFinite(value)) {
      throw new BrowserSmokeWorkerError("request.invalid_json");
    }
    return value;
  }

  private consumeLiteral(literal: string): void {
    if (!this.source.startsWith(literal, this.offset)) {
      throw new BrowserSmokeWorkerError("request.invalid_json");
    }
    this.offset += literal.length;
  }

  private skipWhitespace(): void {
    while (/^[\u0009\u000a\u000d\u0020]$/.test(this.source[this.offset] ?? "")) {
      this.offset += 1;
    }
  }
}

function isDigit(character: string | undefined): boolean {
  return character !== undefined && character >= "0" && character <= "9";
}
