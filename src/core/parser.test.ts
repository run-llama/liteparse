import { vi, describe, it, expect } from "vitest";

const {
  mockPages,
  mockPdfDocument,
  mockParsedPages,
  mockBoundingBoxes,
  mockPdfConvertedResult,
  mockOcrResults,
  mockParseResultJson,
  mockParsedPagesOcr,
  mockParseResultJsonOcr,
  mockTextItems,
  mockParsedPagesWithBoundingBoxes,
  mockParsedPagesWithBoundingBoxesOcr,
  mockScreenshotResults,
} = vi.hoisted(() => {
  const mockPdfConvertedResult = {
    pdfPath: "/tmp/converted.pdf",
    originalExtension: ".docx",
  };

  const mockPdfDocument = {
    numPages: 5,
    data: new TextEncoder().encode("hello world"),
  };

  const mockPages = [
    {
      pageNum: 1,
      width: 612,
      height: 792,
      textItems: [],
      paths: [],
      images: [],
      annotations: [],
    },
    {
      pageNum: 2,
      width: 612,
      height: 792,
      textItems: [],
      paths: [],
      images: [],
    },
    {
      pageNum: 3,
      width: 612,
      height: 792,
      textItems: [],
      paths: [],
      images: [],
      annotations: [],
    },
    {
      pageNum: 4,
      width: 612,
      height: 792,
      textItems: [],
      paths: [],
      images: [],
    },
    {
      pageNum: 5,
      width: 612,
      height: 792,
      textItems: [],
      paths: [],
      images: [],
      annotations: [],
    },
  ];

  const mockBoundingBoxes = [{ x1: 0, y1: 0, x2: 300, y2: 400 }];

  const mockParsedPages = [
    {
      pageNum: 1,
      width: 612,
      height: 792,
      text: "Sample text for page 1",
      textItems: [],
      boundingBoxes: undefined,
    },
    {
      pageNum: 2,
      width: 612,
      height: 792,
      text: "Sample text for page 2",
      textItems: [],
    },
    {
      pageNum: 3,
      width: 612,
      height: 792,
      text: "Sample text for page 3",
      textItems: [],
      boundingBoxes: undefined,
    },
    {
      pageNum: 4,
      width: 612,
      height: 792,
      text: "Sample text for page 4",
      textItems: [],
    },
    {
      pageNum: 5,
      width: 612,
      height: 792,
      text: "Sample text for page 5",
      textItems: [],
      boundingBoxes: undefined,
    },
  ];

  const mockParsedPagesWithBoundingBoxes = mockParsedPages.map((page) => ({
    ...page,
    boundingBoxes: mockBoundingBoxes,
  }));

  const mockOcrResults = [
    { text: "Hello World", bbox: [10, 20, 200, 40], confidence: 0.98 },
    { text: "Sample text", bbox: [10, 50, 180, 70], confidence: 0.85 },
    { text: "Page footer", bbox: [10, 750, 300, 770], confidence: 0.76 },
  ];

  const mockTextItems = mockOcrResults
    .filter((r) => r.confidence > 0.1) // Filter low confidence
    .filter((r) => {
      // Filter out OCR text that already exists in native PDF text
      const ocrText = r.text.trim().toLowerCase();
      return ocrText.length > 0;
    })
    .map((r) => ({
      str: r.text,
      x: r.bbox[0],
      y: r.bbox[1],
      width: r.bbox[2] - r.bbox[0],
      height: r.bbox[3] - r.bbox[1],
      w: r.bbox[2] - r.bbox[0],
      h: r.bbox[3] - r.bbox[1],
      fontName: "OCR",
      fontSize: r.bbox[3] - r.bbox[1],
    }));

  const mockTextItemsJson = mockOcrResults.map((r) => ({
    text: r.text,
    x: r.bbox[0],
    y: r.bbox[1],
    width: r.bbox[2] - r.bbox[0],
    height: r.bbox[3] - r.bbox[1],
    fontName: "OCR",
    fontSize: r.bbox[3] - r.bbox[1],
  }));

  const mockParsedPagesOcr = [
    {
      pageNum: 1,
      width: 612,
      height: 792,
      text: "Sample text for page 1",
      textItems: mockTextItems,
      boundingBoxes: undefined,
    },
    {
      pageNum: 2,
      width: 612,
      height: 792,
      text: "Sample text for page 2",
      textItems: mockTextItems,
    },
    {
      pageNum: 3,
      width: 612,
      height: 792,
      text: "Sample text for page 3",
      textItems: mockTextItems,
      boundingBoxes: undefined,
    },
    {
      pageNum: 4,
      width: 612,
      height: 792,
      text: "Sample text for page 4",
      textItems: mockTextItems,
    },
    {
      pageNum: 5,
      width: 612,
      height: 792,
      text: "Sample text for page 5",
      textItems: mockTextItems,
      boundingBoxes: undefined,
    },
  ];

  const mockParsedPagesWithBoundingBoxesOcr = mockParsedPagesOcr.map((page) => ({
    ...page,
    boundingBoxes: mockBoundingBoxes,
  }));

  const mockParseResultJson = {
    pages: [
      {
        page: 1,
        width: 612,
        height: 792,
        text: "Sample text for page 1",
        textItems: [],
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 2,
        width: 612,
        height: 792,
        text: "Sample text for page 2",
        textItems: [],
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 3,
        width: 612,
        height: 792,
        text: "Sample text for page 3",
        textItems: [],
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 4,
        width: 612,
        height: 792,
        text: "Sample text for page 4",
        textItems: [],
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 5,
        width: 612,
        height: 792,
        text: "Sample text for page 5",
        textItems: [],
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
    ],
  };

  const mockParseResultJsonOcr = {
    pages: [
      {
        page: 1,
        width: 612,
        height: 792,
        text: "Sample text for page 1",
        textItems: mockTextItemsJson,
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 2,
        width: 612,
        height: 792,
        text: "Sample text for page 2",
        textItems: mockTextItemsJson,
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 3,
        width: 612,
        height: 792,
        text: "Sample text for page 3",
        textItems: mockTextItemsJson,
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 4,
        width: 612,
        height: 792,
        text: "Sample text for page 4",
        textItems: mockTextItemsJson,
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
      {
        page: 5,
        width: 612,
        height: 792,
        text: "Sample text for page 5",
        textItems: mockTextItemsJson,
        boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
      },
    ],
  };

  const mockImageBuffer = Buffer.from(new Uint8Array([1, 2, 3, 4, 5]));
  const mockScreenshotResults: ScreenshotResult[] = [
    {
      pageNum: 1,
      width: 612,
      height: 792,
      imageBuffer: mockImageBuffer,
    },
    {
      pageNum: 2,
      width: 612,
      height: 792,
      imageBuffer: mockImageBuffer,
    },
    {
      pageNum: 3,
      width: 612,
      height: 792,
      imageBuffer: mockImageBuffer,
    },
    {
      pageNum: 4,
      width: 612,
      height: 792,
      imageBuffer: mockImageBuffer,
    },
    {
      pageNum: 5,
      width: 612,
      height: 792,
      imageBuffer: mockImageBuffer,
    },
  ];

  return {
    mockPdfConvertedResult,
    mockPdfDocument,
    mockPages,
    mockParsedPages,
    mockBoundingBoxes,
    mockOcrResults,
    mockParseResultJson,
    mockParsedPagesOcr,
    mockParseResultJsonOcr,
    mockTextItems,
    mockParsedPagesWithBoundingBoxes,
    mockParsedPagesWithBoundingBoxesOcr,
    mockScreenshotResults,
  };
});

const { mockRemotePdfBytes, mockRemoteDocBytes } = vi.hoisted(() => ({
  mockRemotePdfBytes: new Uint8Array([0x25, 0x50, 0x44, 0x46]),
  mockRemoteDocBytes: new Uint8Array([0x50, 0x4b, 0x03, 0x04]),
}));

import { LiteParse } from "./parser";
import { LiteParseConfig, ScreenshotResult } from "./types";

vi.mock("axios", async () => {
  const actual = await vi.importActual<typeof import("axios")>("axios");
  return {
    default: {
      ...actual,
      get: vi.fn(async (url: string) => {
        if (url === "https://example.com/document.pdf") {
          return { data: mockRemotePdfBytes.buffer };
        }
        if (url === "https://example.com/document.docx") {
          return { data: mockRemoteDocBytes.buffer };
        }
        throw new Error("network failure");
      }),
    },
  };
});

vi.mock("../conversion/convertToPdf.js", async () => {
  const actual = await vi.importActual<typeof import("../conversion/convertToPdf.js")>(
    "../conversion/convertToPdf.js"
  );
  return {
    ...actual,
    convertToPdf: vi.fn(async () => {
      return mockPdfConvertedResult;
    }),
    convertBufferToPdf: vi.fn(async () => {
      return mockPdfConvertedResult;
    }),
    cleanupConversionFiles: vi.fn(async () => {}),
  };
});

vi.mock("../engines/pdf/pdfjs.js", async () => {
  const actual =
    await vi.importActual<typeof import("../engines/pdf/pdfjs.js")>("../engines/pdf/pdfjs.js");
  return {
    ...actual,
    PdfJsEngine: vi.fn(
      class {
        constructor() {}

        loadDocument = vi.fn().mockResolvedValue(mockPdfDocument);
        extractAllPages = vi.fn().mockResolvedValue(mockPages);
        extractPage = vi.fn().mockResolvedValue(mockPages[0]);
        renderPageImage = vi.fn(async () => Buffer.from(new Uint8Array([1, 2, 3, 4, 5])));
        close = vi.fn(async () => {});
      }
    ),
  };
});

vi.mock("../engines/ocr/tesseract.js", async () => {
  const actual = await vi.importActual<typeof import("../engines/ocr/tesseract.js")>(
    "../engines/ocr/tesseract.js"
  );
  return {
    ...actual,
    TesseractEngine: vi.fn(
      class {
        constructor() {}

        recognize = vi.fn().mockResolvedValue(mockOcrResults);
      }
    ),
  };
});

vi.mock("../engines/ocr/http-simple.js", async () => {
  const actual = await vi.importActual<typeof import("../engines/ocr/http-simple.js")>(
    "../engines/ocr/http-simple.js"
  );
  return {
    ...actual,
    HttpOcrEngine: vi.fn(
      class {
        private url;

        constructor(url: string) {
          this.url = url;
          if (this.url != ("http://localhost:8000" as string)) {
            // works as an assertion that the URL is being passed to the class
            // at init time
            throw new Error("cannot accept a URL that is not http://localhost:8000");
          }
        }

        recognize = vi.fn().mockResolvedValue(mockOcrResults);
      }
    ),
  };
});

vi.mock("../processing/grid.js", async () => {
  const actual =
    await vi.importActual<typeof import("../processing/grid.js")>("../processing/grid.js");
  return {
    ...actual,
    projectPagesToGrid: vi
      .fn()
      // these get modified, so we need to pass a deep copy of the array instead of the original one
      .mockImplementationOnce(() => structuredClone(mockParsedPages)) // no ocr - text
      .mockImplementationOnce(() => structuredClone(mockParsedPages)) // no ocr - json
      .mockImplementationOnce(() => structuredClone(mockParsedPagesOcr)) // ocr (tesseract) - text
      .mockImplementationOnce(() => structuredClone(mockParsedPagesOcr)) // ocr (tesseract) - json
      .mockImplementationOnce(() => structuredClone(mockParsedPagesOcr)) // ocr (http) - text
      .mockImplementationOnce(() => structuredClone(mockParsedPagesOcr)) // ocr (http) - json
      .mockImplementationOnce(() => structuredClone(mockParsedPages)) //no bounding boxes
      .mockImplementationOnce(() => structuredClone(mockParsedPages)) // remote pdf
      .mockImplementationOnce(() => structuredClone(mockParsedPages)), // remote non-pdf
  };
});

vi.mock("../processing/bbox.js", async () => {
  const actual =
    await vi.importActual<typeof import("../processing/bbox.js")>("../processing/bbox.js");
  return {
    ...actual,
    buildBoundingBoxes: vi.fn().mockReturnValue(mockBoundingBoxes),
  };
});

vi.mock("../engines/pdf/pdfium-renderer.js", async () => {
  const actual = await vi.importActual<typeof import("../engines/pdf/pdfium-renderer.js")>(
    "../engines/pdf/pdfium-renderer.js"
  );
  return {
    ...actual,
    PdfiumRenderer: vi.fn(
      class {
        constructor() {}

        renderPageToBuffer = vi.fn(async () => Buffer.from(new Uint8Array([1, 2, 3, 4, 5])));
        close = vi.fn(async () => {});
      }
    ),
  };
});

describe("Parse tests", () => {
  it("test parse without OCR and text format", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().ocrEnabled).toBeFalsy();
    expect(liteparse.getConfig().outputFormat).toBe("text");
    const result = await liteparse.parse("/tmp/test.docx");
    expect(result.json).toBeUndefined();
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxes);
  });

  it("test parse without OCR and json format", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "json" };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().ocrEnabled).toBeFalsy();
    expect(liteparse.getConfig().outputFormat).toBe("json");
    const result = await liteparse.parse("/tmp/test.docx");
    expect(result.json).toBeDefined();
    expect(result.json).toStrictEqual(mockParseResultJson);
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxes);
  });

  it("test parse with OCR (tesseract) and text format", async () => {
    const config: Partial<LiteParseConfig> = {
      ocrEnabled: true,
      outputFormat: "text",
      ocrServerUrl: undefined,
    };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().ocrEnabled).toBeTruthy();
    expect(liteparse.getConfig().outputFormat).toBe("text");
    expect(liteparse.getConfig().ocrServerUrl).toBeUndefined();
    const result = await liteparse.parse("/tmp/test.docx");
    console.log(result.pages.at(0)?.textItems);
    expect(
      result.pages.filter((page) => {
        return page.textItems.length > 0;
      }).length
    ).toBeGreaterThan(0);
    expect(result.json).toBeUndefined();
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxesOcr);
  });

  it("test parse with OCR (tesseract) and json format", async () => {
    const config: Partial<LiteParseConfig> = {
      ocrEnabled: true,
      outputFormat: "json",
      ocrServerUrl: undefined,
    };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().ocrEnabled).toBeTruthy();
    expect(liteparse.getConfig().outputFormat).toBe("json");
    expect(liteparse.getConfig().ocrServerUrl).toBeUndefined();
    const result = await liteparse.parse("/tmp/test.docx");
    expect(
      result.pages.filter((page) => {
        return page.textItems.length > 0;
      }).length
    ).toBe(result.pages.length);
    expect(result.pages.at(0)!.textItems).toStrictEqual(mockTextItems);
    expect(result.json).toBeDefined();
    expect(result.json).toStrictEqual(mockParseResultJsonOcr);
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxesOcr);
  });

  it("test parse with OCR (http) and text format", async () => {
    const config: Partial<LiteParseConfig> = {
      ocrEnabled: true,
      outputFormat: "text",
      ocrServerUrl: "http://localhost:8000",
    };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().ocrEnabled).toBeTruthy();
    expect(liteparse.getConfig().outputFormat).toBe("text");
    expect(liteparse.getConfig().ocrServerUrl).toBe("http://localhost:8000");
    const result = await liteparse.parse("/tmp/test.docx");
    console.log(result.pages.at(0)?.textItems);
    expect(
      result.pages.filter((page) => {
        return page.textItems.length > 0;
      }).length
    ).toBe(result.pages.length);
    expect(result.pages.at(0)!.textItems).toStrictEqual(mockTextItems);
    expect(result.json).toBeUndefined();
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxesOcr);
  });

  it("test parse with OCR (http) and json format", async () => {
    const config: Partial<LiteParseConfig> = {
      ocrEnabled: true,
      outputFormat: "json",
      ocrServerUrl: "http://localhost:8000",
    };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().ocrEnabled).toBeTruthy();
    expect(liteparse.getConfig().outputFormat).toBe("json");
    expect(liteparse.getConfig().ocrServerUrl).toBe("http://localhost:8000");
    const result = await liteparse.parse("/tmp/test.docx");
    expect(
      result.pages.filter((page) => {
        return page.textItems.length > 0;
      }).length
    ).toBe(result.pages.length);
    expect(result.pages.at(0)!.textItems).toStrictEqual(mockTextItems);
    expect(result.json).toBeDefined();
    expect(result.json).toStrictEqual(mockParseResultJsonOcr);
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxesOcr);
  });

  it("test without bounding boxes", async () => {
    const config: Partial<LiteParseConfig> = {
      ocrEnabled: false,
      outputFormat: "text",
      preciseBoundingBox: false,
    };
    const liteparse = new LiteParse(config);
    expect(liteparse.getConfig().preciseBoundingBox).toBeFalsy();
    const result = await liteparse.parse("/tmp/test.docx");
    console.log(result.pages.at(0)!.boundingBoxes);
    expect(
      result.pages.filter((page) => {
        return typeof page.boundingBoxes != "undefined";
      }).length
    ).toBe(0);
  });

  it("test parse remote pdf url", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    const result = await liteparse.parse("https://example.com/document.pdf");
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxes);
  });

  it("test parse remote non-pdf url through buffer conversion", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    const result = await liteparse.parse("https://example.com/document.docx");
    expect(result.text).toBe(mockParsedPages.map((p) => p.text).join("\n\n"));
    expect(result.pages).toStrictEqual(mockParsedPagesWithBoundingBoxes);
  });

  it("test parse remote url failure surfaces error", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    await expect(liteparse.parse("https://example.com/missing.pdf")).rejects.toThrow(
      "network failure"
    );
  });
});

describe("test screenshot", () => {
  it("test screenshot with all pages", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    const results = await liteparse.screenshot("/tmp/test.docx");
    expect(results.length).toBe(5);
    expect(results).toStrictEqual(mockScreenshotResults);
  });
  it("test screenshot with selected pages", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    const results = await liteparse.screenshot("/tmp/test.docx", [1, 3, 4]);
    expect(results.length).toBe(3);
    expect(results).toStrictEqual([
      mockScreenshotResults[0],
      mockScreenshotResults[2],
      mockScreenshotResults[3],
    ]);
  });

  it("test screenshot with remote pdf url", async () => {
    const config: Partial<LiteParseConfig> = { ocrEnabled: false, outputFormat: "text" };
    const liteparse = new LiteParse(config);
    const results = await liteparse.screenshot("https://example.com/document.pdf", [1, 2]);
    expect(results).toStrictEqual([mockScreenshotResults[0], mockScreenshotResults[1]]);
  });
});
