import { test, expect } from "@playwright/test";
import {
  evidence,
  open,
  calls,
  hostCalls,
  release,
  feed,
  expectReadOnly,
} from "./feed-helpers";

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
