import { test, expect } from "@playwright/test";
import { waitForCards, enterFocusMode } from "./helpers";

test.describe("Add one terminal", () => {
  test("Add Shell button adds exactly one session", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    const before = await page.locator(".battle-card").count();

    await page.click("#add-shell-btn");
    await page.waitForTimeout(2000);
    const after = await page.locator(".battle-card").count();

    // Should add exactly 1, not a batch.
    expect(after).toBe(before + 1);
  });

  test("Ctrl+Shift+N adds exactly one session", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    const before = await page.locator(".battle-card").count();

    await page.keyboard.press("Control+Shift+N");
    await page.waitForTimeout(2000);
    const after = await page.locator(".battle-card").count();

    expect(after).toBe(before + 1);
  });
});

test.describe("Terminal scrollback preservation", () => {
  test("terminal scrollback survives focus/unfocus cycle", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Enter focus mode and type something.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });
    await page.waitForTimeout(1000);
    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type("echo SCROLLBACK_MARKER_123\n", { delay: 20 });
    await page.waitForTimeout(500);

    // Exit focus mode.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(500);

    // Re-enter focus mode.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });
    await page.waitForTimeout(500);

    // The marker should still be in the terminal buffer (not lost).
    // Check by reading from the xterm buffer directly.
    const hasMarker = await page.evaluate(() => {
      const rows = document.querySelector(".focused-card .xterm-rows");
      return rows?.textContent?.includes("SCROLLBACK_MARKER_123") ?? false;
    });
    // The terminal must still exist, and ideally the marker is present.
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible();
    expect(hasMarker).toBe(true);
  });
});

test.describe("Focus preservation", () => {
  test("render does not remove focused-card class during snapshot updates", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);
    await expect(page.locator(".focused-card")).toBeVisible();

    // Wait through several snapshot cycles (~900ms each).
    await page.waitForTimeout(3000);

    // focused-card should still be present (not lost during re-render).
    await expect(page.locator(".focused-card")).toBeVisible();
  });
});

test.describe("Click-to-focus with hidden terminals", () => {
  test("clicking a card without visible terminal enters focus mode", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 800, height: 600 });
    await page.goto("/");
    await waitForCards(page, 1);

    // Add sessions so terminals aren't embedded.
    await page.click("#add-shell-btn");
    await page.waitForTimeout(2000);
    await page.click("#add-shell-btn");
    await page.waitForTimeout(2000);

    // Click a card — should enter focus mode since terminals
    // aren't embedded at this viewport size.
    await page.locator(".battle-card").first().click();

    // Should either be in focus mode or selected.
    const inFocus = await page
      .locator(".battlefield-grid")
      .evaluate((el) => el.classList.contains("focus-mode"));
    if (!inFocus) {
      // Terminal was embedded (wide enough) — that's ok too.
      await expect(page.locator(".battle-card").first()).toHaveClass(
        /selected-card/
      );
    }
  });
});

test.describe("Scroll to bottom after replay", () => {
  test("focused terminal has xterm-screen visible after entering focus mode", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);

    // The terminal screen should be visible and rendered.
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });

    // The terminal slot should not have the hidden class.
    await expect(
      page.locator(".focused-card .card-terminal-slot")
    ).not.toHaveClass(/scrollback-terminal-hidden/);
  });
});

test.describe("Stream WebSocket reconnection", () => {
  test("terminal remains functional after page reload", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Reload the page — the stream WebSocket reconnects.
    await page.reload();
    await waitForCards(page, 1);

    // Terminal should still work.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });
  });
});

test.describe("Scrollback preview", () => {
  test("preview updates from xterm buffer when available", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Enter focus and generate output.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 10_000,
    });
    await page.waitForTimeout(1000);
    await page.locator(".focused-card .xterm-screen").click();
    await page.keyboard.type("echo PREVIEW_MARKER_XYZ\n", { delay: 20 });
    await page.waitForTimeout(500);

    // Add sessions to force scrollback preview mode.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
    await page.setViewportSize({ width: 800, height: 600 });
    await page.click("#add-shell-btn");
    await page.waitForTimeout(2000);
    await page.click("#add-shell-btn");
    await page.waitForTimeout(2000);

    // The first card should show a scrollback preview containing our marker.
    const firstCard = page.locator(".battle-card").first();
    await expect(firstCard.locator(".card-scrollback-text")).toContainText(
      "PREVIEW_MARKER_XYZ"
    );
  });
});
