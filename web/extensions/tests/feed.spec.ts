import { test, expect, type Page, type FrameLocator } from "@playwright/test";
import { readFile } from "node:fs/promises";
import path from "node:path";

const evidence = (name: string) =>
  path.join(
    import.meta.dirname,
    `../../../output/playwright/extension-feed-${name}.png`,
  );

async function open(page: Page, query = "") {
  await page.route("http://127.0.0.1:4179/**", async (route) => {
    const isApp = new URL(route.request().url()).pathname === "/app.html";
    await route.fulfill({
      contentType: "text/html",
      headers: isApp
        ? {
            "Content-Security-Policy":
              "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'",
          }
        : {},
      body: await readFile(
        path.join(
          import.meta.dirname,
          "..",
          isApp ? "dist/chats.html" : "tests/host.html",
        ),
        "utf8",
      ),
    });
  });
  await page.goto(`http://127.0.0.1:4179/${query}`);
  const ui = page.frameLocator("#app");
  await expect(
    ui.getByRole("button", { name: "Messages", exact: true }),
  ).toBeVisible();
  return ui;
}

type Call = { name: string; args: Record<string, unknown> };
type HostCall = { method: string; params: Record<string, unknown> };
async function calls(page: Page, name?: string): Promise<Call[]> {
  return page.evaluate((name) => {
    const calls = (window as unknown as { calls: Call[] }).calls;
    return name ? calls.filter((call) => call.name === name) : calls;
  }, name);
}
async function hostCalls(page: Page): Promise<HostCall[]> {
  return page.evaluate(
    () => (window as unknown as { hostCalls: HostCall[] }).hostCalls,
  );
}
async function release(page: Page, name: string, fail = false) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (window as unknown as { pending: { name: string }[] }).pending.some(
            (call) => call.name === name,
          ),
        name,
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ name, fail }) => {
      const pending = (
        window as unknown as {
          pending: {
            name: string;
            resolve: () => void;
            reject: (error: Error) => void;
          }[];
        }
      ).pending;
      const index = pending.findIndex((call) => call.name === name);
      const [call] = pending.splice(index, 1);
      if (fail) call.reject(Error(`Late ${name} failure`));
      else call.resolve();
    },
    { name, fail },
  );
}
async function feed(ui: FrameLocator) {
  await ui.getByRole("button", { name: "Messages", exact: true }).click();
  await expect(
    ui.getByRole("heading", { name: "Messages", exact: true }),
  ).toBeVisible();
}
async function expectReadOnly(page: Page, ui: FrameLocator) {
  for (const id of ["composer", "reply", "read"])
    await expect(ui.locator(`#${id}`)).toBeHidden();
  await expect(ui.locator("#messages .message-actions")).toHaveCount(0);
  await expect(ui.getByRole("dialog")).not.toBeVisible();
  const allowed = new Set([
    "read_unified_messages",
    "open_chat",
    "read_messages",
    "list_contacts",
  ]);
  expect((await calls(page)).filter((call) => !allowed.has(call.name))).toEqual(
    [],
  );
  expect(
    (await hostCalls(page)).filter(
      (call) => call.method === "ui/update-model-context",
    ),
  ).toEqual([]);
}

test("compiled feed preserves equal-time order, exact cursor and source navigation outside contact pages", async ({
  page,
}) => {
  const urls: string[] = [];
  page.on("request", (request) => urls.push(request.url()));
  const ui = await open(page, "?feedCursor=1");
  await expect(
    ui.getByRole("button", { name: "Open chat with Daniel Chen" }),
  ).toHaveCount(0);
  await feed(ui);
  await expect(ui.locator(".feed-item")).toHaveCount(2);
  expect(
    await ui
      .locator(".feed-item")
      .evaluateAll((rows) =>
        rows.map((row) => (row as HTMLElement).dataset.messageId),
      ),
  ).toEqual(["b", "c"]);
  await expectReadOnly(page, ui);
  await ui.getByRole("button", { name: "Load earlier messages" }).click();
  await expect(ui.locator(".feed-item")).toHaveCount(3);
  expect(
    await ui
      .locator(".feed-item")
      .evaluateAll((rows) =>
        rows.map((row) => (row as HTMLElement).dataset.messageId),
      ),
  ).toEqual(["a", "b", "c"]);
  expect((await calls(page, "read_unified_messages"))[1].args).toEqual({
    limit: 50,
    before_timestamp: 100,
    before_message_id: "b",
  });
  await expect(
    ui.getByRole("button", { name: "Load earlier messages" }),
  ).toHaveCount(0);
  await ui
    .getByRole("button", { name: "Open source chat with Daniel Chen" })
    .click();
  await expect(ui.getByRole("heading", { name: "Daniel Chen" })).toBeVisible();
  expect((await calls(page, "open_chat"))[0].args).toEqual({
    contact_id: "dan@s.whatsapp.net",
  });
  expect(
    (await hostCalls(page)).filter(
      (call) => call.method === "ui/update-model-context",
    ),
  ).toEqual([]);
  expect(urls.some((url) => /\/api\/|\/mcp(?:\?|$)/.test(url))).toBe(false);
});

test("feed keeps stored text, direction, group sender, quotes, caption and media placeholders", async ({
  page,
}) => {
  const ui = await open(page);
  await feed(ui);
  await expect(
    ui.locator('.feed-item[data-message-id="feed-4"] .sender'),
  ).toHaveText("Marta");
  await expect(
    ui.locator('.feed-item[data-message-id="feed-4"] .quoted'),
  ).toHaveText("AliceWhich place works for everyone?");
  await expect(
    ui.locator('.feed-item[data-message-id="feed-4"] .message-text'),
  ).toHaveText("Let’s meet at the café.");
  await ui.locator('.feed-item[data-message-id="feed-4"] summary').click();
  await expect(
    ui.getByText("On se retrouve au café.", { exact: true }),
  ).toBeVisible();
  const outgoing = ui.locator('.feed-item[data-message-id="feed-5"]');
  await expect(outgoing.locator(".message-row")).toHaveClass(
    "message-row outgoing",
  );
  await expect(outgoing.locator(".message-text")).toHaveText(
    "I will bring the notes.",
  );
  await expect(
    outgoing.getByText("Delivery uncertain", { exact: true }),
  ).toBeVisible();
  await outgoing.getByText("Sent translation", { exact: true }).click();
  await expect(
    outgoing.getByText("Je vais apporter les notes.", { exact: true }),
  ).toBeVisible();
  await expect(outgoing.locator("time")).toHaveAttribute(
    "datetime",
    "2026-10-07T10:51:40.000Z",
  );
  await expect(ui.getByText("[image message]", { exact: true })).toBeVisible();
  await expect(
    ui.getByText("A photo from the café", { exact: true }),
  ).toBeVisible();
  await expect(
    ui.getByText("[voice note message]", { exact: true }),
  ).toBeVisible();
  await ui.getByRole("button", { name: "Load earlier messages" }).click();
  await expect(
    ui.getByRole("button", { name: "Open source chat with 447700900102" }),
  ).toBeVisible();
  await expect(
    ui.getByRole("button", {
      name: "Open source chat with opaque/room+?@g.us",
    }),
  ).toBeVisible();
  await expect(
    ui.locator('.feed-item[data-message-id="feed-3"] .sender'),
  ).toHaveText("447700900103");
  await expectReadOnly(page, ui);
  await ui
    .getByRole("button", { name: "Open source chat with opaque/room+?@g.us" })
    .click();
  await expect(
    ui.getByRole("heading", { name: "opaque/room+?@g.us" }),
  ).toBeVisible();
  expect((await calls(page, "open_chat"))[0].args.contact_id).toBe(
    "opaque/room+?@g.us",
  );
});

test("source navigation rejects a mismatched conversation and retries the exact source", async ({
  page,
}) => {
  const ui = await open(page, "?wrongSource=1");
  await feed(ui);
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await ui
    .getByRole("button", { name: "Open source chat with Daniel Chen" })
    .click();
  await expect(ui.getByRole("alert")).toContainText("could not be verified");
  await expect(
    ui.getByRole("heading", { name: "Messages", exact: true }),
  ).toBeVisible();
  await ui.getByRole("button", { name: "Retry", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "open_chat")).length)
    .toBe(2);
  expect((await calls(page, "open_chat")).map((call) => call.args)).toEqual([
    { contact_id: "dan@s.whatsapp.net" },
    { contact_id: "dan@s.whatsapp.net" },
  ]);
  await expectReadOnly(page, ui);
});

for (const narrow of [false, true])
  test(`earlier paging preserves a visible bubble and expanded original through the final page (${narrow ? "narrow" : "desktop"})`, async ({
    page,
  }) => {
    if (narrow) await page.setViewportSize({ width: 390, height: 844 });
    const ui = await open(page, "?feedLong=1&holdOlder=1");
    await feed(ui);
    await expect(ui.locator(".feed-item")).toHaveCount(4);
    await ui.locator('.feed-item[data-message-id="long-08"] summary').click();
    await ui.locator("#messages").evaluate((el) => {
      el.scrollTop = 0;
    });
    for (const count of [8, 12]) {
      const more = ui.getByRole("button", { name: "Load earlier messages" });
      await more.click();
      await expect(more).toBeDisabled();
      // Reading can continue while the earlier page is in flight.
      await ui.locator("#messages").evaluate((el) => {
        el.scrollTop = 180;
      });
      const anchor = await ui.locator("#messages").evaluate((el) => {
        const viewport = el.getBoundingClientRect();
        const row = [
          ...el.querySelectorAll<HTMLElement>(".feed-item .message-row"),
        ].find(
          (row) =>
            row.getBoundingClientRect().bottom > viewport.top &&
            row.getBoundingClientRect().top < viewport.bottom,
        )!;
        return {
          key: row.parentElement!.dataset.key,
          top: row.getBoundingClientRect().top,
        };
      });
      await release(page, "read_unified_messages");
      await expect(ui.locator(".feed-item")).toHaveCount(count);
      const after = await ui.locator(".feed-item").evaluateAll(
        (items, key) =>
          items
            .find((item) => (item as HTMLElement).dataset.key === key)!
            .querySelector(".message-row")!
            .getBoundingClientRect().top,
        anchor.key,
      );
      expect(Math.abs(after - anchor.top)).toBeLessThan(2);
      await expect(
        ui.locator('.feed-item[data-message-id="long-08"] details'),
      ).toHaveAttribute("open", "");
    }
    await expect(
      ui.getByRole("button", { name: "Load earlier messages" }),
    ).toHaveCount(0);
    expect(
      (await calls(page, "read_unified_messages")).map(
        (call) => call.args.before_message_id,
      ),
    ).toEqual([undefined, "long-08", "long-04"]);
    await expectReadOnly(page, ui);
  });

test("feed errors retry the same page, refresh starts a new window, and no writes occur with send permission", async ({
  page,
}) => {
  const ui = await open(page, "?feedError=1&olderError=1");
  await feed(ui);
  await expect(ui.getByRole("alert")).toContainText("Synthetic feed failure");
  await expect(
    ui.getByText("No messages here yet.", { exact: true }),
  ).toHaveCount(0);
  await ui.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await ui.getByRole("button", { name: "Load earlier messages" }).click();
  await expect(ui.getByRole("alert")).toContainText(
    "Synthetic older-page failure",
  );
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await ui.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(ui.locator(".feed-item")).toHaveCount(7);
  const reads = await calls(page, "read_unified_messages");
  expect(reads[2].args).toEqual(reads[3].args);
  await ui.getByRole("button", { name: "Refresh messages" }).click();
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  expect((await calls(page, "read_unified_messages")).at(-1)!.args).toEqual({
    limit: 50,
  });
  await expectReadOnly(page, ui);
});

for (const fail of [false, true])
  test(`late feed ${fail ? "failure" : "success"} cannot replace a newer chat or install a retry`, async ({
    page,
  }) => {
    const ui = await open(page, "?holdFeed=1");
    await feed(ui);
    await expect(
      ui.getByText("Loading messages…", { exact: true }),
    ).toBeVisible();
    await ui.getByRole("button", { name: "Chats", exact: true }).click();
    await ui
      .getByRole("button", { name: "Open chat with Weekend in Lisbon" })
      .click();
    await expect(
      ui.getByRole("heading", { name: "Weekend in Lisbon" }),
    ).toBeVisible();
    await release(page, "read_unified_messages", fail);
    await page.waitForTimeout(100);
    await expect(
      ui.getByRole("heading", { name: "Weekend in Lisbon" }),
    ).toBeVisible();
    await expect(ui.locator(".feed-item")).toHaveCount(0);
    await expect(ui.getByRole("alert")).not.toBeVisible();
    await expect(
      ui.getByRole("button", { name: "Retry", exact: true }),
    ).not.toBeVisible();
  });

for (const fail of [false, true])
  test(`late chat ${fail ? "failure" : "success"} cannot replace Messages or share model context`, async ({
    page,
  }) => {
    const ui = await open(page, "?holdChat=1");
    await ui
      .getByRole("button", { name: "Open chat with Alice Laurent" })
      .click();
    await expect(
      ui.getByText("Loading conversation…", { exact: true }),
    ).toBeVisible();
    await feed(ui);
    await expect(ui.locator(".feed-item")).toHaveCount(5);
    await release(page, "read_messages", fail);
    await page.waitForTimeout(100);
    await expect(
      ui.getByRole("heading", { name: "Messages", exact: true }),
    ).toBeVisible();
    await expect(ui.getByRole("alert")).not.toBeVisible();
    await expectReadOnly(page, ui);
  });

for (const fail of [false, true])
  test(`refresh supersedes an older-page ${fail ? "failure" : "success"} and ignores its retry`, async ({
    page,
  }) => {
    const ui = await open(page, "?holdOlder=1");
    await feed(ui);
    await expect(ui.locator(".feed-item")).toHaveCount(5);
    await ui.getByRole("button", { name: "Load earlier messages" }).click();
    await ui.getByRole("button", { name: "Refresh messages" }).click();
    await expect
      .poll(async () => (await calls(page, "read_unified_messages")).length)
      .toBe(3);
    await expect(ui.locator("#messages")).toHaveAttribute("aria-busy", "false");
    await release(page, "read_unified_messages", fail);
    await page.waitForTimeout(100);
    await expect(ui.locator(".feed-item")).toHaveCount(5);
    await expect(ui.getByRole("alert")).not.toBeVisible();
    await expectReadOnly(page, ui);
  });

test("entering Messages invalidates slow preparation and an existing confirmation while retaining chat drafts", async ({
  page,
}) => {
  const ui = await open(page, "?slowPrepare=1");
  await ui
    .getByRole("button", { name: "Open chat with Alice Laurent" })
    .click();
  await ui
    .getByRole("textbox", { name: "Write a message" })
    .fill("Keep this draft");
  await ui.getByRole("button", { name: "Prepare message for review" }).click();
  await feed(ui);
  await page.waitForTimeout(600);
  await expect(ui.getByRole("dialog")).not.toBeVisible();
  expect(await calls(page, "send_message")).toEqual([]);
  await ui.getByRole("button", { name: "Chats", exact: true }).click();
  await expect(
    ui.getByRole("textbox", { name: "Write a message" }),
  ).toHaveValue("Keep this draft");
  await ui.getByRole("button", { name: "Prepare message for review" }).click();
  await expect(ui.getByRole("dialog")).toBeVisible();
  // Also invalidate confirmation if navigation is initiated programmatically by the UI.
  await ui
    .locator('[data-view="messages"]')
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(
    ui.getByRole("heading", { name: "Messages", exact: true }),
  ).toBeVisible();
  await expect(ui.getByRole("dialog")).not.toBeVisible();
  expect(await calls(page, "send_message")).toEqual([]);
  await expect(ui.locator("#composer")).toBeHidden();
  await ui.locator("body").evaluate(() => {
    (document.getElementById("composer") as HTMLFormElement).requestSubmit();
    (document.getElementById("read") as HTMLButtonElement).click();
    (document.getElementById("confirm-send") as HTMLButtonElement).click();
  });
  await page.waitForTimeout(100);
  expect(await calls(page, "prepare_message")).toHaveLength(2);
  expect(await calls(page, "mark_conversation_read")).toEqual([]);
  expect(await calls(page, "send_message")).toEqual([]);
});

test("host interruption retries feed reads and repeated clicks do not duplicate in-flight paging", async ({
  page,
}) => {
  const ui = await open(page, "?holdFeed=1&holdOlder=1");
  await feed(ui);
  await page.evaluate(() =>
    (
      window as unknown as { notify: (method: string, params: object) => void }
    ).notify("ui/notifications/tool-cancelled", { reason: "interrupted" }),
  );
  await expect(ui.getByRole("alert")).toContainText("interrupted");
  await ui.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await release(page, "read_unified_messages");
  await ui.getByRole("button", { name: "Load earlier messages" }).click();
  await ui
    .getByRole("button", { name: "Load earlier messages" })
    .evaluate((button) => {
      (button as HTMLButtonElement).click();
      (button as HTMLButtonElement).click();
    });
  await expect
    .poll(async () =>
      (await calls(page, "read_unified_messages")).filter(
        (call) => call.args.before_message_id,
      ),
    )
    .toHaveLength(1);
  await release(page, "read_unified_messages");
  await expect(ui.locator(".feed-item")).toHaveCount(7);
  expect(
    (await calls(page, "read_unified_messages")).filter(
      (call) => call.args.before_message_id,
    ),
  ).toHaveLength(1);
  await expectReadOnly(page, ui);
});

for (const fail of [false, true])
  test(`late source/deep-link ${fail ? "failure" : "success"} cannot leave the newer feed view`, async ({
    page,
  }) => {
    const ui = await open(page, "?holdSource=1");
    await feed(ui);
    await expect(ui.locator(".feed-item")).toHaveCount(5);
    await ui
      .getByRole("button", { name: "Open source chat with Daniel Chen" })
      .click();
    await ui.getByRole("button", { name: "Chats", exact: true }).click();
    await feed(ui);
    await release(page, "open_chat", fail);
    await page.waitForTimeout(100);
    await expect(
      ui.getByRole("heading", { name: "Messages", exact: true }),
    ).toBeVisible();
    await expect(ui.getByRole("alert")).not.toBeVisible();
    await page.evaluate(() =>
      (
        window as unknown as {
          notify: (method: string, params: object) => void;
        }
      ).notify("ui/notifications/host-context-changed", {
        "openai/deepLink": { url: "/chats/dan%40s.whatsapp.net" },
      }),
    );
    await expect
      .poll(async () => (await calls(page, "open_chat")).length)
      .toBe(2);
    await ui.getByRole("button", { name: "Chats", exact: true }).click();
    await feed(ui);
    await release(page, "open_chat", fail);
    await page.waitForTimeout(100);
    await expect(
      ui.getByRole("heading", { name: "Messages", exact: true }),
    ).toBeVisible();
    await expect(ui.getByRole("alert")).not.toBeVisible();
    await expectReadOnly(page, ui);
  });

test("a fresh host deep link opens the previously selected chat from Messages without feed context sharing", async ({
  page,
}) => {
  const ui = await open(page);
  await ui
    .getByRole("button", { name: "Open chat with Alice Laurent" })
    .click();
  await expect(ui.getByText("Good morning!", { exact: false })).toBeVisible();
  await expect
    .poll(async () =>
      (await hostCalls(page)).filter(
        (call) => call.method === "ui/update-model-context",
      ),
    )
    .toHaveLength(1);
  await feed(ui);
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await page.evaluate(() =>
    (
      window as unknown as { notify: (method: string, params: object) => void }
    ).notify("ui/notifications/host-context-changed", {
      "openai/deepLink": { url: "/chats/alice%40s.whatsapp.net" },
    }),
  );
  await expect(
    ui.getByRole("heading", { name: "Alice Laurent" }),
  ).toBeVisible();
  await expect(
    ui.getByRole("textbox", { name: "Write a message" }),
  ).toBeVisible();
  expect((await calls(page, "open_chat"))[0].args).toEqual({
    contact_id: "alice@s.whatsapp.net",
  });
  expect(
    (await hostCalls(page)).filter(
      (call) => call.method === "ui/update-model-context",
    ),
  ).toHaveLength(1);
});

test("untrusted feed identity, message and quote text remain inert", async ({
  page,
}) => {
  const ui = await open(page, "?feedXss=1");
  await feed(ui);
  await expect(
    ui.getByRole("button", {
      name: "Open source chat with <img src=x onerror=alert(1)>",
    }),
  ).toBeVisible();
  await expect(
    ui.getByText("<button onclick=bad()>Stored text</button>", { exact: true }),
  ).toBeVisible();
  await expect(
    ui.getByText("<svg onload=bad()>", { exact: true }),
  ).toBeVisible();
  await expect(
    ui.getByText("<iframe src=bad></iframe>", { exact: true }),
  ).toBeVisible();
  await expect(
    ui.locator(
      "#messages img, #messages script, #messages iframe, #messages svg",
    ),
  ).toHaveCount(0);
  await expectReadOnly(page, ui);
});

test("compiled feed screenshots cover desktop, dark, narrow, empty, loading and error", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1080, height: 1000 });
  let ui = await open(page);
  await feed(ui);
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await page.screenshot({ path: evidence("desktop") });
  ui = await open(page, "?dark=1");
  await feed(ui);
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  await expect(ui.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.screenshot({ path: evidence("dark") });
  await page.setViewportSize({ width: 390, height: 844 });
  ui = await open(page, "?readonly=1");
  await feed(ui);
  await expect(ui.locator(".feed-item")).toHaveCount(5);
  expect(
    await ui
      .locator("body")
      .evaluate((el) => el.scrollWidth > window.innerWidth),
  ).toBe(false);
  await expectReadOnly(page, ui);
  await page.screenshot({ path: evidence("narrow") });
  await ui.getByRole("button", { name: "Back to chats" }).click();
  await expect(
    ui.getByRole("heading", { name: "Chats", exact: true }),
  ).toBeVisible();
  await page.setViewportSize({ width: 1080, height: 800 });
  ui = await open(page, "?feedEmpty=1");
  await feed(ui);
  await expect(
    ui.getByText("No messages here yet.", { exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: evidence("empty") });
  ui = await open(page, "?holdFeed=1");
  await feed(ui);
  await expect(
    ui.getByText("Loading messages…", { exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: evidence("loading") });
  await release(page, "read_unified_messages", true);
  await expect(ui.getByRole("alert")).toContainText("failure");
  await expect(
    ui.getByText("No messages here yet.", { exact: true }),
  ).toHaveCount(0);
  await page.screenshot({ path: evidence("error") });
});
