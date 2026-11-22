// A small black-box client for the server's HTTP and WebSocket API, used by the
// verify scripts. It records every JSON response by route so the caller can
// check its shape against fixtures/api, and keeps secrets out of what it logs.

export class Step extends Error {
  constructor(step, message) {
    super(`${step}: ${message}`);
    this.step = step;
  }
}

export async function step(name, log, run) {
  const started = Date.now();
  log(`▶ ${name}`);
  try {
    const result = await run();
    log(`✓ ${name} (${Date.now() - started} ms)`);
    return result;
  } catch (error) {
    log(`✗ ${name}: ${error.message}`);
    throw error instanceof Step ? error : new Step(name, error.message);
  }
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export async function until(what, timeoutMs, check, intervalMs = 250) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try {
      const value = await check();
      if (value) return value;
    } catch (error) {
      last = error;
    }
    await sleep(intervalMs);
  }
  throw new Error(
    `timed out after ${timeoutMs} ms waiting for ${what}${last ? ` (${last.message})` : ""}`,
  );
}

export class Client {
  constructor(baseUrl) {
    this.baseUrl = baseUrl.replace(/\/$/, "");
    this.token = null;
    // Route name -> JSON bodies received, for the fixture shape check.
    this.seen = new Map();
  }

  async request(method, path, { body, auth = true, headers = {} } = {}) {
    const init = { method, headers: { ...headers } };
    if (auth && this.token) init.headers.authorization = `Bearer ${this.token}`;
    if (body !== undefined) {
      init.headers["content-type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    const response = await fetch(`${this.baseUrl}${path}`, init);
    const text = await response.text();
    let json;
    if ((response.headers.get("content-type") ?? "").includes("json")) {
      try {
        json = JSON.parse(text);
      } catch {
        json = undefined;
      }
    }
    return { status: response.status, headers: response.headers, text, json };
  }

  // A request that must succeed with JSON. `fixture` names the response file in
  // fixtures/api/responses that this route's body must match.
  async json(method, path, { fixture, ...options } = {}) {
    const response = await this.request(method, path, options);
    if (response.status !== 200) {
      throw new Error(`${method} ${path} returned ${response.status}`);
    }
    if (response.json === undefined) {
      throw new Error(`${method} ${path} did not return JSON`);
    }
    if (fixture) {
      if (!this.seen.has(fixture)) this.seen.set(fixture, []);
      this.seen.get(fixture).push(response.json);
    }
    return response;
  }

  async signIn(password) {
    const { json } = await this.json("POST", "/api/auth", {
      body: { password },
      auth: false,
      fixture: "auth",
    });
    if (json.success !== true || typeof json.token !== "string") {
      throw new Error("sign-in with the right password did not return a token");
    }
    this.token = json.token;
  }

  // Opens /ws and collects every event. Resolves once the socket is open.
  openSocket() {
    const url = new URL(`${this.baseUrl}/ws`);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    if (this.token) url.searchParams.set("token", this.token);
    const socket = new WebSocket(url);
    const events = [];
    const live = { socket, events, closed: false };
    socket.addEventListener("message", (message) => {
      try {
        events.push(JSON.parse(String(message.data)));
      } catch {
        events.push({ type: "unparsable" });
      }
    });
    socket.addEventListener("close", () => {
      live.closed = true;
    });
    return new Promise((resolve, reject) => {
      socket.addEventListener("open", () => resolve(live), { once: true });
      socket.addEventListener(
        "error",
        () => reject(new Error("WebSocket connection was refused")),
        { once: true },
      );
    });
  }
}

// What redactor() puts in place of a secret. It differs from the server's own
// "[redacted]" phone masking, so a leak check can tell the two apart.
export const SECRET_MARK = "[verify-secret]";

// Replaces every secret with SECRET_MARK, for logs and uploaded evidence.
export function redactor(secrets) {
  return (text) => {
    let out = String(text);
    for (const secret of secrets()) {
      if (secret && secret.length >= 6)
        out = out.split(secret).join(SECRET_MARK);
    }
    return out;
  };
}
