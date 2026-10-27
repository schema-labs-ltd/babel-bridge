import {
  $,
  act,
  call,
  dateLabel,
  displayName,
  error,
  initials,
  node,
  state,
  time,
} from "./state";
import { loadMessages, selectChat } from "./views";
import { messageAction } from "./compose";

export function drawContacts() {
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

export async function loadContacts(append = false) {
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

export function controls() {
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

export function drawMessages(scroll = false) {
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

export function invalidatePreparation() {
  if (state.sending) return;
  state.prepared = null;
  state.idempotencyKey = "";
  $<HTMLDialogElement>("confirmation").close();
}

export function rememberDraft() {
  if (state.selected)
    state.drafts.set(state.selected.id, $<HTMLTextAreaElement>("draft").value);
}
