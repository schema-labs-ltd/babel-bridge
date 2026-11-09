import { execFile, spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { sleep } from "./client.mjs";

const run = promisify(execFile);

// Runs docker with an argument list (never a shell). `env` adds variables to
// the docker CLI's own environment, so `-e NAME` passes a secret into the
// container without putting it on a command line.
export async function docker(args, { env = {}, quiet = false } = {}) {
  try {
    const { stdout } = await run("docker", args, {
      env: { ...process.env, ...env },
      maxBuffer: 64 * 1024 * 1024,
    });
    return stdout.trim();
  } catch (error) {
    const detail = quiet ? "" : `: ${(error.stderr || error.message).trim()}`;
    throw new Error(`docker ${args[0]} failed${detail}`);
  }
}

// Builds an image from the working tree, or from `ref` checked out in a
// temporary git worktree. Extra flags come from VERIFY_DOCKER_BUILD_FLAGS, for
// a machine that needs, say, a proxy CA added; CI passes none.
export async function buildImage({ tag, ref, log }) {
  const flags = (process.env.VERIFY_DOCKER_BUILD_FLAGS ?? "")
    .split(/\s+/)
    .filter(Boolean);
  let context = ".";
  if (ref) {
    context = mkdtempSync(join(tmpdir(), "verify-base-"));
    await run("git", ["worktree", "add", "--detach", context, ref]);
  }
  log(`  docker build ${ref ? `of ${ref}` : "of the working tree"} as ${tag}`);
  try {
    await new Promise((resolve, reject) => {
      const child = spawn(
        "docker",
        ["build", "--tag", tag, ...flags, context],
        {
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      let tail = "";
      const keep = (chunk) => {
        tail = (tail + chunk).slice(-8000);
      };
      child.stdout.on("data", keep);
      child.stderr.on("data", keep);
      child.on("close", (code) =>
        code === 0
          ? resolve()
          : reject(new Error(`docker build exited ${code}:\n${tail}`)),
      );
    });
  } finally {
    if (ref) await run("git", ["worktree", "remove", "--force", context]);
  }
}

// Starts a container the way Railway runs the image: PORT set, sign-in on,
// a volume on /data, and the fake bridge and AI at the external boundary.
// The health check command is the image's own; only its interval is shortened.
// `port` fixes the host port, so the address survives a `docker restart`;
// otherwise Docker picks a free one.
export async function startContainer({
  name,
  image,
  volume,
  password,
  env = {},
  port: hostPort,
}) {
  const vars = {
    PORT: "3000",
    WA_PASSWORD: password,
    WA_FAKE_BRIDGE: "true",
    WA_FAKE_AI: "true",
    ...env,
  };
  const args = ["run", "--detach", "--name", name];
  for (const key of Object.keys(vars)) args.push("--env", key);
  args.push(
    "--volume",
    `${volume}:/data`,
    "--publish",
    `127.0.0.1:${hostPort ?? ""}:3000`,
    "--health-interval",
    "2s",
    image,
  );
  await docker(args, { env: vars });
  const mapping = await docker(["port", name, "3000/tcp"]);
  const port = mapping.split("\n")[0].split(":").pop();
  return { name, url: `http://127.0.0.1:${port}` };
}

export async function waitHealthy(name, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const status = await docker([
      "inspect",
      "--format",
      "{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}",
      name,
    ]);
    const [state, health] = status.split(" ");
    // A crashed or unhealthy container fails now, not at the deadline.
    if (state !== "running") throw new Error(`container is ${state}`);
    if (health === "unhealthy")
      throw new Error("HEALTHCHECK reports unhealthy");
    if (health === "healthy") return;
    if (Date.now() > deadline) {
      throw new Error(
        `not healthy after ${timeoutMs / 1000} s (${health || "no health"})`,
      );
    }
    await sleep(1000);
  }
}

export async function logs(name) {
  return new Promise((resolve, reject) => {
    execFile(
      "docker",
      ["logs", name],
      { maxBuffer: 64 * 1024 * 1024 },
      (error, stdout, stderr) => {
        const text = `${stdout}${stderr}`;
        if (error) reject(new Error(`docker logs failed for ${name}`));
        else if (!text.trim()) reject(new Error(`${name} wrote no logs`));
        else resolve(text);
      },
    );
  });
}

export async function processes(name) {
  return docker(["top", name, "-o", "pid,comm"]);
}

// Sends SIGTERM, as Railway does, and reports how the container exited.
export async function stop(name, timeoutSeconds = 20) {
  const started = Date.now();
  await docker(["stop", "--time", String(timeoutSeconds), name]);
  const seconds = (Date.now() - started) / 1000;
  const [exitCode, oomKilled] = (
    await docker([
      "inspect",
      "--format",
      "{{.State.ExitCode}} {{.State.OOMKilled}}",
      name,
    ])
  ).split(" ");
  return {
    seconds,
    exitCode: Number(exitCode),
    oomKilled: oomKilled === "true",
  };
}

export async function remove({ containers = [], volumes = [], images = [] }) {
  for (const name of containers) {
    await docker(["rm", "--force", name], { quiet: true }).catch(() => {});
  }
  await sleep(100);
  for (const volume of volumes) {
    await docker(["volume", "rm", "--force", volume], { quiet: true }).catch(
      () => {},
    );
  }
  for (const image of images) {
    await docker(["image", "rm", "--force", image], { quiet: true }).catch(
      () => {},
    );
  }
}
