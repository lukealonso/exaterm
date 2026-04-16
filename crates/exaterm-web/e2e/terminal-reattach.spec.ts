import { test, expect } from "@playwright/test";
import { waitForCards, ensureSessionCount } from "./helpers";
import type { Page } from "@playwright/test";

async function enterFocusModeWithWait(page: Page) {
  await page.locator(".battle-card").first().click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
    timeout: 15_000,
  });
  await page.waitForTimeout(2000);
}

test.describe("Terminal reattach after embed/scrollback transition", () => {
  test.fixme("terminal is functional after adding shells and re-focusing", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await enterFocusModeWithWait(page);

    // Type a marker.
    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type("echo MARKER_REATTACH_TEST\n", { delay: 20 });
    await page.waitForTimeout(500);

    // Return to battlefield and add shells (disposes the terminal).
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
    await ensureSessionCount(page, 4);
    await page.waitForTimeout(500);

    // Re-focus the first card.
    await page.locator(".battle-card").first().dblclick();
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 15_000,
    });
    await page.waitForTimeout(3000);

    // Type a new command to prove the terminal is functional after reattach.
    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type("echo REATTACH_ALIVE\n", { delay: 30 });
    await expect(page.locator(".xterm-rows").first()).toContainText(
      "REATTACH_ALIVE",
      { timeout: 10_000 }
    );
  });

  test.fixme("terminal is scrollable to bottom after reattach", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await enterFocusModeWithWait(page);

    // Generate lots of output.
    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type(
      "for i in $(seq 1 80); do echo reattach-line-$i; done\n",
      { delay: 10 }
    );
    await page.waitForTimeout(1000);

    // Return to battlefield and add shells.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
    await ensureSessionCount(page, 4);
    await page.waitForTimeout(500);

    // Re-focus.
    await page.locator(".battle-card").first().dblclick();
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 15_000,
    });
    await page.waitForTimeout(3000);

    // Type at the bottom — proves terminal is live and scrolled to bottom.
    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type("echo BOTTOM_CHECK\n", { delay: 30 });
    await expect(page.locator(".xterm-rows").first()).toContainText(
      "BOTTOM_CHECK",
      { timeout: 10_000 }
    );
  });
});
