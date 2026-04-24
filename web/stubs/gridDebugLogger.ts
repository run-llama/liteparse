// Browser stub — debug logger does nothing. Parser never enables debug in the UI.

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

// No-op debug logger. Every method called by gridProjection.ts must exist,
// even if it does nothing, otherwise `TypeError: logger.X is not a function`.
export class GridDebugLogger {
  enabled = false;
  shouldVisualize = false;
  debugConfig: GridDebugConfig = { enabled: false };
  visualizerPages: VisualizerPageData[] = [];

  setPage(_pageNum: number): void {}
  captureRawLines(_lines: string[]): void {}
  captureRender(..._args: unknown[]): void {}
  logAnchors(..._args: unknown[]): void {}
  logBlock(..._args: unknown[]): void {}
  logBlockContext(..._args: unknown[]): void {}
  logFlowingBlock(..._args: unknown[]): void {}
  logFlowingLine(..._args: unknown[]): void {}
  logForwardAnchor(..._args: unknown[]): void {}
  logForwardAnchorMutation(..._args: unknown[]): void {}
  logLineComposition(..._args: unknown[]): void {}
  logRender(..._args: unknown[]): void {}
  logRenderTrace(..._args: unknown[]): void {}
  logSnapAssignment(..._args: unknown[]): void {}
  logStructuredBlock(..._args: unknown[]): void {}
  async flush(): Promise<void> {}
}

export class NoopGridDebugLogger extends GridDebugLogger {}
export const NOOP_LOGGER = new NoopGridDebugLogger();
export function createGridDebugLogger(_config?: GridDebugConfig): GridDebugLogger {
  return NOOP_LOGGER;
}
