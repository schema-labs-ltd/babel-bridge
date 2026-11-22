import { execFile } from "node:child_process";
import {
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { SECRET_MARK } from "./client.mjs";
import { FAKE } from "./journeys.mjs";

const run = promisify(execFile);

// Session tokens the server hands to a browser, as they appear in the
// requests, responses and console lines a Playwright trace records.
const TOKEN_PATTERN =
  /(Bearer\s+|[?&]token=|"token"\s*:\s*"|"access_token"\s*:\s*"|"refresh_token"\s*:\s*")([A-Za-z0-9_~.%-]{16,})/g;

// Every token TOKEN_PATTERN finds in the text.
export function tokensIn(text) {
  return [...String(text).matchAll(TOKEN_PATTERN)].map((match) => match[2]);
}

// Replaces the given secrets, and any token TOKEN_PATTERN finds, with
// SECRET_MARK.
export function redactText(text, secrets) {
  let out = String(text).replace(
    TOKEN_PATTERN,
    (_, prefix) => `${prefix}${SECRET_MARK}`,
  );
  for (const secret of secrets) {
    if (secret && secret.length >= 6) out = out.split(secret).join(SECRET_MARK);
  }
  return out;
}

// Container logs must not report errors or panics, or print a fake phone
// number in plain text (log_privacy masks them), the password or a token.
// Lines matching an `allow` pattern are known problems a caller accepts.
export function checkLogs(raw, { allow = [] } = {}) {
  const text = raw
    .split("\n")
    .filter((line) => !allow.some((pattern) => pattern.test(line)))
    .join("\n");
  const problems = [];
  if (/\bERROR\b/.test(text)) problems.push("an ERROR line");
  if (/panicked|\bpanic:/.test(text)) problems.push("a panic");
  for (const phone of FAKE.phones) {
    if (text.includes(phone)) problems.push(`the phone number ${phone}`);
  }
  if (text.includes(SECRET_MARK)) problems.push("the password or a token");
  if (problems.length > 0) {
    throw new Error(`container logs contain ${problems.join(", ")}`);
  }
}

function* files(dir) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* files(path);
    else if (entry.isFile()) yield path;
  }
}

// Text files only: a NUL byte near the start marks images, video and other
// binary files, which cannot hold a typed password or a token.
const isText = (buffer) => !buffer.subarray(0, 8192).includes(0);

function redactFile(path, secrets) {
  const buffer = readFileSync(path);
  if (!isText(buffer)) return false;
  const text = buffer.toString("utf8");
  const clean = redactText(text, secrets);
  if (clean === text) return false;
  writeFileSync(path, clean);
  return true;
}

// Playwright traces are zip files, so their entries are redacted unpacked.
async function redactZip(path, secrets) {
  const dir = mkdtempSync(join(tmpdir(), "verify-trace-"));
  try {
    await run("unzip", ["-q", "-o", path, "-d", dir]);
    let changed = false;
    for (const file of files(dir))
      changed = redactFile(file, secrets) || changed;
    if (changed) {
      const target = resolve(path);
      rmSync(target);
      await run("zip", ["-q", "-r", "-X", target, "."], { cwd: dir });
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function contents(path) {
  if (path.endsWith(".zip")) {
    const { stdout } = await run("unzip", ["-p", path], {
      encoding: "buffer",
      maxBuffer: 512 * 1024 * 1024,
    });
    return stdout.toString("latin1");
  }
  return readFileSync(path).toString("latin1");
}

// Collects every token in the evidence, removes those tokens and the given
// secrets from every file, then checks that none is left. Returns the paths
// that still hold a secret or could not be redacted or read back, which the
// caller must delete and treat as a failure: redaction fails closed.
export async function redactEvidence(dir, secrets) {
  const all = new Set(secrets.filter((secret) => secret?.length >= 6));
  const paths = [...files(dir)].filter((path) => statSync(path).size > 0);
  const bad = new Set();
  for (const path of paths) {
    try {
      for (const token of tokensIn(await contents(path))) all.add(token);
    } catch {
      bad.add(path);
    }
  }
  const list = [...all];
  for (const path of paths) {
    if (bad.has(path)) continue;
    try {
      if (path.endsWith(".zip")) await redactZip(path, list);
      else redactFile(path, list);
      const text = await contents(path);
      if (list.some((secret) => text.includes(secret))) bad.add(path);
    } catch {
      bad.add(path);
    }
  }
  return [...bad];
}
