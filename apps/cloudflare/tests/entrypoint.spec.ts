// Runs container/entrypoint.sh against the real backup gateway code (with an
// in-memory bucket) and a stand-in server, to check restore, periodic and
// final backups, and that a failed restore never starts the server.
import assert from "node:assert/strict";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { CURRENT_POINTER, handleBackupRequest } from "../src/backup-store.ts";
import { FakeBucket } from "./fake-bucket.ts";

const here = import.meta.dirname;
const ENTRYPOINT = join(here, "../container/entrypoint.sh");
const FAKE_SERVER = join(here, "fixtures/fake-server.sh");
const hasSqlite = spawnSync("sqlite3", ["--version"]).status === 0;

const bucket = new FakeBucket();
let failRestore = false;
const requests: string[] = [];
let gateway: Server;
let gatewayUrl = "";
const scratch = mkdtempSync(join(tmpdir(), "cf-entrypoint-"));

before(async () => {
  gateway = createServer(async (incoming, outgoing) => {
    const chunks: Buffer[] = [];
    for await (const chunk of incoming) chunks.push(chunk as Buffer);
    requests.push(`${incoming.method} ${incoming.url}`);
    let response: Response;
    if (failRestore && incoming.method === "GET") {
      response = new Response("unavailable", { status: 500 });
    } else {
      const body = chunks.length > 0 ? Buffer.concat(chunks) : undefined;
      response = await handleBackupRequest(
        new Request(`http://192.0.2.1${incoming.url}`, {
          method: incoming.method,
          headers: incoming.headers as Record<string, string>,
          body,
        }),
        bucket,
      );
    }
    outgoing.writeHead(response.status, Object.fromEntries(response.headers));
    outgoing.end(Buffer.from(await response.arrayBuffer()));
  });
  await new Promise<void>((resolve) => gateway.listen(0, "127.0.0.1", resolve));
  gatewayUrl = `http://127.0.0.1:${(gateway.address() as AddressInfo).port}`;
});

const children: ChildProcess[] = [];

after(() => {
  // Each entrypoint runs in its own process group; stop any that a failed
  // test left running, with the server and backup loop it started.
  for (const child of children) stopGroup(child);
  gateway.close();
  rmSync(scratch, { recursive: true, force: true });
});

function run(
  name: string,
  server = FAKE_SERVER,
): {
  child: ChildProcess;
  log: string;
  exited: Promise<number | null>;
} {
  const log = join(scratch, `${name}.log`);
  const child = spawn("bash", [ENTRYPOINT], {
    env: {
      PATH: process.env.PATH,
      WA_DATA_DIR: join(scratch, name),
      WA_BACKUP_URL: gatewayUrl,
      WA_SERVER_BIN: server,
      WA_BACKUP_INTERVAL_SECONDS: "1",
      // Small parts, so even a tiny archive is uploaded in several.
      WA_BACKUP_PART_BYTES: "512",
      FAKE_SERVER_LOG: log,
    },
    stdio: ["ignore", "inherit", "inherit"],
    detached: true,
  });
  children.push(child);
  const exited = new Promise<number | null>((resolve) =>
    child.on("exit", resolve),
  );
  return { child, log, exited };
}

async function until(check: () => boolean, what: string, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

const readLog = (path: string) =>
  existsSync(path) ? readFileSync(path, "utf8") : "";

function stopGroup(child: ChildProcess) {
  if (child.exitCode === null && child.pid) {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      // Already gone.
    }
  }
}

async function currentArchiveEntries(): Promise<string[]> {
  const response = await handleBackupRequest(
    new Request("http://192.0.2.1/backup"),
    bucket,
  );
  const path = join(scratch, "current.tar.gz");
  writeFileSync(path, Buffer.from(await response.arrayBuffer()));
  return spawnSync("tar", ["-tzf", path], { encoding: "utf8" })
    .stdout.split("\n")
    .filter(Boolean);
}

test(
  "restores, backs up while running and on SIGTERM",
  { skip: !hasSqlite && "needs sqlite3" },
  async () => {
    const first = run("first");
    try {
      await until(
        () => readLog(first.log).includes("started rows=0 host="),
        "a fresh start",
      );
      await until(
        () => bucket.objects.has(CURRENT_POINTER),
        "a periodic backup",
      );
      // A backup taken while the server holds the database open in WAL mode
      // carries a complete database and no WAL file.
      const entries = await currentArchiveEntries();
      assert.ok(entries.includes("./messages.db"), entries.join(" "));
      assert.ok(
        !entries.some(
          (entry) => entry.endsWith("-wal") || entry.endsWith("-shm"),
        ),
        entries.join(" "),
      );

      first.child.kill("SIGTERM");
      assert.equal(await first.exited, 0);
      assert.match(readLog(first.log), /stopped/);
      assert.ok(
        requests.filter((request) => request.includes("/parts/2")).length > 0,
        "uploads larger archives in several parts",
      );
    } finally {
      stopGroup(first.child);
    }

    const second = run("second");
    try {
      await until(
        () => readLog(second.log).includes("started"),
        "the second start",
      );
      const started = readLog(second.log);
      const rows = Number(/rows=(\d+)/.exec(started)?.[1]);
      assert.ok(rows > 0, `restored rows: ${started}`);
      assert.match(started, /host=host-id/);
      second.child.kill("SIGTERM");
      assert.equal(await second.exited, 0);
    } finally {
      stopGroup(second.child);
    }
  },
);

test(
  "exits without starting the server when the restore fails",
  { skip: !hasSqlite && "needs sqlite3" },
  async () => {
    failRestore = true;
    try {
      requests.length = 0;
      const failed = run("failed");
      assert.equal(await failed.exited, 1);
      assert.equal(readLog(failed.log), "");
      assert.deepEqual(
        requests.filter((request) => !request.startsWith("GET")),
        [],
        "nothing was uploaded",
      );
    } finally {
      failRestore = false;
    }
  },
);

test(
  "does not upload again when a server exits without changing anything",
  { skip: !hasSqlite && "needs sqlite3" },
  async () => {
    assert.ok(
      bucket.objects.has(CURRENT_POINTER),
      "runs after a backup exists",
    );
    requests.length = 0;
    const crashed = run("crash", "/bin/false");
    assert.equal(await crashed.exited, 1);
    assert.deepEqual(requests, ["GET /backup"]);
  },
);

test("runs the server directly, with its arguments, outside Cloudflare", () => {
  const result = spawnSync(
    "bash",
    [ENTRYPOINT, "--verbose", "--port", "4000"],
    {
      env: { PATH: process.env.PATH, WA_SERVER_BIN: "/bin/echo" },
      encoding: "utf8",
    },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, "--verbose --port 4000\n");
});
