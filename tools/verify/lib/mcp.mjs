import { createHash, randomBytes } from "node:crypto";
import { createServer } from "node:http";

// An MCP client that signs in the way ChatGPT and other hosts do: OAuth
// discovery, dynamic client registration, the authorisation-code flow with
// PKCE and a loopback redirect, then JSON-RPC over the streamable HTTP
// transport at /mcp.

const form = (fields) => ({
  method: "POST",
  headers: { "content-type": "application/x-www-form-urlencoded" },
  body: new URLSearchParams(fields),
});

// A one-shot loopback listener for the redirect, as a native client uses.
function loopback() {
  return new Promise((resolve, reject) => {
    let deliver;
    const received = new Promise((done) => (deliver = done));
    const server = createServer((request, response) => {
      deliver(new URL(request.url, "http://127.0.0.1").searchParams);
      response.end("You can close this window.");
    });
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolve({
        uri: `http://127.0.0.1:${port}/callback`,
        received,
        close: () => new Promise((done) => server.close(done)),
      });
    });
  });
}

export async function register(base, redirectUri, scope) {
  const response = await fetch(`${base}/oauth/register`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      client_name: "web-extension:verify",
      redirect_uris: [redirectUri],
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      token_endpoint_auth_method: "none",
      scope,
    }),
  });
  const body = await response.json();
  if (response.status !== 201 || !body.client_id) {
    throw new Error(`registration returned ${response.status}`);
  }
  return body.client_id;
}

// Runs the whole flow and returns the redirect's query, the registered client
// id and, when the redirect carried a code, the token response. `verifier` overrides the PKCE verifier sent to
// the token endpoint, to check a wrong one is refused.
export async function authorize(
  base,
  { password, scope, verifier: tokenVerifier },
) {
  const callback = await loopback();
  try {
    const clientId = await register(base, callback.uri, scope);
    const verifier = randomBytes(32).toString("base64url");
    const challenge = createHash("sha256").update(verifier).digest("base64url");
    const state = randomBytes(8).toString("hex");
    const resource = `${base}/mcp`;
    const query = new URLSearchParams({
      response_type: "code",
      client_id: clientId,
      redirect_uri: callback.uri,
      scope,
      state,
      code_challenge: challenge,
      code_challenge_method: "S256",
      resource,
    });
    const consent = await fetch(`${base}/oauth/authorize?${query}`);
    const html = await consent.text();
    const key = html.match(/name="session_key" value="([^"]+)"/)?.[1];
    if (consent.status !== 200 || !key) {
      throw new Error(`the consent page returned ${consent.status}`);
    }
    const approval = await fetch(`${base}/oauth/approve`, {
      ...form({ session_key: key, approved: "true", password }),
      redirect: "manual",
    });
    const location = approval.headers.get("location");
    if (!location?.startsWith(callback.uri)) {
      throw new Error(`approval returned ${approval.status}, not a redirect`);
    }
    // The browser would follow the redirect to the loopback listener.
    await fetch(location);
    const params = await callback.received;
    if (params.get("state") !== state) throw new Error("state did not match");
    const code = params.get("code");
    if (!code) return { params, clientId };
    const token = await fetch(
      `${base}/oauth/token`,
      form({
        grant_type: "authorization_code",
        code,
        redirect_uri: callback.uri,
        code_verifier: tokenVerifier ?? verifier,
        client_id: clientId,
        resource,
      }),
    );
    return {
      params,
      clientId,
      status: token.status,
      token: await token.json(),
    };
  } finally {
    await callback.close();
  }
}

// One JSON-RPC exchange. The server answers as a single-event SSE stream.
export async function rpc(base, token, method, params, id = 1) {
  const response = await fetch(`${base}/mcp`, {
    method: "POST",
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
    },
    body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
  });
  const text = await response.text();
  if (response.status !== 200) return { status: response.status, response };
  const data = text
    .split("\n")
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice(5).trim())
    .join("");
  return { status: 200, response, ...JSON.parse(data || text) };
}

export class McpSession {
  constructor(base, token) {
    this.base = base;
    this.token = token;
    this.id = 0;
  }

  async request(method, params) {
    const reply = await rpc(this.base, this.token, method, params, ++this.id);
    if (reply.status !== 200)
      throw new Error(`${method} returned ${reply.status}`);
    if (reply.error) throw new Error(`${method}: ${reply.error.message}`);
    return reply.result;
  }

  async initialize() {
    return this.request("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "web-extension:verify", version: "1.0.0" },
    });
  }

  // A tool's raw result, as a host passes it on to the app.
  callRaw(name, args = {}) {
    return this.request("tools/call", { name, arguments: args });
  }

  async call(name, args = {}) {
    const result = await this.callRaw(name, args);
    if (result.isError) {
      throw new Error(`${name}: ${result.content?.[0]?.text ?? "failed"}`);
    }
    return result.structuredContent;
  }
}
