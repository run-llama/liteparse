import { InvalidArgumentError } from "commander";

/** Largest value the native bindings accept for integer options (`u32`). */
const MAX_U32 = 4_294_967_295;

/**
 * Upper bound on the number of pages a `--target-pages` spec may expand to.
 * Mirrors `MAX_TARGET_PAGES` in `crates/liteparse/src/config.rs`, which guards
 * the Rust `parse_target_pages` against `1-4294967295`-style OOMs.
 */
const MAX_TARGET_PAGES = 100_000;

/**
 * Commander argument parser for options that take a positive integer
 * (`--max-pages`, `--num-workers`). Rejects anything `parseInt` would silently
 * coerce: `abc` (NaN), `0` / negatives, `1.5`, `3abc`, and values the native
 * bindings cannot represent.
 */
export function parsePositiveInt(value: string): number {
  const trimmed = value.trim();
  const n = Number(trimmed);
  if (!/^\d+$/.test(trimmed) || n < 1 || n > MAX_U32) {
    throw new InvalidArgumentError(`Expected a positive integer (1-${MAX_U32}), got '${value}'.`);
  }
  return n;
}

/**
 * Commander argument parser for options that take a positive, finite number
 * (`--dpi`). Fractional values are allowed, as before.
 */
export function parsePositiveNumber(value: string): number {
  const trimmed = value.trim();
  const n = Number(trimmed);
  if (trimmed === "" || !Number.isFinite(n) || n <= 0) {
    throw new InvalidArgumentError(`Expected a positive number, got '${value}'.`);
  }
  return n;
}

/** Parse one page number the way the Rust `parse::<u32>()` path does. */
function parsePageNumber(raw: string): number {
  const trimmed = raw.trim();
  const n = Number(trimmed);
  if (!/^\d+$/.test(trimmed) || n > MAX_U32) {
    throw new Error(`invalid page number: ${raw}`);
  }
  return n;
}

/**
 * Expand a `--target-pages` spec (e.g. `"1-5,10,15-20"`) into a sorted,
 * de-duplicated list of 1-based page numbers.
 *
 * This is the TypeScript counterpart of `parse_target_pages` in
 * `crates/liteparse/src/config.rs` and enforces the same rules and limit, so
 * `screenshot` rejects the same inputs `parse` does. Throws an `Error` with a
 * user-facing message on invalid input.
 */
export function parseTargetPages(spec: string): number[] {
  const pages: number[] = [];
  for (const rawPart of spec.split(",")) {
    const part = rawPart.trim();
    const dash = part.indexOf("-");
    if (dash !== -1) {
      const start = parsePageNumber(part.slice(0, dash));
      const end = parsePageNumber(part.slice(dash + 1));
      if (start > end) {
        throw new Error(`invalid range: ${start}-${end}`);
      }
      // Check the span before expanding so an oversized range can never
      // allocate a huge array.
      if (pages.length + (end - start + 1) > MAX_TARGET_PAGES) {
        throw new Error(
          `too many target pages: ${start}-${end} exceeds the limit of ${MAX_TARGET_PAGES}`
        );
      }
      for (let p = start; p <= end; p++) pages.push(p);
    } else {
      const page = parsePageNumber(part);
      if (pages.length + 1 > MAX_TARGET_PAGES) {
        throw new Error(`too many target pages: exceeds the limit of ${MAX_TARGET_PAGES}`);
      }
      pages.push(page);
    }
  }
  return [...new Set(pages)].sort((a, b) => a - b);
}
