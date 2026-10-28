// Contact metadata, reply-only controls and the composer assist panel.

import { canSendInConversation } from '../conversation-settings.js';
import {
  getComposerAssistState,
  getComposerReminderPresets,
  getMessageSnippet,
  getDraftText,
} from '../app-state.js?v=20260912-conversation-translation';

export const composerAssistMethods = {
  getCurrentContact() {
    return (
      this.contacts.find((contact) => contact.id === this.currentContactId) ||
      null
    );
  },

  getLatestIncomingMessage(contactId = this.currentContactId) {
    const messages = this.messages.get(contactId) || [];
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index];
      if (
        !message?.isFromMe &&
        getMessageSnippet(message, 96) !== '[Message]'
      ) {
        return message;
      }
    }
    return null;
  },

  getCurrentComposerAssistState() {
    const input = document.getElementById('message-input');
    const contact = this.getCurrentContact() || { id: this.currentContactId };
    return getComposerAssistState({
      draftText:
        input?.value || getDraftText(this.drafts, this.currentContactId),
      metadata: this.getContactMetadata(this.currentContactId),
      contact,
      latestIncomingMessage: this.getLatestIncomingMessage(),
      messages: this.messages.get(this.currentContactId) || [],
      demoMode: this.demoMode,
    });
  },

  replyOnlyBlocked() {
    return !canSendInConversation(
      this.conversationSettingsClient.get(this.currentContactId),
      this.replyingTo,
    );
  },

  updateReplyOnlyControls() {
    const blocked = this.replyOnlyBlocked();
    for (const id of [
      'message-input',
      'send-button',
      'attach-button',
      'voice-record-button',
    ]) {
      const control = document.getElementById(id);
      if (control)
        control.disabled =
          blocked ||
          (id === 'send-button' &&
            !document.getElementById('message-input')?.value.trim());
    }
    const hint = document.getElementById('reply-only-hint');
    if (hint) hint.hidden = !blocked;
  },

  renderComposerAssist() {
    this.updateReplyOnlyControls();
    const container = document.getElementById('composer-assist');
    if (!container) return;

    if (!this.currentContactId) {
      container.classList.add('hidden');
      return;
    }

    const state = this.getCurrentComposerAssistState();
    const kicker = document.getElementById('composer-assist-kicker');
    const title = document.getElementById('composer-assist-title');
    const preview = document.getElementById('composer-assist-preview');
    const suggestionButton = document.getElementById('composer-use-suggestion');
    const profilePresets = document.getElementById('composer-profile-presets');
    const smartReplies = document.getElementById('composer-smart-replies');
    const reminderPresets = document.getElementById(
      'composer-reminder-presets',
    );
    const readiness = document.getElementById('composer-readiness');

    if (kicker) kicker.textContent = state.previewLabel;
    if (title)
      title.textContent = state.translationEnabled
        ? `${state.targetLanguage} · ${state.styleSummary}`
        : 'Translation off';
    if (preview) {
      if (!state.translationEnabled) {
        preview.textContent =
          'Messages send as written. Enable translation in Conversation settings when needed.';
      } else if (state.translatedPreview) {
        preview.textContent = state.translatedPreview;
      } else if (state.latestIncomingSnippet) {
        preview.textContent = `Latest incoming: ${state.latestIncomingSnippet}`;
      } else {
        preview.textContent =
          'Set a language profile or type a draft to preview the translation route.';
      }
    }
    if (suggestionButton) {
      suggestionButton.disabled = !state.canUseSuggestedReply;
      suggestionButton.title = state.canUseSuggestedReply
        ? 'Use the first reply starter based on the latest incoming message'
        : 'No incoming message available for a smart reply yet';
    }
    if (profilePresets) {
      const languageButtons = state.profilePresets.languages
        .map(
          (preset) => `
        <button
          class="composer-preset-chip ${preset.active ? 'active' : ''}"
          type="button"
          data-composer-language="${this.escapeHtml(preset.value)}"
        >${this.escapeHtml(preset.label)}</button>
      `,
        )
        .join('');
      const toneButtons = state.profilePresets.tones
        .map(
          (preset) => `
        <button
          class="composer-preset-chip ${preset.active ? 'active' : ''}"
          type="button"
          data-composer-tone="${this.escapeHtml(preset.value)}"
        >${this.escapeHtml(preset.label)}</button>
      `,
        )
        .join('');
      profilePresets.innerHTML = `
        <div class="composer-preset-row"><span>Language</span>${languageButtons}</div>
        <div class="composer-preset-row"><span>Tone</span>${toneButtons}</div>
      `;
      profilePresets.querySelectorAll('button').forEach((button) => {
        button.disabled = !state.translationEnabled;
      });
    }
    if (smartReplies) {
      smartReplies.innerHTML =
        state.smartReplies.length > 0
          ? state.smartReplies
              .map(
                (reply) => `
          <button
            class="composer-smart-reply"
            type="button"
            data-smart-reply-id="${this.escapeHtml(reply.id)}"
            title="${this.escapeHtml(reply.text)}"
          >
            <span>${this.escapeHtml(reply.label)}</span>
            <small>${this.escapeHtml(reply.text)}</small>
          </button>
        `,
              )
              .join('')
          : '';
    }
    if (reminderPresets) {
      reminderPresets.innerHTML = state.reminderPresets
        .map(
          (preset) => `
        <button
          class="composer-reminder-chip"
          type="button"
          data-reminder-preset="${this.escapeHtml(preset.id)}"
          title="${this.escapeHtml(preset.reason || 'Set follow-up reminder')}"
        >${this.escapeHtml(preset.label)}</button>
      `,
        )
        .join('');
    }
    if (readiness) {
      readiness.innerHTML = state.readiness.checks
        .map(
          (check) => `
        <span class="composer-readiness-chip ${this.escapeHtml(check.status)}">
          ${this.escapeHtml(check.label)}
        </span>
      `,
        )
        .join('');
      readiness.setAttribute(
        'aria-label',
        `Composer checks: ${state.readiness.scoreLabel}`,
      );
    }

    container.classList.toggle('hidden', !state.showPreview);
  },

  useComposerSuggestedReply(replyId = 'confirm') {
    if (!this.currentContactId) return;
    const state = this.getCurrentComposerAssistState();
    const selectedReply =
      state.smartReplies.find((reply) => reply.id === replyId) ||
      state.smartReplies[0];
    const replyText = selectedReply?.text || state.suggestedReply;
    if (!replyText) return;

    const input = document.getElementById('message-input');
    if (!input) return;

    input.value = replyText;
    this.autoResizeTextarea(input);
    this.updateSendButton();
    this.handleDraftInput();
    input.focus();
  },

  async applyComposerProfilePreset(updates = {}) {
    if (!this.currentContactId) return;

    const nextUpdates = {};
    if (updates.language) {
      nextUpdates.languageOverride = updates.language;
      nextUpdates.targetLanguage = updates.language;
    }
    if (updates.tone) {
      nextUpdates.translationStyle = updates.tone;
    }
    if (Object.keys(nextUpdates).length === 0) return;

    const contactId = this.currentContactId;
    try {
      const saved = await this.conversationSettingsClient.load(contactId);
      if (!saved.translationEnabled) return;
      await this.conversationSettingsClient.save(contactId, {
        ...saved,
        ...nextUpdates,
      });
    } catch (error) {
      alert(error.message);
      return;
    }
    const contact = this.contacts.find((item) => item.id === contactId);
    if (contact) {
      Object.assign(contact, nextUpdates);
    }
    this.updateChatHeaderNote();
    this.renderContacts();
    this.renderComposerAssist();
    this.renderConversationWorkspace();
  },

  setComposerReminderPreset(presetId = 'tomorrow') {
    if (!this.currentContactId) return;

    const contact = this.getCurrentContact() || { id: this.currentContactId };
    const input = document.getElementById('message-input');
    const draft = String(input?.value || '').trim();
    const latestIncoming = this.getLatestIncomingMessage();
    const reminderSource =
      draft || (latestIncoming ? getMessageSnippet(latestIncoming, 72) : '');
    const displayName = this.getContactDisplayName(contact);
    const state = this.getCurrentComposerAssistState();
    const fallbackPresets = getComposerReminderPresets();
    const preset =
      state.reminderPresets.find((entry) => entry.id === presetId) ||
      fallbackPresets.find((entry) => entry.id === 'tomorrow');

    this.updateContactMetadata(this.currentContactId, {
      reminderText: reminderSource
        ? `Follow up: ${reminderSource}`
        : `Follow up with ${displayName}`,
      reminderAt: preset?.reminderAt || Date.now() + 86_400_000,
    });
    this.renderContacts();
    this.updateChatHeaderNote();
    this.renderComposerAssist();
  },

  setComposerReminderTomorrow() {
    this.setComposerReminderPreset('tomorrow');
  },

  buildContactBadgeMarkup(contactId, now = Date.now()) {
    void contactId;
    void now;
    return '';
  },

  buildChatMetadataSummary(contactId, now = Date.now()) {
    if (!contactId) return '';

    const pieces = [];
    const metadata = this.getContactMetadata(contactId);
    const contact = this.contacts.find((item) => item.id === contactId);
    const targetLanguage =
      metadata.languageOverride || contact?.languageOverride || '';
    const translationStyle =
      metadata.translationStyle || contact?.translationStyle || '';

    pieces.push(
      metadata.translationEnabled === true
        ? targetLanguage
          ? `Translates to ${targetLanguage}`
          : 'Auto-translation on'
        : 'Translation off',
    );

    if (translationStyle && metadata.translationEnabled === true) {
      pieces.push(translationStyle);
    }

    return pieces.slice(0, 3).join(' • ');
  },

  scheduleMetadataRefresh() {
    if (this.metadataRefreshTimer) {
      clearTimeout(this.metadataRefreshTimer);
      this.metadataRefreshTimer = null;
    }

    const now = Date.now();
    const nextTransitions = Object.values(this.contactMetadata || {})
      .flatMap((metadata) => [metadata?.reminderAt, metadata?.snoozedUntil])
      .map((value) => Number(value || 0))
      .filter((value) => Number.isFinite(value) && value > now)
      .sort((a, b) => a - b);

    if (nextTransitions.length === 0) {
      return;
    }

    const delay = Math.min(
      Math.max(nextTransitions[0] - now, 250),
      2_147_483_647,
    );
    this.metadataRefreshTimer = window.setTimeout(() => {
      this.metadataRefreshTimer = null;
      this.renderContacts();
      this.updateChatHeaderNote();
      this.renderConversationWorkspace();
      this.scheduleMetadataRefresh();
    }, delay + 50);
  },

  updateContactMetadata(contactId, updates = {}) {
    if (!contactId) return {};

    const current = this.getContactMetadata(contactId);
    const next = { ...current };
    for (const [key, value] of Object.entries(updates)) {
      if (value === null || value === undefined || value === '') {
        delete next[key];
      } else {
        next[key] = value;
      }
    }

    if (Object.keys(next).length === 0) {
      delete this.contactMetadata[contactId];
    } else {
      this.contactMetadata[contactId] = next;
    }

    this.persistContactMetadata();
    this.scheduleMetadataRefresh();
    this.renderVisitorDashboard();
    this.renderConversationWorkspace();
    this.renderComposerAssist();
    return this.getContactMetadata(contactId);
  },

  applyStoredContactMetadata(contact) {
    if (!contact?.id) return contact;
    return { ...contact, ...this.getContactMetadata(contact.id) };
  },
};
