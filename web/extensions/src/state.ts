import { App } from "@modelcontextprotocol/ext-apps";
import { OpenAIExtensions } from "@openai/mcp-extensions/app";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { type FeedMessage } from "./feed";

export type Contact = {
  id: string;
  name?: string;
  phone?: string;
  type?: string;
  unreadCount: number;
  isPinned: boolean;
  lastMessageTime: number;
  lastMessagePreview?: string;
};

type Message = {
  id: string;
  contactId: string;
  timestamp: number;
  isFromMe: boolean;
  senderName?: string;
  text?: string;
  originalText?: string;
  translatedText?: string;
  contentType: string;
  deliveryStatus?: string;
  replyContext?: { text?: string; senderName?: string };
};

type Cursor = { beforeTimestamp: number; beforeMessageId: string };

export type Payload = {
  contacts?: Contact[];
  nextCursor?: number | Cursor | null;
  totalMatched?: number;
  contact?: Contact;
  messages?: Message[];
  hasMore?: boolean;
  permissions?: { read: boolean; send: boolean };
  preparationToken?: string;
  expiresAt?: number;
  recipient?: { id: string; name?: string; phone?: string };
  originalText?: string;
  finalText?: string;
  translated?: boolean;
  targetLanguage?: string;
  replyToMessageId?: string;
};

export const app = new App(
  { name: "whatsapp-translator-chats", version: "1.0.0" },
  { availableDisplayModes: ["inline", "fullscreen"] },
);

export const extensions = new OpenAIExtensions(app);

export const $ = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw Error(`Missing control: ${id}`);
  return element as T;
};

export const state = {
  view: "chats" as "chats" | "messages",
  viewGeneration: 0,
  feedGeneration: 0,
  feedMessages: [] as FeedMessage[],
  feedCursor: null as Cursor | null,
  feedHasMore: false,
  feedFailed: false,
  loadingFeed: false,
  loadingOlderFeed: false,
  feedExpanded: new Set<string>(),
  contacts: [] as Contact[],
  selected: null as Contact | null,
  messages: [] as Message[],
  chatCursor: null as number | null,
  messageCursor: null as Cursor | null,
  query: "",
  filter: "all",
  listGeneration: 0,
  chatGeneration: 0,
  actionGeneration: 0,
  linkGeneration: 0,
  lastDeepLink: "",
  draftRevision: 0,
  send: false,
  loadingList: false,
  loadingChat: false,
  busy: false,
  sending: false,
  reply: null as Message | null,
  prepared: null as Payload | null,
  idempotencyKey: "",
  drafts: new Map<string, string>(),
  retry: null as (() => Promise<void>) | null,
};

export function node<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text = "",
) {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = text;
  return element;
}

export function error(reason: unknown, retry?: () => Promise<void>) {
  $("error-text").textContent =
    reason instanceof Error ? reason.message : String(reason);
  $("error").hidden = false;
  state.retry = retry ?? null;
  $("retry").hidden = !retry;
}

export function clearError() {
  $("error").hidden = true;
  state.retry = null;
}

export function payload(
  result: Pick<CallToolResult, "isError" | "structuredContent" | "content">,
): Payload {
  if (result.isError)
    throw Error(
      result.content
        .filter((item) => item.type === "text")
        .map((item) => item.text)
        .join("\n") || "The request failed.",
    );
  if (!result.structuredContent)
    throw Error("The server did not return conversation data.");
  return result.structuredContent as Payload;
}

export async function call<T = Payload>(
  name: string,
  args: Record<string, unknown> = {},
) {
  return payload(
    await app.callServerTool({ name, arguments: args }, { timeout: 60000 }),
  ) as T;
}

export const displayName = (contact: Contact) =>
  contact.name || contact.phone || contact.id;

export const initials = (contact: Contact) =>
  contact.type === "group"
    ? "◎"
    : displayName(contact)
        .split(/\s+/)
        .slice(0, 2)
        .map((word) => Array.from(word)[0])
        .join("")
        .toUpperCase();

export const time = (value: number) =>
  new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(value));

export const dateLabel = (value: number) =>
  new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(value));

export function act(action: () => Promise<void>) {
  return () => void action().catch((reason) => error(reason));
}
