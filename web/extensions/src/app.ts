import {
  App,
  applyDocumentTheme,
  applyHostStyleVariables,
} from "@modelcontextprotocol/ext-apps";
import { OpenAIExtensions } from "@openai/mcp-extensions/app";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { renderFeed, feedKey, type FeedMessage, type FeedPage } from "./feed";

type Contact = {
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
type Payload = {
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
const app = new App(
  { name: "whatsapp-translator-chats", version: "1.0.0" },
  { availableDisplayModes: ["inline", "fullscreen"] },
);
const extensions = new OpenAIExtensions(app);
const $ = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw Error(`Missing control: ${id}`);
  return element as T;
};
const state = {
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
function node<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text = "",
) {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = text;
  return element;
}
function error(reason: unknown, retry?: () => Promise<void>) {
  $("error-text").textContent =
    reason instanceof Error ? reason.message : String(reason);
  $("error").hidden = false;
  state.retry = retry ?? null;
  $("retry").hidden = !retry;
}
function clearError() {
  $("error").hidden = true;
  state.retry = null;
}
function payload(
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
async function call<T = Payload>(
  name: string,
  args: Record<string, unknown> = {},
) {
  return payload(
    await app.callServerTool({ name, arguments: args }, { timeout: 60000 }),
  ) as T;
}
const displayName = (contact: Contact) =>
  contact.name || contact.phone || contact.id;
const initials = (contact: Contact) =>
  contact.type === "group"
    ? "◎"
    : displayName(contact)
        .split(/\s+/)
        .slice(0, 2)
        .map((word) => Array.from(word)[0])
        .join("")
        .toUpperCase();
const time = (value: number) =>
  new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(value));
const dateLabel = (value: number) =>
  new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(value));
function act(action: () => Promise<void>) {
  return () => void action().catch((reason) => error(reason));
}
function drawContacts() {
  const list = $("chat-list");
  list.replaceChildren();
  if (!state.contacts.length)
    list.append(
      node(
        "p",
        "empty",
        state.loadingList
          ? "Finding your conversations…"
          : state.query
            ? "No conversations match your search."
            : "No conversations here yet.",
      ),
    );
  for (const contact of state.contacts) {
    const row = node(
      "button",
      `chat-row${state.selected?.id === contact.id ? " selected" : ""}`,
    );
    row.setAttribute("aria-label", `Open chat with ${displayName(contact)}`);
    row.setAttribute("aria-pressed", String(state.selected?.id === contact.id));
    row.append(node("span", "avatar", initials(contact)));
    const content = node("span", "row-content");
    const top = node("span", "row-top");
    const bottom = node("span", "row-bottom");
    top.append(
      node("span", "row-name", displayName(contact)),
      node(
        "span",
        "row-time",
        contact.lastMessageTime ? time(contact.lastMessageTime) : "",
      ),
    );
    bottom.append(
      node("span", "preview", contact.lastMessagePreview || "No messages yet"),
    );
    if (contact.isPinned) bottom.append(node("span", "pin", "◆"));
    if (contact.unreadCount > 0)
      bottom.append(node("span", "badge", String(contact.unreadCount)));
    content.append(top, bottom);
    row.append(content);
    row.onclick = act(() => selectChat(contact));
    list.append(row);
  }
  $("more-chats").hidden = state.chatCursor === null;
  ($("more-chats") as HTMLButtonElement).disabled = state.loadingList;
}
async function loadContacts(append = false) {
  if (state.view !== "chats") return;
  if (append && (state.loadingList || state.chatCursor === null)) return;
  const generation = ++state.listGeneration;
  state.loadingList = true;
  if (!append) {
    state.contacts = [];
    state.chatCursor = null;
  }
  drawContacts();
  try {
    const result = await call("list_contacts", {
      query: state.query,
      contact_type: state.filter === "group" ? "group" : "all",
      unread_only: state.filter === "unread",
      limit: 50,
      ...(append ? { cursor: state.chatCursor } : {}),
    });
    if (generation !== state.listGeneration) return;
    state.contacts = append
      ? [
          ...new Map(
            [...state.contacts, ...(result.contacts ?? [])].map((contact) => [
              contact.id,
              contact,
            ]),
          ).values(),
        ]
      : (result.contacts ?? []);
    state.chatCursor =
      typeof result.nextCursor === "number" ? result.nextCursor : null;
    $("count").textContent = String(
      result.totalMatched ?? state.contacts.length,
    );
  } catch (reason) {
    if (generation === state.listGeneration)
      error(reason, () => loadContacts(append));
  } finally {
    if (generation === state.listGeneration) {
      state.loadingList = false;
      drawContacts();
    }
  }
}
function controls() {
  const selected = state.view === "chats" && !!state.selected;
  $("composer").hidden = !selected || !state.send;
  $("readonly").hidden = !selected || state.send;
  $("read").hidden = !selected || !state.send;
  ($("read") as HTMLButtonElement).disabled = state.busy || state.sending;
  ($("prepare") as HTMLButtonElement).disabled =
    !selected ||
    !state.send ||
    state.busy ||
    state.sending ||
    !$<HTMLTextAreaElement>("draft").value.trim();
  $("reply").hidden = !selected || !state.reply;
  $("feed-note").hidden = state.view !== "messages";
  $("reply-text").textContent =
    state.reply?.text || state.reply?.originalText || "Message";
}
function drawMessages(scroll = false) {
  if (state.view !== "chats") return;
  const container = $("messages");
  const oldHeight = container.scrollHeight;
  const oldTop = container.scrollTop;
  container.replaceChildren();
  if (state.loadingChat && !state.messages.length)
    container.append(node("p", "empty", "Loading conversation…"));
  else if (!state.messages.length)
    container.append(
      node("p", "empty", "No messages in this conversation yet."),
    );
  if (state.messageCursor) {
    const more = node("button", "load-more", "Load older messages");
    more.disabled = state.loadingChat;
    more.onclick = act(() => loadMessages(true));
    container.prepend(more);
  }
  let day = "";
  for (const message of state.messages) {
    const label = dateLabel(message.timestamp);
    if (label !== day) {
      const divider = node("div", "date-divider");
      divider.append(node("span", "", label));
      container.append(divider);
      day = label;
    }
    const row = node(
      "div",
      `message-row${message.isFromMe ? " outgoing" : ""}`,
    );
    const bubble = node("article", "bubble");
    if (
      !message.isFromMe &&
      message.senderName &&
      state.selected?.type === "group"
    )
      bubble.append(node("span", "sender", message.senderName));
    if (message.replyContext)
      bubble.append(
        node(
          "div",
          "quoted",
          message.replyContext.text || "Reply to a message",
        ),
      );
    const translated =
      !!message.translatedText &&
      message.translatedText !== message.originalText;
    if (translated)
      bubble.append(node("span", "translation-label", "✦ Translated"));
    bubble.append(
      node(
        "p",
        "",
        message.text ||
          message.originalText ||
          `[${message.contentType} message]`,
      ),
    );
    if (translated) {
      const original = node("details");
      original.append(
        node(
          "summary",
          "",
          message.isFromMe ? "Sent translation" : "Show original",
        ),
        node(
          "p",
          "",
          message.isFromMe ? message.translatedText : message.originalText,
        ),
      );
      bubble.append(original);
    }
    const meta = node("div", "message-meta");
    meta.append(node("span", "", time(message.timestamp)));
    if (message.isFromMe) {
      const delivery = node(
        "span",
        "delivery",
        ["read", "delivered"].includes(message.deliveryStatus ?? "")
          ? "✓✓"
          : "✓",
      );
      delivery.title = message.deliveryStatus || "Sent";
      meta.append(delivery);
    }
    bubble.append(meta);
    if (state.send) {
      const actions = node("div", "message-actions");
      const reply = node("button", "", "Reply");
      reply.disabled = state.sending;
      reply.onclick = () => {
        state.reply = message;
        invalidatePreparation();
        controls();
        $("draft").focus();
      };
      actions.append(reply);
      const react = node("button", "", "♡");
      react.setAttribute("aria-label", "React with heart");
      react.disabled = state.busy || state.sending;
      react.onclick = act(() =>
        messageAction("react_to_message", {
          message_id: message.id,
          emoji: "❤️",
          idempotency_key: crypto.randomUUID(),
        }),
      );
      actions.append(react);
      if (!message.isFromMe && !translated && message.originalText) {
        const translate = node("button", "", "Translate");
        translate.disabled = state.busy || state.sending;
        translate.onclick = act(() =>
          messageAction("translate_message", { message_id: message.id }),
        );
        actions.append(translate);
      }
      bubble.append(actions);
    }
    row.append(bubble);
    container.append(row);
  }
  container.scrollTop = scroll
    ? container.scrollHeight
    : oldTop + container.scrollHeight - oldHeight;
}
function invalidatePreparation() {
  if (state.sending) return;
  state.prepared = null;
  state.idempotencyKey = "";
  $<HTMLDialogElement>("confirmation").close();
}
function rememberDraft() {
  if (state.selected)
    state.drafts.set(state.selected.id, $<HTMLTextAreaElement>("draft").value);
}
function changeView(view: "chats" | "messages") {
  ++state.viewGeneration;
  ++state.chatGeneration;
  ++state.actionGeneration;
  ++state.listGeneration;
  ++state.feedGeneration;
  ++state.linkGeneration;
  ++state.draftRevision;
  clearTimeout(searchTimer);
  state.loadingChat = false;
  state.loadingList = false;
  state.loadingFeed = false;
  state.loadingOlderFeed = false;
  state.feedFailed = false;
  state.busy = false;
  state.reply = null;
  invalidatePreparation();
  clearError();
  state.view = view;
  $("workspace").classList.toggle("feed-open", view === "messages");
  $("chat-navigation").hidden = view !== "chats";
  $("feed-description").hidden = view !== "messages";
  $("show-messages").hidden = view === "messages";
  $("refresh").hidden = view === "messages";
  $("refresh-feed").hidden = view !== "messages";
  $("messages").setAttribute(
    "aria-label",
    view === "messages" ? "Unified messages" : "Chat messages",
  );
  for (const button of document.querySelectorAll<HTMLButtonElement>(
    "[data-view]",
  )) {
    const active = button.dataset.view === view;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  }
  controls();
}
function chatHeader() {
  const contact = state.selected;
  $("chat-name").textContent = contact
    ? displayName(contact)
    : "Your conversations, understood.";
  $("chat-avatar").textContent = contact ? initials(contact) : "▤";
  $("chat-subtitle").textContent = contact
    ? contact.type === "group"
      ? "Group conversation"
      : "WhatsApp conversation"
    : "Choose a chat to get started";
}
function showChats() {
  if (state.sending) return;
  rememberDraft();
  changeView("chats");
  $("workspace").classList.remove("chat-open");
  chatHeader();
  if (state.selected) drawMessages();
  else
    $("messages").replaceChildren(
      node("p", "empty", "Choose a chat to get started."),
    );
  drawContacts();
  if (!state.contacts.length) void loadContacts();
}
async function showFeed() {
  if (state.sending) {
    error("Wait for the current send to finish before switching views.");
    return;
  }
  if (state.view === "messages") return;
  rememberDraft();
  changeView("messages");
  $("chat-name").textContent = "Messages";
  $("chat-avatar").textContent = "▤";
  $("chat-subtitle").textContent = "Recent messages from your conversations";
  drawFeed();
  await loadFeed();
}
function drawFeed(latest = false) {
  if (state.view !== "messages") return;
  $("messages").setAttribute("aria-busy", String(state.loadingFeed));
  $<HTMLButtonElement>("refresh-feed").disabled =
    state.loadingFeed && !state.loadingOlderFeed;
  renderFeed(
    $("messages"),
    {
      messages: state.feedMessages,
      hasMore: state.feedHasMore,
      loading: state.loadingFeed,
      failed: state.feedFailed,
      expanded: state.feedExpanded,
    },
    {
      older: () => void loadFeed(true),
      openChat: (message) => void openFeedChat(message),
    },
    latest,
  );
}
async function loadFeed(older = false) {
  if (
    state.view !== "messages" ||
    (older && (state.loadingFeed || !state.feedCursor)) ||
    (!older && state.loadingFeed && !state.loadingOlderFeed)
  )
    return;
  const view = state.viewGeneration;
  const generation = ++state.feedGeneration;
  ++state.linkGeneration;
  state.loadingFeed = true;
  state.loadingOlderFeed = older;
  state.feedFailed = false;
  clearError();
  drawFeed();
  try {
    const cursor = older ? state.feedCursor : null;
    const result = await call<FeedPage>("read_unified_messages", {
      limit: 50,
      ...(cursor
        ? {
            before_timestamp: cursor.beforeTimestamp,
            before_message_id: cursor.beforeMessageId,
          }
        : {}),
    });
    if (
      view !== state.viewGeneration ||
      generation !== state.feedGeneration ||
      state.view !== "messages"
    )
      return;
    state.feedMessages = older
      ? [
          ...new Map(
            [...result.messages, ...state.feedMessages].map((message) => [
              feedKey(message),
              message,
            ]),
          ).values(),
        ]
      : result.messages;
    state.feedCursor = result.nextCursor;
    state.feedHasMore = result.hasMore;
    state.loadingFeed = false;
    drawFeed(!older);
  } catch (reason) {
    if (
      view === state.viewGeneration &&
      generation === state.feedGeneration &&
      state.view === "messages"
    ) {
      state.feedFailed = true;
      error(reason, async () => {
        if (
          view === state.viewGeneration &&
          generation === state.feedGeneration
        )
          await loadFeed(older);
      });
    }
  } finally {
    if (
      view === state.viewGeneration &&
      generation === state.feedGeneration &&
      state.view === "messages"
    ) {
      state.loadingFeed = false;
      drawFeed();
    }
  }
}
async function openFeedChat(message: FeedMessage) {
  if (state.view !== "messages" || state.sending) return;
  const view = state.viewGeneration;
  const generation = ++state.linkGeneration;
  clearError();
  try {
    const result = await call("open_chat", { contact_id: message.contactId });
    if (
      view !== state.viewGeneration ||
      generation !== state.linkGeneration ||
      state.view !== "messages"
    )
      return;
    if (!result.contact || result.contact.id !== message.contactId)
      throw Error("The source conversation could not be verified.");
    await selectChat(result.contact, result, false);
  } catch (reason) {
    if (
      view === state.viewGeneration &&
      generation === state.linkGeneration &&
      state.view === "messages"
    )
      error(reason, async () => {
        if (
          view === state.viewGeneration &&
          generation === state.linkGeneration
        )
          await openFeedChat(message);
      });
  }
}
async function selectChat(
  contact: Contact,
  initial?: Payload,
  shareSelection = true,
) {
  if (state.sending) {
    error("Wait for the current send to finish before switching chats.");
    return;
  }
  rememberDraft();
  changeView("chats");
  const view = state.viewGeneration;
  state.selected = contact;
  state.reply = null;
  state.messages = [];
  state.messageCursor = null;
  state.loadingChat = !initial;
  state.busy = false;
  invalidatePreparation();
  clearError();
  $("workspace").classList.add("chat-open");
  chatHeader();
  $<HTMLTextAreaElement>("draft").value = state.drafts.get(contact.id) || "";
  controls();
  drawContacts();
  if (initial) {
    state.messages = initial.messages ?? [];
    state.messageCursor =
      typeof initial.nextCursor === "object"
        ? (initial.nextCursor ?? null)
        : null;
    drawMessages(true);
  } else {
    drawMessages();
    await loadMessages();
  }
  if (
    !shareSelection ||
    view !== state.viewGeneration ||
    state.view !== "chats" ||
    state.selected?.id !== contact.id
  )
    return;
  // Share only the selected identifier; message contents stay out of model context until requested.
  void app
    .updateModelContext({
      structuredContent: {
        selectedContactId: contact.id,
        selectedContactName: displayName(contact),
      },
    })
    .catch(() => {});
}
async function loadMessages(older = false) {
  if (state.view !== "chats") return;
  const contact = state.selected;
  if (!contact || (older && state.loadingChat)) return;
  const generation = ++state.chatGeneration;
  state.loadingChat = true;
  if (!older) state.messageCursor = null;
  try {
    const cursor = older ? state.messageCursor : null;
    const result = await call("read_messages", {
      contact_id: contact.id,
      limit: 50,
      ...(cursor
        ? {
            before_timestamp: cursor.beforeTimestamp,
            before_message_id: cursor.beforeMessageId,
          }
        : {}),
    });
    if (
      generation !== state.chatGeneration ||
      contact.id !== state.selected?.id
    )
      return;
    state.messages = older
      ? [
          ...new Map(
            [...(result.messages ?? []), ...state.messages].map((message) => [
              message.id,
              message,
            ]),
          ).values(),
        ]
      : (result.messages ?? []);
    state.messageCursor =
      typeof result.nextCursor === "object"
        ? (result.nextCursor ?? null)
        : null;
  } catch (reason) {
    if (generation === state.chatGeneration)
      error(reason, () => loadMessages(older));
  } finally {
    if (generation === state.chatGeneration) {
      state.loadingChat = false;
      drawMessages(!older);
    }
  }
}
async function messageAction(name: string, args: Record<string, unknown>) {
  const contact = state.selected;
  if (
    state.view !== "chats" ||
    !contact ||
    state.busy ||
    state.sending ||
    !state.send
  )
    return;
  const action = ++state.actionGeneration;
  state.busy = true;
  controls();
  drawMessages();
  try {
    await call(name, { contact_id: contact.id, ...args });
    if (action === state.actionGeneration && state.selected?.id === contact.id)
      await loadMessages();
  } catch (reason) {
    if (action === state.actionGeneration && state.selected?.id === contact.id)
      error(reason);
  } finally {
    if (
      action === state.actionGeneration &&
      state.selected?.id === contact.id
    ) {
      state.busy = false;
      controls();
      drawMessages();
    }
  }
}
async function prepare() {
  if (
    state.view !== "chats" ||
    !state.selected ||
    state.busy ||
    state.sending ||
    !state.send
  )
    return;
  const contact = state.selected;
  const text = $<HTMLTextAreaElement>("draft").value.trim();
  if (!text) return;
  const generation = state.chatGeneration;
  const revision = state.draftRevision;
  const action = ++state.actionGeneration;
  state.busy = true;
  clearError();
  controls();
  try {
    const result = await call("prepare_message", {
      contact_id: contact.id,
      text,
      translation_mode: "auto",
      ...(state.reply ? { reply_to_message_id: state.reply.id } : {}),
    });
    if (
      generation !== state.chatGeneration ||
      revision !== state.draftRevision ||
      contact.id !== state.selected?.id
    )
      return;
    if (
      !result.preparationToken ||
      !result.finalText ||
      result.recipient?.id !== contact.id
    )
      throw Error(
        "The prepared recipient could not be verified. Nothing was sent.",
      );
    state.prepared = result;
    state.idempotencyKey = crypto.randomUUID();
    $("confirm-recipient").textContent =
      `${result.recipient.name || result.recipient.phone || contact.id} · ${result.recipient.phone || contact.id}`;
    $("confirm-text").textContent = result.finalText;
    $("confirm-original-text").textContent = result.originalText ?? text;
    $("confirm-original").hidden = !result.translated;
    $("confirm-language").textContent = result.translated
      ? `Translated to ${result.targetLanguage || "the conversation language"}`
      : "This message will be sent as written.";
    $("send-error").hidden = true;
    $<HTMLButtonElement>("confirm-send").disabled = false;
    $<HTMLDialogElement>("confirmation").showModal();
  } catch (reason) {
    if (generation === state.chatGeneration) error(reason);
  } finally {
    if (
      action === state.actionGeneration &&
      contact.id === state.selected?.id
    ) {
      state.busy = false;
      controls();
    }
  }
}
async function send() {
  const prepared = state.prepared;
  if (state.view !== "chats" || !prepared || state.sending || !state.send)
    return;
  if ((prepared.expiresAt ?? 0) <= Date.now()) {
    $("send-error").textContent =
      "This preview expired. Cancel and prepare the message again.";
    $("send-error").hidden = false;
    return;
  }
  if (prepared.recipient?.id !== state.selected?.id) {
    invalidatePreparation();
    error("The selected conversation changed. Please review a new message.");
    return;
  }
  state.sending = true;
  $<HTMLButtonElement>("confirm-send").disabled = true;
  $<HTMLButtonElement>("cancel-send").disabled = true;
  $<HTMLTextAreaElement>("draft").disabled = true;
  controls();
  try {
    await call(
      prepared.replyToMessageId ? "reply_to_message" : "send_message",
      {
        preparation_token: prepared.preparationToken,
        idempotency_key: state.idempotencyKey,
      },
    );
    $<HTMLTextAreaElement>("draft").value = "";
    rememberDraft();
    state.reply = null;
    state.prepared = null;
    state.idempotencyKey = "";
    $<HTMLDialogElement>("confirmation").close();
    await loadMessages();
  } catch (reason) {
    $("send-error").textContent =
      `${reason instanceof Error ? reason.message : String(reason)} Retry uses the same send key to prevent duplicates.`;
    $("send-error").hidden = false;
  } finally {
    state.sending = false;
    $<HTMLButtonElement>("confirm-send").disabled = false;
    $<HTMLButtonElement>("cancel-send").disabled = false;
    $<HTMLTextAreaElement>("draft").disabled = false;
    controls();
  }
}
function hostContext() {
  const context = app.getHostContext();
  if (context?.theme) applyDocumentTheme(context.theme);
  if (context?.styles?.variables)
    applyHostStyleVariables(context.styles.variables);
  $("expand").hidden =
    !context?.availableDisplayModes?.includes("fullscreen") ||
    context.displayMode === "fullscreen";
  const link = extensions.deepLink.getCurrent()?.url;
  if (link && link !== state.lastDeepLink) {
    state.lastDeepLink = link;
    const linkGeneration = ++state.linkGeneration;
    try {
      const url = new URL(link, "https://extension.invalid");
      if (url.origin !== "https://extension.invalid") return;
      const match = /^\/chats\/([^/]+)$/.exec(url.pathname);
      if (match) {
        const id = decodeURIComponent(match[1]);
        if (state.view !== "chats" || id !== state.selected?.id) {
          const generation = state.chatGeneration;
          const view = state.viewGeneration;
          void call("open_chat", { contact_id: id })
            .then((result) => {
              if (
                generation === state.chatGeneration &&
                view === state.viewGeneration &&
                linkGeneration === state.linkGeneration &&
                result.contact?.id === id
              )
                return selectChat(
                  result.contact,
                  result,
                  state.view === "chats",
                );
            })
            .catch((reason) => {
              if (
                generation === state.chatGeneration &&
                view === state.viewGeneration &&
                linkGeneration === state.linkGeneration
              )
                error(reason);
            });
        }
      }
    } catch (reason) {
      error(reason);
    }
  }
}
$("refresh").onclick = act(async () => {
  if (state.view !== "chats") return;
  const view = state.viewGeneration;
  clearError();
  await loadContacts();
  if (view === state.viewGeneration && state.selected) await loadMessages();
});
$("refresh-feed").onclick = () => void loadFeed();
$("show-messages").onclick = act(showFeed);
for (const button of document.querySelectorAll<HTMLButtonElement>(
  "[data-view]",
))
  button.onclick = () => {
    if (button.dataset.view === "messages") void showFeed();
    else showChats();
  };
$("more-chats").onclick = act(() => loadContacts(true));
$("retry").onclick = act(async () => {
  const retry = state.retry;
  clearError();
  await retry?.();
});
$("dismiss-error").onclick = clearError;
let searchTimer: ReturnType<typeof setTimeout>;
$<HTMLInputElement>("search").oninput = () => {
  if (state.view !== "chats") return;
  state.query = $<HTMLInputElement>("search").value.trim();
  ++state.listGeneration;
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => void loadContacts(), 200);
};
for (const button of document.querySelectorAll<HTMLButtonElement>(
  "[data-filter]",
))
  button.onclick = act(async () => {
    if (state.view !== "chats") return;
    state.filter = button.dataset.filter ?? "all";
    for (const item of document.querySelectorAll("[data-filter]")) {
      const active = (item as HTMLElement).dataset.filter === state.filter;
      item.classList.toggle("active", active);
      item.setAttribute("aria-pressed", String(active));
    }
    await loadContacts();
  });
$("back").onclick = showChats;
$("cancel-reply").onclick = () => {
  state.reply = null;
  ++state.draftRevision;
  invalidatePreparation();
  controls();
};
$("composer").onsubmit = (event) => {
  event.preventDefault();
  void prepare();
};
$<HTMLTextAreaElement>("draft").oninput = () => {
  ++state.draftRevision;
  rememberDraft();
  invalidatePreparation();
  controls();
};
$("confirm-send").onclick = act(send);
$("cancel-send").onclick = invalidatePreparation;
$<HTMLDialogElement>("confirmation").oncancel = (event) => {
  if (state.sending) event.preventDefault();
  else invalidatePreparation();
};
$("read").onclick = act(async () => {
  if (state.view !== "chats") return;
  await messageAction("mark_conversation_read", {});
  await loadContacts();
});
$("expand").onclick = act(async () => {
  await app.requestDisplayMode({ mode: "fullscreen" });
  hostContext();
});
app.ontoolresult = (result) => {
  try {
    const initial = payload(result);
    if (initial.permissions) {
      state.send = initial.permissions.send === true;
      if (state.selected) drawMessages();
    }
    if (initial.contact) {
      if (state.view === "messages") return;
      void selectChat(initial.contact, initial).catch((reason) =>
        error(reason),
      );
      void loadContacts();
    } else if (initial.contacts) {
      ++state.listGeneration;
      state.loadingList = false;
      state.contacts = initial.contacts;
      state.chatCursor =
        typeof initial.nextCursor === "number" ? initial.nextCursor : null;
      $("count").textContent = String(
        initial.totalMatched ?? initial.contacts.length,
      );
      drawContacts();
      controls();
    }
  } catch (reason) {
    error(reason, () => loadContacts());
  }
};
app.ontoolcancelled = () => {
  ++state.viewGeneration;
  ++state.feedGeneration;
  ++state.linkGeneration;
  ++state.chatGeneration;
  ++state.actionGeneration;
  ++state.listGeneration;
  ++state.draftRevision;
  state.busy = false;
  state.loadingChat = false;
  state.loadingList = false;
  state.loadingFeed = false;
  invalidatePreparation();
  controls();
  if (state.view === "messages") {
    state.feedFailed = true;
    drawFeed();
  }
  const view = state.viewGeneration;
  error("The request was interrupted. Refresh to reconnect.", async () => {
    if (view === state.viewGeneration)
      await (state.view === "messages" ? loadFeed() : loadContacts());
  });
};
app.addEventListener("hostcontextchanged", hostContext);
app.onteardown = async () => {
  ++state.viewGeneration;
  ++state.feedGeneration;
  ++state.linkGeneration;
  ++state.chatGeneration;
  ++state.actionGeneration;
  ++state.listGeneration;
  clearTimeout(searchTimer);
  state.drafts.clear();
  state.prepared = null;
  state.feedMessages = [];
  state.feedExpanded.clear();
  return {};
};
void app
  .connect()
  .then(hostContext)
  .catch((reason) =>
    error(
      `Could not connect to ChatGPT. ${reason instanceof Error ? reason.message : String(reason)}`,
    ),
  );
