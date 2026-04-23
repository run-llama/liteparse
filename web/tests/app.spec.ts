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
