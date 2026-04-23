import { expect, test } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIX = (f: string) => resolve(HERE, "fixtures", f);

test("loads the app shell", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/LiteParse/i);
  await expect(page.locator("input[type=file]#file")).toBeVisible();
  await expect(page.locator("input[type=checkbox]#ocr")).toBeVisible();
  await expect(page.locator("input[type=checkbox]#shots")).toBeVisible();
  await expect(page.locator("button#parse")).toBeVisible();
  await expect(page.locator("textarea#text-output")).toBeVisible();
  await expect(page.locator("textarea#json-output")).toBeVisible();
  await expect(page.locator("button.copy[data-target=text-output]")).toBeVisible();
  await expect(page.locator("button.copy[data-target=json-output]")).toBeVisible();
});

test("parse button is disabled until a file is picked", async ({ page }) => {
  await page.goto("/");
  const parseBtn = page.locator("button#parse");
  await expect(parseBtn).toBeDisabled();
  await page.locator("input[type=file]#file").setInputFiles(FIX("sample-text.pdf"));
  await expect(parseBtn).toBeEnabled();
});

test("rejects non-PDF files with a clear error", async ({ page }) => {
  await page.goto("/");
  await page.locator("input#file").setInputFiles(FIX("not-a-pdf.txt"));
  await page.locator("button#parse").click();
  const status = page.locator("#status");
  await expect(status).toContainText(/PDF/i);
  await expect(status).toHaveClass(/error/);
  await expect(page.locator("#text-output")).toHaveValue("");
  await expect(page.locator("#json-output")).toHaveValue("");
});

test("parses a text PDF with OCR off and renders both text and JSON", async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto("/");
  // OCR should already be unchecked by default
  await page.locator("input#file").setInputFiles(FIX("sample-text.pdf"));
  await page.locator("button#parse").click();

  const textArea = page.locator("#text-output");
  await expect(textArea).toHaveValue(/Hello from LiteParse/, { timeout: 45_000 });

  const jsonArea = page.locator("#json-output");
  const jsonValue = await jsonArea.inputValue();
  expect(jsonValue.length).toBeGreaterThan(20);
  expect(jsonValue).toContain("\n");
  const parsed = JSON.parse(jsonValue);
  expect(Array.isArray(parsed.pages)).toBe(true);
  expect(parsed.pages.length).toBeGreaterThan(0);
});

test("copy buttons copy the respective textarea and flash Copied!", async ({
  page,
  browserName,
}) => {
  // Clipboard read permissions are chromium-only in Playwright; skip elsewhere.
  test.skip(browserName !== "chromium", "clipboard API requires chromium permissions");
  await page.goto("/");
  await page.locator("input#file").setInputFiles(FIX("sample-text.pdf"));
  await page.locator("button#parse").click();
  await expect(page.locator("#text-output")).toHaveValue(/Hello from LiteParse/, {
    timeout: 45_000,
  });

  const textCopy = page.locator("button.copy[data-target=text-output]");
  const jsonCopy = page.locator("button.copy[data-target=json-output]");

  // Initial label
  await expect(textCopy).toHaveText("Copy");

  // Copy text
  await textCopy.click();
  await expect(textCopy).toHaveText("Copied!");
  const textClip = await page.evaluate(() => navigator.clipboard.readText());
  expect(textClip).toBe(await page.locator("#text-output").inputValue());

  // After the 1.5s window the label resets
  await expect(textCopy).toHaveText("Copy", { timeout: 3000 });

  // Copy JSON
  await jsonCopy.click();
  await expect(jsonCopy).toHaveText("Copied!");
  const jsonClip = await page.evaluate(() => navigator.clipboard.readText());
  expect(jsonClip).toBe(await page.locator("#json-output").inputValue());
});
