import { test, expect } from "@playwright/test";
import { waitForCards, ensureSessionCount, resetWorkspace } from "./helpers";

test.beforeEach(async ({ page }) => {
  await resetWorkspace(page);
});

test.describe("Context menu", () => {
  test("right-click on card shows context menu items and disabled copy with no selection", async ({
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
    await expect(page.locator('[data-action="copy"]')).toHaveClass(/disabled/);
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

  test("Enter on card with embedded terminal does not enter focus mode", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto("/");
    await waitForCards(page, 1);
    await expect(page.locator(".battle-card .xterm-screen")).toHaveCount(1);

    const firstCard = page.locator(".battle-card").first();
    await firstCard.click();
    await expect(firstCard).toHaveClass(/selected-card/);
    await page.locator(".toolbar").click();

    await page.keyboard.press("Enter");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
    await expect
      .poll(
        () =>
          page.evaluate(
            () =>
              (document.activeElement as HTMLElement | null)?.getAttribute(
                "aria-label"
              ) ?? ""
          ),
        { timeout: 5_000 }
      )
      .toBe("Terminal input");
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

    const nudge = page.locator(".card-nudge-state").first();
    await expect(nudge).toBeVisible();

    await nudge.hover();
    await expect
      .poll(async () => await nudge.textContent())
      .toMatch(/^(ARM|DISARM) AUTONUDGE$/);
  });

  test("nudge pill click toggles state", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const nudge = page.locator(".card-nudge-state").first();
    const textBefore = await nudge.textContent();

    await nudge.click();
    await expect.poll(async () => await nudge.textContent()).not.toBe(textBefore);
  });
});

test.describe("Shortcuts overlay", () => {
  test("? button opens shortcuts panel with keybindings", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await page.click("#shortcuts-btn");
    await expect(page.locator(".shortcuts-overlay")).not.toHaveClass(/hidden/);
    await expect(page.locator(".shortcuts-title")).toHaveText(
      "Keyboard Shortcuts"
    );
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
  test("clicking close button sends exit and session completes", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);
    const before = await page.locator(".battle-card").count();

    // Click close on the first card.
    await page.locator(".card-close-btn").first().click();

    // The card should now show a complete or failed status.
    const firstCard = page.locator(".battle-card").first();
    await expect.poll(() => firstCard.getAttribute("class")).toMatch(
      /card-(complete|failed)/
    );
  });
});
