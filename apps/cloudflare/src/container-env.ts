// Builds the environment the server process gets inside the container.

// The address the container uses to reach the backup gateway. 192.0.2.0/24 is
// reserved for documentation (RFC 5737) and never routed on the Internet, so
// only the outbound intercept that index.ts registers can answer it.
export const BACKUP_GATEWAY_HOST = "192.0.2.1";
export const SERVER_PORT = 3000;
export const DATA_DIR = "/data";

// Worker variables and secrets passed through to the server when set. Every
// one is described in README.md. Anything not listed stays in the Worker.
export const PASSTHROUGH_VARIABLES = [
  "WA_PASSWORD",
  "OPENAI_API_KEY",
  "WA_DEFAULT_LANGUAGE",
  "WA_OPENAI_DETECTION_MODEL",
  "WA_OPENAI_TRANSLATION_MODEL",
  "WA_OPENAI_HIGH_END_MODEL",
  "WA_VERBOSE",
  "APNS_KEY_ID",
  "APNS_TEAM_ID",
  "APNS_BUNDLE_ID",
  "APNS_PRIVATE_KEY_BASE64",
  "WA_GIT_COMMIT",
  "WA_BACKUP_INTERVAL_SECONDS",
] as const;

export function containerEnv(
  source: Partial<Record<string, unknown>>,
): Record<string, string> {
  const env: Record<string, string> = {};
  for (const name of PASSTHROUGH_VARIABLES) {
    const value = source[name];
    if (typeof value === "string" && value !== "") env[name] = value;
  }
  // Fixed values last, so a Worker variable cannot override them.
  Object.assign(env, {
    WA_WEB: "true",
    WA_HOST: "0.0.0.0",
    WA_PORT: String(SERVER_PORT),
    WA_DATA_DIR: DATA_DIR,
    WA_BRIDGE_PATH: "/app/wa-bridge",
    WA_BACKUP_URL: `http://${BACKUP_GATEWAY_HOST}`,
  });
  return env;
}

// A short digest of the environment, stored as a container label. When a
// secret or variable changes, the digest no longer matches and the Durable
// Object restarts the container so the server reads the new values. It is a
// truncated SHA-256 over every value together, so it does not reveal any one
// secret.
export async function configDigest(
  env: Record<string, string>,
): Promise<string> {
  const canonical = JSON.stringify(
    Object.keys(env)
      .sort()
      .map((name) => [name, env[name]]),
  );
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(canonical),
  );
  return Array.from(new Uint8Array(digest).slice(0, 16), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}
