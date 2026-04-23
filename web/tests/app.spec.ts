import { expect, test } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIX = (f: string) => resolve(HERE, "fixtures", f);

test("loads the app shell", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/LiteParse/i);
  // The native file input is hidden behind a styled drop zone.
  await expect(page.locator("input[type=file]#file")).toBeAttached();
  await expect(page.locator(".dropzone")).toBeVisible();
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

test("drop zone exists and shows the chosen filename without overflowing", async ({
  page,
}) => {
  await page.goto("/");
  const zone = page.locator(".dropzone");
  await expect(zone).toBeVisible();
  await expect(zone).toContainText(/drag|drop|click/i);

  // Before a file is chosen, no filename pill is shown
  await expect(page.locator(".dropzone .filename")).toHaveCount(0);

  await page
    .locator("input#file")
    .setInputFiles(FIX("sample-text.pdf"));
  const pill = page.locator(".dropzone .filename");
  await expect(pill).toBeVisible();
  await expect(pill).toContainText("sample-text.pdf");

  // The drop zone must not exceed the width of its container even if the
  // filename is very long. Set a deliberately long filename and assert the
  // .dropzone's clientWidth stays within its parent's bounds.
  await page.evaluate(() => {
    const input = document.getElementById("file") as HTMLInputElement;
    const longName = "a".repeat(250) + ".pdf";
    const file = new File([new Uint8Array([0x25, 0x50, 0x44, 0x46])], longName, {
      type: "application/pdf",
    });
    const dt = new DataTransfer();
    dt.items.add(file);
    input.files = dt.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(page.locator(".dropzone .filename")).toContainText("aaaaa");

  const containerWidth = await page.locator(".controls").evaluate((el) => el.clientWidth);
  const zoneWidth = await zone.evaluate((el) => (el as HTMLElement).offsetWidth);
  expect(zoneWidth).toBeLessThanOrEqual(containerWidth);
});

test("shows a status indicator while parsing", async ({ page }) => {
  await page.goto("/");
  await page.locator("input#file").setInputFiles(FIX("sample-text.pdf"));
  // Start parse and, without awaiting the click's full resolution,
  // assert the status text appears.
  const clickPromise = page.locator("button#parse").click();
  await expect(page.locator("#status")).toHaveText(/parsing/i, { timeout: 5_000 });
  await clickPromise;
  await expect(page.locator("#text-output")).toHaveValue(/Hello from LiteParse/, {
    timeout: 45_000,
  });
  await expect(page.locator("#status")).not.toHaveText(/parsing/i);
});

test("parses a scanned PDF with OCR on and extracts text via Tesseract", async ({
  page,
}) => {
  test.setTimeout(180_000);
  await page.goto("/");
  await page.locator("input#ocr").check();
  await page.locator("input#file").setInputFiles(FIX("sample-scanned.pdf"));
  await page.locator("button#parse").click();
  const textArea = page.locator("#text-output");
  // First run may fetch traineddata from CDN; be generous on timeout.
  await expect(textArea).toHaveValue(/OCR|TEST|PAGE/i, { timeout: 150_000 });
});

test("OCR toggle actually changes behavior on a scanned PDF", async ({ page }) => {
  test.setTimeout(180_000);
  await page.goto("/");

  // Pass 1: OCR off. Expect empty or near-empty text — no PDF text layer.
  await page.locator("input#ocr").uncheck();
  await page.locator("input#file").setInputFiles(FIX("sample-scanned.pdf"));
  await page.locator("button#parse").click();
  await expect(page.locator("#status")).toContainText(/Parsed/, { timeout: 60_000 });
  const withoutOcr = await page.locator("#text-output").inputValue();
  expect(withoutOcr.trim().length).toBeLessThan(5);

  // Pass 2: OCR on. Expect non-trivial text.
  await page.locator("input#ocr").check();
  await page.locator("button#parse").click();
  await expect(page.locator("#text-output")).toHaveValue(/OCR|TEST|PAGE/i, {
    timeout: 150_000,
  });
  const withOcr = await page.locator("#text-output").inputValue();
  expect(withOcr.length).toBeGreaterThan(withoutOcr.length + 3);
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
