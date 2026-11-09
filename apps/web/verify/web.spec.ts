import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  atLatest,
  expect,
  FAKE,
  openApp,
  openChat,
  signIn,
  storedStatus,
  test,
} from "./support.ts";

// The user journeys of issue #138, section 4, in the order a person uses the
// app. The restart and sign-out journeys come last because they change the
// shared container.

const health = (container: string) =>
  execFileSync(
    "docker",
    ["inspect", "--format", "{{.State.Health.Status}}", container],
    { encoding: "utf8" },
  ).trim();

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

test("a wrong password is refused and the right one opens the inbox", async ({
  page,
  guard,
}) => {
  guard.allow(/^HTTP 401 POST \/api\/auth$/, /status of 401/);
  await page.goto("/");
  await signIn(page, `wrong-${randomUUID()}`);
  await expect(page.locator("#password-error")).toBeVisible();
  await expect(page.locator("#main-container")).toBeHidden();
  await signIn(page);
  await expect(page.locator("#main-container")).toBeVisible();
  const names = page.locator("#contacts-list .contact-name");
  for (const name of FAKE.chatNames) {
    await expect(names.filter({ hasText: name })).toHaveCount(1);
  }
});

test("a chat opens at its latest message, and a sent message is read", async ({
  page,
}) => {
  await openApp(page);
  await openChat(page, FAKE.lucia);
  await expect.poll(() => atLatest(page)).toBe(true);

  const text = `verify ${randomUUID().slice(0, 8)}: hola`;
  await page.locator("#message-input").fill(text);
  await page.locator("#send-button").click();
  const sent = page
    .locator("#messages-list .message.outgoing")
    .filter({ hasText: text });
  await expect(sent).toHaveCount(1);
  await expect(page.locator("#message-input")).toHaveValue("");
  // The web app shows "Sending…" until the server confirms the send, and no
  // tick marks after that, so delivery is checked on the stored message.
  await expect(sent.locator(".delivery-pending")).toHaveCount(0);
  await expect(sent.locator(".delivery-uncertain")).toHaveCount(0);
  const id = await sent.getAttribute("data-message-id");
  expect(id).toMatch(/^FAKE-OUT-/);
  await expect
    .poll(() => storedStatus(page, FAKE.lucia, id!), { timeout: 20_000 })
    .toBe("read");
  await expect.poll(() => atLatest(page)).toBe(true);
});

test("a received message translates once translation is on", async ({
  page,
}) => {
  await openApp(page);
  await openChat(page, FAKE.lucia);
  await page.locator("#chat-menu-button").click();
  await page.locator("#chat-settings-button").click();
  await page.locator("#translation-enabled").check();
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-modal")).toBeHidden();

  const message = page.locator(
    `.message[data-message-id="${FAKE.translatable}"]`,
  );
  const original = await message.locator(".message-text").innerText();
  await message.hover();
  await message.locator(".translate-button.can-translate").click();
  await expect(message.locator(".translation-indicator")).toBeVisible();
  await expect(message.locator(".message-text")).not.toHaveText(original);
});

test("the attachment button sends a picture", async ({ page }) => {
  await openApp(page);
  await openChat(page, FAKE.lucia);
  const before = await page.locator("#messages-list .message-image").count();
  const chooser = page.waitForEvent("filechooser");
  await page.locator("#attach-button").click();
  await (
    await chooser
  ).setFiles({ name: "verify.png", mimeType: "image/png", buffer: PNG });
  await expect(
    page.locator("#messages-list .message.outgoing .message-image"),
  ).toHaveCount(before + 1);
});

test("after a server restart the app reconnects and keeps the draft", async ({
  page,
  guard,
}) => {
  test.setTimeout(150_000);
  // Web sessions live in the server's memory, so a restart signs the browser
  // out: the app shows Connecting…, then asks for the password again. The
  // requests cut off by the restart fail on purpose.
  guard.allow(
    /Failed to load resource/,
    /Failed to fetch/,
    /WebSocket/,
    /^HTTP 401 /,
    /status of 401/,
    /Load failed/,
    /network connection was lost/i,
    // WebKit's wording for a fetch cut off by the restart.
    /due to access control checks/,
  );
  await openApp(page);
  // A returning user's session comes from the stored token, not a fresh
  // sign-in, so the password form must still work when it ends.
  await page.reload();
  await expect(page.locator("#main-container")).toBeVisible();
  await openChat(page, FAKE.lucia);
  const draft = `verify ${randomUUID().slice(0, 8)}: written before the restart`;
  await page.locator("#message-input").fill(draft);

  const container = process.env.VERIFY_WEB_CONTAINER;
  if (!container) throw new Error("VERIFY_WEB_CONTAINER is not set");
  execFileSync("docker", ["restart", "--time", "10", container]);
  await expect(page.locator("#chat-connection-status")).toBeVisible();
  await expect
    .poll(() => health(container), { timeout: 60_000 })
    .toBe("healthy");

  // The app asks for the password on its next reconnect attempt, which backs
  // off up to 30 s after the socket closed.
  await expect(page.locator("#password-overlay")).toBeVisible({
    timeout: 45_000,
  });
  await signIn(page);
  await expect(page.locator("#main-container")).toBeVisible();
  if (!(await page.locator("#chat-view").isVisible())) {
    await openChat(page, FAKE.lucia);
  }
  await expect(page.locator("#chat-connection-status")).toBeHidden();
  await expect(page.locator("#message-input")).toHaveValue(draft);

  // One tap must make one request, even after signing in again.
  const sends: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/api/send") sends.push("send");
  });
  await page.locator("#send-button").click();
  const sent = page
    .locator("#messages-list .message.outgoing")
    .filter({ hasText: draft });
  await expect(sent).toHaveCount(1);
  await expect(sent.locator(".delivery-uncertain")).toHaveCount(0);
  expect(sends).toHaveLength(1);
  // The same session reloads the app straight into the inbox.
  await page.reload();
  await expect(page.locator("#main-container")).toBeVisible();
  await openChat(page, FAKE.lucia);
  await expect(
    page.locator("#messages-list .message.outgoing").filter({ hasText: draft }),
  ).toHaveCount(1);
});

test("the PWA manifest, its icons and the apple-touch-icon load", async ({
  page,
  request,
}) => {
  await page.goto("/");
  const href = await page.locator('link[rel="manifest"]').getAttribute("href");
  const manifestUrl = new URL(href!, page.url());
  const response = await request.get(manifestUrl.href);
  expect(response.status()).toBe(200);
  const manifest = await response.json();
  expect(manifest.icons?.length).toBeGreaterThan(0);
  const touchIcon = await page
    .locator('link[rel="apple-touch-icon"]')
    .getAttribute("href");
  const icons = [
    ...manifest.icons.map((icon: { src: string }) => icon.src),
    touchIcon,
  ];
  for (const src of icons) {
    const icon = await request.get(new URL(src, manifestUrl).href);
    expect(icon.status(), src).toBe(200);
    expect(icon.headers()["content-type"], src).toMatch(/^image\//);
  }
});

test("signing out asks first, then unlinks WhatsApp", async ({
  page,
  guard,
}) => {
  // Signing out clears the server's data and sessions too, so the app's
  // next requests are refused until it asks for the password again.
  guard.allow(
    /^HTTP 401 /,
    /status of 401/,
    /Unexpected end of JSON input/,
    /WebSocket/,
  );
  await openApp(page);
  let dialogs = 0;
  page.on("dialog", (dialog) => {
    dialogs += 1;
    // The first prompt is declined, the second accepted.
    void (dialogs === 1 ? dialog.dismiss() : dialog.accept());
  });
  await page.locator("#logout-button").click();
  await expect.poll(() => dialogs).toBe(1);
  await expect(page.locator("#contacts-list .contact-item")).not.toHaveCount(0);
  await expect(page.locator("#password-overlay")).toBeHidden();

  await page.locator("#logout-button").click();
  await expect.poll(() => dialogs).toBe(2);
  await expect(page.locator("#password-overlay")).toBeVisible();
  // Compared in the page, so a failure never prints the token.
  expect(
    await page.evaluate(() => localStorage.getItem("wa_auth_token") === null),
  ).toBe(true);
});
