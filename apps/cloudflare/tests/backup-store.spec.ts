import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CURRENT_POINTER,
  KEEP_PREVIOUS,
  backupKey,
  decodeToken,
  encodeToken,
  handleBackupRequest,
} from "../src/backup-store.ts";
import { FakeBucket } from "./fake-bucket.ts";

const call = (
  bucket: FakeBucket,
  method: string,
  path: string,
  body?: BodyInit,
  now = new Date("2026-11-19T18:00:00.000Z"),
) =>
  handleBackupRequest(
    new Request(`http://192.0.2.1${path}`, {
      method,
      body,
      duplex: "half",
    } as RequestInit),
    bucket,
    () => now,
  );

async function upload(bucket: FakeBucket, chunks: string[], now: Date) {
  const token = await (
    await call(bucket, "POST", "/backup/uploads", undefined, now)
  ).text();
  const parts = [];
  for (const [index, chunk] of chunks.entries()) {
    const response = await call(
      bucket,
      "PUT",
      `/backup/uploads/${token}/parts/${index + 1}`,
      chunk,
    );
    assert.equal(response.status, 200);
    parts.push(await response.json());
  }
  const done = await call(
    bucket,
    "POST",
    `/backup/uploads/${token}/complete`,
    JSON.stringify(parts),
  );
  assert.equal(done.status, 204);
}

test("answers 404 before the first backup", async () => {
  const response = await call(new FakeBucket(), "GET", "/backup");
  assert.equal(response.status, 404);
});

test("serves the archive a multipart upload completed", async () => {
  const bucket = new FakeBucket();
  await upload(
    bucket,
    ["first ", "second ", "third"],
    new Date("2026-11-19T18:00:00.000Z"),
  );
  const response = await call(bucket, "GET", "/backup");
  assert.equal(response.status, 200);
  assert.equal(await response.text(), "first second third");
  assert.equal(response.headers.get("content-length"), "18");
});

test("keeps the last backup until a new upload completes", async () => {
  const bucket = new FakeBucket();
  await upload(bucket, ["good"], new Date("2026-11-19T18:00:00.000Z"));
  const token = await (await call(bucket, "POST", "/backup/uploads")).text();
  await call(bucket, "PUT", `/backup/uploads/${token}/parts/1`, "partial");
  assert.equal(
    (await call(bucket, "DELETE", `/backup/uploads/${token}`)).status,
    204,
  );
  assert.equal(await (await call(bucket, "GET", "/backup")).text(), "good");
  assert.equal(bucket.uploads.size, 0);
});

test("never answers 404 when the pointer is broken, so the container cannot start empty", async () => {
  const bucket = new FakeBucket();
  await bucket.put(
    CURRENT_POINTER,
    backupKey(new Date("2026-11-19T18:00:00.000Z")),
  );
  assert.equal((await call(bucket, "GET", "/backup")).status, 500);
  await bucket.put(CURRENT_POINTER, "../../etc/passwd");
  assert.equal((await call(bucket, "GET", "/backup")).status, 500);
});

test("prunes all but the newest archives", async () => {
  const bucket = new FakeBucket();
  const total = KEEP_PREVIOUS + 3;
  for (let i = 0; i < total; i++) {
    await upload(
      bucket,
      [`backup ${i}`],
      new Date(Date.UTC(2026, 10, 19, 18, i)),
    );
  }
  const archives = [...bucket.objects.keys()].filter(
    (key) => key !== CURRENT_POINTER,
  );
  assert.equal(archives.length, KEEP_PREVIOUS + 1);
  assert.equal(
    await (await call(bucket, "GET", "/backup")).text(),
    `backup ${total - 1}`,
  );
});

test("rejects tokens outside the backup prefix and malformed part lists", async () => {
  const bucket = new FakeBucket();
  const forged = encodeToken({ key: "wa-data/current", uploadId: "x" });
  assert.equal(decodeToken(forged), null);
  assert.equal(
    (await call(bucket, "PUT", `/backup/uploads/${forged}/parts/1`, "x"))
      .status,
    400,
  );
  assert.equal(
    (await call(bucket, "PUT", "/backup/uploads/not-base64!/parts/1", "x"))
      .status,
    400,
  );

  const token = await (await call(bucket, "POST", "/backup/uploads")).text();
  assert.equal(
    (await call(bucket, "PUT", `/backup/uploads/${token}/parts/0`, "x")).status,
    400,
  );
  for (const body of [
    "[]",
    "{}",
    "not json",
    '[{"partNumber":2,"etag":"etag-2"}]',
  ]) {
    const response = await call(
      bucket,
      "POST",
      `/backup/uploads/${token}/complete`,
      body,
    );
    assert.equal(response.status, 400, body);
  }
  assert.equal(bucket.objects.has(CURRENT_POINTER), false);
});

test("round-trips upload tokens with characters that are not URL-safe", () => {
  const token = { key: backupKey(new Date(0)), uploadId: "a+b/c==" };
  const encoded = encodeToken(token);
  assert.match(encoded, /^[A-Za-z0-9_-]+$/);
  assert.deepEqual(decodeToken(encoded), token);
});

test("answers unknown paths and methods with errors", async () => {
  const bucket = new FakeBucket();
  assert.equal((await call(bucket, "GET", "/other")).status, 404);
  assert.equal((await call(bucket, "POST", "/backup")).status, 405);
  assert.equal((await call(bucket, "GET", "/backup/uploads")).status, 405);
});
