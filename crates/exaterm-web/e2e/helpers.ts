import { expect, Page } from "@playwright/test";

/** Wait for exactly `count` battle cards to be visible. */
export async function waitForCards(page: Page, count: number, timeout = 10_000) {
  const cards = page.locator(".battle-card");
  await expect(cards).toHaveCount(count, { timeout });
  await expect(cards.first()).toBeVisible({ timeout });
}

/** Enter focus mode on the nth card via Ctrl+Enter. */
export async function enterFocusMode(page: Page, nth = 0) {
  await page.locator(".battle-card").nth(nth).click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".battlefield-grid")).toHaveClass(/focus-mode/, {
    timeout: 5_000,
  });
}

/** Add shells until we have at least `target` sessions. Fails after 10 attempts. */
export async function ensureSessionCount(page: Page, target: number) {
  const maxAttempts = 10;
  let attempts = 0;
  while ((await page.locator(".battle-card").count()) < target && attempts < maxAttempts) {
    await page.click("#add-shell-btn");
    await page.waitForTimeout(1500);
    attempts++;
  }
  if ((await page.locator(".battle-card").count()) < target) {
    throw new Error(`Failed to reach ${target} sessions after ${maxAttempts} attempts`);
  }
}
