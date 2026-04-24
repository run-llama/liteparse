// Browser stub — HTTP OCR engine is not available in the browser build.
import type { OcrEngine, OcrOptions, OcrResult } from "../../src/engines/ocr/interface.js";

export class HttpOcrEngine implements OcrEngine {
  name = "http-ocr-browser-stub";
  constructor(_serverUrl: string) {
    throw new Error("HttpOcrEngine is not available in the browser build.");
  }
  async recognize(_image: unknown, _options: OcrOptions): Promise<OcrResult[]> {
    return [];
  }
  async recognizeBatch(_images: unknown[], _options: OcrOptions): Promise<OcrResult[][]> {
    return [];
  }
}
