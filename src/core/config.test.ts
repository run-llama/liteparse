import { describe, it, expect } from "vitest";
import { LiteParseConfig } from "./types";
import { DEFAULT_CONFIG, mergeConfig } from "./config";

describe("test config", () => {
  it("test merge config with no partial config", () => {
    const user_config: Partial<LiteParseConfig> = {};
    const config = mergeConfig(user_config);
    expect(config).toStrictEqual(DEFAULT_CONFIG);
  });
  it("test merge config overrides defaults", () => {
    const user_config: Partial<LiteParseConfig> = {
      ocrLanguage: "fr",
      ocrServerUrl: "http://localhost:8000",
      dpi: 200,
    };
    const config = mergeConfig(user_config);
    expect(config.ocrLanguage).toBe(user_config.ocrLanguage);
    expect(config.ocrServerUrl).toBe(user_config.ocrServerUrl);
    expect(config.dpi).toBe(user_config.dpi);
  });
  it("defaults textLineTracking to false", () => {
    expect(DEFAULT_CONFIG.textLineTracking).toBe(false);
  });
});
