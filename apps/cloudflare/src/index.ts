// Runs the Babel Bridge server image on Cloudflare Containers.
//
// One Durable Object, named "primary", owns one container. The container holds
// the WhatsApp connection, so there is exactly one and it is kept running: a
// cron trigger wakes the Durable Object every minute, and its alarm restarts
// the container if it stops. Every HTTP and WebSocket request is passed to the
// server inside the container. The image's entrypoint (container/entrypoint.sh)
// sees WA_BACKUP_URL, restores the server's data directory from R2 through
// BackupGateway each time the container starts, and backs it up while it runs.

import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import { handleBackupRequest } from "./backup-store";
import {
  BACKUP_GATEWAY_HOST,
  SERVER_PORT,
  configDigest,
  containerEnv,
} from "./container-env";
import { forwardedRequest } from "./proxy";

export interface Env {
  BACKEND: DurableObjectNamespace<Backend>;
  BACKUPS: R2Bucket;
  CONTAINER_INSTANCE?: string;
  [variable: string]: unknown;
}

const INSTANCE_NAME = "primary";
const INSTANCE_TYPES = [
  "lite",
  "standard-1",
  "standard-2",
  "standard-3",
  "standard-4",
] as const;
type InstanceType = (typeof INSTANCE_TYPES)[number];
const DEFAULT_INSTANCE: InstanceType = "standard-1";

// The longest inactivity timeout Cloudflare allows. The alarm below keeps the
// Durable Object active long before it runs out.
const INACTIVITY_TIMEOUT_MS = 6 * 60 * 60 * 1000;
const WATCHDOG_INTERVAL_MS = 30 * 1000;
// How long one request waits for a starting server before it gets a 503.
const REQUEST_READY_TIMEOUT_MS = 30 * 1000;
// How long the server may take to start, including restoring the backup and
// running storage migrations, before the health check gives up.
const STARTUP_TIMEOUT_MS = 10 * 60 * 1000;
// Time for the entrypoint to stop the server and upload a final backup after
// SIGTERM before the container is destroyed.
const STOP_GRACE_MS = 5 * 60 * 1000;
const SIGTERM = 15;

function instanceType(value: unknown): InstanceType {
  return INSTANCE_TYPES.find((type) => type === value) ?? DEFAULT_INSTANCE;
}

// Answers the container's backup requests to http://192.0.2.1. Only the
// container can reach this entrypoint, through the intercept registered in
// Backend.start().
export class BackupGateway extends WorkerEntrypoint<Env> {
  async fetch(request: Request): Promise<Response> {
    return handleBackupRequest(request, this.env.BACKUPS);
  }
}

export class Backend extends DurableObject<Env> {
  private starting: Promise<void> | undefined;
  private stopping: Promise<void> | undefined;
  private readiness: Promise<boolean> | undefined;
  private ready = false;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    const container = ctx.container;
    if (container?.running) {
      // A restarted Durable Object (for example after a deploy) loses the
      // timeout and the monitor, but the container keeps running.
      void ctx.blockConcurrencyWhile(() =>
        container.setInactivityTimeout(INACTIVITY_TIMEOUT_MS),
      );
      this.watch();
    }
  }

  async fetch(request: Request): Promise<Response> {
    await this.ensureRunning();
    if (!(await this.waitUntilReady(REQUEST_READY_TIMEOUT_MS))) {
      return new Response("Babel Bridge is starting. Try again shortly.\n", {
        status: 503,
        headers: { "retry-after": "10", "content-type": "text/plain" },
      });
    }
    return this.container()
      .getTcpPort(SERVER_PORT)
      .fetch(forwardedRequest(request));
  }

  // Called by the cron trigger. Makes sure the watchdog alarm is scheduled
  // and the container is running.
  async wake(): Promise<void> {
    await this.scheduleWatchdog();
    await this.ensureRunning();
  }

  async alarm(): Promise<void> {
    try {
      await this.restartIfOutdated();
      await this.ensureRunning();
    } finally {
      await this.ctx.storage.setAlarm(Date.now() + WATCHDOG_INTERVAL_MS);
    }
  }

  private container(): Container {
    const container = this.ctx.container;
    if (!container) throw new Error("No container is configured for Backend");
    return container;
  }

  private async scheduleWatchdog(): Promise<void> {
    if ((await this.ctx.storage.getAlarm()) === null) {
      await this.ctx.storage.setAlarm(Date.now() + WATCHDOG_INTERVAL_MS);
    }
  }

  private async desiredStart() {
    const env = containerEnv(this.env);
    return { env, digest: await configDigest(env) };
  }

  private async ensureRunning(): Promise<void> {
    if (this.stopping) await this.stopping;
    if (this.container().running) return;
    this.starting ??= this.start().finally(() => {
      this.starting = undefined;
    });
    await this.starting;
  }

  private async start(): Promise<void> {
    const container = this.container();
    if (container.running) return;
    const { env, digest } = await this.desiredStart();
    // Register the backup intercept before the entrypoint runs, since its
    // first step is restoring the data directory through it. Intercepts last
    // until the container stops, so each start registers it again.
    const exports = this.ctx.exports as unknown as { BackupGateway: Fetcher };
    await container.interceptOutboundHttp(
      `${BACKUP_GATEWAY_HOST}:80`,
      exports.BackupGateway,
    );
    this.ready = false;
    container.start({
      image: container.images.base,
      instance: instanceType(this.env.CONTAINER_INSTANCE),
      // WhatsApp, OpenAI and APNs are all on the Internet.
      enableInternet: true,
      env,
      labels: { config: digest },
    });
    await container.setInactivityTimeout(INACTIVITY_TIMEOUT_MS);
    this.watch();
    await this.scheduleWatchdog();
  }

  private watch(): void {
    this.ctx.waitUntil(
      this.container()
        .monitor()
        .then(() => console.log("Container exited"))
        .catch((error) =>
          console.error("Container stopped with an error", error),
        )
        .finally(() => {
          this.ready = false;
        }),
    );
  }

  // After a deploy that changes the image, or a change to a secret or
  // variable, restart the container so it runs what is configured now.
  private async restartIfOutdated(): Promise<void> {
    const container = this.container();
    if (!container.running || this.starting || this.stopping) return;
    const info = await container.inspect();
    // inspect() reports an empty image while a container is starting.
    if (!info || info.image === "") return;
    const { digest } = await this.desiredStart();
    if (info.image === container.images.base && info.labels.config === digest) {
      return;
    }
    console.log(
      "Restarting the container to apply a new image or configuration",
    );
    this.stopping = this.stopGracefully().finally(() => {
      this.stopping = undefined;
    });
    await this.stopping;
  }

  private async stopGracefully(): Promise<void> {
    const container = this.container();
    this.ready = false;
    // The entrypoint stops the server and uploads a final backup on SIGTERM.
    container.signal(SIGTERM);
    await Promise.race([
      container.monitor().catch(() => undefined),
      scheduler.wait(STOP_GRACE_MS),
    ]);
    if (container.running) {
      await container.destroy("The container did not stop after SIGTERM");
    }
  }

  private async waitUntilReady(timeoutMs: number): Promise<boolean> {
    if (this.ready) return true;
    this.readiness ??= this.pollHealth().finally(() => {
      this.readiness = undefined;
    });
    return Promise.race([
      this.readiness,
      scheduler.wait(timeoutMs).then(() => false),
    ]);
  }

  private async pollHealth(): Promise<boolean> {
    const container = this.container();
    const port = container.getTcpPort(SERVER_PORT);
    const deadline = Date.now() + STARTUP_TIMEOUT_MS;
    while (Date.now() < deadline && container.running) {
      try {
        const response = await port.fetch("http://container/api/health", {
          signal: AbortSignal.timeout(2000),
        });
        await response.body?.cancel();
        if (response.ok) {
          this.ready = true;
          return true;
        }
      } catch {
        // The server is not listening yet.
      }
      await scheduler.wait(500);
    }
    return false;
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    return env.BACKEND.getByName(INSTANCE_NAME).fetch(request);
  },

  async scheduled(
    _controller: ScheduledController,
    env: Env,
    ctx: ExecutionContext,
  ): Promise<void> {
    ctx.waitUntil(env.BACKEND.getByName(INSTANCE_NAME).wake());
  },
} satisfies ExportedHandler<Env>;
