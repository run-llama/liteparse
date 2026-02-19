import { vi, describe, it, expect } from "vitest";
import { PassThrough } from "stream";

const mockStream = new PassThrough();
const mockOcrItems = [
  {
    text: "Hello World",
    bbox: [0, 0, 100, 20],
    confidence: 0.99,
  },
  {
    text: "Some text",
    bbox: [0, 25, 100, 45],
    confidence: 0.95,
  },
];

vi.mock("fs", async () => {
  const actual = await vi.importActual<typeof import("fs")>("fs");
  return {
    ...actual,
    default: {
      createReadStream: vi.fn(() => {
        return mockStream;
      }),
    },
  };
});

vi.mock("axios", async () => {
  const actual = await vi.importActual<typeof import("axios")>("axios");
  return {
    default: {
      ...actual,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      post: vi.fn(async (url: string, _formData?: any | undefined, _config?: any | undefined) => {
        if (url == "http://localhost:9999") {
          throw new actual.AxiosError("Endpoint not found", "404 Not Found");
        } else if (url == "http://localhost:10000") {
          throw new Error("Unrecheable server");
        } else if (url == "http://localhost:9998") {
          // not an array
          return {
            data: { results: mockOcrItems[0] },
          };
        }
        return {
          data: { results: mockOcrItems },
        };
      }),
    },
  };
});

import { HttpOcrEngine } from "./http-simple";

describe("test OCR simple HTTP server (single image)", () => {
  it("test server success", async () => {
    const server = new HttpOcrEngine("http://localhost:8000");
    expect(server.name).toBe("http-ocr");
    const result = await server.recognize("cat.png", { language: "en" });
    expect(result).toStrictEqual(mockOcrItems);
  });

  it("test server axios error", async () => {
    const server = new HttpOcrEngine("http://localhost:9999");
    const result = await server.recognize("cat.png", { language: "en" });
    expect(result.length).toBe(0);
  });

  it("test server generic error", async () => {
    const server = new HttpOcrEngine("http://localhost:10000");
    const result = await server.recognize("cat.png", { language: "en" });
    expect(result.length).toBe(0);
  });

  it("test server malformed response", async () => {
    const server = new HttpOcrEngine("http://localhost:9998");
    const result = await server.recognize("cat.png", { language: "en" });
    expect(result.length).toBe(0);
  });
});

describe("test OCR simple HTTP server (batch)", () => {
  it("test server success", async () => {
    const server = new HttpOcrEngine("http://localhost:8000");
    const result = await server.recognizeBatch(["cat.png", "dog.png"], { language: "en" });
    expect(result).toStrictEqual([mockOcrItems, mockOcrItems]);
  });

  it("test server axios error", async () => {
    const server = new HttpOcrEngine("http://localhost:9999");
    const result = await server.recognizeBatch(["cat.png", "dog.png"], { language: "en" });
    expect(result.length).toBe(2);
    for (const r of result) {
      expect(r.length).toBe(0);
    }
  });

  it("test server generic error", async () => {
    const server = new HttpOcrEngine("http://localhost:10000");
    const result = await server.recognizeBatch(["cat.png", "dog.png"], { language: "en" });
    expect(result.length).toBe(2);
    for (const r of result) {
      expect(r.length).toBe(0);
    }
  });

  it("test server malformed response", async () => {
    const server = new HttpOcrEngine("http://localhost:9998");
    const result = await server.recognizeBatch(["cat.png", "dog.png"], { language: "en" });
    expect(result.length).toBe(2);
    for (const r of result) {
      expect(r.length).toBe(0);
    }
  });
});
