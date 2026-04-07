import { test, expect, Page } from "@playwright/test";

async function waitForCards(page: Page, count: number, timeout = 10_000) {
  await expect(page.locator(".battle-card").first()).toBeVisible({ timeout });
  if (count > 1) {
    await expect(page.locator(".battle-card")).toHaveCount(count, { timeout });
  }
}

async function ensureSessionCount(page: Page, target: number) {
  while ((await page.locator(".battle-card").count()) < target) {
    await page.click("#add-shell-btn");
    await page.waitForTimeout(1500);
  }
}

async function enterFocusMode(page: Page) {
  await page.locator(".battle-card").first().click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
    timeout: 15_000,
  });
  await page.waitForTimeout(2000);
}

test.describe("Terminal clipping", () => {
  test.fixme("cursor row is fully visible after scrolling in focus mode", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 4);
    await enterFocusMode(page);

    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type(
      "for i in $(seq 1 100); do echo line-$i; done\n",
      { delay: 10 }
    );
    await page.waitForTimeout(1000);

    const overflow = await page.locator(".focused-card .xterm-wrapper").evaluate((el) => {
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
    expect(overflow.overflows).toBe(false);
  });

  test.fixme("last xterm row is not clipped in focus mode", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await enterFocusMode(page);

    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type(
      "for i in $(seq 1 200); do echo line-$i; done\n",
      { delay: 10 }
    );
    await page.waitForTimeout(1000);

    const viewport = page.locator(".focused-card .xterm-viewport").first();
    const screen = page.locator(".focused-card .xterm-screen").first();
    const viewportBox = await viewport.boundingBox();
    const screenBox = await screen.boundingBox();

    expect(viewportBox).not.toBeNull();
    expect(screenBox).not.toBeNull();
    if (viewportBox && screenBox) {
      const screenBottom = screenBox.y + screenBox.height;
      const viewportBottom = viewportBox.y + viewportBox.height;
      expect(screenBottom).toBeLessThanOrEqual(viewportBottom + 2);
    }
  });

  test("xterm rows element fits within terminal slot", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await enterFocusMode(page);

    const overflow = await page.locator(".focused-card .card-terminal-slot").evaluate((el) => {
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
    expect(overflow.overflows).toBe(false);
  });
});
