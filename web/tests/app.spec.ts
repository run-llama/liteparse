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
