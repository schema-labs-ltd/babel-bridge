import { randomUUID } from "node:crypto";
import { Client, step, until } from "./client.mjs";
import { loadFixture, shapeProblems } from "./shape.mjs";

// The user-visible journeys the production container smoke drives, against a
// server that runs with WA_FAKE_BRIDGE=true and WA_FAKE_AI=true. Each journey
// is a named step, so a failure names what broke.

const ROOT = new URL("../../../", import.meta.url);

// The canned chats apps/wa-bridge/fake.go serves.
export const FAKE = {
  lucia: "34600000001@s.whatsapp.net",
  sam: "447700900003@s.whatsapp.net",
  family: "120363000000000001@g.us",
  chatNames: ["Lucía", "Sam", "Family"],
  // Messages from João reach the Family group; he has no private chat.
  senders: ["João"],
  phones: ["34600000001", "351910000002", "447700900003", "447700900000"],
  translatable: { id: "FAKE-L3", text: "¿Quedamos el sábado para cenar?" },
};

const enc = encodeURIComponent;

async function refused(client, path) {
  const response = await client.request("GET", path, { auth: false });
  if (response.status !== 401) {
    throw new Error(
      `GET ${path} without a session returned ${response.status}`,
    );
  }
}

// Local paths referenced by src= and href= in index.html.
export function referencedAssets(html) {
  const assets = new Set();
  for (const match of html.matchAll(/\s(?:src|href)="([^"]+)"/g)) {
    const ref = match[1];
    if (/^(?:[a-z]+:|\/\/|#)/i.test(ref)) continue;
    assets.add(`/${ref.replace(/^\.?\//, "").split(/[?#]/)[0]}`);
  }
  return [...assets].sort();
}

export async function publicSurface(client, log) {
  await step("health is ok", log, async () => {
    const { json } = await client.json("GET", "/api/health", {
      auth: false,
      fixture: "health",
    });
    if (json.ok !== true || json.service !== "whatsapp-translator") {
      throw new Error(`unexpected health body ${JSON.stringify(json)}`);
    }
  });
  await step("index.html and every asset it references load", log, async () => {
    const index = await client.request("GET", "/", { auth: false });
    if (index.status !== 200 || !index.text.includes("<html")) {
      throw new Error(`GET / returned ${index.status}`);
    }
    const assets = referencedAssets(index.text);
    if (assets.length === 0) throw new Error("index.html references no assets");
    for (const asset of assets) {
      const response = await client.request("GET", asset, { auth: false });
      if (response.status !== 200) {
        throw new Error(`${asset} returned ${response.status}`);
      }
    }
    log(`  ${assets.length} assets: ${assets.join(" ")}`);
  });
}

export async function signInJourney(client, password, log) {
  await step("signed-out requests are refused", log, async () => {
    const check = await client.json("GET", "/api/auth/check", {
      auth: false,
      fixture: "auth-check",
    });
    if (check.json.required !== true)
      throw new Error("sign-in is not required");
    await refused(client, "/api/status");
    await refused(client, "/api/contacts");
    await refused(client, "/api/feed");
    const anonymous = new Client(client.baseUrl);
    const opened = await anonymous.openSocket().then(
      (live) => {
        live.socket.close();
        return true;
      },
      () => false,
    );
    if (opened) throw new Error("/ws accepted a connection without a session");
  });
  await step("a wrong password is rejected", log, async () => {
    const { json } = await client.json("POST", "/api/auth", {
      body: { password: `wrong-${randomUUID()}` },
      auth: false,
    });
    if (json.success !== false || json.token) {
      throw new Error("a wrong password was accepted");
    }
  });
  await step("sign in with the password", log, () => client.signIn(password));
}

export async function connectedJourney(client, log) {
  await step("status reaches connected", log, () =>
    until("connected status", 30_000, async () => {
      const { json } = await client.json("GET", "/api/status", {
        fixture: "status",
      });
      return json.connected === true && json.connection_state === "connected";
    }),
  );
  await step("contacts list the fake chats", log, () =>
    until("the fake chats", 30_000, async () => {
      const { json } = await client.json("GET", "/api/contacts", {
        fixture: "contacts",
      });
      const names = json.map((contact) => contact.name);
      return FAKE.chatNames.every((name) => names.includes(name));
    }),
  );
  await step("the Family group shows its members' messages", log, async () => {
    const { json } = await client.json(
      "GET",
      `/api/messages/${enc(FAKE.family)}`,
      { fixture: "messages" },
    );
    const senders = json.messages.map((message) => message.senderName);
    for (const name of FAKE.senders) {
      if (!senders.includes(name)) throw new Error(`no message from ${name}`);
    }
  });
}

// Statuses a sent message reports, in the order they arrived on the socket.
function statusesFor(events, messageId) {
  const seen = [];
  for (const event of events) {
    if (
      event.type === "send_result" &&
      event.message_id === messageId &&
      event.success === true
    ) {
      seen.push("sent");
    } else if (
      event.type === "receipt" &&
      event.message_ids?.includes(messageId)
    ) {
      seen.push(event.status);
    } else if (
      (event.type === "message" || event.type === "message_updated") &&
      event.message?.id === messageId &&
      event.message.deliveryStatus
    ) {
      seen.push(event.message.deliveryStatus);
    }
  }
  return seen.filter((status, index) => status !== seen[index - 1]);
}

const ORDER = ["sent", "delivered", "read"];

export async function sendJourney(client, log, text) {
  const live = await step("the WebSocket accepts the session", log, () =>
    client.openSocket(),
  );
  try {
    const key = `verify-${randomUUID()}`;
    const sent = await step("send returns an outbox id", log, async () => {
      const response = await client.json("POST", "/api/send", {
        body: { contactId: FAKE.lucia, text },
        headers: { "idempotency-key": key },
        fixture: "send-message",
      });
      if (response.headers.get("x-operation-id") !== key) {
        throw new Error("x-operation-id does not echo the Idempotency-Key");
      }
      if (typeof response.json.messageId !== "string") {
        throw new Error("no messageId in the send response");
      }
      return response.json;
    });
    await step("the outbox entry is complete", log, async () => {
      const response = await client.json("GET", `/api/outbox/${enc(key)}`, {
        fixture: "send-message",
      });
      const state = response.headers.get("x-delivery-state");
      if (state !== "confirmed") throw new Error(`outbox state is ${state}`);
      if (response.json.messageId !== sent.messageId) {
        throw new Error("the outbox reports a different message");
      }
    });
    await step(
      "the message goes sent, delivered, read on the socket",
      log,
      () =>
        until("a read receipt", 20_000, async () => {
          const statuses = statusesFor(live.events, sent.messageId);
          if (!statuses.includes("read")) return false;
          if (statuses.join(",") !== ORDER.join(",")) {
            throw new Error(`statuses were ${statuses.join(", ")}`);
          }
          return true;
        }),
    );
    await step("the stored message reaches read", log, () =>
      until("deliveryStatus read", 10_000, async () => {
        const { json } = await client.json(
          "GET",
          `/api/messages/${enc(FAKE.lucia)}`,
          { fixture: "messages" },
        );
        const message = json.messages.find((m) => m.id === sent.messageId);
        return message?.deliveryStatus === "read";
      }),
    );
    await step("the feed includes the sent message", log, async () => {
      const { json } = await client.json("GET", "/api/feed", {
        fixture: "feed",
      });
      if (!json.messages.some((message) => message.id === sent.messageId)) {
        throw new Error("the sent message is not in /api/feed");
      }
    });
    // The apps decode these events too, so they join the fixture check.
    if (!client.seen.has("live-events")) client.seen.set("live-events", []);
    client.seen.get("live-events").push(live.events);
    const types = [...new Set(live.events.map((event) => event.type))];
    log(`  socket event types: ${types.sort().join(", ")}`);
    return sent.messageId;
  } finally {
    live.socket.close();
  }
}

export async function translateJourney(client, log) {
  await step("translation turns on for the chat", log, async () => {
    await client.json("PUT", `/api/contacts/${enc(FAKE.lucia)}/settings`, {
      body: { translationEnabled: true },
      fixture: "update-conversation-settings",
    });
  });
  await step("translate returns the fake translation", log, async () => {
    const { json } = await client.json("POST", "/api/translate", {
      body: {
        text: FAKE.translatable.text,
        messageId: FAKE.translatable.id,
        contactId: FAKE.lucia,
      },
      fixture: "translate",
    });
    if (json.success !== true || !json.translatedText) {
      throw new Error(`translate failed: ${json.error ?? "no text"}`);
    }
  });
}

export async function setToneJourney(client, log, tone) {
  await step("the message tone setting saves", log, async () => {
    await client.json("PUT", "/api/settings/message-tone", {
      body: { tone },
      fixture: "update-message-tone",
    });
  });
}

export async function persistedJourney(
  client,
  log,
  { messageId, tone, translation = false },
) {
  await step("the sent message is still stored", log, async () => {
    const { json } = await client.json(
      "GET",
      `/api/messages/${enc(FAKE.lucia)}`,
      { fixture: "messages" },
    );
    if (!json.messages.some((message) => message.id === messageId)) {
      throw new Error(`message ${messageId} is gone`);
    }
  });
  if (translation) {
    await step("the chat's translation setting is kept", log, async () => {
      const { json } = await client.json(
        "GET",
        `/api/contacts/${enc(FAKE.lucia)}/settings`,
        { fixture: "conversation-settings" },
      );
      if (json.translationEnabled !== true) {
        throw new Error("translation is off again");
      }
    });
  }
  if (tone) {
    await step("the message tone setting is kept", log, async () => {
      const { json } = await client.json("GET", "/api/settings/message-tone", {
        fixture: "message-tone-settings",
      });
      if (json.tone !== tone) throw new Error(`tone is ${json.tone}`);
    });
  }
}

// The warning WA_FAKE_AI prints on purpose at start-up, and the one the
// smoke's own wrong-password attempt causes.
const FAKE_AI_WARNING = /WA_FAKE_AI is set/;
const WRONG_PASSWORD_WARNING = /Failed authentication attempt/;

export async function diagnosticsJourney(
  client,
  log,
  { wrongPasswordTried = false } = {},
) {
  const expected = [FAKE_AI_WARNING];
  if (wrongPasswordTried) expected.push(WRONG_PASSWORD_WARNING);
  await step("diagnostics report no unexpected problems", log, async () => {
    const { json } = await client.json("GET", "/api/diagnostics");
    if (!Array.isArray(json.recentProblems)) {
      throw new Error("no recentProblems list");
    }
    const unexpected = json.recentProblems
      .map((problem) => problem.line)
      .filter((line) => !expected.some((pattern) => pattern.test(line)));
    if (unexpected.length > 0) {
      throw new Error(
        `${unexpected.length} problems: ${unexpected.join(" | ")}`,
      );
    }
  });
}

export async function shapesJourney(client, log) {
  await step("every response matches its fixtures/api shape", log, async () => {
    const problems = [];
    for (const [name, bodies] of client.seen) {
      const fixture = loadFixture(ROOT, name);
      for (const body of bodies) {
        for (const problem of shapeProblems(fixture, body)) {
          problems.push(`${name}.json ${problem}`);
        }
      }
    }
    log(`  checked ${[...client.seen.keys()].sort().join(", ")}`);
    if (problems.length > 0) {
      throw new Error([...new Set(problems)].slice(0, 20).join("; "));
    }
  });
}
