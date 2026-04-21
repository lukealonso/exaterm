import { test, expect } from "@playwright/test";
import {
  terminalConnectionState,
  ensureSessionCount,
  firstSessionId,
  resetWorkspace,
  terminalContainsText,
  waitForTerminalInputFocus,
} from "./helpers";
import type { Page } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await resetWorkspace(page);
});

async function enterFocusModeWithWait(page: Page) {
  await page.locator(".battle-card").first().click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
    timeout: 15_000,
  });
}

async function typeIntoFocusedTerminal(page: Page, text: string) {
  const termScreen = page.locator(".focused-card .xterm-screen");
  await expect(termScreen).toBeVisible({ timeout: 15_000 });
  await waitForTerminalInputFocus(page);
  await page.locator(":focus").pressSequentially(text, { delay: 20 });
}

test.describe("Terminal reattach after embed/scrollback transition", () => {
  test("terminal is functional after adding shells and re-focusing", async ({
    page,
  }) => {
    await enterFocusModeWithWait(page);
    const sessionId = await firstSessionId(page);
    await expect
      .poll(() => terminalConnectionState(page, sessionId))
      .toBe(WebSocket.OPEN);

    // Seed the terminal with output before the embed/preview transition.
    await typeIntoFocusedTerminal(page, "echo MARKER_REATTACH_TEST\n");
    await expect
      .poll(() => terminalContainsText(page, sessionId, "MARKER_REATTACH_TEST"))
      .toBe(true);

    // Return to battlefield and add shells so the first card drops back to a
    // preview and then has to reattach its live terminal when focused again.
    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
    await ensureSessionCount(page, 4);

    // Re-focus the first card using the same deterministic shortcut path we
    // use elsewhere in the suite.
    await enterFocusModeWithWait(page);
    await expect
      .poll(() => terminalConnectionState(page, sessionId))
      .toBe(WebSocket.OPEN);

    // Use the actual keyboard input path after reattach. This catches cases
    // where the terminal redraws but no longer accepts focused input.
    await typeIntoFocusedTerminal(page, "echo REATTACH_ALIVE\n");
    await expect
      .poll(() => terminalContainsText(page, sessionId, "REATTACH_ALIVE"))
      .toBe(true);
  });

  test("terminal is scrollable to bottom after reattach", async ({ page }) => {
    await enterFocusModeWithWait(page);
    const sessionId = await firstSessionId(page);
    await expect
      .poll(() => terminalConnectionState(page, sessionId))
      .toBe(WebSocket.OPEN);

    // Generate lots of output.
    await typeIntoFocusedTerminal(
      page,
      "for i in $(seq 1 80); do echo reattach-line-$i; done\n"
    );
    await expect
      .poll(() => terminalContainsText(page, sessionId, "reattach-line-80"), {
        timeout: 10_000,
      })
      .toBe(true);

    // Return to battlefield and add shells so the terminal has to reattach.
    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
    await ensureSessionCount(page, 4);

    // Re-focus via the deterministic keyboard path.
    await enterFocusModeWithWait(page);
    await expect
      .poll(() => terminalConnectionState(page, sessionId))
      .toBe(WebSocket.OPEN);

    // Type after reattach and verify the newest output lands at the bottom of
    // the viewport instead of getting stuck above a stale scroll position.
    await typeIntoFocusedTerminal(page, "echo BOTTOM_CHECK\n");
    await expect
      .poll(() => terminalContainsText(page, sessionId, "BOTTOM_CHECK"))
      .toBe(true);
    const scrollInfo = await page
      .locator(".focused-card .xterm-viewport")
      .evaluate((el) => ({
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      }));
    const distFromBottom =
      scrollInfo.scrollHeight - scrollInfo.scrollTop - scrollInfo.clientHeight;
    expect(distFromBottom).toBeLessThan(50);
  });
});
