#!/usr/bin/env node

import { program } from "commander";
import { LiteParse, type LiteParseConfig } from "./lib.js";
import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { join, relative, parse as parsePath } from "node:path";

program
  .name("liteparse")
  .description("Fast, lightweight PDF and document parsing")
  .version("2.0.0");

/**
 * Resolve a CLI `<file>` argument into a parser input. `-` means read the
 * document from stdin (e.g. `curl -sL … | liteparse parse -`); anything else is
 * passed through as a path. Streaming stdin (rather than `readFileSync(0)`)
 * avoids EAGAIN on non-blocking pipes.
 */
async function resolveInput(file: string): Promise<string | Buffer> {
  if (file !== "-") return file;
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk as Buffer);
  }
  const bytes = Buffer.concat(chunks);
  if (bytes.length === 0) {
    throw new Error(
      "no data on stdin (input `-` expects a document piped in, e.g. `curl … | liteparse parse -`)",
    );
  }
  return bytes;
}

/** Collect repeated `--ocr-server-header "Name: Value"` flags into an object. */
function collectHeader(
  value: string,
  previous: Record<string, string> = {},
): Record<string, string> {
  const idx = value.indexOf(":");
  if (idx === -1) {
    throw new Error(`invalid header '${value}', expected 'Name: Value'`);
  }
  const name = value.slice(0, idx).trim();
  if (name === "") {
    throw new Error(`invalid header '${value}', empty header name`);
  }
  previous[name] = value.slice(idx + 1).trim();
  return previous;
}

program
  .command("parse")
  .description("Parse a document and extract text")
  .argument("<file>", "Path to the document file")
  .option("-o, --output <file>", "Output file path")
  .option("--format <format>", 'Output format: json|text|markdown (default: "text")')
  .option(
    "--image-mode <mode>",
    "How to surface raster images in markdown: off|placeholder|embed (default: placeholder)",
  )
  .option(
    "--image-output-dir <dir>",
    "Directory to write embedded images to when --image-mode embed is set",
  )
  .option("--no-links", "Disable hyperlink extraction (emit plain anchor text)")
  .option("--extract-annotations", "Include all PDF annotations in page output")
  .option("--ocr-server-url <url>", "HTTP OCR server URL")
  .option(
    "--ocr-server-header <header>",
    'Extra header for OCR server requests, "Name: Value" (repeatable)',
    collectHeader,
  )
  .option("--no-ocr", "Disable OCR")
  .option("--ocr-language <lang>", "OCR language (default: eng)")
  .option("--max-pages <n>", "Max pages to parse", parseInt)
  .option(
    "--target-pages <pages>",
    'Pages to parse (e.g., "1-5,10,15-20")',
  )
  .option("--dpi <dpi>", "Rendering DPI", parseFloat)
  .option("--preserve-small-text", "Keep very small text")
  .option("--password <password>", "Password for encrypted documents")
  .option("--config <file>", "JSON config file path")
  .option("-q, --quiet", "Suppress progress output")
  .option("--num-workers <n>", "Number of concurrent OCR workers", parseInt)
  .option(
    "--complexity",
    "Include per-page complexity signals in JSON output",
  )
  .action(async (file: string, opts: Record<string, unknown>) => {
    try {
      const config: Partial<LiteParseConfig> = {};

      // Load config file if provided
      if (opts.config) {
        const fileConfig = JSON.parse(
          readFileSync(opts.config as string, "utf-8"),
        );
        Object.assign(config, fileConfig);
      }

      // CLI options override config file
      if (opts.format) config.outputFormat = opts.format as "json" | "text" | "markdown";
      if (opts.imageMode)
        config.imageMode = opts.imageMode as "off" | "placeholder" | "embed";
      if (opts.links === false) config.extractLinks = false;
      if (opts.extractAnnotations) config.extractAnnotations = true;
      if (opts.ocrServerUrl)
        config.ocrServerUrl = opts.ocrServerUrl as string;
      if (opts.ocrServerHeader)
        config.ocrServerHeaders = opts.ocrServerHeader as Record<string, string>;
      if (opts.ocr === false) config.ocrEnabled = false;
      if (opts.ocrLanguage) config.ocrLanguage = opts.ocrLanguage as string;
      if (opts.maxPages) config.maxPages = opts.maxPages as number;
      if (opts.targetPages) config.targetPages = opts.targetPages as string;
      if (opts.dpi) config.dpi = opts.dpi as number;
      if (opts.preserveSmallText) config.preserveVerySmallText = true;
      if (opts.password) config.password = opts.password as string;
      if (opts.quiet) config.quiet = true;
      if (opts.numWorkers) config.numWorkers = opts.numWorkers as number;
      if (opts.complexity) config.includeComplexity = true;

      // Default CLI output to text (library defaults to json)
      if (!config.outputFormat) config.outputFormat = "text";

      const parser = new LiteParse(config);
      const result = await parser.parse(await resolveInput(file));

      const output =
        config.outputFormat === "json"
          ? JSON.stringify(
              {
                pages: result.pages.map((p) => ({
                  page: p.pageNum,
                  width: p.width,
                  height: p.height,
                  text: p.text,
                  textItems: p.textItems,
                  annotations: p.annotations,
                })),
              },
              null,
              2,
            )
          : result.text;

      if (opts.imageOutputDir && result.images.length > 0) {
        const dir = opts.imageOutputDir as string;
        mkdirSync(dir, { recursive: true });
        for (const img of result.images) {
          writeFileSync(join(dir, `image_${img.id}.${img.format}`), img.bytes);
        }
        if (!opts.quiet) {
          console.error(
            `[liteparse] wrote ${result.images.length} image(s) to ${dir}`,
          );
        }
      }

      if (opts.output) {
        writeFileSync(opts.output as string, output, "utf-8");
      } else {
        process.stdout.write(output);
      }
    } catch (err) {
      console.error(
        `Error: ${err instanceof Error ? err.message : String(err)}`,
      );
      process.exit(1);
    }
  });

program
  .command("is-complex")
  .description(
    "Check if a document is 'complex' enough to require OCR or other advanced parsing",
  )
  .argument("<file>", "Path to the document file")
  .option("--compact", "Emit dense, whitespace-free JSON instead of pretty")
  .option("--max-pages <n>", "Max pages to parse", parseInt)
  .option(
    "--target-pages <pages>",
    'Pages to check (e.g., "1-5,10,15-20")',
  )
  .option("--password <password>", "Password for encrypted documents")
  .option("-q, --quiet", "Suppress progress output")
  .action(async (file: string, opts: Record<string, unknown>) => {
    try {
      const config: Partial<LiteParseConfig> = {};
      if (opts.maxPages) config.maxPages = opts.maxPages as number;
      if (opts.targetPages) config.targetPages = opts.targetPages as string;
      if (opts.password) config.password = opts.password as string;
      if (opts.quiet) config.quiet = true;

      const parser = new LiteParse(config);
      const stats = await parser.isComplex(await resolveInput(file));

      const complexPages = stats.filter((s) => s.needsOcr).length;

      // Always emit JSON on stdout so the command composes with `jq` without a
      // flag. Pretty by default; `--compact` drops the whitespace. Both parse
      // identically for any reader.
      process.stdout.write(
        opts.compact
          ? JSON.stringify(stats)
          : JSON.stringify(stats, null, 2),
      );
      process.stdout.write("\n");

      // Human verdict goes to stderr so it never pollutes the JSON on stdout;
      // the exit code below carries the same signal for scripts.
      if (!opts.quiet) {
        const verdict = complexPages > 0 ? "COMPLEX" : "SIMPLE";
        const layoutCount = (reason: string) =>
          stats.filter((s) => s.layout?.reasons.includes(reason)).length;
        console.error(
          `${verdict} — ${complexPages}/${stats.length} page(s) need OCR; ` +
            `layout: ${layoutCount("multi-column")} multi-column, ` +
            `${layoutCount("table-likely")} table, ` +
            `${layoutCount("dense-graphics")} graphics-dense`,
        );
      }

      // Exit non-zero when any page needs OCR, so the command is usable as a
      // shell predicate (e.g. `is-complex doc.pdf && parse --no-ocr`).
      if (complexPages > 0) process.exit(1);
    } catch (err) {
      console.error(
        `Error: ${err instanceof Error ? err.message : String(err)}`,
      );
      process.exit(1);
    }
  });

program
  .command("screenshot")
  .description("Generate screenshots of document pages")
  .argument("<file>", "Path to the document file")
  .option(
    "-o, --output-dir <dir>",
    "Output directory for screenshots",
    "./screenshots",
  )
  .option(
    "--target-pages <pages>",
    'Pages to screenshot (e.g., "1,3,5" or "1-5")',
  )
  .option("--dpi <dpi>", "Rendering DPI", parseFloat)
  .option("--password <password>", "Password for encrypted documents")
  .option("-q, --quiet", "Suppress progress output")
  .action(async (file: string, opts: Record<string, unknown>) => {
    try {
      const config: Partial<LiteParseConfig> = {};
      if (opts.dpi) config.dpi = opts.dpi as number;
      if (opts.password) config.password = opts.password as string;
      if (opts.quiet) config.quiet = true;
      if (opts.targetPages) config.targetPages = opts.targetPages as string;

      const parser = new LiteParse(config);

      // Parse target pages into number array
      let pageNumbers: number[] | undefined;
      if (opts.targetPages) {
        pageNumbers = [];
        for (const part of (opts.targetPages as string).split(",")) {
          const trimmed = part.trim();
          if (trimmed.includes("-")) {
            const [start, end] = trimmed.split("-").map(Number);
            for (let i = start; i <= end; i++) pageNumbers.push(i);
          } else {
            pageNumbers.push(Number(trimmed));
          }
        }
      }

      const outputDir = opts.outputDir as string;
      mkdirSync(outputDir, { recursive: true });

      const results = await parser.screenshot(file, pageNumbers);

      for (const result of results) {
        const outputPath = join(outputDir, `page_${result.pageNum}.png`);
        writeFileSync(outputPath, result.imageBuffer);
        if (!opts.quiet) {
          console.error(
            `[liteparse] screenshot page ${result.pageNum} → ${outputPath}`,
          );
        }
      }
    } catch (err) {
      console.error(
        `Error: ${err instanceof Error ? err.message : String(err)}`,
      );
      process.exit(1);
    }
  });

program
  .command("batch-parse")
  .description("Parse multiple documents in batch mode")
  .argument("<input-dir>", "Input directory")
  .argument("<output-dir>", "Output directory")
  .option("--format <format>", 'Output format: json|text|markdown (default: "text")')
  .option("--no-ocr", "Disable OCR")
  .option("--ocr-language <lang>", "OCR language (default: eng)")
  .option("--ocr-server-url <url>", "HTTP OCR server URL")
  .option(
    "--ocr-server-header <header>",
    'Extra header for OCR server requests, "Name: Value" (repeatable)',
    collectHeader,
  )
  .option("--max-pages <n>", "Max pages to parse per file", parseInt)
  .option("--dpi <dpi>", "Rendering DPI", parseFloat)
  .option("--recursive", "Recursively search input directory")
  .option("--extension <ext>", "Only process files with this extension")
  .option("--password <password>", "Password for encrypted documents")
  .option("-q, --quiet", "Suppress progress output")
  .option("--num-workers <n>", "Number of concurrent OCR workers", parseInt)
  .action(
    async (
      inputDir: string,
      outputDir: string,
      opts: Record<string, unknown>,
    ) => {
      try {
        const config: Partial<LiteParseConfig> = {};
        const format = (opts.format as string) ?? "text";
        config.outputFormat = format as "json" | "text" | "markdown";
        if (opts.ocr === false) config.ocrEnabled = false;
        if (opts.ocrLanguage) config.ocrLanguage = opts.ocrLanguage as string;
        if (opts.ocrServerUrl)
          config.ocrServerUrl = opts.ocrServerUrl as string;
        if (opts.ocrServerHeader)
          config.ocrServerHeaders = opts.ocrServerHeader as Record<string, string>;
        if (opts.maxPages) config.maxPages = opts.maxPages as number;
        if (opts.dpi) config.dpi = opts.dpi as number;
        if (opts.password) config.password = opts.password as string;
        if (opts.quiet) config.quiet = true;
        if (opts.numWorkers) config.numWorkers = opts.numWorkers as number;

        const parser = new LiteParse(config);
        const outExt = format === "json" ? ".json" : format === "markdown" ? ".md" : ".txt";

        mkdirSync(outputDir, { recursive: true });

        const extFilter = opts.extension
          ? (opts.extension as string).startsWith(".")
            ? (opts.extension as string).toLowerCase()
            : `.${(opts.extension as string).toLowerCase()}`
          : undefined;

        const files = collectFiles(
          inputDir,
          !!opts.recursive,
          extFilter,
        );

        if (files.length === 0) {
          console.error(
            `[liteparse] no matching files found in ${inputDir}`,
          );
          return;
        }
        if (!opts.quiet) {
          console.error(
            `[liteparse] found ${files.length} files to process`,
          );
        }

        let success = 0;
        let errors = 0;

        for (const filePath of files) {
          const t0 = Date.now();
          const rel = relative(inputDir, filePath);
          const parsed = parsePath(rel);
          const outPath = join(
            outputDir,
            parsed.dir,
            parsed.name + outExt,
          );
          mkdirSync(join(outputDir, parsed.dir), { recursive: true });

          try {
            const result = await parser.parse(filePath);
            const output =
              format === "json"
                ? JSON.stringify(
                    {
                      pages: result.pages.map((p) => ({
                        page: p.pageNum,
                        width: p.width,
                        height: p.height,
                        text: p.text,
                        textItems: p.textItems,
                      })),
                    },
                    null,
                    2,
                  )
                : result.text;
            writeFileSync(outPath, output, "utf-8");
            success++;
            if (!opts.quiet) {
              const elapsed = Date.now() - t0;
              console.error(
                `[liteparse] ${filePath} → ${outPath} (${elapsed}ms)`,
              );
            }
          } catch (err) {
            console.error(
              `[liteparse] error parsing ${filePath}: ${err instanceof Error ? err.message : String(err)}`,
            );
            errors++;
          }
        }

        console.error(
          `[liteparse] batch complete: ${success} succeeded, ${errors} failed`,
        );
        if (errors > 0) process.exit(1);
      } catch (err) {
        console.error(
          `Error: ${err instanceof Error ? err.message : String(err)}`,
        );
        process.exit(1);
      }
    },
  );

const SUPPORTED_EXTENSIONS = new Set([
  ".pdf",
  ".doc", ".docx", ".docm", ".dot", ".dotm", ".dotx", ".odt", ".ott", ".rtf", ".pages",
  ".ppt", ".pptx", ".pptm", ".pot", ".potm", ".potx", ".odp", ".otp", ".key",
  ".xls", ".xlsx", ".xlsm", ".xlsb", ".ods", ".ots", ".csv", ".tsv", ".numbers",
  ".jpg", ".jpeg", ".png", ".gif", ".bmp", ".tiff", ".tif", ".webp", ".svg",
  ".txt", ".md", ".markdown", ".log",
]);

function collectFiles(
  dir: string,
  recursive: boolean,
  extFilter?: string,
): string[] {
  const files: string[] = [];
  collectFilesInner(dir, recursive, extFilter, files);
  files.sort();
  return files;
}

function collectFilesInner(
  dir: string,
  recursive: boolean,
  extFilter: string | undefined,
  files: string[],
): void {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const fullPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (recursive) collectFilesInner(fullPath, recursive, extFilter, files);
      continue;
    }
    const lower = entry.name.toLowerCase();
    if (extFilter) {
      if (!lower.endsWith(extFilter)) continue;
    } else {
      const ext = lower.lastIndexOf(".") >= 0 ? lower.slice(lower.lastIndexOf(".")) : "";
      if (!SUPPORTED_EXTENSIONS.has(ext)) continue;
    }
    files.push(fullPath);
  }
}

program.parse(process.argv);
