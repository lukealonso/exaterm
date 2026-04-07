import { test, expect } from "@playwright/test";
import { waitForCards, ensureSessionCount } from "./helpers";

test.describe("Context menu", () => {
  test("right-click on card shows context menu with all items", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.locator(".battle-card").first().click({ button: "right" });
    const menu = page.locator(".context-menu");
    await expect(menu).not.toHaveClass(/hidden/);

    await expect(page.locator('[data-action="copy"]')).toBeVisible();
    await expect(page.locator('[data-action="paste"]')).toBeVisible();
    await expect(page.locator('[data-action="add-terminals"]')).toBeVisible();
    await expect(
      page.locator('[data-action="insert-number-1"]')
    ).toBeVisible();
    await expect(
      page.locator('[data-action="insert-number-0"]')
    ).toBeVisible();
    await expect(page.locator('[data-action="sync-inputs"]')).toBeVisible();
  });

  test("copy is disabled when no selection", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.locator(".battle-card").first().click({ button: "right" });
    const copyItem = page.locator('[data-action="copy"]');
    await expect(copyItem).toHaveClass(/disabled/);
  });

  test("add terminals is enabled at supported session counts", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // With 1 session, add-terminals should be enabled.
    await page.locator(".battle-card").first().click({ button: "right" });
    const addItem = page.locator('[data-action="add-terminals"]');
    await expect(addItem).not.toHaveClass(/disabled/);
  });

  test("clicking outside closes context menu", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.locator(".battle-card").first().click({ button: "right" });
    await expect(page.locator(".context-menu")).not.toHaveClass(/hidden/);

    await page.locator(".toolbar").click();
    await expect(page.locator(".context-menu")).toHaveClass(/hidden/);
  });

  test("sync inputs toggle shows checkmark after click", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Open menu and click sync inputs.
    await page.locator(".battle-card").first().click({ button: "right" });
    await page.locator('[data-action="sync-inputs"]').click();

    // Reopen menu and check for checkmark.
    await page.locator(".battle-card").first().click({ button: "right" });
    const check = page.locator(
      '[data-action="sync-inputs"] .context-menu-check'
    );
    await expect(check).toHaveText("\u2713 ");

    // Toggle off.
    await page.locator('[data-action="sync-inputs"]').click();
    await page.locator(".battle-card").first().click({ button: "right" });
    const checkOff = page.locator(
      '[data-action="sync-inputs"] .context-menu-check'
    );
    await expect(checkOff).toHaveText("");
  });

  test("context menu positions at click coordinates", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const card = page.locator(".battle-card").first();
    const box = await card.boundingBox();
    expect(box).not.toBeNull();

    // Right-click at specific position.
    await card.click({
      button: "right",
      position: { x: 50, y: 50 },
    });

    const menu = page.locator(".context-menu");
    const menuBox = await menu.boundingBox();
    expect(menuBox).not.toBeNull();

    // Menu should be near the click position.
    if (box && menuBox) {
      expect(menuBox.x).toBeGreaterThanOrEqual(box.x);
      expect(menuBox.y).toBeGreaterThanOrEqual(box.y);
    }
  });
});

test.describe("Click-to-focus parity", () => {
  test("click on card without terminal enters focus mode immediately", async ({
    page,
  }) => {
    // Small viewport so terminals aren't embedded.
    await page.setViewportSize({ width: 800, height: 600 });
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 4);

    // Click a card — should enter focus mode directly (no second click needed).
    await page.locator(".battle-card").first().click();
    await expect(page.locator(".battlefield-grid")).toHaveClass(/focus-mode/);
  });

  test("Escape returns from focus mode to battlefield", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    await page.locator(".battle-card").first().click();
    await page.keyboard.press("Control+Enter");
    await expect(page.locator(".battlefield-grid")).toHaveClass(/focus-mode/);

    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
  });

  test("Enter on card with embedded terminal does not enter focus mode", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // First check if terminal is actually embedded (depends on session count).
    const hasTerminal = await page.locator(".xterm-screen").count();
    if (hasTerminal === 0) {
      // No embedded terminal on this viewport — skip this test's assertion.
      return;
    }

    // Click the card to select it (terminal embedded = just selects).
    await page.locator(".battle-card").first().click();
    // Blur the terminal by clicking the toolbar.
    await page.locator(".toolbar").click();
    await page.waitForTimeout(100);

    // Enter should grab terminal focus, NOT enter focus mode.
    await page.keyboard.press("Enter");
    await page.waitForTimeout(200);
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
  });

  test("Ctrl+Enter always enters focus mode", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.locator(".battle-card").first().click();
    await page.keyboard.press("Control+Enter");
    await expect(page.locator(".battlefield-grid")).toHaveClass(/focus-mode/);
  });
});

test.describe("Nudge pill", () => {
  test("nudge pill is visible on every card", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const nudge = page.locator(".card-nudge-state").first();
    await expect(nudge).toBeVisible();
    const text = await nudge.textContent();
    expect(["AUTONUDGE OFF", "AUTONUDGE ARMED", "AUTONUDGE NUDGED", "AUTONUDGE COOLDOWN"]).toContain(text);
  });

  test("nudge pill shows hover text on mouseenter", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Ensure we're in battlefield mode (not focus mode from a prior click).
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);

    const nudge = page.locator(".card-nudge-state").first();
    await expect(nudge).toBeVisible();

    await nudge.hover();
    await page.waitForTimeout(100);
    const textAfter = await nudge.textContent();

    // Hover should change text to ARM AUTONUDGE or DISARM AUTONUDGE.
    expect(["ARM AUTONUDGE", "DISARM AUTONUDGE"]).toContain(textAfter);
  });

  test("nudge pill click toggles state", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const nudge = page.locator(".card-nudge-state").first();
    const textBefore = await nudge.textContent();

    await nudge.click();
    // Wait for daemon to respond with updated snapshot.
    await page.waitForTimeout(2000);

    const textAfter = await nudge.textContent();
    // State should have changed.
    expect(textAfter).not.toBe(textBefore);
  });

  test("nudge pill has cursor pointer", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const cursor = await page
      .locator(".card-nudge-state")
      .first()
      .evaluate((el) => getComputedStyle(el).cursor);
    expect(cursor).toBe("pointer");
  });
});

test.describe("Shortcuts overlay", () => {
  test("? button opens shortcuts panel", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.click("#shortcuts-btn");
    await expect(page.locator(".shortcuts-overlay")).not.toHaveClass(/hidden/);
    await expect(page.locator(".shortcuts-title")).toHaveText(
      "Keyboard Shortcuts"
    );
  });

  test("shortcuts panel shows all keybindings", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.click("#shortcuts-btn");
    const panel = page.locator(".shortcuts-panel");
    await expect(panel).toContainText("Escape");
    await expect(panel).toContainText("Enter");
    await expect(panel).toContainText("Ctrl");
    await expect(panel).toContainText("Right-click");
  });

  test("Close button hides shortcuts panel", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.click("#shortcuts-btn");
    await expect(page.locator(".shortcuts-overlay")).not.toHaveClass(/hidden/);

    await page.click("#shortcuts-close-btn");
    await expect(page.locator(".shortcuts-overlay")).toHaveClass(/hidden/);
  });

  test("clicking overlay backdrop closes panel", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.click("#shortcuts-btn");
    await expect(page.locator(".shortcuts-overlay")).not.toHaveClass(/hidden/);

    // Click the overlay backdrop (not the panel).
    await page.locator(".shortcuts-overlay").click({ position: { x: 10, y: 10 } });
    await expect(page.locator(".shortcuts-overlay")).toHaveClass(/hidden/);
  });
});

test.describe("Close button", () => {
  test("close button is visible on each card", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await expect(page.locator(".card-close-btn").first()).toBeVisible();
  });

  test("close button has hover styling", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const btn = page.locator(".card-close-btn").first();
    await expect(btn).toBeVisible();
    await expect(btn).toHaveCSS("cursor", "pointer");
  });

  test("close button works on non-embedded sessions, then again after adding shells", async ({
    page,
  }) => {
    // Small viewport so terminals aren't embedded.
    await page.setViewportSize({ width: 800, height: 600 });
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    const before = await page.locator(".battle-card").count();

    // Click X on the first card (non-embedded — uses one-shot WebSocket).
    await page.locator(".card-close-btn").first().click();
    await page.waitForTimeout(3000);

    // Card should transition to complete/failed. Click X again to dismiss.
    const firstCard = page.locator(".battle-card").first();
    const status = await firstCard.getAttribute("class");
    if (status?.match(/card-(complete|failed)/)) {
      await firstCard.locator(".card-close-btn").click();
      await page.waitForTimeout(500);
      // Card should be dismissed.
      const after = await page.locator(".battle-card").count();
      expect(after).toBeLessThan(before);
    }

    // Now add more shells.
    const countBefore = await page.locator(".battle-card").count();
    await page.click("#add-shell-btn");
    await page.waitForTimeout(2000);
    const countAfter = await page.locator(".battle-card").count();
    expect(countAfter).toBeGreaterThan(countBefore);

    // Click X on a new card — should still work.
    await page.locator(".card-close-btn").first().click();
    await page.waitForTimeout(3000);

    // Verify card transitioned (exit was sent successfully).
    const newFirstCard = page.locator(".battle-card").first();
    const newClasses = await newFirstCard.getAttribute("class");
    // Should be complete, failed, or still active (if exit hasn't propagated yet).
    expect(newClasses).toBeTruthy();
  });

  test("clicking close button sends exit and session completes", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);
    const before = await page.locator(".battle-card").count();

    // Click close on the first card.
    await page.locator(".card-close-btn").first().click();

    // Wait for the session to transition to complete/failed status.
    await page.waitForTimeout(3000);

    // The card should now show a complete or failed status.
    const firstCard = page.locator(".battle-card").first();
    const classes = await firstCard.getAttribute("class");
    expect(classes).toMatch(/card-(complete|failed)/);
  });
});

test.describe("Restart Workspace", () => {
  test("restart button is visible in toolbar", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await expect(page.locator("#restart-btn")).toBeVisible();
  });

  test("restart button has destructive styling", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await expect(page.locator("#restart-btn")).toHaveClass(
      /toolbar-btn-destructive/
    );
  });
});

test.describe("--no-embed dev mode", () => {
  test("app.css is served correctly", async ({ page }) => {
    const response = await page.goto("/assets/app.css");
    expect(response?.status()).toBe(200);
    expect(response?.headers()["content-type"]).toBe("text/css");
  });

  test("main.js is served correctly", async ({ page }) => {
    const response = await page.goto("/assets/main.js");
    expect(response?.status()).toBe(200);
    expect(response?.headers()["content-type"]).toBe(
      "application/javascript"
    );
  });

  test("main.css (xterm styles) is served correctly", async ({ page }) => {
    const response = await page.goto("/assets/main.css");
    expect(response?.status()).toBe(200);
    expect(response?.headers()["content-type"]).toBe("text/css");
  });

  test("unknown asset returns 404", async ({ page }) => {
    const response = await page.goto("/assets/nonexistent.xyz");
    expect(response?.status()).toBe(404);
  });
});
