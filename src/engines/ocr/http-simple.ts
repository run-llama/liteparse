import axios from 'axios';
import FormData from 'form-data';
import fs from 'fs';
import { OcrEngine, OcrOptions, OcrResult } from './interface.js';

/**
 * HTTP-based OCR engine that conforms to LiteParse OCR API specification.
 *
 * The server must implement the API defined in OCR_API_SPEC.md:
 * - POST /ocr endpoint
 * - Accepts multipart/form-data with 'file' and 'language' fields
 * - Returns JSON: { results: [{ text, bbox: [x1,y1,x2,y2], confidence }] }
 *
 * See ocr/easyocr/ and ocr/paddleocr/ for example server implementations.
 */
export class HttpOcrEngine implements OcrEngine {
  name = 'http-ocr';
  private serverUrl: string;

  constructor(serverUrl: string) {
    this.serverUrl = serverUrl;
  }

  async recognize(
    imagePath: string,
    options: OcrOptions
  ): Promise<OcrResult[]> {
    try {
      // Prepare request
      const formData = new FormData();
      formData.append('file', fs.createReadStream(imagePath));

      const language = Array.isArray(options.language)
        ? options.language[0]
        : options.language;
      formData.append('language', language);

      // Make HTTP request
      const response = await axios.post(this.serverUrl, formData, {
        headers: formData.getHeaders(),
        timeout: 60000,
      });

      // Parse response (conforming to OCR_API_SPEC.md)
      const { results } = response.data;

      if (!Array.isArray(results)) {
        console.warn('OCR server response missing results array:', response.data);
        return [];
      }

      console.warn("Got OCR results from server");

      return results.map((item: any) => ({
        text: item.text,
        bbox: item.bbox as [number, number, number, number],
        confidence: item.confidence,
      }));
    } catch (error) {
      if (axios.isAxiosError(error)) {
        console.error(
          `OCR HTTP error for ${imagePath}:`,
          error.response?.status,
          error.response?.data?.error || error.message
        );
      } else {
        console.error(`OCR error for ${imagePath}:`, error);
      }
      return [];
    }
  }

  async recognizeBatch(
    imagePaths: string[],
    options: OcrOptions
  ): Promise<OcrResult[][]> {
    const results: OcrResult[][] = [];
    for (const imagePath of imagePaths) {
      const result = await this.recognize(imagePath, options);
      results.push(result);
    }
    return results;
  }
}
