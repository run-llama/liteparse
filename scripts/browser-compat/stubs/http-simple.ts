// Browser stub — HTTP OCR is not available, use Tesseract.js instead.
export class HttpOcrEngine {
  name = "http-ocr";
  constructor() {
    throw new Error("HTTP OCR engine is not available in browser environments.");
  }
  async recognize() { throw new Error("Not available in browser."); }
  async recognizeBatch() { throw new Error("Not available in browser."); }
}
