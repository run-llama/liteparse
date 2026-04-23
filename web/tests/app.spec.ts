import { expect, test } from "@playwright/test";

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
