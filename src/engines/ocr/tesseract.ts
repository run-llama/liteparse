import { createWorker, Worker } from 'tesseract.js';
import { OcrEngine, OcrOptions, OcrResult } from './interface.js';


export class TesseractEngine implements OcrEngine {
  name = 'tesseract';
  private worker?: Worker;
  private currentLanguage?: string;

  async initialize(language: string = 'eng'): Promise<void> {
    if (this.worker && this.currentLanguage === language) {
      return; // Already initialized for this language
    }

    // Clean up existing worker if language changed
    if (this.worker) {
      await this.worker.terminate();
    }

    console.log(`Initializing Tesseract OCR for language: ${language}`);
    this.worker = await createWorker(language, 1, {
      logger: (m) => {
        // Only log progress, not every detail
        if (m.status === 'recognizing text') {
          process.stdout.write(`\rOCR Progress: ${Math.round(m.progress * 100)}%`);
        }
      },
    });
    this.currentLanguage = language;
  }

  async recognize(
    imagePath: string,
    options: OcrOptions
  ): Promise<OcrResult[]> {
    // Handle language - tesseract.js uses language codes like 'eng', 'fra', 'deu'
    const language = this.normalizeLanguage(
      Array.isArray(options.language) ? options.language[0] : options.language
    );

    // Initialize worker if needed
    await this.initialize(language);

    if (!this.worker) {
      throw new Error('Tesseract worker not initialized');
    }

    try {
      // Recognize text from image
      const {
        data: { words },
      } = await this.worker.recognize(imagePath);

      // Convert to our OcrResult format
      const results: OcrResult[] = words.map((word) => ({
        text: word.text,
        bbox: [
          word.bbox.x0,
          word.bbox.y0,
          word.bbox.x1,
          word.bbox.y1,
        ] as [number, number, number, number],
        confidence: word.confidence / 100, // Tesseract returns 0-100, we want 0-1
      }));

      // Filter out low confidence results (below 30%)
      return results.filter((r) => r.confidence > 0.3);
    } catch (error) {
      console.error(`\nTesseract OCR error for ${imagePath}:`, error);
      return [];
    }
  }

  async recognizeBatch(
    imagePaths: string[],
    options: OcrOptions
  ): Promise<OcrResult[][]> {
    const results: OcrResult[][] = [];

    console.log(`\nProcessing ${imagePaths.length} images with Tesseract OCR...`);

    for (let i = 0; i < imagePaths.length; i++) {
      const imagePath = imagePaths[i];
      console.log(`\nProcessing image ${i + 1}/${imagePaths.length}: ${imagePath}`);
      const result = await this.recognize(imagePath, options);
      results.push(result);
    }

    console.log('\n'); // Clear progress line
    return results;
  }

  async terminate(): Promise<void> {
    if (this.worker) {
      await this.worker.terminate();
      this.worker = undefined;
      this.currentLanguage = undefined;
    }
  }

  /**
   * Normalize language codes to Tesseract format
   * Common mappings: en->eng, fr->fra, de->deu, es->spa, zh->chi_sim, ja->jpn
   */
  private normalizeLanguage(lang: string): string {
    const languageMap: Record<string, string> = {
      en: 'eng',
      fr: 'fra',
      de: 'deu',
      es: 'spa',
      it: 'ita',
      pt: 'por',
      ru: 'rus',
      zh: 'chi_sim',
      'zh-cn': 'chi_sim',
      'zh-tw': 'chi_tra',
      ja: 'jpn',
      ko: 'kor',
      ar: 'ara',
      hi: 'hin',
      th: 'tha',
      vi: 'vie',
    };

    const normalized = lang.toLowerCase().trim();
    return languageMap[normalized] || normalized;
  }
}
