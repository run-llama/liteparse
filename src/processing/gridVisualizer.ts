import sharp from "sharp";
import { mkdirSync } from "fs";
import type { VisualizerPageData, RenderedSegment } from "./gridDebugLogger.js";

/** Monospace character metrics in pixels. */
const CHAR_WIDTH = 7;
const CHAR_HEIGHT = 14;
const LINE_HEIGHT = 16;
const PADDING = 12;
const LEGEND_WIDTH = 140;
const LEGEND_HEIGHT = 100;

/** Color scheme for snap types */
const COLORS: Record<RenderedSegment["snap"], { bg: string; text: string }> = {
  left: { bg: "rgba(59,130,246,0.18)", text: "#2563eb" },
  right: { bg: "rgba(239,68,68,0.18)", text: "#dc2626" },
  center: { bg: "rgba(34,197,94,0.18)", text: "#16a34a" },
  floating: { bg: "rgba(156,163,175,0.15)", text: "#4b5563" },
  flowing: { bg: "rgba(234,179,8,0.15)", text: "#a16207" },
};

function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

/**
 * Render the projected text output as a monospace grid image.
 *
 * Each character of the output text is rendered at its grid position.
 * Text segments are color-coded by snap type so you can visually compare
 * the projection against the original PDF page screenshot.
 */
export async function renderGridVisualization(
  data: VisualizerPageData,
  outputPath: string
): Promise<void> {
  const { rawLines, segments } = data;
  if (!rawLines || rawLines.length === 0) return;

  // Trim trailing empty lines
  let lastNonEmpty = rawLines.length - 1;
  while (lastNonEmpty >= 0 && rawLines[lastNonEmpty].trimEnd().length === 0) {
    lastNonEmpty--;
  }
  const lines = rawLines.slice(0, lastNonEmpty + 1);
  if (lines.length === 0) return;

  const maxLineLen = Math.max(0, ...lines.map((l) => (l ?? "").length));
  if (maxLineLen === 0 || !Number.isFinite(maxLineLen)) return;

  // Image dimensions
  const contentWidth = maxLineLen * CHAR_WIDTH;
  const contentHeight = lines.length * LINE_HEIGHT;
  const imgWidth = Math.max(1, contentWidth + PADDING * 2 + LEGEND_WIDTH + PADDING);
  const imgHeight = Math.max(1, contentHeight + PADDING * 2);

  // Build segment lookup: for each (lineIndex, colRange) → snap type
  // Sort segments by line then column for efficient lookup
  const segmentsByLine = new Map<number, RenderedSegment[]>();
  for (const seg of segments) {
    if (!segmentsByLine.has(seg.lineIndex)) {
      segmentsByLine.set(seg.lineIndex, []);
    }
    segmentsByLine.get(seg.lineIndex)!.push(seg);
  }

  const svgParts: string[] = [];
  svgParts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${imgWidth}" height="${imgHeight}" viewBox="0 0 ${imgWidth} ${imgHeight}">`
  );

  // Draw colored background rectangles for each segment
  for (const [lineIdx, lineSegments] of segmentsByLine) {
    if (lineIdx > lastNonEmpty) continue;
    for (const seg of lineSegments) {
      const color = COLORS[seg.snap];
      const x = PADDING + seg.gridCol * CHAR_WIDTH;
      const y = PADDING + lineIdx * LINE_HEIGHT;
      const w = seg.text.length * CHAR_WIDTH;
      svgParts.push(
        `<rect x="${x}" y="${y}" width="${w}" height="${LINE_HEIGHT}" fill="${color.bg}"/>`
      );
    }
  }

  // Render text line by line using colored spans for segments
  for (let lineIdx = 0; lineIdx < lines.length; lineIdx++) {
    const line = lines[lineIdx] ?? "";
    if (line.length === 0) continue;

    const y = PADDING + lineIdx * LINE_HEIGHT + CHAR_HEIGHT;
    const lineSegs = segmentsByLine.get(lineIdx) ?? [];

    if (lineSegs.length === 0) {
      // No segment data — render as plain gray text
      svgParts.push(
        `<text x="${PADDING}" y="${y}" font-family="monospace" font-size="12" fill="#6b7280">${escapeXml(line)}</text>`
      );
      continue;
    }

    // Sort segments by column position
    lineSegs.sort((a, b) => a.gridCol - b.gridCol);

    // Build colored tspans for each segment, with gaps in default color
    let col = 0;
    const textParts: string[] = [];
    textParts.push(`<text y="${y}" font-family="monospace" font-size="12">`);

    for (const seg of lineSegs) {
      // Gap before this segment (whitespace or untracked text)
      if (seg.gridCol > col) {
        const gapText = line.substring(col, seg.gridCol);
        if (gapText.length > 0) {
          textParts.push(
            `<tspan x="${PADDING + col * CHAR_WIDTH}" fill="#9ca3af">${escapeXml(gapText)}</tspan>`
          );
        }
        col = seg.gridCol;
      }

      // Render the segment with its snap color
      const color = COLORS[seg.snap];
      const segText = line.substring(col, col + seg.text.length);
      if (segText.length > 0) {
        textParts.push(
          `<tspan x="${PADDING + col * CHAR_WIDTH}" fill="${color.text}">${escapeXml(segText)}</tspan>`
        );
        col += seg.text.length;
      }
    }

    // Trailing text after last segment
    if (col < line.length) {
      const trailing = line.substring(col);
      if (trailing.trimEnd().length > 0) {
        textParts.push(
          `<tspan x="${PADDING + col * CHAR_WIDTH}" fill="#9ca3af">${escapeXml(trailing)}</tspan>`
        );
      }
    }

    textParts.push("</text>");
    svgParts.push(textParts.join(""));
  }

  // Legend
  const legendX = contentWidth + PADDING * 2;
  const legendY = PADDING;
  svgParts.push(
    `<rect x="${legendX}" y="${legendY}" width="${LEGEND_WIDTH}" height="${LEGEND_HEIGHT}" fill="white" stroke="#d1d5db" stroke-width="1" rx="3"/>`
  );
  const legendItems: Array<{ label: string; snap: RenderedSegment["snap"] }> = [
    { label: "Left snap", snap: "left" },
    { label: "Right snap", snap: "right" },
    { label: "Center snap", snap: "center" },
    { label: "Floating", snap: "floating" },
    { label: "Flowing text", snap: "flowing" },
  ];
  for (let i = 0; i < legendItems.length; i++) {
    const item = legendItems[i];
    const ly = legendY + 8 + i * 17;
    const color = COLORS[item.snap];
    svgParts.push(
      `<rect x="${legendX + 8}" y="${ly}" width="12" height="12" fill="${color.bg}" stroke="${color.text}" stroke-width="1"/>`
    );
    svgParts.push(
      `<text x="${legendX + 26}" y="${ly + 10}" font-family="sans-serif" font-size="11" fill="#374151">${item.label}</text>`
    );
  }

  svgParts.push("</svg>");

  const svgBuffer = Buffer.from(svgParts.join("\n"));

  await sharp({
    create: {
      width: imgWidth,
      height: imgHeight,
      channels: 4,
      background: { r: 255, g: 255, b: 255, alpha: 1 },
    },
  })
    .composite([{ input: svgBuffer, top: 0, left: 0 }])
    .png({ compressionLevel: 6 })
    .toFile(outputPath);
}

/**
 * Render all captured visualization pages to PNG files.
 */
export async function renderAllVisualizations(
  pages: VisualizerPageData[],
  outputDir: string
): Promise<string[]> {
  mkdirSync(outputDir, { recursive: true });

  const paths: string[] = [];
  for (const page of pages) {
    const filePath = `${outputDir}/page-${page.pageNum}-grid.png`;
    await renderGridVisualization(page, filePath);
    paths.push(filePath);
  }
  return paths;
}
