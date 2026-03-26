import { vi, describe, it, expect } from "vitest";

// In tesseract.js v6+, words are nested in blocks → paragraphs → lines → words
const mockWords = [
  {
    text: "Hello",
    confidence: 95,
    bbox: { x0: 0, y0: 0, x1: 50, y1: 20 },
  },
  {
    text: "World",
    confidence: 92,
    bbox: { x0: 60, y0: 0, x1: 120, y1: 20 },
  },
];

const mockTesseractResult = {
  data: {
    text: "Hello World",
    blocks: [
      {
        paragraphs: [
          {
            lines: [
              {
                words: mockWords,
              },
            ],
          },
        ],
      },
    ],
    confidence: 93,
  },
};

const mockResults = mockWords.map((word) => ({
  text: word.text,
  bbox: [word.bbox.x0, word.bbox.y0, word.bbox.x1, word.bbox.y1] as [
    number,
    number,
    number,
    number,
  ],
  confidence: word.confidence / 100, // Tesseract returns 0-100, we want 0-1
}));

const mockTesseractWorker = {
  terminate: vi.fn(async () => {}),
  recognize: vi.fn(async () => {
    return mockTesseractResult;
  }),
};

vi.mock("tesseract.js", async () => {
  const actual = await vi.importActual<typeof import("tesseract.js")>("tesseract.js");
  return {
    ...actual,
    createWorker: vi.fn(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      async (language: string, _num: number, options?: { errorHandler?: (arg: any) => void }) => {
        if (language == "it" || language == "ita") {
          return;
        }
        if (language == "offline" || language == "fetchfail") {
          options?.errorHandler?.("TypeError: fetch failed");
          throw new Error("TypeError: fetch failed");
        }
        return mockTesseractWorker;
      }
    ),
  };
});

import { TesseractEngine } from "./tesseract";

describe("test Tesseract OCR (single image)", () => {
  it("test engine success", async () => {
    const engine = new TesseractEngine();
    expect(engine.name).toBe("tesseract");
    const result = await engine.recognize("cat.png", { language: "en" });
    expect(result).toStrictEqual(mockResults);
  });

  it("test engine failure (failed to initialize)", async () => {
    const engine = new TesseractEngine();
    expect(engine.name).toBe("tesseract");
    await expect(engine.recognize("cat.png", { language: "it" })).rejects.toThrow(
      "Tesseract worker not initialized"
    );
  });

  it("test engine failure (fetch failed) returns actionable guidance", async () => {
    const engine = new TesseractEngine();
    await expect(engine.recognize("cat.png", { language: "offline" })).rejects.toThrow(
      'Tesseract failed to download language data for "offline"'
    );
  });
});

describe("test OCR simple HTTP server (batch)", () => {
  it("test engine success", async () => {
    const engine = new TesseractEngine();
    expect(engine.name).toBe("tesseract");
    const result = await engine.recognizeBatch(["cat.png", "dog.png"], { language: "en" });
    expect(result).toStrictEqual([mockResults, mockResults]);
  });

  it("test engine failure (failed to initialize)", async () => {
    const engine = new TesseractEngine();
    expect(engine.name).toBe("tesseract");
    await expect(engine.recognizeBatch(["cat.png", "dog.png"], { language: "it" })).rejects.toThrow(
      "Tesseract worker not initialized"
    );
  });
});
