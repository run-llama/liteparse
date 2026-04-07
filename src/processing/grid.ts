import { PageData } from "../engines/pdf/interface.js";
import { ParsedPage, LiteParseConfig } from "../core/types.js";
import { projectPagesToGrid as projectPagesToGridComplete } from "./gridProjection.js";
import { DEFAULT_CONFIG } from "../core/config.js";

/**
 * Projects text items onto a grid for spatial text extraction
 */
export async function projectPagesToGrid(
  pages: PageData[],
  config?: Partial<LiteParseConfig>
): Promise<ParsedPage[]> {
  const fullConfig = {
    ...DEFAULT_CONFIG,
    ...config,
  };

  return projectPagesToGridComplete(pages, fullConfig);
}
