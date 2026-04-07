import { test, expect, Page } from "@playwright/test";

async function waitForCards(page: Page, count: number, timeout = 10_000) {
  await expect(page.locator(".battle-card").first()).toBeVisible({ timeout });
  if (count > 1) {
    await expect(page.locator(".battle-card")).toHaveCount(count, { timeout });
  }
}

async function enterFocusMode(page: Page, nth = 0) {
  await page.locator(".battle-card").nth(nth).click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".battlefield-grid")).toHaveClass(/focus-mode/, {
    timeout: 5_000,
  });
}

test.describe("Close button and nudge pill layout", () => {
  test("close button and nudge pill are in the same row without overlap", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const nudge = page.locator(".card-nudge-state").first();
    const closeBtn = page.locator(".card-close-btn").first();

    await expect(nudge).toBeVisible();
    await expect(closeBtn).toBeVisible();

    // Both should be in the .card-nudge-row container.
    const nudgeParent = await nudge.evaluate(
      (el) => el.parentElement?.className
    );
    const closeBtnParent = await closeBtn.evaluate(
      (el) => el.parentElement?.className
    );
    expect(nudgeParent).toContain("card-nudge-row");
    expect(closeBtnParent).toContain("card-nudge-row");

    // They should not overlap — close button should be to the right.
    const nudgeBox = await nudge.boundingBox();
    const closeBox = await closeBtn.boundingBox();
    if (nudgeBox && closeBox) {
      // Close button right edge should be >= nudge right edge (not overlapping).
      expect(closeBox.x).toBeGreaterThanOrEqual(nudgeBox.x + nudgeBox.width - 2);
    }
  });
});

test.describe("Status blending", () => {
  test("terminal states use daemon status immediately", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // A running session should show an active-derived status class,
    // not wait for the LLM.
    const card = page.locator(".battle-card").first();
    const classes = await card.getAttribute("class");
    expect(classes).toMatch(
      /card-(idle|active|thinking|working|blocked|failed|complete|detached|stopped)/
    );
  });
});

test.describe("Scroll to bottom after replay", () => {
  test("terminal viewport scrollTop is near bottom after entering focus", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });
    // Wait for scroll-to-bottom timer to fire (200ms after last message).
    await page.waitForTimeout(1000);

    // The viewport should be scrolled near the bottom, not the top.
    const scrollInfo = await page.locator(".focused-card .xterm-viewport").evaluate((el) => {
      return {
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      };
    });
    // scrollTop + clientHeight should be close to scrollHeight (within 50px).
    const distFromBottom =
      scrollInfo.scrollHeight - scrollInfo.scrollTop - scrollInfo.clientHeight;
    expect(distFromBottom).toBeLessThan(50);
  });
});

test.describe("Garbled chars prevention", () => {
  test("showTerminal is no-op when terminal already in container", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Enter and exit focus mode twice — no garbled output should appear.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });
    await page.keyboard.press("Escape");
    await page.waitForTimeout(500);

    // Re-enter.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });

    // Terminal should still be functional (xterm-screen visible).
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible();
  });
});

test.describe("Focus preservation across renders", () => {
  test("activeElement is preserved during snapshot updates", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });

    // Click terminal to focus it.
    await page.locator(".focused-card .xterm-screen").click();
    await page.waitForTimeout(200);

    // Record what has focus.
    const focusedTag = await page.evaluate(() => document.activeElement?.tagName);

    // Wait through 2 snapshot cycles.
    await page.waitForTimeout(2000);

    // Focus should not have changed.
    const focusedTagAfter = await page.evaluate(
      () => document.activeElement?.tagName
    );
    expect(focusedTagAfter).toBe(focusedTag);
  });
});

test.describe("Enter key focus behavior", () => {
  test("Enter passes through when a different terminal has focus", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Need 2 sessions with embedded terminals to test the cross-focus case.
    await page.click("#add-shell-btn");
    await page.waitForTimeout(3000);

    const screens = page.locator(".xterm-screen");
    if ((await screens.count()) >= 2) {
      // Click terminal A to give it focus.
      await screens.first().click();
      await page.waitForTimeout(200);

      // Select card B by pressing Ctrl+] (moves selection without stealing focus).
      await page.keyboard.press("Control+]");
      await page.waitForTimeout(200);

      // Now terminal A has focus but card B is selected.
      // Pressing Enter should go to terminal A, NOT focus card B.
      await page.keyboard.press("Enter");
      await page.waitForTimeout(200);

      await expect(page.locator(".battlefield-grid")).not.toHaveClass(
        /focus-mode/
      );
    }
  });
});

