// ep9-x86-sandbox: authenticated x86_64 Linux sandbox for ep9 test / bench / debug.
// Copied from the webcodecs-x86-sandbox-v2 harness
// (/Users/brooklyn/workspace/github/webcodecs-node-work/x86-sandbox); only the
// names, the cap and these comments differ.
//
// Shape (Sandbox SDK 1.0 style, see https://developers.cloudflare.com/sandbox/reference/):
//
//   client --HTTPS + Bearer--> Worker (https check, auth, route, id check)
//                                 |  env.Sandbox.getByName(id).fetch(...)
//                                 v
//                              Durable Object `Sandbox` (one per id)
//                                 |  this.ctx.container.start/exec/destroy
//                                 |  (`durable_object` scheduling policy: the
//                                 |   container belongs to this Durable Object)
//                                 v
//                              Container (standard-4, linux/amd64, ./Dockerfile)
//
//   Durable Object `Registry` (one, name "global"): app-level cap of
//   MAX_RUNNING sandboxes (the `durable_object` policy has no max_instances).
//
// Routes (all require `Authorization: Bearer <SBX_TOKEN>`; plain http gets 403
// and a bad token gets 401 before any Durable Object or container is touched):
//   POST /exec    {id, cmd, timeout_ms?, cwd?, env?} -> {exitCode, stdout, stderr, ...}
//   POST /write   {id, path, content_b64}            -> {ok, path, bytes, sha256}
//   POST /destroy {id}                               -> {ok, wasRunning, runningAfter}
//                                                       (500 + ok:false when a container still runs)
//   GET  /health[?id=<id>]                           -> {ok, ...[, running, gen, storedGen, registered]}
//   anything else                                    -> 404 (after auth)
//
// Start and destroy of one id run one at a time inside its Durable Object
// (`Sandbox.serial`), so a destroy never deletes the gen or frees the Registry
// slot of a container that an exec started while the destroy was waiting.
// /exec errors that happen before the command is launched carry `ran: false`
// (the client may retry those; it never retries a command that may have run).

import { Files, SandboxFileError } from "@cloudflare/sandbox";
import { DurableObject } from "cloudflare:workers";

const WORKER_NAME = "ep9-x86-sandbox";
const SDK_VERSION = "1.0.1";
const SCHEDULING_POLICY = "durable_object";
// standard-4 = 4 vCPU / 12 GiB / 20 GB: the account limit per container
// (GET /accounts/<id>/containers/me on 2026-10-21: vcpu_per_deployment 4,
// memory_mib_per_deployment 12288, disk_mb_per_deployment 20000).
const INSTANCE_TYPE = "standard-4";
// App-level cap on running sandboxes (cost guard). Enforced by `Registry`.
const MAX_RUNNING = 2;
const REGISTRY_NAME = "global";

// The container keeps running this long after the last request (sleepAfter).
// Code running inside the container does NOT count as activity, so long jobs
// must be polled (client `sbx wait`) more often than this.
const INACTIVITY_TIMEOUT_MS = 30 * 60 * 1000;

const DEFAULT_EXEC_TIMEOUT_MS = 10 * 60 * 1000;
const MAX_EXEC_TIMEOUT_MS = 6 * 60 * 60 * 1000;
const EXEC_WATCHDOG_SLACK_MS = 30 * 1000;
// Output kept in Durable Object memory per stream. The isolate has a 128 MB
// limit and one buffered response costs ~4-5x its stdout size (bytes, string,
// JSON), so keep this small; large output goes to a file + `sbx get`.
const STDOUT_CAP = 8 * 1024 * 1024;
const STDERR_CAP = 1 * 1024 * 1024;
// /write payload: raw bytes after base64 decode (client sends 2 MiB chunks).
const MAX_WRITE_BYTES = 8 * 1024 * 1024;

const ID_RE = /^[a-z0-9-]{1,40}$/;
const B64_RE = /^[A-Za-z0-9+/]*={0,2}$/;

// exec() does not inherit image ENV except PATH; set what tools expect.
const BASE_ENV: Record<string, string> = {
	HOME: "/root",
	USER: "root",
	LANG: "C.UTF-8",
	LC_ALL: "C.UTF-8",
	PATH: "/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
};

class HttpError extends Error {
	constructor(
		readonly status: number,
		message: string,
		// Extra JSON fields for the error body (e.g. `ran: false`).
		readonly extra: Record<string, unknown> = {},
	) {
		super(message);
	}
}

// /exec: wrap an error thrown before the command was launched, so the answer
// says `ran: false` (safe for the client to retry). Same status and message.
function notLaunched(err: unknown): HttpError {
	if (err instanceof HttpError) {
		return new HttpError(err.status, err.message, { ...err.extra, ran: false });
	}
	const message = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
	return new HttpError(500, message, { ran: false });
}

function json(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { "content-type": "application/json; charset=utf-8" },
	});
}

function errorResponse(err: unknown): Response {
	if (err instanceof HttpError) {
		return json({ error: err.message, ...err.extra }, err.status);
	}
	if (SandboxFileError.is(err)) {
		return json({ error: err.message, code: err.code }, 400);
	}
	const message = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
	return json({ error: message }, 500);
}

// Plain http: the client refuses http:// URLs itself; this is the server side.
// `wrangler dev` (localhost) is the only place http is allowed.
function isPlainHttp(request: Request, url: URL): boolean {
	if (url.hostname === "localhost" || url.hostname === "127.0.0.1") {
		return false;
	}
	if (url.protocol !== "https:") {
		return true;
	}
	const visitor = request.headers.get("cf-visitor");
	return visitor !== null && /"scheme"\s*:\s*"http"/.test(visitor);
}

async function tokenMatches(provided: string, expected: string): Promise<boolean> {
	// Hash both to a fixed size first, then compare in constant time
	// (https://developers.cloudflare.com/workers/best-practices/workers-best-practices/).
	const enc = new TextEncoder();
	const [a, b] = await Promise.all([
		crypto.subtle.digest("SHA-256", enc.encode(provided)),
		crypto.subtle.digest("SHA-256", enc.encode(expected)),
	]);
	return crypto.subtle.timingSafeEqual(a, b);
}

async function isAuthorized(request: Request, env: Env): Promise<boolean> {
	const expected = env.SBX_TOKEN;
	if (typeof expected !== "string" || expected.length < 32) {
		// Fail closed when the secret is missing.
		return false;
	}
	const header = request.headers.get("authorization") ?? "";
	const m = /^Bearer ([^\s]+)$/.exec(header);
	if (!m) {
		return false;
	}
	return tokenMatches(m[1], expected);
}

function checkId(id: unknown): string {
	if (typeof id !== "string" || !ID_RE.test(id)) {
		throw new HttpError(400, "id must match ^[a-z0-9-]{1,40}$");
	}
	return id;
}

function checkAbsPath(p: unknown, name: string): string {
	if (typeof p !== "string" || !p.startsWith("/") || p.includes("\0") || p.length > 4096) {
		throw new HttpError(400, `${name} must be an absolute path`);
	}
	return p;
}

async function readJson(request: Request): Promise<{ text: string; body: Record<string, unknown> }> {
	const text = await request.text();
	let body: unknown;
	try {
		body = JSON.parse(text);
	} catch {
		throw new HttpError(400, "body must be JSON");
	}
	if (typeof body !== "object" || body === null || Array.isArray(body)) {
		throw new HttpError(400, "body must be a JSON object");
	}
	return { text, body: body as Record<string, unknown> };
}

export default {
	async fetch(request: Request, env: Env): Promise<Response> {
		const url = new URL(request.url);
		// 0. Plain http: 403 before auth, nothing below runs.
		if (isPlainHttp(request, url)) {
			return json({ error: "https required" }, 403);
		}
		// 1. Auth. Nothing below runs for an unauthenticated request.
		if (!(await isAuthorized(request, env))) {
			return json({ error: "unauthorized" }, 401);
		}

		try {
			const route = `${request.method} ${url.pathname}`;
			switch (route) {
				case "GET /health": {
					const base = {
						ok: true,
						worker: WORKER_NAME,
						sdk: SDK_VERSION,
						schedulingPolicy: SCHEDULING_POLICY,
						instanceType: INSTANCE_TYPE,
						maxRunning: MAX_RUNNING,
						inactivityTimeoutMs: INACTIVITY_TIMEOUT_MS,
						time: new Date().toISOString(),
					};
					const id = url.searchParams.get("id");
					if (id === null) {
						return json(base);
					}
					// Reports state only; never starts a container.
					const stub = env.Sandbox.getByName(checkId(id));
					const res = await stub.fetch(`https://sandbox/status?id=${id}`, { method: "GET" });
					return json({ ...base, id, ...((await res.json()) as object) }, res.status);
				}
				case "POST /exec":
				case "POST /write":
				case "POST /destroy": {
					const { text, body } = await readJson(request);
					const id = checkId(body.id);
					const stub = env.Sandbox.getByName(id);
					return await stub.fetch(`https://sandbox${url.pathname}`, {
						method: "POST",
						headers: { "content-type": "application/json" },
						body: text,
					});
				}
				default:
					return json({ error: "not found" }, 404);
			}
		} catch (err) {
			return errorResponse(err);
		}
	},
} satisfies ExportedHandler<Env>;

interface Drained {
	bytes: Uint8Array;
	total: number;
	truncated: boolean;
}

async function drain(stream: ReadableStream<Uint8Array> | null | undefined, cap: number): Promise<Drained> {
	if (!stream) {
		return { bytes: new Uint8Array(0), total: 0, truncated: false };
	}
	const reader = stream.getReader();
	const chunks: Uint8Array[] = [];
	let kept = 0;
	let total = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		total += value.byteLength;
		if (kept < cap) {
			const take = Math.min(value.byteLength, cap - kept);
			chunks.push(take === value.byteLength ? value : value.subarray(0, take));
			kept += take;
		}
		// Keep reading past the cap so the process never blocks on a full pipe.
	}
	const bytes = new Uint8Array(kept);
	let off = 0;
	for (const c of chunks) {
		bytes.set(c, off);
		off += c.byteLength;
	}
	return { bytes, total, truncated: total > kept };
}

function toHex(buf: ArrayBuffer): string {
	return [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

// Storage key (Durable Object SQLite KV) for the generation id of the
// container this Durable Object started last. The same value is set as the
// container label `gen`, so every request can check that the container it
// talks to is the one it started.
const GEN_KEY = "gen";

function short(gen: string | null): string {
	return gen === null ? "none" : gen.slice(0, 8);
}

export class Sandbox extends DurableObject<Env> {
	private readonly container: Container;
	private readonly files: Files;
	// Lifecycle ops (start in ensureRunning, /destroy) run one at a time: each
	// op waits for the one before it. `lifecycleBusy` counts queued + running
	// ops; while it is > 0, ensureRunning() does not take its fast path.
	private lifecycleTail: Promise<void> = Promise.resolve();
	private lifecycleBusy = 0;
	// One random id per Durable Object incarnation (log only).
	private readonly inc = crypto.randomUUID().slice(0, 8);

	constructor(ctx: DurableObjectState, env: Env) {
		super(ctx, env);
		const container = ctx.container;
		if (!container) {
			throw new Error("The container binding is not configured");
		}
		this.container = container;
		this.files = new Files(container);
		this.log("do-ctor", { running: container.running });
		// A restarted Durable Object (deploy, eviction) starts without a timeout.
		if (container.running) {
			void ctx.blockConcurrencyWhile(() => container.setInactivityTimeout(INACTIVITY_TIMEOUT_MS));
		}
	}

	// Lifecycle log line (no headers, no request bodies, no command text).
	private log(ev: string, fields: Record<string, unknown> = {}): void {
		console.log(JSON.stringify({ sbx: ev, inc: this.inc, id: this.ctx.id.name ?? null, ...fields }));
	}

	private storedGen(): string | null {
		return this.ctx.storage.kv.get<string>(GEN_KEY) ?? null;
	}

	// RPC for `Registry`: is this sandbox's container running?
	isRunning(): boolean {
		return this.container.running;
	}

	// Run one lifecycle op after the previous one has settled (ok or not).
	// The awaits inside an op (container.destroy, Registry calls, the readiness
	// probe) let other requests in; those requests queue here instead of
	// starting or destroying a container in the middle of this op.
	private serial<T>(op: () => Promise<T>): Promise<T> {
		this.lifecycleBusy++;
		const run = this.lifecycleTail.then(op).finally(() => {
			this.lifecycleBusy--;
		});
		this.lifecycleTail = run.then(
			() => undefined,
			() => undefined,
		);
		return run;
	}

	// Make sure this id's container runs; returns the gen it is running under.
	private async ensureRunning(id: string): Promise<string | null> {
		if (this.container.running && this.lifecycleBusy === 0) {
			return this.storedGen();
		}
		return this.serial(async () => {
			if (!this.container.running) {
				const reg = await this.env.Registry.getByName(REGISTRY_NAME).acquire(id);
				if (!reg.ok) {
					throw new HttpError(
						429,
						`Maximum number of running container instances exceeded (${MAX_RUNNING} sandboxes running: ${reg.running.join(", ")}); destroy one first`,
					);
				}
				const image = this.container.images.base;
				if (!image) {
					throw new Error("named image 'base' is missing from ctx.container.images");
				}
				// A stored gen means the last container was not destroyed via
				// /destroy (idle timeout, crash, or the platform lost it): log it.
				const prev = this.storedGen();
				const gen = crypto.randomUUID();
				this.ctx.storage.kv.put(GEN_KEY, gen);
				this.log("start", { gen: short(gen), prevGen: prev === null ? null : short(prev) });
				// `durable_object` scheduling policy: image + size chosen here.
				this.container.start({
					image,
					instance: INSTANCE_TYPE,
					enableInternet: true,
					labels: { gen },
				});
			}
			await this.container.setInactivityTimeout(INACTIVITY_TIMEOUT_MS);
			// exec() waits for a container that is still starting: use it as readiness.
			const probe = await this.container.exec(["true"], { stdout: "ignore", stderr: "ignore" });
			await probe.exitCode;
			return this.storedGen();
		});
	}

	// One id = one container: refuse to work on a container this Durable
	// Object did not start (its `gen` label must match the stored one).
	// `ensured` is the gen ensureRunning() returned for this request.
	private async checkIdentity(id: string, ensured: string | null): Promise<void> {
		const info = await this.container.inspect();
		const want = this.storedGen();
		const got = info?.labels?.gen ?? null;
		if (want !== ensured) {
			// A /destroy (maybe followed by a new start) finished while this
			// request waited for inspect() above.
			this.log("identity-mismatch", { why: "destroyed-in-flight", ensured: short(ensured), want: short(want), got: short(got) });
			throw new HttpError(
				409,
				`sandbox ${id} was destroyed by a /destroy that ran while this request was in flight (request was for gen ${short(ensured)}, stored gen is now ${short(want)}); nothing ran; send the request again to use a fresh container`,
			);
		}
		if (want === null || got !== want) {
			this.log("identity-mismatch", { why: "foreign-container", want: short(want), got: short(got) });
			throw new HttpError(
				409,
				`sandbox ${id} is attached to a container it did not start (container gen ${short(got)}, gen this id started ${short(want)}); nothing ran; run 'sbx destroy ${id}' and start again`,
			);
		}
	}

	async fetch(request: Request): Promise<Response> {
		const url = new URL(request.url);
		try {
			if (request.method === "GET" && url.pathname === "/status") {
				const running = this.container.running;
				const info = running ? await this.container.inspect() : null;
				const gen = info?.labels?.gen ?? null;
				const stored = this.storedGen();
				// Does the Registry count this id against the cap? (The id is
				// this Durable Object's name; the query is only a fallback.)
				const name = checkId(this.ctx.id.name ?? url.searchParams.get("id"));
				const registered = await this.env.Registry.getByName(REGISTRY_NAME).has(name);
				return json({
					running,
					gen: gen === null ? null : short(gen),
					storedGen: stored === null ? null : short(stored),
					registered,
				});
			}
			if (request.method !== "POST") {
				return json({ error: "not found" }, 404);
			}
			const { body } = await readJson(request);
			const id = checkId(body.id);
			this.log("req", { path: url.pathname, running: this.container.running });
			switch (url.pathname) {
				case "/exec":
					return await this.exec(id, body);
				case "/write":
					return await this.write(id, body);
				case "/destroy":
					return await this.destroy(id);
				default:
					return json({ error: "not found" }, 404);
			}
		} catch (err) {
			return errorResponse(err);
		}
	}

	private async exec(id: string, body: Record<string, unknown>): Promise<Response> {
		const cmd = body.cmd;
		if (typeof cmd !== "string" || cmd.length === 0 || cmd.includes("\0")) {
			throw new HttpError(400, "cmd must be a non-empty string");
		}
		let timeoutMs = DEFAULT_EXEC_TIMEOUT_MS;
		if (body.timeout_ms !== undefined) {
			const t = body.timeout_ms;
			if (typeof t !== "number" || !Number.isInteger(t) || t < 1 || t > MAX_EXEC_TIMEOUT_MS) {
				throw new HttpError(400, `timeout_ms must be an integer in [1, ${MAX_EXEC_TIMEOUT_MS}]`);
			}
			timeoutMs = t;
		}
		const cwd = body.cwd === undefined ? "/work" : checkAbsPath(body.cwd, "cwd");
		const env: Record<string, string> = { ...BASE_ENV };
		if (body.env !== undefined) {
			if (typeof body.env !== "object" || body.env === null || Array.isArray(body.env)) {
				throw new HttpError(400, "env must be an object of strings");
			}
			for (const [k, v] of Object.entries(body.env as Record<string, unknown>)) {
				if (typeof v !== "string" || k.length === 0 || k.includes("=") || k.includes("\0") || v.includes("\0")) {
					throw new HttpError(400, `env.${k} must be a string without '=' in the name or NUL`);
				}
				env[k] = v;
			}
		}

		// Nothing has been launched until container.exec() below: an error here
		// answers `ran: false`, so the client knows a retry cannot run it twice.
		try {
			const ensured = await this.ensureRunning(id);
			await this.checkIdentity(id, ensured);
		} catch (err) {
			throw notLaunched(err);
		}
		const started = Date.now();
		// GNU timeout puts the command in its own process group and signals the
		// whole group on expiry (exit 124), so children of `bash -c` stop too.
		const proc = await this.container.exec(
			["timeout", "-k", "10s", `${timeoutMs / 1000}s`, "bash", "-c", cmd],
			{ cwd, env },
		);
		const work = Promise.all([drain(proc.stdout, STDOUT_CAP), drain(proc.stderr, STDERR_CAP), proc.exitCode]);
		// Watchdog: `timeout` bounds the process, so if the output streams have
		// not finished well after the deadline, report it instead of hanging.
		let timer: number | null = null;
		const watchdog = new Promise<"stuck">((resolve) => {
			timer = setTimeout(() => resolve("stuck"), timeoutMs + EXEC_WATCHDOG_SLACK_MS);
		});
		const raced = await Promise.race([work, watchdog]);
		if (timer !== null) clearTimeout(timer);
		if (raced === "stuck") {
			work.catch(() => {});
			throw new HttpError(504, `exec output did not finish within timeout_ms + ${EXEC_WATCHDOG_SLACK_MS} ms (pid ${proc.pid})`);
		}
		const [out, err, exitCode] = raced;
		const durationMs = Date.now() - started;
		const dec = new TextDecoder();
		return json({
			exitCode,
			stdout: dec.decode(out.bytes),
			stderr: dec.decode(err.bytes),
			// 124: timeout sent SIGTERM; 137 after the deadline: SIGKILL after -k grace.
			timedOut: exitCode === 124 || (exitCode === 137 && durationMs >= timeoutMs),
			durationMs,
			stdoutBytes: out.total,
			stderrBytes: err.total,
			stdoutTruncated: out.truncated,
			stderrTruncated: err.truncated,
		});
	}

	private async write(id: string, body: Record<string, unknown>): Promise<Response> {
		const path = checkAbsPath(body.path, "path");
		const b64 = body.content_b64;
		if (typeof b64 !== "string" || b64.length % 4 !== 0 || !B64_RE.test(b64)) {
			throw new HttpError(400, "content_b64 must be standard padded base64");
		}
		if ((b64.length / 4) * 3 > MAX_WRITE_BYTES + 2) {
			throw new HttpError(413, `content larger than ${MAX_WRITE_BYTES} bytes`);
		}
		const bin = atob(b64);
		const bytes = new Uint8Array(bin.length);
		for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
		if (bytes.byteLength > MAX_WRITE_BYTES) {
			throw new HttpError(413, `content larger than ${MAX_WRITE_BYTES} bytes`);
		}
		const ensured = await this.ensureRunning(id);
		await this.checkIdentity(id, ensured);
		const parent = path.slice(0, path.lastIndexOf("/")) || "/";
		await this.files.mkdir(parent, { recursive: true });
		await this.files.writeFile(path, bytes);
		const sha256 = toHex(await crypto.subtle.digest("SHA-256", bytes));
		return json({ ok: true, path, bytes: bytes.byteLength, sha256 });
	}

	// Runs as a lifecycle op: no start can happen while it waits for
	// container.destroy(), so the container it stops is the one whose gen is
	// stored and whose Registry slot it frees.
	private destroy(id: string): Promise<Response> {
		return this.serial(async () => {
			const gen = this.storedGen();
			const wasRunning = this.container.running;
			if (wasRunning) {
				await this.container.destroy("destroyed via /destroy");
			}
			const runningAfter = this.container.running;
			if (runningAfter) {
				// The container still runs: keep its gen and its Registry slot,
				// so it stays this id's container and still counts against the cap.
				this.log("destroy-failed", { wasRunning, runningAfter, gen: short(gen) });
				return json(
					{
						ok: false,
						error: `destroy did not stop the container of sandbox ${id} (gen ${short(gen)}, runningAfter ${runningAfter}); it still runs and still counts against the cap; run 'sbx destroy ${id}' again`,
						wasRunning,
						runningAfter,
					},
					500,
				);
			}
			this.ctx.storage.kv.delete(GEN_KEY);
			await this.env.Registry.getByName(REGISTRY_NAME).release(id);
			this.log("destroy", { wasRunning, runningAfter, gen: short(gen) });
			return json({ ok: true, wasRunning, runningAfter });
		});
	}
}

// App-level cap on running sandboxes. Rows = ids that started a container.
// A row is removed on /destroy, and lazily when the cap is hit and the
// sandbox's container is no longer running (idle timeout, crash).
export class Registry extends DurableObject<Env> {
	constructor(ctx: DurableObjectState, env: Env) {
		super(ctx, env);
		ctx.storage.sql.exec("CREATE TABLE IF NOT EXISTS live (id TEXT PRIMARY KEY, since INTEGER NOT NULL)");
	}

	private ids(): string[] {
		return this.ctx.storage.sql
			.exec<{ id: string }>("SELECT id FROM live ORDER BY since")
			.toArray()
			.map((r) => r.id);
	}

	async acquire(id: string): Promise<{ ok: boolean; running: string[] }> {
		if (this.ids().includes(id)) {
			return { ok: true, running: this.ids() };
		}
		if (this.ids().length >= MAX_RUNNING) {
			const others = this.ids();
			const states = await Promise.all(
				others.map(async (o) => [o, await this.env.Sandbox.getByName(o).isRunning()] as const),
			);
			for (const [o, running] of states) {
				if (!running) this.ctx.storage.sql.exec("DELETE FROM live WHERE id = ?", o);
			}
		}
		// No await between this check and the insert, so two concurrent
		// acquires cannot both take the last slot.
		const now = this.ids();
		if (now.includes(id)) {
			return { ok: true, running: now };
		}
		if (now.length >= MAX_RUNNING) {
			return { ok: false, running: now };
		}
		this.ctx.storage.sql.exec("INSERT INTO live (id, since) VALUES (?, ?)", id, Date.now());
		return { ok: true, running: this.ids() };
	}

	async release(id: string): Promise<void> {
		this.ctx.storage.sql.exec("DELETE FROM live WHERE id = ?", id);
	}

	// Read only (for /health?id=): does this id hold a slot?
	async has(id: string): Promise<boolean> {
		return this.ids().includes(id);
	}
}
