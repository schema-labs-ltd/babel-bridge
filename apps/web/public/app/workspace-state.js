// Stored preferences, appearance, drafts and workspace import/export.

import {
  getAppearanceThemeCatalog,
  getChecklistSummary,
  getContactDisplayName,
  getContactLabels,
  getPriorityInfo,
  getReminderStatus,
  getReplyState,
  getTimezoneInfo,
  isContactSnoozed,
  resolveAppearanceTheme,
} from '../app-state.js?v=20260912-conversation-translation';

export const workspaceStateMethods = {
  loadStoredJson(key) {
    try {
      return JSON.parse(localStorage.getItem(key) || '{}');
    } catch (err) {
      console.warn(`Failed to load local state for ${key}:`, err);
      return {};
    }
  },

  loadStoredArray(key) {
    try {
      const parsed = JSON.parse(localStorage.getItem(key) || '[]');
      return Array.isArray(parsed) ? parsed : [];
    } catch (err) {
      console.warn(`Failed to load local array state for ${key}:`, err);
      return [];
    }
  },

  saveStoredJson(key, value) {
    localStorage.setItem(key, JSON.stringify(value || {}));
  },

  saveStoredArray(key, value) {
    localStorage.setItem(
      key,
      JSON.stringify(Array.isArray(value) ? value : []),
    );
  },

  getSystemAppearanceMode() {
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light';
  },

  setupAppearanceSync() {
    if (!window.matchMedia) return;
    this.systemAppearanceQuery = window.matchMedia(
      '(prefers-color-scheme: dark)',
    );
    if (typeof this.systemAppearanceQuery.addEventListener === 'function') {
      this.systemAppearanceQuery.addEventListener(
        'change',
        this.handleSystemAppearanceChange,
      );
    } else if (typeof this.systemAppearanceQuery.addListener === 'function') {
      this.systemAppearanceQuery.addListener(this.handleSystemAppearanceChange);
    }
  },

  handleSystemAppearanceChange() {
    if ((this.appearancePreferences?.mode || 'system') === 'system') {
      this.applyAppearanceTheme();
    }
  },

  persistAppearancePreferences() {
    this.saveStoredJson(this.appearanceStorageKey, this.appearancePreferences);
  },

  syncAppearanceControls(preferences = this.appearancePreferences) {
    const themeSelect = document.getElementById('appearance-theme-select');
    const modeSelect = document.getElementById('appearance-mode-select');
    const preview = document.getElementById('appearance-preview-text');
    const previewState = resolveAppearanceTheme(preferences, {
      systemMode: this.getSystemAppearanceMode(),
    });
    const theme = previewState.theme;
    const mode = previewState.mode;
    const resolvedMode = previewState.resolvedMode;
    const label = previewState.definition.label || 'WhatsApp';

    if (themeSelect) {
      const currentOptions = Array.from(themeSelect.options).map(
        (option) => option.value,
      );
      const expectedOptions = getAppearanceThemeCatalog().map(
        (themeOption) => themeOption.id,
      );
      if (currentOptions.join('|') !== expectedOptions.join('|')) {
        themeSelect.innerHTML = getAppearanceThemeCatalog()
          .map(
            (themeOption) =>
              `<option value="${themeOption.id}">${themeOption.label}</option>`,
          )
          .join('');
      }
      themeSelect.value = theme;
    }
    if (modeSelect) modeSelect.value = mode;
    if (preview) {
      const modeLabel = resolvedMode === 'dark' ? 'Dark' : 'Light';
      const sourceLabel =
        mode === 'system' ? 'following system' : `${mode} mode`;
      preview.textContent = `${label} · ${modeLabel} variant (${sourceLabel})`;
    }
  },

  applyAppearanceTheme() {
    this.appearanceState = resolveAppearanceTheme(this.appearancePreferences, {
      systemMode: this.getSystemAppearanceMode(),
    });

    const root = document.documentElement;
    if (root) {
      root.dataset.theme = this.appearanceState.dataTheme;
      root.style.colorScheme = this.appearanceState.resolvedMode;
    }

    const themeMeta = document.querySelector('meta[name="theme-color"]');
    if (themeMeta) {
      themeMeta.setAttribute(
        'content',
        this.appearanceState.definition.themeColor,
      );
    }

    this.syncAppearanceControls();
  },

  async openAppearanceModal() {
    const modal = document.getElementById('appearance-modal');
    if (!modal) return;
    this.syncAppearanceControls();
    modal.classList.remove('hidden');
    this.messageTones?.refreshLabel();
    this.chatGpt?.refresh();
    try {
      const response = await this.apiFetch('/api/settings/openai');
      if (!response.ok)
        throw new Error(`Settings request failed (${response.status})`);
      const settings = await response.json();
      document.getElementById('openai-model-select').value =
        settings.model || '';
      document.getElementById('openai-reasoning-select').value =
        settings.reasoningEffort || '';
    } catch (error) {
      console.error('Failed to load OpenAI settings:', error);
    }
  },

  closeAppearanceModal() {
    document.getElementById('appearance-modal')?.classList.add('hidden');
  },

  async saveAppearanceSettings() {
    const theme =
      document.getElementById('appearance-theme-select')?.value || 'whatsapp';
    const mode =
      document.getElementById('appearance-mode-select')?.value || 'system';
    this.appearancePreferences = { theme, mode };
    this.persistAppearancePreferences();
    this.applyAppearanceTheme();
    const response = await this.apiFetch('/api/settings/openai', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        model: document.getElementById('openai-model-select')?.value || null,
        reasoningEffort:
          document.getElementById('openai-reasoning-select')?.value || null,
      }),
    });
    if (!response.ok) {
      console.error('Failed to save OpenAI settings:', await response.text());
      return;
    }
    this.closeAppearanceModal();
  },

  persistDrafts() {
    this.saveStoredJson(this.draftsStorageKey, this.drafts);
  },

  persistStarredMessages() {
    this.saveStoredJson(this.starredStorageKey, this.starredMessages);
  },

  persistContactMetadata() {
    this.saveStoredJson(this.contactMetadataStorageKey, this.contactMetadata);
  },

  persistQuickReplies() {
    this.saveStoredArray(this.quickRepliesStorageKey, this.quickReplies);
  },

  persistRecentEmojis() {
    this.saveStoredArray('wa_recent_emojis', this.recentEmojis);
  },

  persistInboxPreferences() {
    this.saveStoredJson(this.inboxPreferencesStorageKey, {
      searchQuery: this.sidebarSearchQuery,
      ...this.contactFilters,
    });
  },

  cloneWorkspaceObject(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      return {};
    }

    try {
      return JSON.parse(JSON.stringify(value));
    } catch (err) {
      console.warn('Failed to clone workspace object:', err);
      return {};
    }
  },

  cloneWorkspaceArray(value) {
    if (!Array.isArray(value)) {
      return [];
    }

    try {
      return JSON.parse(JSON.stringify(value));
    } catch (err) {
      console.warn('Failed to clone workspace array:', err);
      return [];
    }
  },

  getInboxPreferencesSnapshot() {
    return {
      searchQuery: this.sidebarSearchQuery,
      ...this.contactFilters,
    };
  },

  getWorkspaceStateExport() {
    return {
      app: 'whatsapp-translator',
      schemaVersion: 1,
      exportedAt: new Date().toISOString(),
      state: {
        drafts: this.cloneWorkspaceObject(this.drafts),
        starredMessages: this.cloneWorkspaceObject(this.starredMessages),
        contactMetadata: this.cloneWorkspaceObject(this.contactMetadata),
        inboxPreferences: this.getInboxPreferencesSnapshot(),
        quickReplies: this.cloneWorkspaceArray(this.quickReplies),
        appearancePreferences: this.cloneWorkspaceObject(
          this.appearancePreferences,
        ),
        recentEmojis: this.cloneWorkspaceArray(this.recentEmojis),
      },
    };
  },

  downloadWorkspaceStateExport() {
    const payload = JSON.stringify(this.getWorkspaceStateExport(), null, 2);
    const blob = new Blob([payload], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `whatsapp-translator-workspace-${new Date().toISOString().slice(0, 10)}.json`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
  },

  applyWorkspaceStateImport(payload) {
    if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
      throw new Error('Workspace import must be a JSON object.');
    }
    if (payload.app !== 'whatsapp-translator' || payload.schemaVersion !== 1) {
      throw new Error('Workspace import uses an unsupported schema.');
    }
    if (
      !payload.state ||
      typeof payload.state !== 'object' ||
      Array.isArray(payload.state)
    ) {
      throw new Error('Workspace import is missing state.');
    }

    const state = payload.state;
    const inboxPreferences = this.cloneWorkspaceObject(state.inboxPreferences);

    this.drafts = this.cloneWorkspaceObject(state.drafts);
    this.starredMessages = this.cloneWorkspaceObject(state.starredMessages);
    this.contactMetadata = this.cloneWorkspaceObject(state.contactMetadata);
    this.quickReplies = this.cloneWorkspaceArray(state.quickReplies);
    this.appearancePreferences = this.cloneWorkspaceObject(
      state.appearancePreferences,
    );
    this.recentEmojis = this.cloneWorkspaceArray(state.recentEmojis)
      .filter((emoji) => typeof emoji === 'string' && emoji.trim())
      .slice(0, 32);

    this.sidebarSearchQuery =
      typeof inboxPreferences.searchQuery === 'string'
        ? inboxPreferences.searchQuery
        : '';
    for (const key of Object.keys(this.contactFilters)) {
      this.contactFilters[key] = Boolean(inboxPreferences[key]);
    }

    this.persistDrafts();
    this.persistStarredMessages();
    this.persistContactMetadata();
    this.persistQuickReplies();
    this.persistAppearancePreferences();
    this.persistRecentEmojis();
    this.persistInboxPreferences();

    this.applyAppearanceTheme();
    this.syncInboxControls();
    this.restoreDraftForCurrentContact();
    this.updateSendButton();
    this.renderQuickReplies();
    this.renderContacts();
    this.updateChatHeaderNote();
    this.updateWorkspaceUI();
    this.renderComposerAssist();

    const currentContact = this.contacts.find(
      (contact) => contact.id === this.currentContactId,
    );
    if (currentContact) {
      const displayName = this.getContactDisplayName(currentContact);
      const nameEl = document.getElementById('chat-name');
      if (nameEl) nameEl.textContent = displayName;
      const avatarContainer = document.querySelector('.chat-header .avatar');
      if (avatarContainer && !this.avatarCache.get(currentContact.id)) {
        const initial = (displayName || '?').charAt(0).toUpperCase();
        avatarContainer.innerHTML = `<span>${this.escapeHtml(initial)}</span>`;
      }
    }
  },

  async importWorkspaceStateFromFile(file) {
    if (!file) return;

    try {
      const payload = JSON.parse(await file.text());
      this.applyWorkspaceStateImport(payload);
      alert('Workspace data imported.');
    } catch (err) {
      console.error('Failed to import workspace data:', err);
      alert(`Workspace import failed: ${err.message}`);
    }
  },

  getContactMetadata(contactId) {
    const local = this.contactMetadata?.[contactId] || {};
    const saved = this.conversationSettingsClient?.get(contactId);
    return {
      ...local,
      translationEnabled: false,
      ...saved,
      ...(saved ? { targetLanguage: saved.languageOverride || '' } : {}),
    };
  },

  getContactNotePreview(contactId, maxLength = 64) {
    const notes = String(this.getContactMetadata(contactId).notes || '').trim();
    if (!notes) return '';
    return notes.length > maxLength
      ? `${notes.slice(0, maxLength - 1)}…`
      : notes;
  },

  getContactLabels(contactId) {
    return getContactLabels(this.getContactMetadata(contactId));
  },

  getContactDisplayName(contact) {
    return getContactDisplayName(contact, this.getContactMetadata(contact?.id));
  },

  getPriorityInfo(contactId) {
    return getPriorityInfo(this.getContactMetadata(contactId));
  },

  getChecklistSummary(contactId) {
    return getChecklistSummary(this.getContactMetadata(contactId));
  },

  getTimezoneInfo(contactId, now = Date.now()) {
    return getTimezoneInfo(this.getContactMetadata(contactId), now);
  },

  formatChecklistForTextarea(contactId) {
    const checklist = this.getContactMetadata(contactId).checklist || [];
    return checklist
      .map((item) => `- [${item.done ? 'x' : ' '}] ${item.text}`)
      .join('\n');
  },

  getReplyState(contactId, now = Date.now()) {
    const contact = this.contacts.find((entry) => entry.id === contactId) || {
      id: contactId,
    };
    return getReplyState({
      contact,
      messages: this.messages.get(contactId) || [],
      drafts: this.drafts,
      metadata: this.getContactMetadata(contactId),
      contactId,
      now,
    });
  },

  getReplySummary(contactId, now = Date.now()) {
    const replyState = this.getReplyState(contactId, now);
    if (replyState === 'drafting') return 'Draft reply saved';
    if (replyState === 'needs-reply') return 'Awaiting your reply';
    if (replyState === 'waiting') return 'Waiting for their response';
    return '';
  },

  getReminderSummary(contactId, now = Date.now()) {
    const metadata = this.getContactMetadata(contactId);
    const status = getReminderStatus(metadata, now);
    if (status === 'none') return '';

    const reminderText = String(metadata.reminderText || '').trim();
    const reminderAt = Number(metadata.reminderAt || 0);
    const when = reminderAt ? this.formatMetadataTime(reminderAt) : 'soon';
    const prefix = status === 'due' ? 'Reminder due' : 'Reminder';
    return reminderText
      ? `${prefix}: ${reminderText} · ${when}`
      : `${prefix} · ${when}`;
  },

  getSnoozeSummary(contactId, now = Date.now()) {
    const metadata = this.getContactMetadata(contactId);
    if (!isContactSnoozed(metadata, now)) return '';
    return `Snoozed until ${this.formatMetadataTime(Number(metadata.snoozedUntil))}`;
  },
};
