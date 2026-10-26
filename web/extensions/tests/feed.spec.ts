import { test, expect } from "@playwright/test";
import {
  open,
  calls,
  hostCalls,
  release,
  feed,
  expectReadOnly,
} from "./feed-helpers";

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
