import { describe, it, expect } from "vitest";
import { buildJSON, formatJSON } from "./json";

const results = [
  { text: "Hello World", bbox: [10, 20, 200, 40], confidence: 0.98 },
  { text: "Sample text", bbox: [10, 50, 180, 70], confidence: 0.85 },
  { text: "Page footer", bbox: [10, 750, 300, 770], confidence: 0.76 },
];

const textItems = results
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

const textItemsJSON = results.map((r) => ({
  text: r.text,
  x: r.bbox[0],
  y: r.bbox[1],
  width: r.bbox[2] - r.bbox[0],
  height: r.bbox[3] - r.bbox[1],
  fontName: "OCR",
  fontSize: r.bbox[3] - r.bbox[1],
  confidence: 1.0,
}));

const pages = [
  {
    pageNum: 1,
    width: 612,
    height: 792,
    text: "Sample text for page 1",
    textItems: textItems,
    boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
  },
  {
    pageNum: 2,
    width: 612,
    height: 792,
    text: "Sample text for page 2",
    textItems: textItems,
    boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
  },
  {
    pageNum: 3,
    width: 612,
    height: 792,
    text: "Sample text for page 3",
    textItems: textItems,
    boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
  },
  {
    pageNum: 4,
    width: 612,
    height: 792,
    text: "Sample text for page 4",
    textItems: textItems,
    boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
  },
  {
    pageNum: 5,
    width: 612,
    height: 792,
    text: "Sample text for page 5",
    textItems: textItems,
    boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
  },
];

// Native PDF text item (confidence defaults to 1.0)
const nativeTextItem = {
  str: "Native text",
  x: 10,
  y: 20,
  width: 100,
  height: 15,
  w: 100,
  h: 15,
  fontName: "Helvetica",
  fontSize: 12,
  confidence: 1.0,
};

// OCR text item (confidence from OCR engine)
const ocrTextItem = {
  str: "OCR detected text",
  x: 10,
  y: 50,
  width: 150,
  height: 20,
  w: 150,
  h: 20,
  fontName: "OCR",
  fontSize: 20,
  confidence: 0.95,
};

const mixedPage = {
  pageNum: 1,
  width: 612,
  height: 792,
  text: "Native text\nOCR detected text",
  textItems: [nativeTextItem, ocrTextItem],
  boundingBoxes: [],
};

const pagesJSON = {
  pages: [
    {
      page: 1,
      width: 612,
      height: 792,
      text: "Sample text for page 1",
      textItems: textItemsJSON,
      boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
    },
    {
      page: 2,
      width: 612,
      height: 792,
      text: "Sample text for page 2",
      textItems: textItemsJSON,
      boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
    },
    {
      page: 3,
      width: 612,
      height: 792,
      text: "Sample text for page 3",
      textItems: textItemsJSON,
      boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
    },
    {
      page: 4,
      width: 612,
      height: 792,
      text: "Sample text for page 4",
      textItems: textItemsJSON,
      boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
    },
    {
      page: 5,
      width: 612,
      height: 792,
      text: "Sample text for page 5",
      textItems: textItemsJSON,
      boundingBoxes: [{ x1: 0, y1: 0, x2: 300, y2: 400 }],
    },
  ],
};

const parseResult = {
  pages: pages,
  text: "hello world",
  json: undefined,
};

describe("test json utilities", () => {
  it("test buildJSON", () => {
    const result = buildJSON(pages);
    expect(result).toStrictEqual(pagesJSON);
  });

  it("test formatJSON", () => {
    const result = formatJSON(parseResult);
    expect(result).toBe(JSON.stringify(pagesJSON, null, 2));
  });
});

describe("confidence field", () => {
  it("includes OCR confidence score for OCR items", () => {
    const result = buildJSON([mixedPage]);
    const items = result.pages[0].textItems;
    const ocrItem = items.find((i) => i.text === "OCR detected text");
    expect(ocrItem?.confidence).toBe(0.95);
  });

  it("defaults to 1.0 for native PDF items", () => {
    const result = buildJSON([mixedPage]);
    const items = result.pages[0].textItems;
    const nativeItem = items.find((i) => i.text === "Native text");
    expect(nativeItem?.confidence).toBe(1.0);
  });

  it("preserves confidence of 0.0", () => {
    const zeroConfidenceItem = { ...ocrTextItem, confidence: 0.0 };
    const page = { ...mixedPage, textItems: [zeroConfidenceItem] };
    const result = buildJSON([page]);
    const item = result.pages[0].textItems[0];
    expect(item.confidence).toBe(0.0);
  });

  it("defaults to 1.0 when confidence is undefined", () => {
    const { confidence: _confidence, ...itemWithoutConfidence } = nativeTextItem;
    const page = { ...mixedPage, textItems: [itemWithoutConfidence] };
    const result = buildJSON([page]);
    const item = result.pages[0].textItems[0];
    expect(item.confidence).toBe(1.0);
  });
});

describe("textLines in JSON output", () => {
  it("includes textLines when present on page", () => {
    const pageWithTextLines = {
      ...mixedPage,
      textLines: [
        {
          text: "Native text",
          bbox: { x: 10, y: 20, w: 100, h: 15 },
          pageNum: 1,
        },
        {
          text: "OCR detected text",
          bbox: { x: 10, y: 50, w: 150, h: 20 },
          lineNumber: 5,
          pageNum: 1,
        },
      ],
    };
    const result = buildJSON([pageWithTextLines]);
    expect(result.pages[0].textLines).toBeDefined();
    expect(result.pages[0].textLines!.length).toBe(2);
    expect(result.pages[0].textLines![0]).toStrictEqual({
      text: "Native text",
      x: 10,
      y: 20,
      width: 100,
      height: 15,
      lineNumber: undefined,
    });
    expect(result.pages[0].textLines![1]).toStrictEqual({
      text: "OCR detected text",
      x: 10,
      y: 50,
      width: 150,
      height: 20,
      lineNumber: 5,
    });
  });

  it("omits textLines when not present on page", () => {
    const result = buildJSON([mixedPage]);
    expect(result.pages[0].textLines).toBeUndefined();
  });
});
