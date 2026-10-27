import { renderFeed, feedKey, type FeedMessage, type FeedPage } from "./feed";
import {
  $,
  type Contact,
  type Payload,
  app,
  call,
  clearError,
  displayName,
  error,
  initials,
  node,
  state,
} from "./state";
import {
  controls,
  drawContacts,
  drawMessages,
  invalidatePreparation,
  loadContacts,
  rememberDraft,
} from "./chats";

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

export function showChats() {
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

export async function showFeed() {
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

export function drawFeed(latest = false) {
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

export async function loadFeed(older = false) {
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

export async function selectChat(
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

export async function loadMessages(older = false) {
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

export let searchTimer: ReturnType<typeof setTimeout>;

$<HTMLInputElement>("search").oninput = () => {
  if (state.view !== "chats") return;
  state.query = $<HTMLInputElement>("search").value.trim();
  ++state.listGeneration;
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => void loadContacts(), 200);
};
