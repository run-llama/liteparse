import { Command } from "commander";
import fs from "fs/promises";
import { existsSync, readdirSync, statSync } from "fs";
import os from "os";
import path from "path";
import { LiteParse } from "../src/core/parser.js";
import { LiteParseConfig, OutputFormat } from "../src/core/types.js";
import { performance } from "perf_hooks";

const DEFAULT_MAX_PAGES = 10000;
const DEFAULT_DPI = 150;
const DEFAULT_LANGUAGE = "en";
const DEFAULT_OUTPUT_FORMAT = "text";
const DEFAULT_SCREENSHOT_FORMAT = "png";
const DEFAULT_SCREENSHOT_DIR = "./screenshots";

interface ParseCommandOptions {
  output?: string;
  format?: string;
  ocrServerUrl?: string;
  ocr?: boolean;
  ocrLanguage?: string;
  numWorkers?: string;
  maxPages?: string;
  targetPages?: string;
  dpi?: string;
  preciseBbox?: boolean;
  preserveSmallText?: boolean;
  password?: string;
  config?: string;
  quiet?: boolean;
}

interface ScreenshotCommandOptions {
  outputDir?: string;
  targetPages?: string;
  dpi?: string;
  format?: string;
  password?: string;
  config?: string;
  quiet?: boolean;
}

interface BatchParseCommandOptions {
  format?: string;
  ocrServerUrl?: string;
  ocr?: boolean;
  ocrLanguage?: string;
  numWorkers?: string;
  maxPages?: string;
  dpi?: string;
  preciseBbox?: boolean;
  recursive?: boolean;
  extension?: string;
  password?: string;
  config?: string;
  quiet?: boolean;
}

const program = new Command();

program
  .name("lit")
  .description("OSS document parsing tool (supports PDF, DOCX, XLSX, images, and more)")
  .version("0.1.0");

program
  .command("parse <file>")
  .description("Parse a document file (PDF, DOCX, XLSX, PPTX, images, etc.)")
  .option("-o, --output <file>", "Output file path")
  .option("--format <format>", "Output format: json|text", DEFAULT_OUTPUT_FORMAT)
  .option("--ocr-server-url <url>", "HTTP OCR server URL (uses Tesseract if not provided)")
  .option("--no-ocr", "Disable OCR")
  .option("--ocr-language <lang>", "OCR language(s)", DEFAULT_LANGUAGE)
  .option(
    "--num-workers <n>",
    "Number of pages to OCR in parallel. Defaults to number of CPU cores minus one."
  )
  .option("--max-pages <n>", "Max pages to parse", DEFAULT_MAX_PAGES.toString())
  .option("--target-pages <pages>", 'Target pages (e.g., "1-5,10,15-20")')
  .option("--dpi <dpi>", "DPI for rendering", DEFAULT_DPI.toString())
  .option("--no-precise-bbox", "Disable precise bounding boxes")
  .option("--preserve-small-text", "Preserve very small text")
  .option("--password <password>", "Password for encrypted/protected documents")
  .option("--config <file>", "Config file (JSON)")
  .option("-q, --quiet", "Suppress progress output")
  .action(async (file: string, options: ParseCommandOptions) => {
    try {
      const quiet = options.quiet || false;
      const isStdin = file === "-";

      // Check if file exists (skip for stdin)
      if (!isStdin && !existsSync(file)) {
        console.error(`Error: File not found: ${file}`);
        process.exit(1);
      }

      let config: Partial<LiteParseConfig> = {};

      // Load config file if provided
      if (options.config) {
        if (!existsSync(options.config)) {
          console.error(`Error: Config file not found: ${options.config}`);
          process.exit(1);
        }
        const configData = await fs.readFile(options.config, "utf-8");
        config = JSON.parse(configData);
      }

      // Override with CLI options
      let calculatedNumWorkers = os.cpus().length - 1;
      if (calculatedNumWorkers < 1) {
        calculatedNumWorkers = 1;
      }

      config = {
        ...config,
        outputFormat: options.format as OutputFormat,
        ocrEnabled: options.ocr !== false,
        ocrServerUrl: options.ocrServerUrl,
        ocrLanguage: options.ocrLanguage,
        numWorkers: parseInt(options.numWorkers || calculatedNumWorkers.toString()),
        maxPages: parseInt(options.maxPages || DEFAULT_MAX_PAGES.toString()),
        targetPages: options.targetPages,
        dpi: parseInt(options.dpi || DEFAULT_DPI.toString()),
        preciseBoundingBox: options.preciseBbox !== false,
        preserveVerySmallText: options.preserveSmallText || false,
        password: options.password,
      };

      // Create parser
      const parser = new LiteParse(config);

      // Read from stdin or file
      let input: string | Buffer;
      if (isStdin) {
        const chunks: Buffer[] = [];
        for await (const chunk of process.stdin) {
          chunks.push(chunk);
        }
        input = Buffer.concat(chunks);
        if (input.length === 0) {
          console.error("Error: No data received from stdin");
          process.exit(1);
        }
      } else {
        input = file;
      }

      // Parse document (quiet flag controls progress output)
      const result = await parser.parse(input, quiet);

      // Format output based on format
      let output: string;
      switch (config.outputFormat) {
        case "json":
          output = JSON.stringify(result.json, null, 2);
          break;
        case "text":
        default:
          output = result.text;
          break;
      }

      // Write to file or stdout
      if (options.output) {
        await fs.writeFile(options.output, output);
        if (!quiet) {
          console.error(`\n✓ Parsed ${result.pages.length} pages → ${options.output}`);
        }
      } else {
        // Output result to stdout (can be piped)
        console.log(output);
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      const stack = error instanceof Error ? error.stack : undefined;
      console.error(`\nError: ${message}`);
      if (stack) {
        console.error(stack);
      }
      process.exit(1);
    }
  });

program
  .command("screenshot <file>")
  .description("Generate screenshots of PDF pages")
  .option("-o, --output-dir <dir>", "Output directory for screenshots", DEFAULT_SCREENSHOT_DIR)
  .option("--target-pages <pages>", 'Page numbers to screenshot (e.g., "1,3,5" or "1-5")')
  .option("--dpi <dpi>", "DPI for rendering", DEFAULT_DPI.toString())
  .option("--format <format>", "Image format: png|jpg", DEFAULT_SCREENSHOT_FORMAT)
  .option("--password <password>", "Password for encrypted/protected documents")
  .option("--config <file>", "Config file (JSON)")
  .option("-q, --quiet", "Suppress progress output")
  .action(async (file: string, options: ScreenshotCommandOptions) => {
    try {
      const quiet = options.quiet || false;

      // Check if file exists
      if (!existsSync(file)) {
        console.error(`Error: File not found: ${file}`);
        process.exit(1);
      }

      let config: Partial<LiteParseConfig> = {};

      // Load config file if provided
      if (options.config) {
        if (!existsSync(options.config)) {
          console.error(`Error: Config file not found: ${options.config}`);
          process.exit(1);
        }
        const configData = await fs.readFile(options.config, "utf-8");
        config = JSON.parse(configData);
      }

      // Override with CLI options
      config = {
        ...config,
        dpi: parseInt(options.dpi || DEFAULT_DPI.toString()),
        password: options.password,
      };

      // Parse target pages
      let pageNumbers: number[] | undefined;
      if (options.targetPages) {
        pageNumbers = parsePageNumbers(options.targetPages);
      }

      const outputDir = options.outputDir || DEFAULT_SCREENSHOT_DIR;

      // Create output directory
      if (!existsSync(outputDir)) {
        await fs.mkdir(outputDir, { recursive: true });
      }

      // Create parser
      const parser = new LiteParse(config);

      // Generate screenshots
      const results = await parser.screenshot(file, pageNumbers, quiet);

      // Save screenshots
      for (const result of results) {
        const filename = `page_${result.pageNum}.${options.format || DEFAULT_SCREENSHOT_FORMAT}`;
        const filepath = `${outputDir}/${filename}`;
        await fs.writeFile(filepath, result.imageBuffer);
        if (!quiet) {
          console.error(`✓ ${filepath} (${result.width}x${result.height})`);
        }
      }

      if (!quiet) {
        console.error(`\n✓ Generated ${results.length} screenshots → ${outputDir}`);
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      const stack = error instanceof Error ? error.stack : undefined;
      console.error(`\nError: ${message}`);
      if (stack) {
        console.error(stack);
      }
      process.exit(1);
    }
  });

// Supported file extensions for batch parsing
const SUPPORTED_EXTENSIONS = new Set([
  ".pdf",
  ".doc",
  ".docx",
  ".docm",
  ".dot",
  ".dotm",
  ".dotx",
  ".odt",
  ".ott",
  ".ppt",
  ".pptx",
  ".pptm",
  ".pot",
  ".potm",
  ".potx",
  ".odp",
  ".otp",
  ".xls",
  ".xlsx",
  ".xlsm",
  ".xlsb",
  ".ods",
  ".ots",
  ".csv",
  ".tsv",
  ".jpg",
  ".jpeg",
  ".png",
  ".gif",
  ".bmp",
  ".tiff",
  ".tif",
  ".webp",
  ".svg",
  ".rtf",
  ".pages",
  ".key",
  ".numbers",
]);

program
  .command("batch-parse <input-dir> <output-dir>")
  .description("Parse multiple documents in batch mode")
  .option("--format <format>", "Output format: json|text", DEFAULT_OUTPUT_FORMAT)
  .option("--ocr-server-url <url>", "HTTP OCR server URL (uses Tesseract if not provided)")
  .option("--no-ocr", "Disable OCR")
  .option("--ocr-language <lang>", "OCR language(s)", DEFAULT_LANGUAGE)
  .option(
    "--num-workers <n>",
    "Number of pages to OCR in parallel. Defaults to number of CPU cores minus one."
  )
  .option("--max-pages <n>", "Max pages to parse per file", DEFAULT_MAX_PAGES.toString())
  .option("--dpi <dpi>", "DPI for rendering", DEFAULT_DPI.toString())
  .option("--no-precise-bbox", "Disable precise bounding boxes")
  .option("--recursive", "Recursively search input directory")
  .option("--extension <ext>", 'Only process files with this extension (e.g., ".pdf")')
  .option(
    "--password <password>",
    "Password for encrypted/protected documents (applied to all files)"
  )
  .option("--config <file>", "Config file (JSON)")
  .option("-q, --quiet", "Suppress progress output")
  .action(async (inputDir: string, outputDir: string, options: BatchParseCommandOptions) => {
    try {
      const quiet = options.quiet || false;
      const startTime = performance.now();

      // Validate input directory
      if (!existsSync(inputDir)) {
        console.error(`Error: Input directory not found: ${inputDir}`);
        process.exit(1);
      }

      const inputStat = statSync(inputDir);
      if (!inputStat.isDirectory()) {
        console.error(`Error: Input path is not a directory: ${inputDir}`);
        process.exit(1);
      }

      // Create output directory
      if (!existsSync(outputDir)) {
        await fs.mkdir(outputDir, { recursive: true });
      }

      // Find all files to process
      const files = findFiles(inputDir, options.recursive || false, options.extension);

      if (files.length === 0) {
        console.error("No supported files found in input directory");
        process.exit(1);
      }

      if (!quiet) {
        console.error(`Found ${files.length} files to process`);
      }

      // Load config
      let config: Partial<LiteParseConfig> = {};
      if (options.config) {
        if (!existsSync(options.config)) {
          console.error(`Error: Config file not found: ${options.config}`);
          process.exit(1);
        }
        const configData = await fs.readFile(options.config, "utf-8");
        config = JSON.parse(configData);
      }

      // Apply CLI options
      let calculatedNumWorkers = os.cpus().length - 1;
      if (calculatedNumWorkers < 1) {
        calculatedNumWorkers = 1;
      }

      config = {
        ...config,
        outputFormat: options.format as OutputFormat,
        ocrEnabled: options.ocr !== false,
        ocrServerUrl: options.ocrServerUrl,
        ocrLanguage: options.ocrLanguage,
        numWorkers: parseInt(options.numWorkers || calculatedNumWorkers.toString()),
        maxPages: parseInt(options.maxPages || DEFAULT_MAX_PAGES.toString()),
        dpi: parseInt(options.dpi || DEFAULT_DPI.toString()),
        preciseBoundingBox: options.preciseBbox !== false,
        password: options.password,
      };

      // Create a SINGLE parser instance for all files (key for batch efficiency)
      const parser = new LiteParse(config);

      // Process files
      let successCount = 0;
      let errorCount = 0;
      const outputExt = options.format === "json" ? ".json" : ".txt";

      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        const relativePath = path.relative(inputDir, file);
        const outputPath = path.join(outputDir, relativePath.replace(/\.[^.]+$/, outputExt));

        // Create output subdirectory if needed
        const outputSubdir = path.dirname(outputPath);
        if (!existsSync(outputSubdir)) {
          await fs.mkdir(outputSubdir, { recursive: true });
        }

        try {
          const fileStart = performance.now();
          const result = await parser.parse(file, true); // Always quiet for individual files

          // Format output
          let output: string;
          if (options.format === "json") {
            output = JSON.stringify(result.json, null, 2);
          } else {
            output = result.text;
          }

          await fs.writeFile(outputPath, output);
          successCount++;

          if (!quiet) {
            const fileTime = (performance.now() - fileStart).toFixed(0);
            console.error(
              `[${i + 1}/${files.length}] ✓ ${relativePath} (${result.pages.length} pages, ${fileTime}ms)`
            );
          }
        } catch (error: unknown) {
          errorCount++;
          if (!quiet) {
            const message = error instanceof Error ? error.message : String(error);
            console.error(`[${i + 1}/${files.length}] ✗ ${relativePath}: ${message}`);
          }
        }
      }

      const totalTime = ((performance.now() - startTime) / 1000).toFixed(2);
      const avgTime =
        files.length > 0 ? ((performance.now() - startTime) / files.length).toFixed(0) : 0;

      if (!quiet) {
        console.error("");
        console.error(`Batch complete: ${successCount} succeeded, ${errorCount} failed`);
        console.error(`Total time: ${totalTime}s (avg ${avgTime}ms/file)`);
        console.error(`Output: ${outputDir}`);
      }

      if (errorCount > 0) {
        process.exit(1);
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      const stack = error instanceof Error ? error.stack : undefined;
      console.error(`\nError: ${message}`);
      if (stack) {
        console.error(stack);
      }
      process.exit(1);
    }
  });

/**
 * Find all supported files in a directory
 */
function findFiles(dir: string, recursive: boolean, filterExt?: string): string[] {
  const files: string[] = [];

  function scan(currentDir: string) {
    const entries = readdirSync(currentDir);

    for (const entry of entries) {
      const fullPath = path.join(currentDir, entry);
      const stat = statSync(fullPath);

      if (stat.isDirectory()) {
        if (recursive) {
          scan(fullPath);
        }
      } else if (stat.isFile()) {
        const ext = path.extname(entry).toLowerCase();

        // Filter by extension if specified
        if (filterExt && ext !== filterExt.toLowerCase()) {
          continue;
        }

        // Check if supported
        if (SUPPORTED_EXTENSIONS.has(ext)) {
          files.push(fullPath);
        }
      }
    }
  }

  scan(dir);
  return files.sort();
}

/**
 * Parse page numbers from string like "1,3,5" or "1-5,10"
 */
function parsePageNumbers(pagesStr: string): number[] {
  const pages: number[] = [];
  const parts = pagesStr.split(",");

  for (const part of parts) {
    const trimmed = part.trim();
    if (trimmed.includes("-")) {
      const [start, end] = trimmed.split("-").map((n) => parseInt(n.trim()));
      for (let i = start; i <= end; i++) {
        pages.push(i);
      }
    } else {
      pages.push(parseInt(trimmed));
    }
  }

  return [...new Set(pages)].sort((a, b) => a - b);
}

export { program };
