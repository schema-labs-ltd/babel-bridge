// Inbox filters, visitor dashboard and the demo workspace.

import {
  buildVisitorDashboard,
  createDemoWorkspace,
  getContactActionSnapshot,
  normalizeInboxFilters,
  getDraftText,
  getReminderStatus,
  getVisibleContacts,
} from '../app-state.js?v=20260912-conversation-translation';

export const inboxMethods = {
  syncInboxControls() {
    const controlsAvailable = this.inboxFilterControlsAvailable();
    const normalizedFilters = normalizeInboxFilters(this.contactFilters, {
      controlsAvailable,
    });
    const filtersChanged = Object.keys(this.contactFilters).some(
      (key) => this.contactFilters[key] !== normalizedFilters[key],
    );
    if (filtersChanged) {
      this.contactFilters = normalizedFilters;
      this.persistInboxPreferences();
    }

    const searchInput = document.getElementById('inbox-search-input');
    if (searchInput && searchInput.value !== this.sidebarSearchQuery) {
      searchInput.value = this.sidebarSearchQuery;
    }

    document.querySelectorAll('[data-inbox-filter]').forEach((button) => {
      const key = button.dataset.inboxFilter;
      button.classList.toggle('active', Boolean(this.contactFilters[key]));
    });
  },

  inboxFilterControlsAvailable() {
    const filterRow = document.querySelector('.inbox-filter-row');
    return Boolean(
      filterRow && window.getComputedStyle(filterRow).display !== 'none',
    );
  },

  setInboxSearchQuery(value) {
    this.sidebarSearchQuery = String(value || '').trimStart();
    this.persistInboxPreferences();
    this.renderContacts();
  },

  toggleInboxFilter(filterName) {
    if (!(filterName in this.contactFilters)) return;
    this.contactFilters[filterName] = !this.contactFilters[filterName];
    this.persistInboxPreferences();
    this.syncInboxControls();
    this.renderContacts();
  },

  clearInboxFilters() {
    this.sidebarSearchQuery = '';
    for (const key of Object.keys(this.contactFilters)) {
      this.contactFilters[key] = false;
    }
    this.persistInboxPreferences();
    this.syncInboxControls();
    this.renderContacts();
  },

  buildMessagePreviewLookup() {
    const previewByContact = {};
    for (const contact of this.contacts) {
      const messages = this.messages.get(contact.id) || [];
      const lastMessage = this.getLastDisplayableMessage(messages);
      if (lastMessage) {
        previewByContact[contact.id] = this.getMessagePreview(lastMessage);
      } else if (contact.lastMessagePreview) {
        previewByContact[contact.id] = contact.lastMessagePreview;
      }
    }
    return previewByContact;
  },

  getFilteredContacts() {
    const now = Date.now();
    const messagesByContact = Object.fromEntries(this.messages.entries());
    const effectiveFilters = normalizeInboxFilters(this.contactFilters, {
      controlsAvailable: this.inboxFilterControlsAvailable(),
    });
    return getVisibleContacts({
      contacts: this.contacts,
      drafts: this.drafts,
      searchQuery: this.sidebarSearchQuery,
      filters: effectiveFilters,
      messagePreviewByContact: this.buildMessagePreviewLookup(),
      metadataByContact: this.contactMetadata,
      messagesByContact,
      now,
    }).sort((a, b) => {
      const aMeta = this.getContactMetadata(a.id);
      const bMeta = this.getContactMetadata(b.id);
      const aPinned = a.pinnedAt != null;
      const bPinned = b.pinnedAt != null;
      if (aPinned && !bPinned) return -1;
      if (!aPinned && bPinned) return 1;
      if (aPinned && bPinned) return a.pinnedAt - b.pinnedAt;

      const aPriority = this.getPriorityInfo(a.id);
      const bPriority = this.getPriorityInfo(b.id);
      if (aPriority.rank !== bPriority.rank)
        return aPriority.rank - bPriority.rank;

      const aChecklist = this.getChecklistSummary(a.id);
      const bChecklist = this.getChecklistSummary(b.id);
      if (aChecklist.open > 0 && bChecklist.open === 0) return -1;
      if (aChecklist.open === 0 && bChecklist.open > 0) return 1;

      const aReminder = getReminderStatus(aMeta, now);
      const bReminder = getReminderStatus(bMeta, now);
      if (aReminder === 'due' && bReminder !== 'due') return -1;
      if (aReminder !== 'due' && bReminder === 'due') return 1;
      if (aReminder === 'upcoming' && bReminder === 'none') return -1;
      if (aReminder === 'none' && bReminder === 'upcoming') return 1;

      const replyRank = {
        drafting: 0,
        'needs-reply': 1,
        waiting: 2,
        idle: 3,
        snoozed: 4,
      };
      const aReplyRank = replyRank[this.getReplyState(a.id, now)] ?? 99;
      const bReplyRank = replyRank[this.getReplyState(b.id, now)] ?? 99;
      if (aReplyRank !== bReplyRank) return aReplyRank - bReplyRank;

      const aDraft = getDraftText(this.drafts, a.id);
      const bDraft = getDraftText(this.drafts, b.id);
      if (aDraft && !bDraft) return -1;
      if (!aDraft && bDraft) return 1;

      return (b.lastMessageTime || 0) - (a.lastMessageTime || 0);
    });
  },

  getContactActionSnapshot(contactId, now = Date.now()) {
    const contact = this.contacts.find((entry) => entry.id === contactId) || {
      id: contactId,
    };
    return getContactActionSnapshot({
      contact,
      drafts: this.drafts,
      metadata: this.getContactMetadata(contactId),
      messages: this.messages.get(contactId) || [],
      now,
    });
  },

  getVisitorDashboardData(now = Date.now()) {
    return buildVisitorDashboard({
      contacts: this.contacts,
      drafts: this.drafts,
      metadataByContact: this.contactMetadata,
      messagesByContact: Object.fromEntries(this.messages.entries()),
      now,
    });
  },

  applyInboxPreset(preset = 'all') {
    this.sidebarSearchQuery = '';
    for (const key of Object.keys(this.contactFilters)) {
      this.contactFilters[key] = false;
    }

    const filterKeyByPreset = {
      needsReply: 'needsReplyOnly',
      dueReminders: 'dueRemindersOnly',
      drafts: 'draftsOnly',
      openTasks: 'tasksOnly',
      pinned: 'pinnedOnly',
    };

    const filterKey = filterKeyByPreset[preset];
    if (filterKey) {
      this.contactFilters[filterKey] = true;
    }

    this.persistInboxPreferences();
    this.syncInboxControls();
    this.renderContacts();
  },

  renderVisitorDashboard() {
    const container = document.getElementById('visitor-dashboard');
    if (container) {
      container.classList.add('hidden');
    }
    return;
  },

  startDemoWorkspace() {
    const demo = createDemoWorkspace();
    this.demoMode = true;
    this.connected = true;

    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = null;
    }

    this.contacts = demo.contacts.map((contact) => ({ ...contact }));
    this.messages = new Map(
      Object.entries(demo.messagesByContact).map(([contactId, messages]) => [
        contactId,
        messages.map((message) => ({
          ...message,
          content: { ...(message.content || {}) },
        })),
      ]),
    );
    this.contactMetadata = {
      ...this.contactMetadata,
      ...demo.metadataByContact,
    };
    this.drafts = {
      ...demo.drafts,
      ...this.drafts,
    };
    this.quickReplies =
      this.quickReplies.length > 0 ? this.quickReplies : demo.quickReplies;
    this.globalUsage = demo.globalUsage;
    this.currentContactId = null;

    document.getElementById('password-overlay')?.classList.add('hidden');
    document.getElementById('qr-overlay')?.classList.add('hidden');
    document.getElementById('connecting-overlay')?.classList.add('hidden');
    document.getElementById('main-container')?.classList.remove('hidden');
    document.getElementById('no-chat-selected')?.classList.remove('hidden');
    document.getElementById('chat-view')?.classList.add('hidden');
    document.getElementById('main-container')?.classList.remove('chat-open');

    const userName = document.getElementById('user-name');
    const userPhone = document.getElementById('user-phone');
    if (userName) userName.textContent = 'Demo Visitor';
    if (userPhone) userPhone.textContent = 'Sample inbox';

    this.persistDrafts();
    this.saveStoredArray(this.quickRepliesStorageKey, this.quickReplies);
    this.syncInboxControls();
    this.renderContacts();
    this.updateGlobalUsageDisplay();
    this.renderQuickReplies();
    this.renderVisitorDashboard();
    this.updateWorkspaceUI();
  },
};
