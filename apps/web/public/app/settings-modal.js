// Contact context menu and conversation settings modal.

export const settingsModalMethods = {
  // ==================== Conversation Settings ====================

  // Show context menu for a contact
  showContactContextMenu(event, contactId) {
    const menu = document.getElementById('contact-context-menu');
    if (!menu) return;

    this.contextMenuContactId = contactId;

    // Update pin button text based on current state
    const contact = this.contacts.find((c) => c.id === contactId);
    const pinText = menu.querySelector('.pin-text');
    if (pinText && contact) {
      pinText.textContent = contact.pinnedAt
        ? 'Unpin conversation'
        : 'Pin conversation';
    }

    // Position the menu near the cursor
    menu.style.left = `${event.clientX}px`;
    menu.style.top = `${event.clientY}px`;
    menu.classList.remove('hidden');

    // Ensure menu stays within viewport
    const rect = menu.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
      menu.style.left = `${window.innerWidth - rect.width - 10}px`;
    }
    if (rect.bottom > window.innerHeight) {
      menu.style.top = `${window.innerHeight - rect.height - 10}px`;
    }
  },

  // Hide context menu
  hideContactContextMenu() {
    const menu = document.getElementById('contact-context-menu');
    if (menu) {
      menu.classList.add('hidden');
    }
    this.contextMenuContactId = null;
  },

  // Open settings modal for current contact
  async openSettingsModal(contactId = this.currentContactId) {
    if (!contactId) {
      console.warn('No contact selected');
      return;
    }

    const modal = document.getElementById('settings-modal');
    if (!modal) return;

    this.settingsContactId = contactId;
    let settings;
    try {
      settings = await this.conversationSettingsClient.load(contactId);
    } catch (error) {
      if (this.settingsContactId === contactId) {
        this.settingsContactId = null;
        alert(error.message);
      }
      return;
    }
    if (this.settingsContactId !== contactId) return;
    const localSettings = this.getContactMetadata(contactId);
    const contact = this.contacts.find((item) => item.id === contactId);
    const mergedSettings = {
      ...settings,
      alias: localSettings.alias || contact?.alias || '',
      timezone: localSettings.timezone || contact?.timezone || '',
    };

    const setFieldValue = (id, value = '') => {
      const field = document.getElementById(id);
      if (field) field.value = value || '';
    };

    document.getElementById('reply-only').checked =
      mergedSettings.replyOnly === true;
    document.getElementById('translation-enabled').checked =
      mergedSettings.translationEnabled === true;
    this.updateTranslationSettingsControls();
    setFieldValue('contact-alias', mergedSettings.alias);
    setFieldValue('conversation-timezone', mergedSettings.timezone);
    setFieldValue('language-override', mergedSettings.languageOverride);
    setFieldValue('translation-style', mergedSettings.translationStyle);
    const sendOriginalFollowUp = document.getElementById(
      'send-original-follow-up',
    );
    if (sendOriginalFollowUp) {
      sendOriginalFollowUp.checked = Boolean(
        mergedSettings.sendOriginalFollowUp,
      );
    }

    modal.classList.remove('hidden');
    this.messageTones?.refreshLabel(contactId);
  },

  updateChatHeaderNote() {
    const noteEl = document.getElementById('chat-note-indicator');
    if (!noteEl) return;
    const summary = this.currentContactId
      ? this.buildChatMetadataSummary(this.currentContactId)
      : '';
    if (summary) {
      noteEl.textContent = summary;
      noteEl.classList.remove('hidden');
    } else {
      noteEl.textContent = '';
      noteEl.classList.add('hidden');
    }
  },

  updateTranslationSettingsControls() {
    const enabled =
      document.getElementById('translation-enabled')?.checked === true;
    for (const id of [
      'language-override',
      'translation-style',
      'send-original-follow-up',
    ]) {
      const field = document.getElementById(id);
      if (field)
        field.disabled = !enabled || Boolean(this.savingConversationSettings);
    }
    const hint = document.getElementById('translation-enabled-hint');
    if (hint)
      hint.textContent = enabled
        ? 'Translates messages, captions and voice notes for this conversation on every connected device, including Messages.'
        : 'Off by default. Sends messages as written and uses your original voice recording, without waiting for translation.';
  },

  // Close settings modal
  closeSettingsModal() {
    if (this.savingConversationSettings) return;
    this.settingsContactId = null;
    const modal = document.getElementById('settings-modal');
    if (modal) {
      modal.classList.add('hidden');
    }
  },

  // Save conversation settings
  async saveConversationSettings() {
    if (this.savingConversationSettings) return;
    const targetContactId = this.settingsContactId || this.currentContactId;
    if (!targetContactId) return;

    const languageOverride =
      document.getElementById('language-override')?.value?.trim() || null;
    const translationStyle =
      document.getElementById('translation-style')?.value?.trim() || null;
    const sendOriginalFollowUp = Boolean(
      document.getElementById('send-original-follow-up')?.checked,
    );
    const alias =
      document.getElementById('contact-alias')?.value?.trim() || null;
    const timezone =
      document.getElementById('conversation-timezone')?.value?.trim() || null;

    const translationEnabled =
      document.getElementById('translation-enabled')?.checked === true;
    const modal = document.getElementById('settings-modal');
    this.savingConversationSettings = true;
    modal?.querySelectorAll('button, input').forEach((control) => {
      control.disabled = true;
    });
    try {
      await this.conversationSettingsClient.save(targetContactId, {
        replyOnly: document.getElementById('reply-only')?.checked === true,
        translationEnabled,
        languageOverride,
        translationStyle,
        sendOriginalFollowUp,
      });
      this.updateContactMetadata(targetContactId, { alias, timezone });
      const contact = this.contacts.find((item) => item.id === targetContactId);
      if (contact) Object.assign(contact, { alias, timezone });
    } catch (error) {
      alert(error.message);
      return;
    } finally {
      this.savingConversationSettings = false;
      modal?.querySelectorAll('button, input').forEach((control) => {
        control.disabled = false;
      });
      this.updateTranslationSettingsControls();
    }

    this.closeSettingsModal();
    this.syncInboxControls();
    this.renderContacts();
    this.updateChatHeaderNote();
    this.renderConversationWorkspace();
    this.updateConversationMenuUI();
  },
};
