/**
 * Playwright test that loads the LiteParse WASM module in a real browser
 * and verifies it can parse a PDF end-to-end.
 *
 * Usage: node scripts/browser-compat/wasm-test.mjs
 *
 * Requires: playwright (npx playwright install chromium)
 * Expects:  packages/wasm/pkg/ to contain the built WASM files
 *           demo/docs/apple-10k-2024.pdf to exist
 * Set PLAYWRIGHT_CHROMIUM_EXECUTABLE to use an existing browser executable.
 */

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = resolve(fileURLToPath(import.meta.url), "../../..");

const MIME_TYPES = {
  ".html": "text/html",
  ".js": "application/javascript",
  ".wasm": "application/wasm",
  ".pdf": "application/pdf",
  ".json": "application/json",
};

function startServer() {
  return new Promise((resolvePromise) => {
    const server = createServer(async (req, res) => {
      const url = new URL(req.url, "http://localhost");
      const filePath = resolve(ROOT, "." + url.pathname);

      // Basic security: don't serve outside ROOT
      if (!filePath.startsWith(ROOT)) {
        res.writeHead(403);
        res.end("Forbidden");
        return;
      }

      try {
        const data = await readFile(filePath);
        const ext = extname(filePath);
        res.writeHead(200, {
          "Content-Type": MIME_TYPES[ext] || "application/octet-stream",
          "Cross-Origin-Opener-Policy": "same-origin",
          "Cross-Origin-Embedder-Policy": "require-corp",
        });
        res.end(data);
      } catch {
        res.writeHead(404);
        res.end("Not found");
      }
    });

    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port;
      resolvePromise({ server, port });
    });
  });
}

async function main() {
  const { server, port } = await startServer();
  const baseUrl = `http://127.0.0.1:${port}`;
  console.log(`Static server listening on ${baseUrl}`);

  let browser;
  try {
    browser = await chromium.launch({
      executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE,
    });
    const page = await browser.newPage();

    const errors = [];
    let reportBrowserFailure;
    const browserFailure = new Promise((resolvePromise) => {
      reportBrowserFailure = resolvePromise;
    });
    page.on("pageerror", (error) => {
      errors.push(error.message);
      reportBrowserFailure(error);
    });
    page.on("console", (message) => {
      if (message.type() === "error") {
        const error = new Error(message.text());
        errors.push(error.message);
        reportBrowserFailure(error);
      }
    });

    console.log("Navigating to test page...");
    await page.goto(`${baseUrl}/scripts/browser-compat/wasm-test.html`);

    const completion = page
      .waitForSelector("#result[style*='block'], #error[style*='block']", { timeout: 120_000 })
      .then(() => null)
      .catch((error) => error);
    const failure = await Promise.race([completion, browserFailure]);
    if (failure) throw failure;

    const errorText = await page.locator("#error").textContent();
    if (errorText) {
      throw new Error(`${errorText}${errors.length ? `\nPage errors: ${errors.join("\n")}` : ""}`);
    }

    const resultText = await page.locator("#result").textContent();
    const pages = await page.locator("#result").getAttribute("data-pages");
    const textLength = await page.locator("#result").getAttribute("data-text-length");
    const ocrCallbacks = await page.locator("#result").getAttribute("data-ocr-callbacks");
    const ocrBytes = await page.locator("#result").getAttribute("data-ocr-bytes");
    const ocrMaxConcurrency = await page.locator("#result").getAttribute("data-ocr-max-concurrency");
    const ocrFailureCases = await page.locator("#result").getAttribute("data-ocr-failure-cases");

    console.log(`PASS: ${resultText}`);
    console.log(`  Pages: ${pages}`);
    console.log(`  Text length: ${textLength}`);
    console.log(`  OCR callback calls: ${ocrCallbacks}`);
    console.log(`  OCR input bytes: ${ocrBytes}`);
    console.log(`  OCR maximum concurrency: ${ocrMaxConcurrency}`);
    console.log(`  OCR failure cases: ${ocrFailureCases}`);

    if (errors.length) {
      console.warn("Browser console errors (non-fatal):", errors);
    }
  } finally {
    if (browser) await browser.close();
    server.close();
  }
}

main().catch((error) => {
  console.error("Test runner failed:", error);
  process.exit(1);
});
