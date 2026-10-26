import { expect, type Page, type FrameLocator } from "@playwright/test";
import { readFile } from "node:fs/promises";
import path from "node:path";

export const evidence = (name: string) =>
  path.join(
    import.meta.dirname,
    `../../../output/playwright/extension-feed-${name}.png`,
  );

export async function open(page: Page, query = "") {
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

export type Call = { name: string; args: Record<string, unknown> };
export type HostCall = { method: string; params: Record<string, unknown> };
export async function calls(page: Page, name?: string): Promise<Call[]> {
  return page.evaluate((name) => {
    const calls = (window as unknown as { calls: Call[] }).calls;
    return name ? calls.filter((call) => call.name === name) : calls;
  }, name);
}
export async function hostCalls(page: Page): Promise<HostCall[]> {
  return page.evaluate(
    () => (window as unknown as { hostCalls: HostCall[] }).hostCalls,
  );
}
export async function release(page: Page, name: string, fail = false) {
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
export async function feed(ui: FrameLocator) {
  await ui.getByRole("button", { name: "Messages", exact: true }).click();
  await expect(
    ui.getByRole("heading", { name: "Messages", exact: true }),
  ).toBeVisible();
}
export async function expectReadOnly(page: Page, ui: FrameLocator) {
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
