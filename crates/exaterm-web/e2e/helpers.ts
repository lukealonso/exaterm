import { expect, Page } from "@playwright/test";

/** Wait for exactly `count` battle cards to be visible. */
export async function waitForCards(page: Page, count: number, timeout = 10_000) {
  const cards = page.locator(".battle-card");
  await expect(cards).toHaveCount(count, { timeout });
  await expect(cards.first()).toBeVisible({ timeout });
}

/** Restart the shared daemon-backed workspace so each test starts clean. */
export async function resetWorkspace(page: Page) {
  await page.addInitScript(() => {
    window.confirm = () => true;
  });
  await page.goto("/");
  await expect(page.locator("#reconnect-overlay")).toHaveClass(/hidden/, {
    timeout: 15_000,
  });
  await page.waitForFunction(
    () =>
      document.querySelector(".battle-card") !== null ||
      document.querySelector(".empty-state") !== null
  );
  await page.click("#restart-btn");
  await waitForCards(page, 1, 20_000);
}

/** Enter focus mode on the nth card via Ctrl+Enter. */
export async function enterFocusMode(page: Page, nth = 0) {
  await page.locator(".battle-card").nth(nth).click();
  await page.keyboard.press("Control+Enter");
  await expect(page.locator(".battlefield-grid")).toHaveClass(/focus-mode/, {
    timeout: 5_000,
  });
}

export async function waitForCardCountIncrease(
  page: Page,
  previousCount: number,
  timeout = 10_000
): Promise<number> {
  let latestCount = previousCount;
  await expect
    .poll(
      async () => {
        latestCount = await page.locator(".battle-card").count();
        return latestCount;
      },
      { timeout }
    )
    .toBeGreaterThan(previousCount);
  return latestCount;
}

export async function waitForTerminalInputFocus(page: Page, timeout = 5_000) {
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            (document.activeElement as HTMLElement | null)?.getAttribute(
              "aria-label"
            ) ?? ""
        ),
      { timeout }
    )
    .toBe("Terminal input");
}

/** Add shells until we have at least `target` sessions. Fails after 10 attempts. */
export async function ensureSessionCount(page: Page, target: number) {
  const maxAttempts = 10;
  let attempts = 0;
  while ((await page.locator(".battle-card").count()) < target && attempts < maxAttempts) {
    const before = await page.locator(".battle-card").count();
    await page.click("#add-shell-btn");
    await waitForCardCountIncrease(page, before);
    attempts++;
  }
  if ((await page.locator(".battle-card").count()) < target) {
    throw new Error(`Failed to reach ${target} sessions after ${maxAttempts} attempts`);
  }
}

export async function installClipboardStub(page: Page, initialText = "") {
  await page.addInitScript((seed) => {
    let clipboard = seed;
    Object.defineProperty(window, "__EXATERM_CLIPBOARD__", {
      configurable: true,
      value: {
        getText: () => clipboard,
        setText: (text: string) => {
          clipboard = text;
        },
      },
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          clipboard = text;
        },
        readText: async () => clipboard,
      },
    });
  }, initialText);
}

export async function installExecCommandCopyStub(page: Page) {
  await page.evaluate(() => {
    let copiedText = "";
    Object.defineProperty(window, "__EXATERM_EXEC_COMMAND_COPY__", {
      configurable: true,
      value: {
        getText: () => copiedText,
      },
    });
    try {
      Object.defineProperty(window, "isSecureContext", {
        configurable: true,
        value: false,
      });
    } catch {
      // Some browsers may not allow overriding isSecureContext; removing the
      // async Clipboard API is still enough to force the fallback path.
    }
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: undefined,
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: (command: string) => {
        if (command !== "copy") return false;
        const active = document.activeElement;
        copiedText = active instanceof HTMLTextAreaElement ? active.value : "";
        return true;
      },
    });
  });
}

export async function getClipboardText(page: Page): Promise<string> {
  return page.evaluate(() => (window as any).__EXATERM_CLIPBOARD__.getText());
}

export async function getExecCommandCopyText(page: Page): Promise<string> {
  return page.evaluate(
    () => (window as any).__EXATERM_EXEC_COMMAND_COPY__?.getText?.() ?? ""
  );
}

export async function setClipboardText(page: Page, text: string) {
  await page.evaluate((value) => {
    (window as any).__EXATERM_CLIPBOARD__.setText(value);
  }, text);
}

export async function firstSessionId(page: Page): Promise<number> {
  const value = await page.locator(".battle-card").first().getAttribute("data-session-id");
  if (!value) {
    throw new Error("missing session id on first battle card");
  }
  return Number(value);
}

export async function selectTerminalText(page: Page, sessionId: number, needle: string) {
  const selected = await page.evaluate(
    ({ needle, sessionId: id }) =>
      (window as any).__EXATERM_TEST__?.selectTerminalText(id, needle) ?? false,
    { needle, sessionId }
  );
  if (!selected) {
    throw new Error(`failed to select terminal text: ${needle}`);
  }
}

export async function clearTerminalSelection(page: Page, sessionId: number) {
  const cleared = await page.evaluate(
    (id) => (window as any).__EXATERM_TEST__?.clearTerminalSelection(id) ?? false,
    sessionId
  );
  if (!cleared) {
    throw new Error(`failed to clear terminal selection for session ${sessionId}`);
  }
}

export async function terminalConnectionState(
  page: Page,
  sessionId: number
): Promise<number | null> {
  return page.evaluate(
    (id) => (window as any).__EXATERM_TEST__?.connectionState(id) ?? null,
    sessionId
  );
}

export async function terminalContainsText(
  page: Page,
  sessionId: number,
  needle: string
): Promise<boolean> {
  return page.evaluate(
    ({ needle: text, sessionId: id }) =>
      (window as any).__EXATERM_TEST__?.terminalContainsText(id, text) ?? false,
    { needle, sessionId }
  );
}

export async function terminalTextCenter(
  page: Page,
  sessionId: number,
  needle: string
): Promise<{ x: number; y: number }> {
  const point = await page.evaluate(
    ({ needle: text, sessionId: id }) =>
      (window as any).__EXATERM_TEST__?.terminalTextCenter(id, text) ?? null,
    { needle, sessionId }
  );
  if (!point) {
    throw new Error(`failed to locate terminal text: ${needle}`);
  }
  return point;
}
