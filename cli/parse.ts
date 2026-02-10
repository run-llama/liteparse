import { Command } from 'commander';
import fs from 'fs/promises';
import { existsSync } from 'fs';
import { LiteParse } from '../src/core/parser.js';
import { LiteParseConfig, OutputFormat } from '../src/core/types.js';

const program = new Command();

program
  .name('lp')
  .description('OSS document parsing tool (supports PDF, DOCX, XLSX, images, and more)')
  .version('1.0.0');

program
  .command('parse <file>')
  .description('Parse a document file (PDF, DOCX, XLSX, PPTX, images, etc.)')
  .option('-o, --output <file>', 'Output file path')
  .option('--format <format>', 'Output format: json|text', 'text')
  .option('--ocr-server-url <url>', 'HTTP OCR server URL (uses Tesseract if not provided)')
  .option('--no-ocr', 'Disable OCR')
  .option('--ocr-language <lang>', 'OCR language(s)', 'en')
  .option('--max-pages <n>', 'Max pages to parse', '1000')
  .option('--target-pages <pages>', 'Target pages (e.g., "1-5,10,15-20")')
  .option('--dpi <dpi>', 'DPI for rendering', '150')
  .option('--no-tables', 'Disable table detection')
  .option('--no-precise-bbox', 'Disable precise bounding boxes')
  .option('--skip-diagonal-text', 'Skip diagonal text')
  .option('--preserve-small-text', 'Preserve very small text')
  .option('--config <file>', 'Config file (JSON)')
  .option('-q, --quiet', 'Suppress progress output')
  .action(async (file: string, options: any) => {
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
        const configData = await fs.readFile(options.config, 'utf-8');
        config = JSON.parse(configData);
      }

      // Override with CLI options
      config = {
        ...config,
        outputFormat: options.format as OutputFormat,
        ocrEnabled: options.ocr !== false,
        ocrServerUrl: options.ocrServerUrl,
        ocrLanguage: options.ocrLanguage,
        maxPages: parseInt(options.maxPages),
        targetPages: options.targetPages,
        dpi: parseInt(options.dpi),
        tableDetection: options.tables !== false,
        preciseBoundingBox: options.preciseBbox !== false,
        skipDiagonalText: options.skipDiagonalText || false,
        preserveVerySmallText: options.preserveSmallText || false,
      };

      // Create parser
      const parser = new LiteParse(config);

      // Parse PDF (quiet flag controls progress output)
      const result = await parser.parse(file, quiet);

      // Format output based on format
      let output: string;
      switch (config.outputFormat) {
        case 'json':
          output = JSON.stringify(result.json, null, 2);
          break;
        case 'text':
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
    } catch (error: any) {
      console.error(`\nError: ${error.message}`);
      if (error.stack) {
        console.error(error.stack);
      }
      process.exit(1);
    }
  });

program
  .command('screenshot <file>')
  .description('Generate screenshots of PDF pages')
  .option('-o, --output-dir <dir>', 'Output directory for screenshots', './screenshots')
  .option('--pages <pages>', 'Page numbers to screenshot (e.g., "1,3,5" or "1-5")')
  .option('--dpi <dpi>', 'DPI for rendering', '150')
  .option('--format <format>', 'Image format: png|jpg', 'png')
  .option('--config <file>', 'Config file (JSON)')
  .option('-q, --quiet', 'Suppress progress output')
  .action(async (file: string, options: any) => {
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
        const configData = await fs.readFile(options.config, 'utf-8');
        config = JSON.parse(configData);
      }

      // Override with CLI options
      config = {
        ...config,
        dpi: parseInt(options.dpi),
      };

      // Parse target pages
      let pageNumbers: number[] | undefined;
      if (options.pages) {
        pageNumbers = parsePageNumbers(options.pages);
      }

      // Create output directory
      if (!existsSync(options.outputDir)) {
        await fs.mkdir(options.outputDir, { recursive: true });
      }

      // Create parser
      const parser = new LiteParse(config);

      // Generate screenshots
      const results = await parser.screenshot(file, pageNumbers, quiet);

      // Save screenshots
      for (const result of results) {
        const filename = `page_${result.pageNum}.${options.format}`;
        const filepath = `${options.outputDir}/${filename}`;
        await fs.writeFile(filepath, result.imageBuffer);
        if (!quiet) {
          console.error(`✓ ${filepath} (${result.width}x${result.height})`);
        }
      }

      if (!quiet) {
        console.error(`\n✓ Generated ${results.length} screenshots → ${options.outputDir}`);
      }
    } catch (error: any) {
      console.error(`\nError: ${error.message}`);
      if (error.stack) {
        console.error(error.stack);
      }
      process.exit(1);
    }
  });

/**
 * Parse page numbers from string like "1,3,5" or "1-5,10"
 */
function parsePageNumbers(pagesStr: string): number[] {
  const pages: number[] = [];
  const parts = pagesStr.split(',');

  for (const part of parts) {
    const trimmed = part.trim();
    if (trimmed.includes('-')) {
      const [start, end] = trimmed.split('-').map((n) => parseInt(n.trim()));
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
