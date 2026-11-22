import { spawn } from "node:child_process";
import { createServer } from "node:net";

// A free loopback port, for a container that must keep its address across a
// `docker restart`.
export function freePort() {
  return new Promise((done, fail) => {
    const server = createServer();
    server.once("error", fail);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => done(port));
    });
  });
}

// Runs `playwright test` with a config and one project, logging every line
// through `log` (which redacts), and rejects unless every test passed.
export function runPlaywright({ config, project, env, log }) {
  return new Promise((done, fail) => {
    const args = ["--no-install", "playwright", "test", "--config", config];
    if (project) args.push("--project", project);
    const child = spawn("npx", args, {
      env: { ...process.env, ...env },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let rest = "";
    const print = (chunk) => {
      const lines = (rest + chunk).split("\n");
      rest = lines.pop();
      for (const line of lines) log(`  ${line}`);
    };
    child.stdout.on("data", print);
    child.stderr.on("data", print);
    child.on("error", fail);
    child.on("close", (code) => {
      if (rest) log(`  ${rest}`);
      if (code === 0) done();
      else fail(new Error(`Playwright exited ${code}; see the log above`));
    });
  });
}
