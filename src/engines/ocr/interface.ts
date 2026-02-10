export interface OcrEngine {
  name: string;
  recognize(imagePath: string, options: OcrOptions): Promise<OcrResult[]>;
  recognizeBatch(
    imagePaths: string[],
    options: OcrOptions
  ): Promise<OcrResult[][]>;
}

export interface OcrOptions {
  language: string | string[];
  correctRotation?: boolean;
}

export interface OcrResult {
  text: string;
  bbox: [number, number, number, number];
  confidence: number;
}
