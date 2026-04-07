import { expect, describe, it } from "vitest";
import { bboxToLine, projectPagesToGrid, projectToGrid } from "./gridProjection";
import { ProjectionTextBox } from "../core/types";
import { DEFAULT_CONFIG } from "../core/config";
import { NOOP_LOGGER } from "./gridDebugLogger";

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

  it("uses representative line height for bullet-line spacing", () => {
    const textBbox = [
      { str: "Recommendation", x: 7.68, y: 6.24, w: 66.24, h: 6.24, strLength: 14 },
      { str: "summary", x: 79.68, y: 8.16, w: 32.64, h: 5.76, strLength: 7 },
      { str: "-", x: 8.16, y: 30.72, w: 2.4, h: 0.48, strLength: 1 },
      { str: "Primary", x: 17.28, y: 26.88, w: 32.64, h: 7.68, strLength: 7 },
      { str: "choice:", x: 55.2, y: 26.88, w: 32.16, h: 6.24, strLength: 7 },
      { str: "Apache", x: 93.6, y: 26.88, w: 28.32, h: 7.68, strLength: 6 },
      { str: "ECharts", x: 127.68, y: 26.88, w: 32.64, h: 6.24, strLength: 7 },
    ];

    const result = bboxToLine(textBbox, 4.32, 6.24);
    const renderedLines = result.map((line) => line.map((bbox) => bbox.str).join(" "));

    expect(renderedLines).toStrictEqual([
      "Recommendation summary",
      "",
      "- Primary choice: Apache ECharts",
    ]);
  });

  it("preserves blank lines for regular non-bulleted text", () => {
    const textBbox = [
      { str: "Heading", x: 10, y: 10, w: 42, h: 6, strLength: 7 },
      { str: "First", x: 10, y: 28, w: 26, h: 6, strLength: 5 },
      { str: "paragraph", x: 40, y: 28, w: 44, h: 6, strLength: 9 },
      { str: "Second", x: 10, y: 34, w: 34, h: 6, strLength: 6 },
      { str: "line", x: 48, y: 34, w: 20, h: 6, strLength: 4 },
    ];

    const result = bboxToLine(textBbox, 5, 6);
    const renderedLines = result.map((line) => line.map((bbox) => bbox.str).join(" "));

    expect(renderedLines).toStrictEqual(["Heading", "", "First paragraph", "Second line"]);
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
    const result = projectToGrid(
      config,
      page,
      projectionBoxes,
      prevAnchors,
      totalPages,
      NOOP_LOGGER
    );
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
    const result = projectToGrid(
      config,
      page,
      projectionBoxes,
      prevAnchors,
      totalPages,
      NOOP_LOGGER
    );
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
    const result = projectToGrid(
      config,
      page,
      projectionBoxes,
      prevAnchors,
      totalPages,
      NOOP_LOGGER
    );
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
  it("test pages projection", async () => {
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
    const result = await projectPagesToGrid(mockPageData, config);
    expect(result).toStrictEqual(expectedOutput);
  });
});
