import { test, expect } from "@playwright/test";
import {
  waitForCards,
  ensureSessionCount,
  resetWorkspace,
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
  await page.waitForTimeout(2000);
}

test.describe("Terminal clipping", () => {
  test("focused terminal content stays fully inside the visible slot after scrolling", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 4);
    await enterFocusModeWithWait(page);

    const screen = page.locator(".focused-card .xterm-screen");
    await screen.click();
    await page.keyboard.type(
      "for i in $(seq 1 200); do echo line-$i; done\n",
      { delay: 10 }
    );
    await page.waitForTimeout(1000);

    const wrapperOverflow = await page.locator(".focused-card .xterm-wrapper").evaluate((el) => {
      const wrapper = el as HTMLElement;
      const screen = wrapper.querySelector(".xterm-screen") as HTMLElement | null;
      if (!screen) return { overflows: false, reason: "no screen" };
      const wrapperRect = wrapper.getBoundingClientRect();
      const screenRect = screen.getBoundingClientRect();
      return {
        overflows: screenRect.bottom > wrapperRect.bottom + 2,
        diff: screenRect.bottom - wrapperRect.bottom,
      };
    });
    expect(wrapperOverflow.overflows).toBe(false);

    const viewport = page.locator(".xterm-viewport").first();
    await expect(viewport).toBeVisible({ timeout: 10_000 });
    await expect(screen).toBeVisible({ timeout: 10_000 });
    const viewportBox = await viewport.boundingBox();
    const screenBox = await screen.boundingBox();

    expect(viewportBox).not.toBeNull();
    expect(screenBox).not.toBeNull();
    if (viewportBox && screenBox) {
      const screenBottom = screenBox.y + screenBox.height;
      const viewportBottom = viewportBox.y + viewportBox.height;
      expect(screenBottom).toBeLessThanOrEqual(viewportBottom + 2);
    }

    const slotOverflow = await page.locator(".focused-card .card-terminal-slot").evaluate((el) => {
      const slot = el as HTMLElement;
      const xterm = slot.querySelector(".xterm") as HTMLElement | null;
      if (!xterm) return { overflows: false, reason: "no xterm" };
      const slotRect = slot.getBoundingClientRect();
      const xtermRect = xterm.getBoundingClientRect();
      return {
        overflows: xtermRect.bottom > slotRect.bottom + 2,
        diff: xtermRect.bottom - slotRect.bottom,
      };
    });
    expect(slotOverflow.overflows).toBe(false);
  });
});
