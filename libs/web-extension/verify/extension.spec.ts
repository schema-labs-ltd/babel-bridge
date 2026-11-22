import {
  test as base,
  expect,
  type FrameLocator,
  type Page,
} from "@playwright/test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { McpSession } from "../../../tools/verify/lib/mcp.mjs";

// The chats and feed journeys of tests/chats.spec.ts and tests/feed.spec.ts,
// against the resource and tools of a real server instead of mocks.

const LUCIA = "34600000001@s.whatsapp.net";
const HOST = "http://127.0.0.1:4180";
// The policy MCP Apps hosts apply to the resource, as tests/chats.spec.ts.
const CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'";

const mcp = new McpSession(
  process.env.VERIFY_MCP_URL ?? "",
  process.env.VERIFY_MCP_TOKEN ?? "",
);

type Fixtures = { ui: FrameLocator };

// Every test fails on a page error or console.error in the host or the app.
const test = base.extend<Fixtures>({
  ui: async ({ page }, use) => {
    const problems: string[] = [];
    page.on("pageerror", (error) => problems.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") problems.push(message.text());
    });
    await use(await open(page));
    expect(problems, "page errors or console errors").toEqual([]);
  },
});

async function open(page: Page) {
  const resource = await mcp.request("resources/read", {
    uri: "ui://whatsapp-translator/chats",
  });
  const app = resource.contents[0].text;
  const host = await readFile(path.join(import.meta.dirname, "host.html"));
  await page.route(`${HOST}/**`, async (route) => {
    const { pathname } = new URL(route.request().url());
    if (pathname === "/app.html") {
      await route.fulfill({
        contentType: "text/html",
        headers: { "Content-Security-Policy": CSP },
        body: app,
      });
    } else if (pathname === "/proxy") {
      const { name, arguments: args } = route.request().postDataJSON();
      try {
        await route.fulfill({ json: await mcp.callRaw(name, args) });
      } catch (error) {
        await route.fulfill({
          status: 502,
          json: { error: (error as Error).message },
        });
      }
    } else {
      await route.fulfill({ contentType: "text/html", body: host });
    }
  });
  await page.goto(`${HOST}/`);
  return page.frameLocator("#app");
}

test("the served app lists the fake chats and opens one", async ({ ui }) => {
  for (const name of ["Lucía", "Family"]) {
    await expect(
      ui.getByRole("button", { name: `Open chat with ${name}` }),
    ).toBeVisible();
  }
  await ui.getByRole("button", { name: "Open chat with Lucía" }).click();
  await expect(
    ui.getByText("¿Quedamos el sábado para cenar?", { exact: false }),
  ).toBeVisible();
});

test("a reviewed message is sent through the real tools and is read", async ({
  ui,
}) => {
  await ui.getByRole("button", { name: "Open chat with Lucía" }).click();
  const text = `verify ${randomUUID().slice(0, 8)}: desde la extensión`;
  await ui.getByRole("textbox", { name: "Write a message" }).fill(text);
  await ui.getByRole("button", { name: "Prepare message for review" }).click();
  await expect(ui.getByRole("dialog")).toBeVisible();
  await expect(ui.locator("#confirm-text")).toHaveText(text);
  await ui.getByRole("button", { name: "Send message", exact: true }).click();
  await expect(ui.getByRole("dialog")).not.toBeVisible();
  await expect(
    ui.getByRole("textbox", { name: "Write a message" }),
  ).toHaveValue("");
  await expect
    .poll(
      async () => {
        const chat = await mcp.call("read_messages", {
          contact_id: LUCIA,
          limit: 20,
        });
        return chat.messages.find(
          (message: { text: string }) => message.text === text,
        )?.deliveryStatus;
      },
      { timeout: 20_000 },
    )
    .toBe("read");
});

test("the Messages feed shows stored messages and opens their chat", async ({
  ui,
}) => {
  await ui.getByRole("button", { name: "Messages", exact: true }).click();
  await expect(
    ui.getByRole("heading", { name: "Messages", exact: true }),
  ).toBeVisible();
  await expect(ui.locator(".feed-item").first()).toBeVisible();
  await ui
    .getByRole("button", { name: "Open source chat with Lucía" })
    .first()
    .click();
  await expect(ui.getByRole("heading", { name: "Lucía" })).toBeVisible();
  await expect(
    ui.getByText("¿Quedamos el sábado para cenar?", { exact: false }),
  ).toBeVisible();
});
