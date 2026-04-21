import { test, expect } from "@playwright/test";
import {
  ensureSessionCount,
  waitForCards,
  enterFocusMode,
  firstSessionId,
  resetWorkspace,
  terminalContainsText,
  waitForTerminalInputFocus,
} from "./helpers";

test.beforeEach(async ({ page }) => {
  await resetWorkspace(page);
});

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
    await expect
      .poll(async () => {
        const scrollInfo = await page
          .locator(".focused-card .xterm-viewport")
          .evaluate((el) => ({
            scrollTop: el.scrollTop,
            scrollHeight: el.scrollHeight,
            clientHeight: el.clientHeight,
          }));
        return (
          scrollInfo.scrollHeight -
          scrollInfo.scrollTop -
          scrollInfo.clientHeight
        );
      })
      .toBeLessThan(50);
  });
});

test.describe("Garbled chars prevention", () => {
  test("re-entering focus keeps one live terminal with intact output", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);
    const sessionId = await firstSessionId(page);
    const screen = page.locator(".focused-card .xterm-screen");
    await expect(screen).toBeVisible({ timeout: 10_000 });
    await screen.click();
    await waitForTerminalInputFocus(page);
    await page.keyboard.type("echo FIRST_FOCUS_MARKER_123\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "FIRST_FOCUS_MARKER_123"))
      .toBe(true);

    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );

    await enterFocusMode(page);
    await expect(screen).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".focused-card .card-terminal-slot .xterm")).toHaveCount(1);
    await screen.click();
    await waitForTerminalInputFocus(page);
    await page.keyboard.type("echo SECOND_FOCUS_MARKER_456\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "SECOND_FOCUS_MARKER_456"))
      .toBe(true);
  });
});

test.describe("Enter key focus behavior", () => {
  test("Enter passes through when a different terminal has focus", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Need 2 sessions with embedded terminals to test the cross-focus case.
    await ensureSessionCount(page, 2);

    const screens = page.locator(".xterm-screen");
    await expect(screens).toHaveCount(2, { timeout: 5000 });

    // Click terminal A to give it focus.
    await screens.first().click();
    await waitForTerminalInputFocus(page);

    // Select card B by pressing Ctrl+] (moves selection without stealing focus).
    await page.keyboard.press("Control+]");
    const secondCard = page.locator(".battle-card").nth(1);
    await expect(secondCard).toHaveClass(/selected-card/);

    // Now terminal A has focus but card B is selected.
    // Pressing Enter should go to terminal A, NOT focus card B.
    await page.keyboard.press("Enter");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
    await expect(secondCard).toHaveClass(/selected-card/);
    await waitForTerminalInputFocus(page);
  });
});
