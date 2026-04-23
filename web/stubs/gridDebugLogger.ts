// Browser stub — debug logger does nothing. Parser never enables debug in the UI.
import type { ProjectionTextBox } from "../../src/core/types.js";

export interface GridDebugConfig {
  enabled: boolean;
  textFilter?: string[];
  lineFilter?: number[];
  pageFilter?: number;
  regionFilter?: { x1: number; y1: number; x2: number; y2: number };
  outputPath?: string;
  visualize?: boolean;
  visualizePath?: string;
  trace?: boolean;
}

export const DEFAULT_DEBUG_CONFIG: GridDebugConfig = { enabled: false };

export interface RenderedSegment {
  lineIndex: number;
  gridCol: number;
  text: string;
  snap: "left" | "right" | "center" | "floating" | "flowing";
}
export interface RenderTraceContext {
  snapType: "left" | "right" | "center" | "floating";
  initialTargetX: number;
  medianWidth: number;
  lineMax: number;
  lastSnapLeft?: number;
  lastSnapLeftKey?: number;
  rawLineTrimLength: number;
  shouldSpace: number;
  lineMaxCandidates?: unknown[];
  forwardAnchorValue?: number;
  prevAnchorValue?: number;
  pdfFallbackUsed?: boolean;
  pdfFallbackTargetX?: number;
  floatingAnchorBump?: number;
  isSparseAnchor?: boolean;
  finalTargetX: number;
  bindingConstraint: string;
}
export interface VisualizerPageData {
  pageNum: number;
  segments: RenderedSegment[];
  rawLines: string[];
}

export class GridDebugLogger {
  enabled = false;
  capturePages = false;
  traceEnabled = false;
  log(_: unknown): void {}
  logLine(): void {}
  logBox(_phase: string, _box: ProjectionTextBox, _msg: string, _data?: unknown): void {}
  logPhase(_phase: string, _msg: string, _data?: unknown): void {}
  logAnchor(): void {}
  logRender(): void {}
  logTrace(): void {}
  startPage(_pageNum: number): void {}
  recordSegment(_seg: RenderedSegment): void {}
  recordRawLines(_lines: string[]): void {}
  finishPage(): void {}
  getCapturedPages(): VisualizerPageData[] {
    return [];
  }
  async flush(): Promise<void> {}
  shouldLogText(_text: string): boolean {
    return false;
  }
  shouldLogLine(_lineIndex: number): boolean {
    return false;
  }
  shouldLogPage(_pageNum: number): boolean {
    return false;
  }
  shouldLogRegion(): boolean {
    return false;
  }
}
export class NoopGridDebugLogger extends GridDebugLogger {}
export const NOOP_LOGGER = new NoopGridDebugLogger();
export function createGridDebugLogger(_config?: GridDebugConfig): GridDebugLogger {
  return NOOP_LOGGER;
}
