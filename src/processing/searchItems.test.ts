import { describe, expect, it } from "vitest";
import { searchItems, searchTextLines } from "./searchItems";
import { JsonTextItem, TextLine } from "../core/types";

function item(
  text: string,
  x: number,
  y: number,
  width: number,
  height = 12,
  fontSize = 12
): JsonTextItem {
  return { text, x, y, width, height, fontSize };
}

describe("searchItems", () => {
  it("matches a phrase within a single item", () => {
    const items = [item("hello world", 10, 20, 100)];
    const results = searchItems(items, { phrase: "hello world" });
    expect(results).toHaveLength(1);
    expect(results[0].text).toBe("hello world");
    expect(results[0].x).toBe(10);
    expect(results[0].width).toBe(100);
  });

  it("matches a phrase spanning multiple items", () => {
    const items = [item("0°C", 10, 50, 30), item("to", 45, 50, 15), item("70°C", 65, 50, 35)];
    const results = searchItems(items, { phrase: "0°C to 70°C" });
    expect(results).toHaveLength(1);
    expect(results[0].text).toBe("0°C to 70°C");
    expect(results[0].x).toBe(10);
    expect(results[0].width).toBe(90); // 65 + 35 - 10
  });

  it("narrows match and does not include unrelated leading items", () => {
    const items = [item("Operating", 10, 50, 70), item("0°C to 70°C", 85, 50, 90)];
    const results = searchItems(items, { phrase: "0°C to 70°C" });
    expect(results).toHaveLength(1);
    expect(results[0].x).toBe(85);
    expect(results[0].width).toBe(90);
  });

  it("is case-insensitive by default", () => {
    const items = [item("Revenue Grew", 10, 20, 100)];
    const results = searchItems(items, { phrase: "revenue grew" });
    expect(results).toHaveLength(1);
    expect(results[0].text).toBe("revenue grew");
  });

  it("respects caseSensitive option", () => {
    const items = [item("pH Level", 10, 20, 80)];
    expect(searchItems(items, { phrase: "pH", caseSensitive: true })).toHaveLength(1);
    expect(searchItems(items, { phrase: "ph", caseSensitive: true })).toHaveLength(0);
    expect(searchItems(items, { phrase: "PH", caseSensitive: true })).toHaveLength(0);
  });

  it("returns empty array when no match", () => {
    const items = [item("hello", 10, 20, 50)];
    const results = searchItems(items, { phrase: "goodbye" });
    expect(results).toHaveLength(0);
  });

  it("matches spatially adjacent items without inserting a space", () => {
    // "29-CA-" and "261755" are adjacent (gap=0), so joined as "29-CA-261755"
    // "Case No." and "29-CA-" have a word gap (gap=5 > tolerance), so space inserted
    const items = [
      item("Case No.", 10, 50, 60),
      item("29-CA-", 75, 50, 50),
      item("261755", 125, 50, 50),
    ];
    const results = searchItems(items, { phrase: "Case No. 29-CA-261755" });
    expect(results).toHaveLength(1);
    expect(results[0].text).toBe("Case No. 29-CA-261755");
  });

  it("matches adjacent items with en-dash", () => {
    // "pages 10–" and "20" are adjacent (gap=0)
    const items = [item("pages 10\u2013", 10, 50, 60), item("20", 70, 50, 20)];
    const results = searchItems(items, { phrase: "pages 10\u201320" });
    expect(results).toHaveLength(1);
  });

  it("narrows correctly past adjacent items", () => {
    // "prefix" has word gap to "29-CA-", "29-CA-" is adjacent to "261755"
    const items = [
      item("prefix", 10, 50, 40),
      item("29-CA-", 55, 50, 50),
      item("261755", 105, 50, 50),
    ];
    const results = searchItems(items, { phrase: "29-CA-261755" });
    expect(results).toHaveLength(1);
    expect(results[0].x).toBe(55);
  });

  it("merges bounding boxes vertically for wrapped phrases", () => {
    const items = [item("temperature", 10, 50, 80, 12), item("range", 10, 65, 40, 12)];
    const results = searchItems(items, { phrase: "temperature range" });
    expect(results).toHaveLength(1);
    expect(results[0].y).toBe(50);
    expect(results[0].height).toBe(27); // 65 + 12 - 50
  });
});

function textLine(text: string, x: number, y: number, w: number, h: number, pageNum = 1): TextLine {
  return { text, bbox: { x, y, w, h }, pageNum };
}

describe("searchTextLines", () => {
  it("matches a phrase within a single line", () => {
    const lines = [
      textLine("The quick brown fox", 10, 100, 200, 12),
      textLine("jumps over the lazy dog", 10, 115, 250, 12),
    ];
    const results = searchTextLines(lines, { phrase: "quick brown" });
    expect(results).toHaveLength(1);
    expect(results[0].text).toBe("The quick brown fox");
    expect(results[0].bbox).toStrictEqual({ x: 10, y: 100, w: 200, h: 12 });
  });

  it("matches a phrase spanning multiple consecutive lines", () => {
    const lines = [
      textLine("This agreement shall be", 10, 100, 300, 12),
      textLine("governed by the laws of", 10, 115, 300, 12),
      textLine("the State of California.", 10, 130, 300, 12),
    ];
    // Phrase spans lines 1 and 2 (joined with \n)
    const results = searchTextLines(lines, { phrase: "shall be\ngoverned by" });
    expect(results).toHaveLength(2);
    expect(results[0].text).toBe("This agreement shall be");
    expect(results[1].text).toBe("governed by the laws of");
    // Each result carries its own bbox
    expect(results[0].bbox.y).toBe(100);
    expect(results[1].bbox.y).toBe(115);
  });

  it("matches a phrase spanning three lines", () => {
    const lines = [
      textLine("Line one content", 10, 100, 200, 12),
      textLine("Line two content", 10, 115, 200, 12),
      textLine("Line three content", 10, 130, 200, 12),
    ];
    const results = searchTextLines(lines, {
      phrase: "one content\nLine two content\nLine three",
    });
    expect(results).toHaveLength(3);
  });

  it("returns empty array when no match", () => {
    const lines = [textLine("hello world", 10, 100, 100, 12)];
    const results = searchTextLines(lines, { phrase: "goodbye" });
    expect(results).toHaveLength(0);
  });

  it("is case-insensitive by default", () => {
    const lines = [textLine("Revenue Growth Report", 10, 100, 200, 12)];
    const results = searchTextLines(lines, { phrase: "revenue growth" });
    expect(results).toHaveLength(1);
  });

  it("respects caseSensitive option", () => {
    const lines = [textLine("pH Level measurement", 10, 100, 200, 12)];
    expect(searchTextLines(lines, { phrase: "pH", caseSensitive: true })).toHaveLength(1);
    expect(searchTextLines(lines, { phrase: "ph", caseSensitive: true })).toHaveLength(0);
    expect(searchTextLines(lines, { phrase: "PH", caseSensitive: true })).toHaveLength(0);
  });

  it("finds multiple occurrences across different lines", () => {
    const lines = [
      textLine("The defendant argued that", 10, 100, 300, 12),
      textLine("the evidence was insufficient.", 10, 115, 300, 12),
      textLine("The plaintiff argued that", 10, 145, 300, 12),
      textLine("the evidence was sufficient.", 10, 160, 300, 12),
    ];
    const results = searchTextLines(lines, { phrase: "argued that" });
    expect(results).toHaveLength(2);
    expect(results[0].bbox.y).toBe(100);
    expect(results[1].bbox.y).toBe(145);
  });

  it("preserves lineNumber on matched lines", () => {
    const line1 = textLine("Section 1. Definitions.", 80, 100, 300, 12);
    line1.lineNumber = 1;
    const line2 = textLine("Section 2. Obligations.", 80, 115, 300, 12);
    line2.lineNumber = 2;

    const results = searchTextLines([line1, line2], { phrase: "Obligations" });
    expect(results).toHaveLength(1);
    expect(results[0].lineNumber).toBe(2);
  });
});
