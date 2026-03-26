import { createWorker, createScheduler, Scheduler, Worker } from "tesseract.js";
import { OcrEngine, OcrOptions, OcrResult } from "./interface.js";

export class TesseractEngine implements OcrEngine {
  name = "tesseract";
  private scheduler?: Scheduler;
  private workers: Worker[] = [];
  private currentLanguage?: string;
  private concurrency: number;
  private tessdataPath?: string;

  constructor(concurrency: number = 4, tessdataPath?: string) {
    this.concurrency = concurrency;
    // Use explicit path, then TESSDATA_PREFIX env var, then let tesseract.js default (CDN)
    this.tessdataPath = tessdataPath || process.env.TESSDATA_PREFIX || undefined;
  }

  async initialize(language: string = "eng"): Promise<void> {
    if (this.scheduler && this.currentLanguage === language) {
      return; // Already initialized for this language
    }

    // Clean up existing scheduler and workers if language changed
    await this.terminate();

    // Create scheduler
    this.scheduler = createScheduler();

    // Build worker options for local tessdata support
    const workerOptions: Record<string, unknown> = {};
    if (this.tessdataPath) {
      workerOptions.langPath = this.tessdataPath;
      workerOptions.cachePath = this.tessdataPath;
      workerOptions.gzip = false; // Pre-cached files are not gzipped
    }
    workerOptions.errorHandler = () => {
      // Let createWorker reject so LiteParse can convert the failure into
      // an actionable initialization error instead of crashing the process.
    };

    // Create worker pool
    for (let i = 0; i < this.concurrency; i++) {
      let worker: Worker;
      try {
        worker = await createWorker(
          language,
          1,
          Object.keys(workerOptions).length > 0 ? workerOptions : undefined
        );
      } catch (error) {
        // Clean up any workers already created
        await this.terminate();
        const message = error instanceof Error ? error.message : String(error);

        // Provide actionable guidance for common failures
        if (
          message.includes("fetch") ||
          message.includes("network") ||
          message.includes("ENOTFOUND") ||
          message.includes("ERR_INVALID_URL")
        ) {
          throw new Error(
            `Tesseract failed to download language data for "${language}". ` +
              `This usually means the machine has no internet access. ` +
              `To fix this, either:\n` +
              `  1. Set the TESSDATA_PREFIX env var to a directory containing ${language}.traineddata\n` +
              `  2. Use --ocr-server-url to use an external OCR server instead\n` +
              `  3. Use --no-ocr to disable OCR entirely`,
            {
              cause: error,
            }
          );
        }
        if (
          message.includes("traineddata") ||
          message.includes("TESSDATA") ||
          message.includes("loading language")
        ) {
          throw new Error(
            `Tesseract failed to load language data for "${language}": ${message}\n` +
              `Ensure ${language}.traineddata exists in your tessdata directory and set ` +
              `the TESSDATA_PREFIX env var accordingly.`,
            {
              cause: error,
            }
          );
        }
        throw new Error(`Tesseract OCR initialization failed: ${message}`, { cause: error });
      }
      if (!worker) {
        await this.terminate();
        throw new Error("Tesseract worker not initialized");
      }
      this.workers.push(worker);
      this.scheduler.addWorker(worker);
    }

    this.currentLanguage = language;
  }

  async recognize(image: string | Buffer, options: OcrOptions): Promise<OcrResult[]> {
    // Handle language - tesseract.js uses language codes like 'eng', 'fra', 'deu'
    const language = this.normalizeLanguage(
      Array.isArray(options.language) ? options.language[0] : options.language
    );

    // Initialize scheduler if needed
    await this.initialize(language);

    if (!this.scheduler) {
      throw new Error("Tesseract scheduler not initialized");
    }

    try {
      // Recognize text from image using scheduler
      // tesseract.js accepts string (path/URL) or Buffer/Uint8Array
      // In tesseract.js v6+, we need to enable blocks output to get word-level data
      const {
        data: { blocks },
      } = await this.scheduler.addJob("recognize", image, {}, { blocks: true });

      // Extract words from hierarchical blocks structure: blocks → paragraphs → lines → words
      const results: OcrResult[] = [];
      for (const block of blocks || []) {
        for (const paragraph of block.paragraphs || []) {
          for (const line of paragraph.lines || []) {
            for (const word of line.words || []) {
              results.push({
                text: word.text,
                bbox: [word.bbox.x0, word.bbox.y0, word.bbox.x1, word.bbox.y1] as [
                  number,
                  number,
                  number,
                  number,
                ],
                confidence: word.confidence / 100, // Tesseract returns 0-100, we want 0-1
              });
            }
          }
        }
      }

      // Filter out low confidence results (below 30%)
      return results.filter((r) => r.confidence > 0.3);
    } catch (error) {
      const label = typeof image === "string" ? image : "<buffer>";
      console.error(`\nTesseract OCR error for ${label}:`, error);
      return [];
    }
  }

  async recognizeBatch(images: (string | Buffer)[], options: OcrOptions): Promise<OcrResult[][]> {
    // Handle language
    const language = this.normalizeLanguage(
      Array.isArray(options.language) ? options.language[0] : options.language
    );

    // Initialize scheduler if needed
    await this.initialize(language);

    if (!this.scheduler) {
      throw new Error("Tesseract scheduler not initialized");
    }

    // Process all images in parallel - scheduler handles distribution
    const jobs = images.map((image) => this.recognize(image, options));

    return Promise.all(jobs);
  }

  async terminate(): Promise<void> {
    if (this.scheduler) {
      await this.scheduler.terminate();
      this.scheduler = undefined;
    }
    this.workers = [];
    this.currentLanguage = undefined;
  }

  /**
   * Normalize language codes to Tesseract format
   * Common mappings: en->eng, fr->fra, de->deu, es->spa, zh->chi_sim, ja->jpn
   */
  private normalizeLanguage(lang: string): string {
    const languageMap: Record<string, string> = {
      en: "eng",
      fr: "fra",
      de: "deu",
      es: "spa",
      it: "ita",
      pt: "por",
      ru: "rus",
      zh: "chi_sim",
      "zh-cn": "chi_sim",
      "zh-tw": "chi_tra",
      ja: "jpn",
      ko: "kor",
      ar: "ara",
      hi: "hin",
      th: "tha",
      vi: "vie",
    };

    const normalized = lang.toLowerCase().trim();
    return languageMap[normalized] || normalized;
  }
}
