// An in-memory stand-in for the parts of R2Bucket that backup-store.ts uses.
import type { BackupBucket, UploadedPart } from "../src/backup-store.ts";

async function bytesOf(
  value: ReadableStream | ArrayBuffer,
): Promise<Uint8Array> {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  return new Uint8Array(await new Response(value).arrayBuffer());
}

export class FakeBucket implements BackupBucket {
  objects = new Map<string, Uint8Array>();
  uploads = new Map<string, { key: string; parts: Map<number, Uint8Array> }>();
  private nextUpload = 0;

  async get(key: string) {
    const bytes = this.objects.get(key);
    if (!bytes) return null;
    return {
      body: new Response(bytes).body!,
      size: bytes.byteLength,
      text: async () => new TextDecoder().decode(bytes),
    };
  }

  async put(key: string, value: string) {
    this.objects.set(key, new TextEncoder().encode(value));
  }

  async delete(keys: string | string[]) {
    for (const key of Array.isArray(keys) ? keys : [keys])
      this.objects.delete(key);
  }

  async list({ prefix }: { prefix: string; cursor?: string }) {
    const objects = [...this.objects.keys()]
      .filter((key) => key.startsWith(prefix))
      .sort()
      .map((key) => ({ key }));
    return { objects, truncated: false };
  }

  async createMultipartUpload(key: string) {
    // Contains characters that are not URL-safe, like real upload IDs can.
    const uploadId = `upload+${this.nextUpload++}/id=`;
    this.uploads.set(uploadId, { key, parts: new Map() });
    return { uploadId };
  }

  resumeMultipartUpload(key: string, uploadId: string) {
    const upload = () => {
      const found = this.uploads.get(uploadId);
      if (!found || found.key !== key) throw new Error("no such upload");
      return found;
    };
    return {
      uploadPart: async (
        partNumber: number,
        value: ReadableStream | ArrayBuffer,
      ) => {
        upload().parts.set(partNumber, await bytesOf(value));
        return { partNumber, etag: `etag-${partNumber}` };
      },
      complete: async (parts: UploadedPart[]) => {
        const found = upload();
        const chunks = parts.map((part) => {
          const bytes = found.parts.get(part.partNumber);
          if (!bytes || part.etag !== `etag-${part.partNumber}`) {
            throw new Error(`part ${part.partNumber} does not match`);
          }
          return bytes;
        });
        const total = chunks.reduce((sum, chunk) => sum + chunk.byteLength, 0);
        const joined = new Uint8Array(total);
        let offset = 0;
        for (const chunk of chunks) {
          joined.set(chunk, offset);
          offset += chunk.byteLength;
        }
        this.objects.set(key, joined);
        this.uploads.delete(uploadId);
      },
      abort: async () => {
        upload();
        this.uploads.delete(uploadId);
      },
    };
  }
}
