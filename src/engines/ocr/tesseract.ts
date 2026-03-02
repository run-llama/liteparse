import { createWorker, createScheduler, Scheduler, Worker } from "tesseract.js";
import { OcrEngine, OcrOptions, OcrResult } from "./interface.js";

export class TesseractEngine implements OcrEngine {
  name = "tesseract";
  private scheduler?: Scheduler;
  private workers: Worker[] = [];
  private currentLanguage?: string;
  private concurrency: number;

  constructor(concurrency: number = 4) {
    this.concurrency = concurrency;
  }

  async initialize(language: string = "eng"): Promise<void> {
    if (this.scheduler && this.currentLanguage === language) {
      return; // Already initialized for this language
    }

    // Clean up existing scheduler and workers if language changed
    await this.terminate();

    // Create scheduler
    this.scheduler = createScheduler();

    // Create worker pool
    for (let i = 0; i < this.concurrency; i++) {
      const worker = await createWorker(language, 1);
      if (!worker) {
        throw new Error("Tesseract worker not initialized");
      }
      this.workers.push(worker);
      this.scheduler.addWorker(worker);
    }

    this.currentLanguage = language;
  }

  async recognize(imagePath: string, options: OcrOptions): Promise<OcrResult[]> {
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
      // In tesseract.js v6+, we need to enable blocks output to get word-level data
      const {
        data: { blocks },
      } = await this.scheduler.addJob("recognize", imagePath, {}, { blocks: true });

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
      console.error(`\nTesseract OCR error for ${imagePath}:`, error);
      return [];
    }
  }

  async recognizeBatch(imagePaths: string[], options: OcrOptions): Promise<OcrResult[][]> {
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
    const jobs = imagePaths.map((imagePath) => this.recognize(imagePath, options));

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
