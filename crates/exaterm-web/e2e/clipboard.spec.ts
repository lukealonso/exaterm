import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  clearTerminalSelection,
  firstSessionId,
  getClipboardText,
  getExecCommandCopyText,
  installClipboardStub,
  installExecCommandCopyStub,
  resetWorkspace,
  setClipboardText,
  selectTerminalText,
  terminalContainsText,
  terminalTextCenter,
} from "./helpers";

async function focusTerminal(page: Page): Promise<number> {
  await page.locator(".battle-card").first().click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
    timeout: 10_000,
  });
  await page.locator(".focused-card .xterm-screen").click();
  return firstSessionId(page);
}

async function rightClickTerminalText(page: Page, sessionId: number, needle: string) {
  const point = await terminalTextCenter(page, sessionId, needle);
  await page.mouse.click(point.x, point.y, { button: "right" });
}

test.beforeEach(async ({ page }) => {
  await installClipboardStub(page);
  await resetWorkspace(page);
});

test.describe("Clipboard flows", () => {
  test("context-menu copy uses the right-click snapshot even if selection clears", async ({
    page,
  }) => {
    const sessionId = await focusTerminal(page);
    await page.keyboard.type("printf 'SNAPSHOT_COPY_123\\n'\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "SNAPSHOT_COPY_123"))
      .toBe(true);
    await selectTerminalText(page, sessionId, "SNAPSHOT_COPY_123");

    await page.locator(".battle-card").first().click({ button: "right" });
    await expect(page.locator('[data-action="copy"]')).not.toHaveClass(
      /disabled/
    );

    await clearTerminalSelection(page, sessionId);
    await page.locator('[data-action="copy"]').click();

    await expect
      .poll(() => getClipboardText(page))
      .toBe("SNAPSHOT_COPY_123");
  });

  test("paste keeps the existing clipboard when right-clicking terminal text", async ({
    page,
  }) => {
    const sessionId = await focusTerminal(page);
    await page.keyboard.type("printf 'RIGHTCLICK_TARGET_123\\n'\n", {
      delay: 20,
    });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "RIGHTCLICK_TARGET_123"))
      .toBe(true);

    const pasteCommand = "printf 'PASTE_MARKER_123\\n'";
    await setClipboardText(page, pasteCommand);

    await rightClickTerminalText(page, sessionId, "RIGHTCLICK_TARGET_123");
    await page.locator('[data-action="paste"]').click();

    await expect
      .poll(() => getClipboardText(page))
      .toBe(pasteCommand);

    await page.keyboard.press("Enter");
    await expect
      .poll(() => terminalContainsText(page, sessionId, "PASTE_MARKER_123"))
      .toBe(true);
  });

  test("copy stays disabled after selection is cleared and does not reuse stale text", async ({
    page,
  }) => {
    const sessionId = await focusTerminal(page);
    await page.keyboard.type("printf 'STALE_SELECTION_123\\n'\n", {
      delay: 20,
    });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "STALE_SELECTION_123"))
      .toBe(true);
    await selectTerminalText(page, sessionId, "STALE_SELECTION_123");
    await clearTerminalSelection(page, sessionId);

    await setClipboardText(page, "UNCHANGED_CLIPBOARD_456");

    await page.locator(".battle-card").first().click({ button: "right" });
    const copyItem = page.locator('[data-action="copy"]');
    await expect(copyItem).toHaveClass(/disabled/);

    await copyItem.click({ force: true });
    await expect
      .poll(() => getClipboardText(page))
      .toBe("UNCHANGED_CLIPBOARD_456");
  });

  test("copy stays disabled after a selected terminal is hidden into preview mode", async ({
    page,
  }) => {
    const sessionId = await focusTerminal(page);
    await page.keyboard.type("printf 'HIDDEN_SELECTION_123\\n'\n", {
      delay: 20,
    });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "HIDDEN_SELECTION_123"))
      .toBe(true);
    await selectTerminalText(page, sessionId, "HIDDEN_SELECTION_123");
    await setClipboardText(page, "UNCHANGED_CLIPBOARD_789");

    await page.keyboard.press("Escape");
    const copyItem = page.locator('[data-action="copy"]');
    await page.locator(".battle-card").first().click({ button: "right" });
    await expect(copyItem).toHaveClass(/disabled/);

    await copyItem.click({ force: true });
    await expect
      .poll(() => getClipboardText(page))
      .toBe("UNCHANGED_CLIPBOARD_789");
  });

  test("copy falls back to execCommand when async clipboard is unavailable", async ({
    page,
  }) => {
    const sessionId = await focusTerminal(page);
    await page.keyboard.type("printf 'FALLBACK_COPY_123\\n'\n", { delay: 20 });
    await expect
      .poll(() => terminalContainsText(page, sessionId, "FALLBACK_COPY_123"))
      .toBe(true);
    await selectTerminalText(page, sessionId, "FALLBACK_COPY_123");

    await installExecCommandCopyStub(page);
    await page.locator(".battle-card").first().click({ button: "right" });
    await page.locator('[data-action="copy"]').click();

    await expect
      .poll(() => getExecCommandCopyText(page))
      .toBe("FALLBACK_COPY_123");
  });
});
