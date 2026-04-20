import { test, expect } from "@playwright/test";
import {
  waitForCards,
  enterFocusMode,
  ensureSessionCount,
  firstSessionId,
  resetWorkspace,
  terminalContainsText,
  waitForCardCountIncrease,
  waitForTerminalInputFocus,
} from "./helpers";

test.beforeEach(async ({ page }) => {
  await resetWorkspace(page);
});

test.describe("Add one terminal", () => {
  test("Add Shell button and Ctrl+Shift+N each add exactly one session", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    let before = await page.locator(".battle-card").count();
    await page.click("#add-shell-btn");
    let after = await waitForCardCountIncrease(page, before);
    expect(after).toBe(before + 1);

    before = after;
    await page.keyboard.press("Control+Shift+N");
    after = await waitForCardCountIncrease(page, before);
    expect(after).toBe(before + 1);
  });
});

test.describe("Terminal scrollback preservation", () => {
  test("terminal scrollback survives focus/unfocus cycle", async ({
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
    await page.keyboard.type("echo SCROLLBACK_MARKER_123\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "SCROLLBACK_MARKER_123"))
      .toBe(true);

    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );

    await enterFocusMode(page);
    await expect(screen).toBeVisible({ timeout: 10_000 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "SCROLLBACK_MARKER_123"))
      .toBe(true);
  });
});

test.describe("Focus preservation", () => {
  test("render does not remove focused-card class during snapshot updates", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);
    await page.waitForTimeout(2200);
    await expect(page.locator(".focused-card")).toBeVisible();
  });
});

test.describe("Stream WebSocket reconnection", () => {
  test("terminal remains functional after page reload", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await page.reload();
    await waitForCards(page, 1);
    const sessionId = await firstSessionId(page);

    await enterFocusMode(page);
    const screen = page.locator(".focused-card .xterm-screen");
    await expect(screen).toBeVisible({ timeout: 10_000 });
    await screen.click();
    await waitForTerminalInputFocus(page);
    await page.keyboard.type("echo RELOAD_ALIVE_123\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "RELOAD_ALIVE_123"))
      .toBe(true);
  });
});

test.describe("Scrollback preview", () => {
  test("preview updates from xterm buffer when available", async ({
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
    await page.keyboard.type("echo PREVIEW_MARKER_XYZ\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "PREVIEW_MARKER_XYZ"))
      .toBe(true);

    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
    await page.setViewportSize({ width: 800, height: 600 });
    await ensureSessionCount(page, 3);

    const firstCard = page.locator(".battle-card").first();
    await expect(firstCard.locator(".card-scrollback-text")).toContainText(
      "PREVIEW_MARKER_XYZ"
    );
  });
});
