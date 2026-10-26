import {
  applyDocumentTheme,
  applyHostStyleVariables,
} from "@modelcontextprotocol/ext-apps";
import { $, app, call, clearError, error, extensions, state } from "./state";
import {
  controls,
  drawMessages,
  invalidatePreparation,
  rememberDraft,
} from "./chats";
import { loadMessages, selectChat } from "./views";

export async function messageAction(
  name: string,
  args: Record<string, unknown>,
) {
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

export async function prepare() {
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

export async function send() {
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

export function hostContext() {
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
