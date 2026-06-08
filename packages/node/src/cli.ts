#!/usr/bin/env node

import { program } from "commander";
import { LiteParse, type LiteParseConfig } from "./lib.js";
import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { join, relative, parse as parsePath } from "node:path";

program
  .name("liteparse")
  .description("Fast, lightweight PDF and document parsing")
  .version("2.0.0");

program
  .command("parse")
  .description("Parse a document and extract text")
  .argument("<file>", "Path to the document file")
  .option("-o, --output <file>", "Output file path")
  .option("--format <format>", 'Output format: json|text (default: "text")')
  .option("--ocr-server-url <url>", "HTTP OCR server URL")
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
  .option("--no-clean-artifacts", "Disable auto-removal of OCR table artifacts")
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
      if (opts.format) config.outputFormat = opts.format as "json" | "text";
      if (opts.ocrServerUrl)
        config.ocrServerUrl = opts.ocrServerUrl as string;
      if (opts.ocr === false) config.ocrEnabled = false;
      if (opts.ocrLanguage) config.ocrLanguage = opts.ocrLanguage as string;
      if (opts.maxPages) config.maxPages = opts.maxPages as number;
      if (opts.targetPages) config.targetPages = opts.targetPages as string;
      if (opts.dpi) config.dpi = opts.dpi as number;
      if (opts.preserveSmallText) config.preserveVerySmallText = true;
      if (opts.password) config.password = opts.password as string;
      if (opts.quiet) config.quiet = true;
      if (opts.numWorkers) config.numWorkers = opts.numWorkers as number;
      if (opts.cleanArtifacts === false) config.cleanArtifacts = false;

      // Default CLI output to text (library defaults to json)
      if (!config.outputFormat) config.outputFormat = "text";

      const parser = new LiteParse(config);
      const result = await parser.parse(file);

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
                })),
              },
              null,
              2,
            )
          : result.text;

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
  .option("--format <format>", 'Output format: json|text (default: "text")')
  .option("--no-ocr", "Disable OCR")
  .option("--ocr-language <lang>", "OCR language (default: eng)")
  .option("--ocr-server-url <url>", "HTTP OCR server URL")
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
        config.outputFormat = format as "json" | "text";
        if (opts.ocr === false) config.ocrEnabled = false;
        if (opts.ocrLanguage) config.ocrLanguage = opts.ocrLanguage as string;
        if (opts.ocrServerUrl)
          config.ocrServerUrl = opts.ocrServerUrl as string;
        if (opts.maxPages) config.maxPages = opts.maxPages as number;
        if (opts.dpi) config.dpi = opts.dpi as number;
        if (opts.password) config.password = opts.password as string;
        if (opts.quiet) config.quiet = true;
        if (opts.numWorkers) config.numWorkers = opts.numWorkers as number;

        const parser = new LiteParse(config);
        const outExt = format === "json" ? ".json" : ".txt";

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
