// Stored feed data only. This renderer has no message actions or host API access.
export type FeedMessage = {
  id: string;
  contactId: string;
  contactName: string | null;
  contactPhone: string | null;
  chatType: string;
  timestamp: number;
  timestampIso: string | null;
  isFromMe: boolean;
  senderName: string | null;
  senderPhone: string | null;
  text: string | null;
  originalText: string | null;
  translatedText: string | null;
  isTranslated: boolean;
  sourceLanguage: string | null;
  contentType: string;
  deliveryStatus: string | null;
  replyContext: {
    messageId: string | null;
    senderName: string | null;
    text: string | null;
  } | null;
};
export type FeedPage = {
  messages: FeedMessage[];
  hasMore: boolean;
  nextCursor: { beforeTimestamp: number; beforeMessageId: string } | null;
};
export const feedKey = (message: FeedMessage) =>
  JSON.stringify([message.contactId, message.id]);
export const sourceName = (message: FeedMessage) =>
  message.contactName || message.contactPhone || message.contactId;

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text = "",
) {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

export function renderFeed(
  container: HTMLElement,
  state: {
    messages: FeedMessage[];
    hasMore: boolean;
    loading: boolean;
    expanded: Set<string>;
  },
  actions: { older: () => void; openChat: (message: FeedMessage) => void },
  scrollToLatest = false,
) {
  const viewport = container.getBoundingClientRect();
  const anchor = [
    ...container.querySelectorAll<HTMLElement>(".feed-item .message-row"),
  ].find((row) => {
    const rect = row.getBoundingClientRect();
    return rect.bottom > viewport.top && rect.top < viewport.bottom;
  });
  const anchorKey = anchor?.parentElement?.dataset.key;
  const anchorOffset = anchor
    ? anchor.getBoundingClientRect().top - viewport.top
    : 0;
  const oldTop = container.scrollTop;
  for (const disclosure of container.querySelectorAll<HTMLDetailsElement>(
    "details[data-key]",
  )) {
    const key = disclosure.dataset.key!;
    if (disclosure.open) state.expanded.add(key);
    else state.expanded.delete(key);
  }
  container.replaceChildren();
  if (state.hasMore) {
    const more = element("button", "load-more", "Load earlier messages");
    more.disabled = state.loading;
    more.onclick = actions.older;
    container.append(more);
  }
  if (!state.messages.length)
    container.append(
      element(
        "p",
        "empty",
        state.loading ? "Loading messages…" : "No messages here yet.",
      ),
    );
  let day = "";
  let previousContact = "";
  for (const message of state.messages) {
    const key = feedKey(message);
    const item = element("div", "feed-item");
    item.dataset.key = key;
    item.dataset.messageId = message.id;
    item.dataset.contactId = message.contactId;
    const label = new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
    }).format(message.timestamp);
    const newDay = label !== day;
    if (newDay) {
      const divider = element("div", "date-divider");
      divider.append(element("span", "", label));
      item.append(divider);
      day = label;
    }
    if (newDay || previousContact !== message.contactId) {
      const source = element("button", "feed-source");
      source.setAttribute(
        "aria-label",
        `Open source chat with ${sourceName(message)}`,
      );
      source.append(
        element(
          "span",
          "source-icon",
          message.chatType.toLowerCase() === "group" ? "◎" : "◉",
        ),
        element("span", "source-name", sourceName(message)),
        element("span", "source-arrow", "›"),
      );
      source.onclick = () => actions.openChat(message);
      item.append(source);
    }
    previousContact = message.contactId;
    const row = element(
      "div",
      `message-row${message.isFromMe ? " outgoing" : ""}`,
    );
    const bubble = element("article", "bubble");
    if (!message.isFromMe && message.chatType.toLowerCase() === "group")
      bubble.append(
        element(
          "span",
          "sender",
          message.senderName || message.senderPhone || "Group member",
        ),
      );
    if (message.replyContext) {
      const quote = element("div", "quoted");
      if (message.replyContext.senderName)
        quote.append(
          element("strong", "quoted-sender", message.replyContext.senderName),
        );
      quote.append(
        element("span", "", message.replyContext.text || "Quoted message"),
      );
      bubble.append(quote);
    }
    const translated =
      message.isTranslated &&
      !!message.translatedText &&
      message.translatedText !== message.originalText;
    if (translated)
      bubble.append(element("span", "translation-label", "✦ Translated"));
    if (message.contentType.toLowerCase() !== "text")
      bubble.append(
        element("p", "media-placeholder", `[${message.contentType} message]`),
      );
    const text = message.text || message.originalText;
    if (text) bubble.append(element("p", "message-text", text));
    if (translated) {
      const original = element("details");
      original.dataset.key = key;
      original.open = state.expanded.has(key);
      original.append(
        element(
          "summary",
          "",
          message.isFromMe ? "Sent translation" : "Show original",
        ),
        element(
          "p",
          "",
          (message.isFromMe ? message.translatedText : message.originalText) ||
            "",
        ),
      );
      bubble.append(original);
    }
    const meta = element("div", "message-meta");
    const timestamp = element(
      "time",
      "",
      new Intl.DateTimeFormat(undefined, {
        hour: "numeric",
        minute: "2-digit",
      }).format(message.timestamp),
    );
    timestamp.dateTime =
      message.timestampIso || new Date(message.timestamp).toISOString();
    timestamp.title = new Date(message.timestamp).toLocaleString();
    meta.append(timestamp);
    if (message.isFromMe && message.deliveryStatus) {
      const status = message.deliveryStatus;
      const delivery = element(
        "span",
        "delivery",
        status === "uncertain"
          ? "Delivery uncertain"
          : ["read", "delivered"].includes(status)
            ? "✓✓"
            : status === "sent"
              ? "✓"
              : status,
      );
      delivery.title = status;
      delivery.setAttribute("aria-label", status);
      meta.append(delivery);
    }
    bubble.append(meta);
    row.append(bubble);
    item.append(row);
    container.append(item);
  }
  if (scrollToLatest) container.scrollTop = container.scrollHeight;
  else {
    const restored = [...container.querySelectorAll<HTMLElement>(".feed-item")]
      .find((row) => row.dataset.key === anchorKey)
      ?.querySelector<HTMLElement>(".message-row");
    container.scrollTop = oldTop;
    if (restored)
      container.scrollTop +=
        restored.getBoundingClientRect().top - viewport.top - anchorOffset;
  }
}
