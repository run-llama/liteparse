import { expect, describe, it } from "vitest";
import { bboxToLine, projectPagesToGrid, projectToGrid } from "./gridProjection";
import { searchTextLines } from "./searchItems";
import { ProjectionTextBox } from "../core/types";
import { DEFAULT_CONFIG } from "../core/config";

describe("test bboxToLine", () => {
  it("test  same line that can merge", () => {
    const textBbox = [
      { str: "Hello", x: 0, y: 10, w: 50, h: 12, strLength: 5 },
      { str: " World", x: 50, y: 10, w: 55, h: 12, strLength: 6 },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [
        {
          str: "Hello World",
          x: 0,
          y: 10,
          w: 105,
          h: 12,
          strLength: 11,
          pageBbox: { x: 0, y: 10, w: 105, h: 12 },
        },
      ],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test same line that cannot merge (gap too large)", () => {
    const textBbox = [
      { str: "Hello", x: 0, y: 10, w: 50, h: 12, strLength: 5 },
      { str: "World", x: 100, y: 10, w: 50, h: 12, strLength: 5 },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [
        { str: "Hello", x: 0, y: 10, w: 50, h: 12, strLength: 5 },
        { str: "World", x: 100, y: 10, w: 50, h: 12, strLength: 5 },
      ],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test boxes on different lines", () => {
    const textBbox = [
      { str: "Line1", x: 0, y: 10, w: 50, h: 12, strLength: 5 },
      { str: "Line2", x: 0, y: 30, w: 50, h: 12, strLength: 5 },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [{ str: "Line1", x: 0, y: 10, w: 50, h: 12, strLength: 5 }],
      [{ str: "Line2", x: 0, y: 30, w: 50, h: 12, strLength: 5 }],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test same y/h but small negative xDelta", () => {
    const textBbox = [
      { str: "AB", x: 0, y: 5, w: 20.3, h: 10, strLength: 2 },
      { str: "CD", x: 20.1, y: 5, w: 20, h: 10, strLength: 2 },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [
        {
          str: "ABCD",
          x: 0,
          y: 5,
          w: 40.1,
          h: 10,
          strLength: 4,
          pageBbox: { x: 0, y: 5, w: 40.1, h: 10 },
        },
      ],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test matching markup that can merge", () => {
    const textBbox = [
      {
        str: "Bold",
        x: 0,
        y: 0,
        w: 40,
        h: 10,
        strLength: 4,
        markup: { highlight: "no", underline: true, squiggly: false, strikeout: false },
      },
      {
        str: "Text",
        x: 40,
        y: 0,
        w: 40,
        h: 10,
        strLength: 4,
        markup: { highlight: "no", underline: true, squiggly: false, strikeout: false },
      },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [
        {
          str: "BoldText",
          x: 0,
          y: 0,
          w: 80,
          h: 10,
          strLength: 8,
          markup: { highlight: "no", underline: true, squiggly: false, strikeout: false },
          pageBbox: { x: 0, y: 0, w: 80, h: 10 },
        },
      ],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test mismatched markup that cannot merge", () => {
    const textBbox = [
      {
        str: "A",
        x: 0,
        y: 0,
        w: 10,
        h: 10,
        strLength: 1,
        markup: { highlight: "yes", underline: false, squiggly: false, strikeout: false },
      },
      {
        str: "B",
        x: 10,
        y: 0,
        w: 10,
        h: 10,
        strLength: 1,
        markup: { highlight: "no", underline: false, squiggly: false, strikeout: false },
      },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [
        {
          str: "A",
          x: 0,
          y: 0,
          w: 10,
          h: 10,
          strLength: 1,
          markup: { highlight: "yes", underline: false, squiggly: false, strikeout: false },
        },
        {
          str: "B",
          x: 10,
          y: 0,
          w: 10,
          h: 10,
          strLength: 1,
          markup: { highlight: "no", underline: false, squiggly: false, strikeout: false },
        },
      ],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test unsorted -> sorted", () => {
    const textBbox = [
      { str: "C", x: 20, y: 0, w: 10, h: 10, strLength: 1 },
      { str: "A", x: 0, y: 0, w: 10, h: 10, strLength: 1 },
      { str: "B", x: 10, y: 0, w: 10, h: 10, strLength: 1 },
    ];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput = [
      [
        {
          str: "ABC",
          x: 0,
          y: 0,
          w: 30,
          h: 10,
          strLength: 3,
          pageBbox: { x: 0, y: 0, w: 30, h: 10 },
        },
      ],
    ];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test empty input", () => {
    const textBbox: ProjectionTextBox[] = [];
    const medianWidth = 10;
    const medianHeight = 12;
    const expectedOutput: ProjectionTextBox[][] = [];

    const result = bboxToLine(textBbox, medianWidth, medianHeight);
    expect(result).toStrictEqual(expectedOutput);
  });
});

describe("test projectToGrid", () => {
  it("test simple single-column text", () => {
    const config = { ...DEFAULT_CONFIG, preserveLayoutAlignmentAcrossPages: false };
    const page = {
      pageNum: 1,
      width: 612,
      height: 792,
      textItems: [],
      images: [],
    };
    const projectionBoxes = [
      { str: "Hello", x: 10, y: 100, w: 50, h: 12, r: 0, strLength: 5 },
      { str: "World", x: 10, y: 115, w: 50, h: 12, r: 0, strLength: 5 },
    ];
    const prevAnchors = { forwardAnchorLeft: {}, forwardAnchorRight: {}, forwardAnchorCenter: {} };
    const totalPages = 1;
    const expectedOutput = {
      text: " Hello\n World",
      prevAnchors: {
        forwardAnchorLeft: {
          "10": 1,
        },
        forwardAnchorRight: {},
        forwardAnchorCenter: {},
      },
    };
    const result = projectToGrid(config, page, projectionBoxes, prevAnchors, totalPages);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test two-column layout", () => {
    const config = { ...DEFAULT_CONFIG, preserveLayoutAlignmentAcrossPages: false };
    const page = {
      pageNum: 1,
      width: 612,
      height: 792,
      textItems: [],
      images: [],
    };
    const projectionBoxes = [
      { str: "Name", x: 10, y: 100, w: 40, h: 12, r: 0, strLength: 4 },
      { str: "Age", x: 300, y: 100, w: 30, h: 12, r: 0, strLength: 3 },
      { str: "Alice", x: 10, y: 115, w: 50, h: 12, r: 0, strLength: 5 },
      { str: "30", x: 300, y: 115, w: 20, h: 12, r: 0, strLength: 2 },
    ];
    const prevAnchors = {
      forwardAnchorLeft: {
        "10": 1,
        "300": 11,
      },
      forwardAnchorRight: {},
      forwardAnchorCenter: {},
    };
    const totalPages = 1;
    const expectedOutput = {
      text: " Name      Age\n Alice     30",
      prevAnchors: prevAnchors,
    };
    const result = projectToGrid(config, page, projectionBoxes, prevAnchors, totalPages);
    expect(result).toStrictEqual(expectedOutput);
  });

  it("test dot-garbage filtering", () => {
    const config = { ...DEFAULT_CONFIG, preserveLayoutAlignmentAcrossPages: false };
    const page = {
      pageNum: 1,
      width: 612,
      height: 792,
      textItems: [],
      images: [],
    };
    const projectionBoxes = [
      ...Array(110).fill({ str: "...", x: 0, y: 0, w: 10, h: 10, r: 0, strLength: 3 }),
      { str: "Revenue", x: 10, y: 100, w: 70, h: 12, r: 0, strLength: 7 },
      { str: "500", x: 300, y: 100, w: 30, h: 12, r: 0, strLength: 3 },
    ];
    const prevAnchors = { forwardAnchorLeft: {}, forwardAnchorRight: {}, forwardAnchorCenter: {} };
    const totalPages = 1;
    const expectedOutput = {
      text: " Revenue    500",
      prevAnchors: prevAnchors,
    };
    const result = projectToGrid(config, page, projectionBoxes, prevAnchors, totalPages);
    expect(result).toStrictEqual(expectedOutput);
  });
});

const mockPageData = [
  {
    pageNum: 1,
    width: 595,
    height: 842,
    textItems: [
      {
        str: "Hello World",
        x: 50,
        y: 750,
        width: 120,
        height: 12,
        w: 120,
        h: 12,
        fontName: "Arial",
        fontSize: 12,
      },
    ],
    images: [
      {
        x: 100,
        y: 600,
        width: 200,
        height: 150,
        type: "jpeg",
        scaleFactor: 1.0,
      },
    ],
  },
  {
    pageNum: 2,
    width: 595,
    height: 842,
    textItems: [
      {
        str: "Section 2",
        x: 50,
        y: 800,
        width: 90,
        height: 16,
        w: 90,
        h: 16,
        fontName: "Times New Roman",
        fontSize: 16,
        r: 0,
      },
      {
        str: "Some body text on page 2",
        x: 50,
        y: 770,
        width: 300,
        height: 12,
        w: 300,
        h: 12,
        fontName: "Times New Roman",
        fontSize: 12,
      },
    ],
    images: [],
  },
  {
    pageNum: 3,
    width: 595,
    height: 842,
    textItems: [
      {
        str: "Rotated Text",
        x: 400,
        y: 400,
        width: 100,
        height: 12,
        w: 100,
        h: 12,
        fontName: "Arial",
        fontSize: 12,
        r: 90,
        rx: 400,
        ry: 400,
      },
    ],
    images: [
      {
        x: 50,
        y: 500,
        width: 300,
        height: 200,
        type: "png",
        scaleFactor: 1.5,
        originalOrientationAngle: 0,
        coords: { x: 50, y: 500, w: 300, h: 200 },
      },
    ],
  },
];

describe("test projectPagesToGrid", () => {
  it("test pages projection", () => {
    const config = { ...DEFAULT_CONFIG, preserveLayoutAlignmentAcrossPages: false };
    const expectedOutput = [
      {
        pageNum: 1,
        width: 595,
        height: 842,
        text: "Hello World",
        textItems: [
          {
            str: "Hello World",
            x: 50,
            y: 750,
            width: 120,
            height: 12,
            w: 120,
            h: 12,
            fontName: "Arial",
            fontSize: 12,
          },
        ],
        boundingBoxes: [],
      },
      {
        pageNum: 2,
        width: 595,
        height: 842,
        text: "Some body text on page 2\n\nSection 2",
        textItems: [
          {
            str: "Section 2",
            x: 50,
            y: 800,
            width: 90,
            height: 16,
            w: 90,
            h: 16,
            fontName: "Times New Roman",
            fontSize: 16,
            r: 0,
          },
          {
            str: "Some body text on page 2",
            x: 50,
            y: 770,
            width: 300,
            height: 12,
            w: 300,
            h: 12,
            fontName: "Times New Roman",
            fontSize: 12,
          },
        ],
        boundingBoxes: [],
      },
      {
        pageNum: 3,
        width: 595,
        height: 842,
        text: "Rotated Text",
        textItems: [
          {
            str: "Rotated Text",
            x: 400,
            y: 400,
            width: 100,
            height: 12,
            w: 100,
            h: 12,
            fontName: "Arial",
            fontSize: 12,
            r: 90,
            rx: 400,
            ry: 400,
          },
        ],
        boundingBoxes: [],
      },
    ];
    const result = projectPagesToGrid(mockPageData, config);
    expect(result).toStrictEqual(expectedOutput);
  });
});

describe("test projectToGrid with textLineTracking", () => {
  const baseConfig = { ...DEFAULT_CONFIG, textLineTracking: true };
  const page = { pageNum: 1, width: 612, height: 792, textItems: [], images: [] };
  const prevAnchors = { forwardAnchorLeft: {}, forwardAnchorRight: {}, forwardAnchorCenter: {} };

  it("returns rawLineBoxes when textLineTracking is enabled", () => {
    const projectionBoxes = [{ str: "Hello", x: 10, y: 100, w: 50, h: 12, r: 0, strLength: 5 }];
    const result = projectToGrid(baseConfig, page, projectionBoxes, prevAnchors, 1);
    expect(result.rawLineBoxes).toBeDefined();
    expect(result.rawLineBoxes!.length).toBeGreaterThan(0);
    // Find the line that has boxes
    const nonEmpty = result.rawLineBoxes!.filter((b) => b && b.length > 0);
    expect(nonEmpty.length).toBe(1);
    expect(nonEmpty[0][0].str).toBe("Hello");
  });

  it("does not return rawLineBoxes when textLineTracking is disabled", () => {
    const config = { ...DEFAULT_CONFIG, textLineTracking: false };
    const projectionBoxes = [{ str: "Hello", x: 10, y: 100, w: 50, h: 12, r: 0, strLength: 5 }];
    const result = projectToGrid(config, page, projectionBoxes, prevAnchors, 1);
    expect(result.rawLineBoxes).toBeUndefined();
  });

  it("tracks boxes for multi-line text", () => {
    const projectionBoxes = [
      { str: "Line1", x: 10, y: 100, w: 50, h: 12, r: 0, strLength: 5 },
      { str: "Line2", x: 10, y: 115, w: 50, h: 12, r: 0, strLength: 5 },
    ];
    const result = projectToGrid(baseConfig, page, projectionBoxes, prevAnchors, 1);
    const nonEmpty = result.rawLineBoxes!.filter((b) => b && b.length > 0);
    expect(nonEmpty.length).toBe(2);
    expect(nonEmpty[0][0].str).toBe("Line1");
    expect(nonEmpty[1][0].str).toBe("Line2");
  });

  it("tracks both column items on the same line", () => {
    const projectionBoxes = [
      { str: "Name", x: 10, y: 100, w: 40, h: 12, r: 0, strLength: 4 },
      { str: "Age", x: 300, y: 100, w: 30, h: 12, r: 0, strLength: 3 },
    ];
    const result = projectToGrid(baseConfig, page, projectionBoxes, prevAnchors, 1);
    const nonEmpty = result.rawLineBoxes!.filter((b) => b && b.length > 0);
    expect(nonEmpty.length).toBe(1);
    expect(nonEmpty[0].length).toBe(2);
    expect(nonEmpty[0].map((b) => b.str)).toContain("Name");
    expect(nonEmpty[0].map((b) => b.str)).toContain("Age");
  });
});

describe("test projectPagesToGrid with textLineTracking", () => {
  const config = { ...DEFAULT_CONFIG, textLineTracking: true };

  it("populates textLines with correct text and bbox", () => {
    const pages = [
      {
        pageNum: 1,
        width: 595,
        height: 842,
        textItems: [
          {
            str: "Hello World",
            x: 50,
            y: 750,
            width: 120,
            height: 12,
            w: 120,
            h: 12,
            fontName: "Arial",
            fontSize: 12,
          },
        ],
        images: [],
      },
    ];
    const result = projectPagesToGrid(pages, config);
    expect(result[0].textLines).toBeDefined();
    expect(result[0].textLines!.length).toBe(1);
    expect(result[0].textLines![0].text).toBe("Hello World");
    expect(result[0].textLines![0].pageNum).toBe(1);
    expect(result[0].textLines![0].bbox.w).toBeGreaterThan(0);
    expect(result[0].textLines![0].bbox.h).toBeGreaterThan(0);
  });

  it("computes union bbox spanning multiple items on same line", () => {
    const pages = [
      {
        pageNum: 1,
        width: 612,
        height: 792,
        textItems: [
          { str: "Left", x: 10, y: 100, width: 40, height: 12, w: 40, h: 12 },
          { str: "Right", x: 300, y: 100, width: 50, height: 12, w: 50, h: 12 },
        ],
        images: [],
      },
    ];
    const result = projectPagesToGrid(pages, config);
    expect(result[0].textLines).toBeDefined();
    const tl = result[0].textLines![0];
    // Union bbox should span from x=10 to x=350 (300+50)
    expect(tl.bbox.x).toBe(10);
    expect(tl.bbox.x + tl.bbox.w).toBe(350);
  });

  it("does not include empty lines in textLines", () => {
    const pages = [
      {
        pageNum: 1,
        width: 595,
        height: 842,
        textItems: [
          { str: "Line1", x: 50, y: 100, width: 50, height: 12, w: 50, h: 12 },
          { str: "Line2", x: 50, y: 200, width: 50, height: 12, w: 50, h: 12 },
        ],
        images: [],
      },
    ];
    const result = projectPagesToGrid(pages, config);
    // All textLines entries should have non-empty text
    for (const tl of result[0].textLines!) {
      expect(tl.text.trim().length).toBeGreaterThan(0);
    }
  });

  it("does not populate textLines when textLineTracking is disabled", () => {
    const disabledConfig = { ...DEFAULT_CONFIG, textLineTracking: false };
    const pages = [
      {
        pageNum: 1,
        width: 595,
        height: 842,
        textItems: [{ str: "Hello", x: 50, y: 750, width: 50, height: 12, w: 50, h: 12 }],
        images: [],
      },
    ];
    const result = projectPagesToGrid(pages, disabledConfig);
    expect(result[0].textLines).toBeUndefined();
  });
});

describe("test legal line number detection", () => {
  const config = { ...DEFAULT_CONFIG, textLineTracking: true };

  it("detects sequential line numbers in left margin", () => {
    const textItems = [];
    // Create 10 lines with line numbers in left margin + body text
    for (let i = 0; i < 10; i++) {
      const lineNum = i + 1;
      const y = 100 + i * 15;
      // Line number in left margin
      textItems.push({
        str: String(lineNum),
        x: 20,
        y,
        width: 10,
        height: 12,
        w: 10,
        h: 12,
      });
      // Body text
      textItems.push({
        str: `This is line ${lineNum} of the document`,
        x: 80,
        y,
        width: 400,
        height: 12,
        w: 400,
        h: 12,
      });
    }
    const pages = [{ pageNum: 1, width: 612, height: 792, textItems, images: [] }];
    const result = projectPagesToGrid(pages, config);
    const withLineNum = result[0].textLines!.filter((tl) => tl.lineNumber !== undefined);
    expect(withLineNum.length).toBeGreaterThanOrEqual(5);
    // Line numbers should be sequential
    for (let i = 1; i < withLineNum.length; i++) {
      expect(withLineNum[i].lineNumber!).toBeGreaterThan(withLineNum[i - 1].lineNumber!);
    }
  });

  it("does not detect line numbers when there are too few candidates", () => {
    const textItems = [];
    // Only 3 lines with numbers — not enough for detection
    for (let i = 0; i < 3; i++) {
      const y = 100 + i * 15;
      textItems.push({ str: String(i + 1), x: 20, y, width: 10, height: 12, w: 10, h: 12 });
      textItems.push({ str: "Some text", x: 80, y, width: 200, height: 12, w: 200, h: 12 });
    }
    const pages = [{ pageNum: 1, width: 612, height: 792, textItems, images: [] }];
    const result = projectPagesToGrid(pages, config);
    const withLineNum = result[0].textLines!.filter((tl) => tl.lineNumber !== undefined);
    expect(withLineNum.length).toBe(0);
  });

  it("does not detect non-sequential numbers as line numbers", () => {
    const textItems = [];
    const randomNums = [42, 7, 99, 3, 15, 88, 21, 56, 11, 73];
    for (let i = 0; i < 10; i++) {
      const y = 100 + i * 15;
      textItems.push({
        str: String(randomNums[i]),
        x: 20,
        y,
        width: 10,
        height: 12,
        w: 10,
        h: 12,
      });
      textItems.push({ str: "Some text", x: 80, y, width: 200, height: 12, w: 200, h: 12 });
    }
    const pages = [{ pageNum: 1, width: 612, height: 792, textItems, images: [] }];
    const result = projectPagesToGrid(pages, config);
    const withLineNum = result[0].textLines!.filter((tl) => tl.lineNumber !== undefined);
    expect(withLineNum.length).toBe(0);
  });
});

describe("end-to-end: parse → searchTextLines → bounding boxes", () => {
  const config = { ...DEFAULT_CONFIG, textLineTracking: true };

  it("finds text spanning multiple lines and returns correct bounding boxes", () => {
    // Simulate a 5-line document with distinct coordinates per line
    const textItems = [
      { str: "ARTICLE 1. DEFINITIONS", x: 50, y: 100, width: 250, height: 14, w: 250, h: 14 },
      {
        str: "In this Agreement, the following",
        x: 50,
        y: 120,
        width: 300,
        height: 12,
        w: 300,
        h: 12,
      },
      {
        str: "terms shall have the meanings",
        x: 50,
        y: 135,
        width: 280,
        height: 12,
        w: 280,
        h: 12,
      },
      { str: "set forth below unless the", x: 50, y: 150, width: 260, height: 12, w: 260, h: 12 },
      { str: "context requires otherwise.", x: 50, y: 165, width: 240, height: 12, w: 240, h: 12 },
    ];
    const pages = [{ pageNum: 1, width: 612, height: 792, textItems, images: [] }];

    const result = projectPagesToGrid(pages, config);
    const page = result[0];

    // Verify textLines were built
    expect(page.textLines).toBeDefined();
    expect(page.textLines!.length).toBeGreaterThanOrEqual(4);

    // Search for a phrase that spans lines 2-3 of the output
    // (the text "terms shall have the meanings\nset forth below")
    const matches = searchTextLines(page.textLines!, {
      phrase: "terms shall have the meanings\nset forth below",
    });

    // Should return exactly 2 lines
    expect(matches).toHaveLength(2);

    // First matched line should contain "terms shall have the meanings"
    expect(matches[0].text).toContain("terms shall have the meanings");
    // Second matched line should contain "set forth below"
    expect(matches[1].text).toContain("set forth below");

    // Each line should have valid bounding box coordinates
    for (const match of matches) {
      expect(match.bbox.x).toBeGreaterThanOrEqual(0);
      expect(match.bbox.y).toBeGreaterThanOrEqual(0);
      expect(match.bbox.w).toBeGreaterThan(0);
      expect(match.bbox.h).toBeGreaterThan(0);
      expect(match.pageNum).toBe(1);
    }

    // The bboxes should cover the vertical range of the source text items
    // Line with "terms shall..." is at y=135, line with "set forth..." is at y=150
    expect(matches[0].bbox.y).toBeLessThanOrEqual(135);
    expect(matches[1].bbox.y).toBeLessThanOrEqual(150);

    // Union of matched bboxes should span from first match to bottom of last match
    const unionTop = Math.min(matches[0].bbox.y, matches[1].bbox.y);
    const unionBottom = Math.max(
      matches[0].bbox.y + matches[0].bbox.h,
      matches[1].bbox.y + matches[1].bbox.h
    );
    expect(unionBottom - unionTop).toBeGreaterThanOrEqual(24); // at least 2 lines of height
  });

  it("finds single-line text within full pipeline output", () => {
    const textItems = [
      { str: "Revenue:", x: 50, y: 100, width: 80, height: 12, w: 80, h: 12 },
      { str: "$1,234,567", x: 300, y: 100, width: 100, height: 12, w: 100, h: 12 },
      { str: "Expenses:", x: 50, y: 115, width: 90, height: 12, w: 90, h: 12 },
      { str: "$987,654", x: 300, y: 115, width: 90, height: 12, w: 90, h: 12 },
    ];
    const pages = [{ pageNum: 1, width: 612, height: 792, textItems, images: [] }];

    const result = projectPagesToGrid(pages, config);
    const matches = searchTextLines(result[0].textLines!, { phrase: "$987,654" });

    expect(matches).toHaveLength(1);
    expect(matches[0].text).toContain("$987,654");
    // The bbox should span the full line (both "Expenses:" and "$987,654")
    expect(matches[0].bbox.x).toBeLessThanOrEqual(50);
    expect(matches[0].bbox.x + matches[0].bbox.w).toBeGreaterThanOrEqual(390); // 300 + 90
  });

  it("returns empty when searched text is not in the document", () => {
    const textItems = [
      { str: "Hello World", x: 50, y: 100, width: 120, height: 12, w: 120, h: 12 },
    ];
    const pages = [{ pageNum: 1, width: 612, height: 792, textItems, images: [] }];

    const result = projectPagesToGrid(pages, config);
    const matches = searchTextLines(result[0].textLines!, { phrase: "nonexistent text" });
    expect(matches).toHaveLength(0);
  });
});
