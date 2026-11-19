// Keeps the server's data directory (WA_DATA_DIR) in R2 so it survives
// container restarts and redeploys. The container reaches this code over plain
// HTTP through an outbound intercept (see index.ts); nothing outside the
// container can call it.
//
// Each backup is a gzipped tar archive uploaded as an R2 multipart upload, so
// no single request body comes near the 100 MiB limit on container outbound
// requests. A completed upload is written to its own key, then the `current`
// pointer is moved to it, so a failed or partial upload never replaces the
// last good backup. Older archives are pruned after the pointer moves.
//
//   GET    /backup                              the current archive, or 404 if none
//   POST   /backup/uploads                      start an upload, returns a token
//   PUT    /backup/uploads/<token>/parts/<n>    upload part n (1-based)
//   POST   /backup/uploads/<token>/complete     finish with a JSON list of parts
//   DELETE /backup/uploads/<token>              abandon an upload

export const BACKUP_PREFIX = "wa-data/backups/";
export const CURRENT_POINTER = "wa-data/current";
// Archives kept besides the current one, so a bad backup can be rolled back by
// hand by pointing `current` at an older key.
export const KEEP_PREVIOUS = 5;
// Also kept: the newest archive at least this much older than the current one,
// so a run of bad backups (for example after a bad release) cannot push out
// every copy from before it.
export const KEEP_ONE_OLDER_THAN_MS = 24 * 60 * 60 * 1000;

const KEY_PATTERN = /^wa-data\/backups\/[0-9TZ-]+\.tar\.gz$/;
const MAX_PARTS = 10_000;

export interface UploadedPart {
  partNumber: number;
  etag: string;
}

interface StoredObject {
  body: ReadableStream;
  size: number;
  text(): Promise<string>;
}

interface MultipartUpload {
  uploadPart(
    partNumber: number,
    value: ReadableStream | ArrayBuffer,
  ): Promise<UploadedPart>;
  complete(parts: UploadedPart[]): Promise<unknown>;
  abort(): Promise<void>;
}

// The subset of R2Bucket this module uses, so tests can pass a fake.
export interface BackupBucket {
  get(key: string): Promise<StoredObject | null>;
  put(key: string, value: string): Promise<unknown>;
  delete(keys: string | string[]): Promise<void>;
  list(options: { prefix: string; cursor?: string }): Promise<{
    objects: { key: string }[];
    truncated: boolean;
    cursor?: string;
  }>;
  createMultipartUpload(key: string): Promise<{ uploadId: string }>;
  resumeMultipartUpload(key: string, uploadId: string): MultipartUpload;
}

interface UploadToken {
  key: string;
  uploadId: string;
}

const text = (body: string, status: number) =>
  new Response(body, {
    status,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });

export function backupKey(now: Date): string {
  // 2026-11-19T18:21:41.123Z -> 2026-11-19T18-21-41-123Z, which sorts by time.
  return `${BACKUP_PREFIX}${now.toISOString().replace(/[:.]/g, "-")}.tar.gz`;
}

// The time a backupKey() was made, or NaN for any other key.
export function backupKeyTime(key: string): number {
  const match =
    /^wa-data\/backups\/(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})-(\d{3})Z\.tar\.gz$/.exec(
      key,
    );
  if (!match) return Number.NaN;
  const [, date, hours, minutes, seconds, millis] = match;
  return Date.parse(`${date}T${hours}:${minutes}:${seconds}.${millis}Z`);
}

export function encodeToken(token: UploadToken): string {
  const bytes = new TextEncoder().encode(JSON.stringify(token));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

export function decodeToken(value: string): UploadToken | null {
  try {
    const base64 = value.replace(/-/g, "+").replace(/_/g, "/");
    const binary = atob(base64);
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
    const parsed: unknown = JSON.parse(new TextDecoder().decode(bytes));
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      typeof (parsed as UploadToken).key === "string" &&
      typeof (parsed as UploadToken).uploadId === "string" &&
      KEY_PATTERN.test((parsed as UploadToken).key) &&
      (parsed as UploadToken).uploadId.length > 0
    ) {
      return {
        key: (parsed as UploadToken).key,
        uploadId: (parsed as UploadToken).uploadId,
      };
    }
  } catch {
    // Fall through to null.
  }
  return null;
}

function parseParts(value: unknown): UploadedPart[] | null {
  if (!Array.isArray(value) || value.length === 0 || value.length > MAX_PARTS)
    return null;
  const parts: UploadedPart[] = [];
  for (const [index, item] of value.entries()) {
    if (
      typeof item !== "object" ||
      item === null ||
      item.partNumber !== index + 1 ||
      typeof item.etag !== "string" ||
      item.etag.length === 0
    ) {
      return null;
    }
    parts.push({ partNumber: item.partNumber, etag: item.etag });
  }
  return parts;
}

async function readCurrent(bucket: BackupBucket): Promise<Response> {
  const pointer = await bucket.get(CURRENT_POINTER);
  if (!pointer) return text("no backup yet", 404);
  const key = (await pointer.text()).trim();
  if (!KEY_PATTERN.test(key)) {
    // Never answer 404 here: the container would start empty and its next
    // backup would replace the real one.
    return text("backup pointer is invalid", 500);
  }
  const archive = await bucket.get(key);
  if (!archive) return text("backup pointer names a missing archive", 500);
  return new Response(archive.body, {
    headers: {
      "content-type": "application/gzip",
      "content-length": String(archive.size),
    },
  });
}

async function prune(bucket: BackupBucket, current: string): Promise<void> {
  const keys: string[] = [];
  let cursor: string | undefined;
  do {
    const page = await bucket.list({ prefix: BACKUP_PREFIX, cursor });
    keys.push(...page.objects.map((object) => object.key));
    cursor = page.truncated ? page.cursor : undefined;
  } while (cursor);
  const older = keys.filter((key) => key < current).sort();
  const keep = new Set(older.slice(-KEEP_PREVIOUS));
  const cutoff = backupKeyTime(current) - KEEP_ONE_OLDER_THAN_MS;
  const dayOld = older.filter((key) => backupKeyTime(key) <= cutoff).at(-1);
  if (dayOld) keep.add(dayOld);
  const stale = older.filter((key) => !keep.has(key));
  if (stale.length > 0) await bucket.delete(stale);
}

export async function handleBackupRequest(
  request: Request,
  bucket: BackupBucket,
  now: () => Date = () => new Date(),
): Promise<Response> {
  const segments = new URL(request.url).pathname.split("/").filter(Boolean);
  if (segments[0] !== "backup") return text("not found", 404);

  if (segments.length === 1) {
    if (request.method !== "GET") return text("method not allowed", 405);
    return readCurrent(bucket);
  }

  if (segments[1] !== "uploads") return text("not found", 404);

  if (segments.length === 2) {
    if (request.method !== "POST") return text("method not allowed", 405);
    const key = backupKey(now());
    const { uploadId } = await bucket.createMultipartUpload(key);
    return text(encodeToken({ key, uploadId }), 201);
  }

  const token = decodeToken(segments[2]);
  if (!token) return text("invalid upload token", 400);
  const upload = bucket.resumeMultipartUpload(token.key, token.uploadId);

  if (segments.length === 3) {
    if (request.method !== "DELETE") return text("method not allowed", 405);
    await upload.abort();
    return new Response(null, { status: 204 });
  }

  if (segments.length === 5 && segments[3] === "parts") {
    if (request.method !== "PUT") return text("method not allowed", 405);
    const partNumber = Number(segments[4]);
    if (
      !Number.isInteger(partNumber) ||
      partNumber < 1 ||
      partNumber > MAX_PARTS
    ) {
      return text("invalid part number", 400);
    }
    if (!request.body) return text("missing part body", 400);
    const part = await upload.uploadPart(partNumber, request.body);
    // One JSON object per line, so the container script can join them into
    // the completion list without a JSON parser.
    return Response.json({ partNumber: part.partNumber, etag: part.etag });
  }

  if (segments.length === 4 && segments[3] === "complete") {
    if (request.method !== "POST") return text("method not allowed", 405);
    let parts: UploadedPart[] | null;
    try {
      parts = parseParts(await request.json());
    } catch {
      parts = null;
    }
    if (!parts) return text("invalid part list", 400);
    await upload.complete(parts);
    await bucket.put(CURRENT_POINTER, token.key);
    await prune(bucket, token.key);
    return new Response(null, { status: 204 });
  }

  return text("not found", 404);
}
