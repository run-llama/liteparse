// Process-isolated worker pool for LiteParse.
//
// Not public API; use `new LiteParse({ poolSize, parseTimeoutMs })`.

import { fork, type ChildProcess } from "node:child_process";
import { fileURLToPath } from "node:url";
import type { ParseResult } from "./lib.js";

/** A pooled parse exceeded `parseTimeoutMs` and its worker was killed.
 *
 * Only thrown in pool mode, where the deadline is enforced by killing the
 * worker process — the timed-out parse is guaranteed dead, not still running
 * in the background. `source` names the document (file path, or `<N bytes>`
 * for byte inputs); log it to identify the documents that stall your
 * pipeline.
 */
export class ParseTimeoutError extends Error {
  readonly source: string;
  readonly timeoutMs: number;

  constructor(message: string, source: string, timeoutMs: number) {
    super(message);
    this.name = "ParseTimeoutError";
    this.source = source;
    this.timeoutMs = timeoutMs;
  }
}

class WorkerTimeout extends Error {}
class WorkerCrashed extends Error {}

type WorkerResponse =
  | { type: "ready" }
  | { type: "initError"; message: string }
  | { type: "ok"; result: ParseResult }
  | { type: "err"; message: string };

const WORKER_PATH = fileURLToPath(new URL("./pool-worker.js", import.meta.url));

/** Workers run a file, so stdin/eval-only options must not be inherited.
 * Supplying execArgv also bypasses fork()'s automatic removal of the parent
 * eval source; remove it here to avoid running the caller again in a worker. */
function workerExecArgv(): string[] {
  const args: string[] = [];
  for (let i = 0; i < process.execArgv.length; i++) {
    const arg = process.execArgv[i];
    if (arg === "--input-type" || arg === "--eval" || arg === "-e") {
      i++;
    } else if (arg === "--print" || /^-[ep]+$/.test(arg)) {
      // Print can read stdin, in which case the next option is not a source.
      if (process.execArgv[i + 1] && !process.execArgv[i + 1].startsWith("-")) i++;
    } else if (!/^--(?:input-type|eval|print)=/.test(arg)) {
      args.push(arg);
    }
  }
  return args;
}

/** IPC deserialization yields plain Uint8Arrays where the API promises
 * Buffers; rewrap in place (no copy). */
function reviveBuffers(result: ParseResult): ParseResult {
  for (const image of result.images ?? []) {
    if (image.bytes && !Buffer.isBuffer(image.bytes)) {
      const b = image.bytes as Uint8Array;
      image.bytes = Buffer.from(b.buffer, b.byteOffset, b.byteLength);
    }
  }
  for (const shot of result.screenshots ?? []) {
    if (shot.imageBuffer && !Buffer.isBuffer(shot.imageBuffer)) {
      const b = shot.imageBuffer as Uint8Array;
      shot.imageBuffer = Buffer.from(b.buffer, b.byteOffset, b.byteLength);
    }
  }
  return result;
}

class WorkerHandle {
  private child: ChildProcess;
  private readyPromise: Promise<void>;
  private pending: {
    resolve: (r: ParseResult) => void;
    reject: (e: Error) => void;
  } | null = null;
  private dead = false;

  constructor(config: Record<string, unknown>) {
    this.child = fork(WORKER_PATH, [], {
      execArgv: workerExecArgv(),
      serialization: "advanced",
      // stdout/stderr inherited: parse logs and crash traces stay visible.
      stdio: ["ignore", "inherit", "inherit", "ipc"],
    });

    let readyResolve!: () => void;
    let readyReject!: (e: Error) => void;
    this.readyPromise = new Promise<void>((resolve, reject) => {
      readyResolve = resolve;
      readyReject = reject;
    });
    // ready() may never be awaited if the worker is retired first.
    this.readyPromise.catch(() => {});

    this.child.on("message", (msg: WorkerResponse) => {
      if (msg.type === "ready") {
        readyResolve();
        if (this.pending === null) this.idle();
      } else if (msg.type === "initError") {
        readyReject(new Error(msg.message));
      } else if (this.pending) {
        const { resolve, reject } = this.pending;
        this.pending = null;
        this.idle();
        if (msg.type === "ok") resolve(reviveBuffers(msg.result));
        else reject(new WorkerCrashed(msg.message));
      }
    });
    const onGone = (cause: string) => {
      this.dead = true;
      readyReject(new WorkerCrashed(cause));
      if (this.pending) {
        const { reject } = this.pending;
        this.pending = null;
        reject(new WorkerCrashed(cause));
      }
    };
    this.child.on("error", (e) => onGone(e.message));
    this.child.on("exit", (code, signal) =>
      onGone(`worker exited (code=${code}, signal=${signal})`),
    );

    // Stay ref'd until the init handshake arrives — a caller awaiting
    // warmUp()/ready() must keep the event loop alive. idle() runs on
    // "ready"; after that, an idle worker never holds the loop open.
    this.child.send({ type: "init", config });
  }

  /** Resolves when the worker's native parser is constructed. Init time
   * never counts toward the parse deadline — the deadline is a promise about
   * parsing, not about process startup. */
  ready(): Promise<void> {
    return this.readyPromise;
  }

  /** An idle pool must not hold the parent's event loop open. */
  private idle(): void {
    this.child.unref();
    this.child.channel?.unref();
  }

  request(
    payload: string | Buffer,
    timeoutMs: number | undefined,
  ): Promise<ParseResult> {
    if (this.dead) {
      return Promise.reject(new WorkerCrashed("worker already exited"));
    }
    this.child.ref();
    this.child.channel?.ref();
    return new Promise<ParseResult>((resolve, reject) => {
      let timer: NodeJS.Timeout | undefined;
      const settle =
        (fn: (v: never) => void) =>
        (value: never): void => {
          if (timer !== undefined) clearTimeout(timer);
          fn(value);
        };
      this.pending = {
        resolve: settle(resolve) as (r: ParseResult) => void,
        reject: settle(reject) as (e: Error) => void,
      };
      if (timeoutMs !== undefined) {
        timer = setTimeout(() => {
          if (this.pending) {
            const { reject: rejectPending } = this.pending;
            this.pending = null;
            rejectPending(new WorkerTimeout());
          }
        }, timeoutMs);
      }
      this.child.send({ type: "parse", payload });
    });
  }

  kill(): void {
    this.dead = true;
    this.child.kill("SIGKILL");
  }

  /** Graceful shutdown; escalates to SIGKILL if the worker doesn't exit. */
  stop(): void {
    if (this.dead) return;
    this.dead = true;
    try {
      this.child.send({ type: "stop" });
    } catch {
      // channel already closed
    }
    const escalate = setTimeout(() => this.child.kill("SIGKILL"), 5000);
    escalate.unref();
    this.child.once("exit", () => clearTimeout(escalate));
    this.idle();
  }
}

export class WorkerPool {
  private config: Record<string, unknown>;
  private timeoutMs: number | undefined;
  private workers = new Set<WorkerHandle>();
  private idle: WorkerHandle[] = [];
  private waiters: Array<{
    resolve: (w: WorkerHandle) => void;
    reject: (e: Error) => void;
  }> = [];
  private closed = false;

  constructor(
    config: Record<string, unknown>,
    poolSize: number,
    parseTimeoutMs?: number,
  ) {
    if (!Number.isInteger(poolSize) || poolSize < 1) {
      throw new Error("poolSize must be an integer >= 1");
    }
    if (parseTimeoutMs !== undefined && !(parseTimeoutMs > 0)) {
      throw new Error("parseTimeoutMs must be > 0");
    }
    this.config = config;
    this.timeoutMs = parseTimeoutMs;
    // Spawn eagerly: children load the addon and construct their native
    // parsers concurrently while the caller goes on with its own startup.
    for (let i = 0; i < poolSize; i++) {
      this.spawnWorker();
    }
  }

  private spawnWorker(): void {
    const worker = new WorkerHandle(this.config);
    this.workers.add(worker);
    this.release(worker);
  }

  private acquire(): Promise<WorkerHandle> {
    const worker = this.idle.pop();
    if (worker !== undefined) return Promise.resolve(worker);
    return new Promise((resolve, reject) =>
      this.waiters.push({ resolve, reject }),
    );
  }

  private release(worker: WorkerHandle): void {
    if (this.closed) {
      this.workers.delete(worker);
      worker.stop();
      return;
    }
    const waiter = this.waiters.shift();
    if (waiter !== undefined) waiter.resolve(worker);
    else this.idle.push(worker);
  }

  private retire(worker: WorkerHandle): void {
    worker.kill();
    this.workers.delete(worker);
    if (!this.closed) this.spawnWorker();
  }

  /** Run one parse on an idle worker.
   *
   * Waits for a free worker first; `parseTimeoutMs` bounds the parse itself,
   * not the wait. */
  async parse(payload: string | Buffer, source: string): Promise<ParseResult> {
    if (this.closed) throw new Error("parser pool is closed");
    const worker = await this.acquire();
    try {
      await worker.ready();
      const result = await worker.request(payload, this.timeoutMs);
      this.release(worker);
      return result;
    } catch (e) {
      this.retire(worker);
      if (e instanceof WorkerTimeout) {
        throw new ParseTimeoutError(
          `parse of ${source} exceeded ${this.timeoutMs}ms; the worker process was killed`,
          source,
          this.timeoutMs!,
        );
      }
      if (e instanceof WorkerCrashed) {
        throw new Error(
          `liteparse worker process died while parsing ${source}: ${e.message}`,
        );
      }
      throw e;
    }
  }

  /** Resolves when every worker is initialized. Optional — the first parse
   * per worker waits for init anyway. */
  async warmUp(): Promise<void> {
    await Promise.all([...this.workers].map((w) => w.ready()));
  }

  /** Shut down all workers. Idempotent. Busy workers are stopped as their
   * in-flight parses finish. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const waiter of this.waiters.splice(0)) {
      waiter.reject(new Error("parser pool is closed"));
    }
    for (const worker of this.idle.splice(0)) {
      this.workers.delete(worker);
      worker.stop();
    }
  }
}
