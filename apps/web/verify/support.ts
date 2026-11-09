import { test as base, expect, type Page } from "@playwright/test";

export { expect };

// The canned chats apps/wa-bridge/fake.go serves.
export const FAKE = {
  lucia: "34600000001@s.whatsapp.net",
  chatNames: ["Lucía", "Family", "Sam"],
  translatable: "FAKE-L3",
};

export const PASSWORD = process.env.VERIFY_WEB_PASSWORD ?? "";

type Guard = {
  // Problems matching one of these patterns do not fail the test.
  allow: (...patterns: RegExp[]) => void;
};

// Every test fails on an uncaught page error, a console.error, or a 4xx or 5xx
// response from the app's own server, unless the test expects it.
export const test = base.extend<{ guard: Guard }>({
  guard: [
    async ({ page, baseURL }, use) => {
      const problems: string[] = [];
      const allowed: RegExp[] = [];
      const origin = new URL(baseURL ?? "").origin;
      page.on("pageerror", (error) =>
        problems.push(`page error: ${error.message}`),
      );
      page.on("console", (message) => {
        if (message.type() === "error") {
          problems.push(`console.error: ${message.text()}`);
        }
      });
      page.on("response", (response) => {
        const url = new URL(response.url());
        if (url.origin === origin && response.status() >= 400) {
          const method = response.request().method();
          problems.push(`HTTP ${response.status()} ${method} ${url.pathname}`);
        }
      });
      await use({ allow: (...patterns) => allowed.push(...patterns) });
      const unexpected = problems.filter(
        (problem) => !allowed.some((pattern) => pattern.test(problem)),
      );
      expect(unexpected, "unexpected page, console or HTTP errors").toEqual([]);
    },
    { auto: true },
  ],
});

export async function signIn(page: Page, password = PASSWORD) {
  await expect(page.locator("#password-overlay")).toBeVisible();
  await page.locator("#password-input").fill(password);
  await page.locator("#password-submit").click();
}

export async function openApp(page: Page) {
  await page.goto("/");
  await signIn(page);
  await expect(page.locator("#main-container")).toBeVisible();
}

export async function openChat(page: Page, contactId: string) {
  await page.locator(`.contact-item[data-contact-id="${contactId}"]`).click();
  await expect(page.locator("#chat-view")).toBeVisible();
  await expect(page.locator("#messages-list .message").first()).toBeVisible();
}

// Whether the message list is scrolled to its newest message.
export async function atLatest(page: Page) {
  return page.locator("#messages-list").evaluate((list) => {
    return list.scrollTop + list.clientHeight >= list.scrollHeight - 4;
  });
}

// A stored message's delivery status, read with the browser's own session so
// the test never handles the token.
export async function storedStatus(page: Page, contactId: string, id: string) {
  return page.evaluate(
    async ({ contactId, id }) => {
      const token = localStorage.getItem("wa_auth_token");
      const response = await fetch(
        `/api/messages/${encodeURIComponent(contactId)}`,
        { headers: { Authorization: `Bearer ${token}` } },
      );
      const body = await response.json();
      return body.messages.find((message: { id: string }) => message.id === id)
        ?.deliveryStatus;
    },
    { contactId, id },
  );
}
