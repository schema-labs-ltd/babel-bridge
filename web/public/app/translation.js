// Conversation briefs, manual translation and reaction rendering.

import {
  buildConversationHandoffBrief,
  simulateTranslation,
} from '../app-state.js?v=20260912-conversation-translation';

export const translationMethods = {
  getConversationHandoffBrief() {
    if (!this.currentContactId) return '';

    const contact = this.contacts.find(
      (entry) => entry.id === this.currentContactId,
    ) || { id: this.currentContactId };
    return buildConversationHandoffBrief({
      contact,
      metadata: this.getContactMetadata(this.currentContactId),
      messages: this.messages.get(this.currentContactId) || [],
      drafts: this.drafts,
    });
  },

  async copyConversationBrief() {
    const text = this.getConversationHandoffBrief();
    if (!text) return;

    const button = document.getElementById('workspace-copy-brief');
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const textarea = document.createElement('textarea');
        textarea.value = text;
        textarea.setAttribute('readonly', '');
        textarea.style.position = 'fixed';
        textarea.style.opacity = '0';
        document.body.appendChild(textarea);
        textarea.select();
        document.execCommand('copy');
        textarea.remove();
      }
      if (button) {
        button.textContent = 'Copied';
        window.setTimeout(() => this.renderConversationWorkspace(), 1600);
      }
    } catch (err) {
      console.error('Failed to copy conversation brief:', err);
      alert('Failed to copy conversation brief: ' + err.message);
    }
  },

  async translateUntranslatedIncomingMessages() {
    if (!this.currentContactId) return;
    if (!this.getContactMetadata(this.currentContactId).translationEnabled) {
      alert(
        'Translation is off for this conversation. Enable Translate messages in Conversation settings first.',
      );
      return;
    }

    const messages =
      this.getUntranslatedIncomingMessagesForCurrentConversation();
    if (messages.length === 0) {
      this.updateConversationMenuUI();
      this.renderConversationWorkspace();
      return;
    }

    const buttons = [
      document.getElementById('chat-translate-all'),
      document.getElementById('workspace-translate-all'),
    ].filter(Boolean);
    buttons.forEach((button) => {
      button.disabled = true;
      button.textContent = 'Translating...';
    });

    try {
      for (const message of messages) {
        await this.translateMessage(message.id, {
          silent: true,
          rerender: false,
        });
      }
      this.refreshCurrentConversationView();
      this.renderConversationWorkspace();
    } finally {
      this.updateConversationMenuUI();
      this.renderConversationWorkspace();
    }
  },

  // Translate a message manually
  async translateMessage(messageId, options = {}) {
    const { silent = false, rerender = true } = options;
    if (!this.getContactMetadata(this.currentContactId).translationEnabled) {
      if (!silent)
        alert(
          'Translation is off for this conversation. Enable Translate messages in Conversation settings first.',
        );
      return;
    }
    const messages = this.messages.get(this.currentContactId);
    if (!messages) return;

    const message = messages.find((m) => m.id === messageId);
    if (!message) return;

    if (
      ['audio', 'voice note'].includes(
        String(
          message.content?.type ||
            message.content_type ||
            message.contentType ||
            '',
        ).toLowerCase(),
      )
    ) {
      this.translateVoiceMessage?.(messageId);
      return;
    }

    // Check if already translated
    if (message.is_translated || message.isTranslated) {
      if (!silent) alert('This message has already been translated.');
      return;
    }

    // Get the text to translate
    const text =
      message.content?.body ||
      message.content?.caption ||
      message.content?.text;
    if (!text) {
      if (!silent) alert('No text to translate in this message.');
      return;
    }

    if (this.demoMode) {
      const metadata = this.getContactMetadata(this.currentContactId);
      const targetLanguage =
        metadata.targetLanguage || message.sourceLanguage || 'English';
      const translatedText = simulateTranslation(text, targetLanguage);
      message.original_text = text;
      message.originalText = text;
      message.translated_text = translatedText;
      message.translatedText = translatedText;
      message.source_language = targetLanguage;
      message.sourceLanguage = targetLanguage;
      message.is_translated = true;
      message.isTranslated = true;
      if (rerender) {
        this.renderMessages(this.getVisibleMessagesForCurrentConversation());
        this.renderConversationWorkspace();
        this.updateConversationMenuUI();
      }
      return;
    }

    try {
      // Call the translation API
      const response = await this.apiFetch('/api/translate', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.getAuthHeaders(),
        },
        body: JSON.stringify({
          text: text,
          messageId: messageId,
          contactId: this.currentContactId,
        }),
      });

      const result = await response.json();
      if (!response.ok || !result.success) {
        throw new Error(result.error || 'Translation failed');
      }
      if (
        result.needsTranslation !== false &&
        (!result.translatedText || !String(result.translatedText).trim())
      ) {
        throw new Error(
          result.error || 'Translation did not return translated text',
        );
      }

      // Update the local message with translation
      message.translated_text = result.translatedText;
      message.translatedText = result.translatedText;
      message.source_language = result.sourceLanguage;
      message.sourceLanguage = result.sourceLanguage;
      message.is_translated = result.needsTranslation !== false;
      message.isTranslated = result.needsTranslation !== false;

      // Re-render messages to show translation
      if (rerender) {
        const currentMessages = this.getVisibleMessagesForCurrentConversation();
        this.renderMessages(currentMessages);
        this.renderConversationWorkspace();
        this.updateConversationMenuUI();
      }
    } catch (err) {
      console.error('Translation failed:', err);
      if (!silent) alert('Translation failed: ' + err.message);
    }
  },

  // Update reactions display for a specific message
  updateMessageReactions(messageId) {
    const messageEl = document.querySelector(
      `.message[data-message-id="${messageId}"]`,
    );
    if (!messageEl) return;

    const messages = this.messages.get(this.currentContactId);
    if (!messages) return;

    const message = messages.find((m) => m.id === messageId);
    if (!message || !message.reactions) return;

    // Remove existing reactions container
    const existingReactions = messageEl.querySelector('.message-reactions');
    if (existingReactions) {
      existingReactions.remove();
    }

    // Build reactions HTML
    const reactionsHtml = this.renderReactions(message.reactions);
    if (reactionsHtml) {
      // Insert before message-footer
      const footer = messageEl.querySelector('.message-footer');
      if (footer) {
        footer.insertAdjacentHTML('beforebegin', reactionsHtml);
      }
    }
  },

  // Render reactions for a message
  renderReactions(reactions) {
    if (!reactions || Object.keys(reactions).length === 0) return '';

    const reactionItems = Object.entries(reactions)
      .filter(([emoji, reactors]) => reactors.length > 0)
      .map(([emoji, reactors]) => {
        const count =
          reactors.length > 1
            ? `<span class="reaction-count">${reactors.length}</span>`
            : '';
        return `<span class="reaction-item">${emoji}${count}</span>`;
      })
      .join('');

    if (!reactionItems) return '';

    return `<div class="message-reactions">${reactionItems}</div>`;
  },
};
