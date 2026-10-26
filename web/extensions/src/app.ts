import { $, act, app, clearError, error, payload, state } from "./state";
import {
  controls,
  drawContacts,
  drawMessages,
  invalidatePreparation,
  loadContacts,
  rememberDraft,
} from "./chats";
import {
  drawFeed,
  loadFeed,
  loadMessages,
  searchTimer,
  selectChat,
  showChats,
  showFeed,
} from "./views";
import { hostContext, messageAction, prepare, send } from "./compose";

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
