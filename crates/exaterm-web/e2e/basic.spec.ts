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

test.describe("Page load", () => {
  test("shows connected workspace chrome and an embedded terminal", async ({
    page,
  }) => {
    await page.goto("/");
    await expect(page.locator(".toolbar-title")).toHaveText("Exaterm");
    await expect(page.locator("#add-shell-btn")).toBeVisible();
    await waitForCards(page, 1);
    await expect(page.locator("#session-count")).toContainText("session");
    await expect(
      page.locator(".battle-card .xterm-screen").first()
    ).toBeVisible({ timeout: 5_000 });
    await expect(page.locator("#reconnect-overlay")).toHaveClass(/hidden/);
  });
});

test.describe("Session management", () => {
  test("session count label updates after adding", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    const before = await page.locator(".battle-card").count();

    await page.click("#add-shell-btn");
    const after = await waitForCardCountIncrease(page, before);
    await expect(page.locator("#session-count")).toHaveText(
      `${after} sessions`
    );
  });

  test("each card has a unique session ID", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);
    const ids = await page
      .locator(".battle-card")
      .evaluateAll((els) =>
        els.map((el) => (el as HTMLElement).dataset.sessionId)
      );
    const unique = new Set(ids);
    expect(unique.size).toBe(ids.length);
  });
});

test.describe("Card selection", () => {
  test("clicking an embedded card selects it without entering focus mode", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto("/");
    await waitForCards(page, 1);
    await expect(page.locator(".battle-card .xterm-screen")).toHaveCount(1);

    const first = page.locator(".battle-card").first();
    await first.click();
    await expect(first).toHaveClass(/selected-card/);
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
  });

  test("selecting a different embedded card clears the previous selection", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);
    await expect(page.locator(".battle-card .xterm-screen")).toHaveCount(2);

    const first = page.locator(".battle-card").first();
    const second = page.locator(".battle-card").nth(1);
    await first.click();
    await expect(first).toHaveClass(/selected-card/);
    await expect(second).not.toHaveClass(/selected-card/);

    await second.click();
    await expect(page.locator(".selected-card")).toHaveCount(1);
    await expect(second).toHaveClass(/selected-card/);
    await expect(first).not.toHaveClass(/selected-card/);
  });
});

test.describe("Focus mode", () => {
  test("only focused card is visible in focus mode", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    await enterFocusMode(page);
    const visible = page.locator('.battle-card:not([style*="display: none"])');
    await expect(visible).toHaveCount(1);
    await expect(visible.first()).toHaveClass(/focused-card/);
  });

  test("Escape returns to battlefield", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);
    const total = await page.locator(".battle-card").count();

    await enterFocusMode(page);

    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
    const allVisible = page.locator(
      '.battle-card:not([style*="display: none"])'
    );
    await expect(allVisible).toHaveCount(total);
  });

  test("Escape works even when terminal has focus", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    await enterFocusMode(page);
    const termScreen = page.locator(".focused-card .xterm-screen");
    await termScreen.click();
    await page.keyboard.type("a");

    await page.keyboard.press("Escape");
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
  });

  test("clicking focused card returns to battlefield", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    await enterFocusMode(page);
    await page.locator(".focused-card .card-header-row").click();
    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /focus-mode/
    );
  });
});

test.describe("Card status styling", () => {
  test("cards render a labeled status chip", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    const chip = page.locator(".card-status").first();
    const classes = await chip.getAttribute("class");
    expect(classes).toMatch(
      /battle-(idle|active|thinking|working|blocked|failed|complete|detached|stopped)/
    );
    const chipText = await chip.textContent();
    expect(chipText?.trim().length ?? 0).toBeGreaterThan(0);
  });
});

test.describe("Grid layout", () => {
  test("single-session class is present when only one session", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await expect(page.locator(".battlefield-grid")).toHaveClass(
      /single-session/
    );
  });

  test("multi-session layout drops single-session mode and uses multiple columns", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 4);

    await expect(page.locator(".battlefield-grid")).not.toHaveClass(
      /single-session/
    );
    const columns = await page
      .locator(".battlefield-grid")
      .evaluate(
        (el) => getComputedStyle(el).gridTemplateColumns.split(" ").length
      );
    expect(columns).toBeGreaterThanOrEqual(2);
  });
});

test.describe("Terminal interaction", () => {
  test("typing echo produces output via focus mode", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    await enterFocusMode(page);
    const sessionId = await firstSessionId(page);
    // Wait for the xterm canvas to render inside the focused card.
    const termScreen = page.locator(".focused-card .xterm-screen");
    await expect(termScreen).toBeVisible({ timeout: 15_000 });

    await termScreen.click();
    await waitForTerminalInputFocus(page);
    await page.keyboard.type("echo hello-exaterm\n", { delay: 30 });

    await expect
      .poll(() => terminalContainsText(page, sessionId, "hello-exaterm"))
      .toBe(true);

    await page.keyboard.press("Escape");
  });

  test("terminal in focus mode is interactive with second session", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    await enterFocusMode(page, 1);
    const sessionId = Number(
      await page.locator(".battle-card").nth(1).getAttribute("data-session-id")
    );
    expect(Number.isNaN(sessionId)).toBe(false);
    const termScreen = page.locator(".focused-card .xterm-screen");
    await expect(termScreen).toBeVisible({ timeout: 15_000 });

    await termScreen.click();
    await waitForTerminalInputFocus(page);
    await page.keyboard.type("echo focus-test-2\n", { delay: 30 });

    await expect
      .poll(() => terminalContainsText(page, sessionId, "focus-test-2"))
      .toBe(true);

    await page.keyboard.press("Escape");
  });
});

test.describe("Keyboard navigation", () => {
  test("Ctrl+[ and Ctrl+] cycle selected cards", async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto("/");
    await waitForCards(page, 1);
    await ensureSessionCount(page, 2);

    const firstId = await page
      .locator(".battle-card")
      .nth(0)
      .getAttribute("data-session-id");
    const secondId = await page
      .locator(".battle-card")
      .nth(1)
      .getAttribute("data-session-id");

    await page.keyboard.press("Control+]");
    await expect(page.locator(".selected-card")).toHaveAttribute(
      "data-session-id",
      firstId ?? ""
    );
    await page.keyboard.press("Control+]");
    await expect(page.locator(".selected-card")).toHaveAttribute(
      "data-session-id",
      secondId ?? ""
    );
    await page.keyboard.press("Control+[");
    await expect(page.locator(".selected-card")).toHaveAttribute(
      "data-session-id",
      firstId ?? ""
    );
  });
});

test.describe("Terminal resize", () => {
  test("terminal adapts to viewport resize in focus mode", async ({ page }) => {
    await page.goto("/");
    await waitForCards(page, 1);

    // Enter focus mode to guarantee embedded terminal.
    await enterFocusMode(page);
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible({
      timeout: 5_000,
    });

    // Get initial terminal dimensions.
    const sizeBefore = await page.locator(".focused-card .xterm-screen").boundingBox();

    // Resize viewport.
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.waitForTimeout(300); // Wait for debounced resize.

    // Terminal should still be visible.
    await expect(page.locator(".focused-card .xterm-screen")).toBeVisible();

    // And dimensions should have changed.
    const sizeAfter = await page.locator(".focused-card .xterm-screen").boundingBox();
    expect(sizeAfter).not.toBeNull();
    // Width or height should differ from before.
    if (sizeBefore && sizeAfter) {
      const changed =
        Math.abs(sizeBefore.width - sizeAfter.width) > 10 ||
        Math.abs(sizeBefore.height - sizeAfter.height) > 10;
      expect(changed).toBe(true);
    }

    await page.keyboard.press("Escape");
  });
});
