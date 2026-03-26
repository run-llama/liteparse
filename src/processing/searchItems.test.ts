import { describe, expect, it } from "vitest";
import { searchItems } from "./searchItems";
import { JsonTextItem } from "../core/types";

function item(text: string, x: number, y: number, width: number, height = 12): JsonTextItem {
  return { text, x, y, width, height };
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

  it("merges bounding boxes vertically for wrapped phrases", () => {
    const items = [item("temperature", 10, 50, 80, 12), item("range", 10, 65, 40, 12)];
    const results = searchItems(items, { phrase: "temperature range" });
    expect(results).toHaveLength(1);
    expect(results[0].y).toBe(50);
    expect(results[0].height).toBe(27); // 65 + 12 - 50
  });
});
