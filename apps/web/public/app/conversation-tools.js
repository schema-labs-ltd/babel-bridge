// Conversation menu, quick replies, starred messages and chat search.

import {
  countMatchingMessages,
  filterMessagesByQuery,
  getUntranslatedIncomingMessages,
  getDraftText,
  removeQuickReply,
  toggleStarredMessage,
  upsertQuickReply,
} from '../app-state.js?v=20260912-conversation-translation';

export const conversationToolsMethods = {
  toggleConversationMenu() {
    const menu = document.getElementById('chat-actions-menu');
    if (!menu) return;

    const shouldOpen = menu.classList.contains('hidden');
    this.closeConversationMenu();
    if (!shouldOpen) return;

    menu.classList.remove('hidden');
    this.updateConversationMenuUI();
  },

  closeConversationMenu() {
    document.getElementById('chat-actions-menu')?.classList.add('hidden');
    this.updateConversationMenuUI();
  },

  updateConversationMenuUI() {
    const button = document.getElementById('chat-menu-button');
    const menu = document.getElementById('chat-actions-menu');
    const searchButton = document.getElementById('chat-search-toggle');
    const starredButton = document.getElementById('chat-starred-toggle');
    const translateAllButton = document.getElementById('chat-translate-all');
    const searchBarVisible = !document
      .getElementById('chat-search-bar')
      ?.classList.contains('hidden');

    if (button && menu) {
      button.setAttribute(
        'aria-expanded',
        String(!menu.classList.contains('hidden')),
      );
      button.classList.toggle('active', !menu.classList.contains('hidden'));
    }

    if (searchButton) {
      searchButton.classList.toggle(
        'active',
        searchBarVisible || Boolean(this.messageSearchQuery),
      );
      searchButton.textContent = searchBarVisible
        ? 'Hide search'
        : 'Search messages';
    }

    if (starredButton) {
      starredButton.classList.toggle('active', this.starredOnly);
      starredButton.textContent = this.starredOnly
        ? 'Show all messages'
        : 'Show starred messages only';
    }

    if (translateAllButton) {
      const count =
        this.getUntranslatedIncomingMessagesForCurrentConversation().length;
      translateAllButton.disabled = count === 0;
      translateAllButton.textContent =
        count === 0
          ? 'All incoming translated'
          : `Translate ${count} incoming message${count === 1 ? '' : 's'}`;
    }
  },

  getUntranslatedIncomingMessagesForCurrentConversation() {
    return getUntranslatedIncomingMessages(
      this.messages.get(this.currentContactId) || [],
    );
  },

  renderQuickReplies() {
    const bar = document.querySelector('.quick-replies-bar');
    const container = document.getElementById('quick-replies-list');
    const saveButton = document.getElementById('quick-reply-save');
    const draftText = this.currentContactId
      ? getDraftText(this.drafts, this.currentContactId)
      : '';
    const hasReplies =
      Array.isArray(this.quickReplies) && this.quickReplies.length > 0;
    const shouldShowBar =
      Boolean(this.currentContactId) && (hasReplies || Boolean(draftText));

    if (saveButton) {
      saveButton.disabled = !this.currentContactId || !draftText;
      saveButton.textContent = hasReplies ? 'Save draft' : 'Save this draft';
    }

    if (bar) {
      bar.classList.toggle('hidden', !shouldShowBar);
    }

    if (!container) return;

    if (!shouldShowBar) {
      container.innerHTML = '';
      return;
    }

    if (!hasReplies) {
      container.innerHTML =
        '<span class="quick-replies-empty">Saved replies will appear here once you keep one.</span>';
      return;
    }

    container.innerHTML = this.quickReplies
      .map(
        (reply) => `
      <button class="quick-reply-chip" type="button" data-quick-reply-id="${reply.id}" title="Insert saved reply">
        <span class="quick-reply-text">${this.escapeHtml(reply.text)}</span>
        <span class="quick-reply-remove" data-quick-reply-remove="${reply.id}" title="Remove saved reply">×</span>
      </button>
    `,
      )
      .join('');
  },

  saveCurrentDraftAsQuickReply() {
    if (!this.currentContactId) return;
    const draftText = getDraftText(this.drafts, this.currentContactId);
    if (!draftText) return;

    this.quickReplies = upsertQuickReply(this.quickReplies, draftText);
    this.persistQuickReplies();
    this.renderQuickReplies();
  },

  applyQuickReply(quickReplyId) {
    const quickReply = (this.quickReplies || []).find(
      (entry) => entry.id === quickReplyId,
    );
    if (!quickReply) return;

    const input = document.getElementById('message-input');
    if (!input) return;

    input.value = quickReply.text;
    this.autoResizeTextarea(input);
    this.updateSendButton();
    this.handleDraftInput();
    input.focus();
  },

  deleteQuickReply(quickReplyId) {
    this.quickReplies = removeQuickReply(this.quickReplies, quickReplyId);
    this.persistQuickReplies();
    this.renderQuickReplies();
  },

  toggleMessageStar(messageId) {
    if (!this.currentContactId) return;

    const messages = this.messages.get(this.currentContactId) || [];
    const message = messages.find((entry) => entry.id === messageId);
    if (!message) return;

    this.starredMessages = toggleStarredMessage(this.starredMessages, message);
    this.persistStarredMessages();
    this.updateStarredToggleUI();

    const visibleMessages = this.getVisibleMessagesForCurrentConversation();
    this.renderMessages(visibleMessages);
  },

  async toggleStarredOnly() {
    this.starredOnly = !this.starredOnly;
    if (this.starredOnly) {
      await this.ensureFullConversationLoaded();
    }
    this.updateStarredToggleUI();
    this.refreshCurrentConversationView();
  },

  toggleChatSearch() {
    const searchBar = document.getElementById('chat-search-bar');
    const input = document.getElementById('chat-search-input');
    if (!searchBar || !input) return;

    const shouldShow = searchBar.classList.contains('hidden');
    searchBar.classList.toggle('hidden', !shouldShow);

    if (shouldShow) {
      input.focus();
      input.select();
    } else {
      this.messageSearchQuery = '';
      input.value = '';
      this.updateChatSearchUI();
      this.refreshCurrentConversationView();
    }

    this.updateConversationMenuUI();
  },

  clearChatSearch() {
    this.messageSearchQuery = '';
    const input = document.getElementById('chat-search-input');
    if (input) input.value = '';
    this.updateChatSearchUI();
    this.refreshCurrentConversationView();
    this.updateConversationMenuUI();
  },

  async setChatSearchQuery(query) {
    this.messageSearchQuery = query || '';
    if (this.messageSearchQuery) {
      await this.ensureFullConversationLoaded();
    }
    this.updateChatSearchUI();
    this.refreshCurrentConversationView();
    this.updateConversationMenuUI();
  },

  updateChatSearchUI() {
    const countEl = document.getElementById('chat-search-count');
    this.updateConversationMenuUI();
    if (!countEl) return;

    const visibleMessages = this.getVisibleMessagesForCurrentConversation();
    const count = countMatchingMessages(
      (this.messages.get(this.currentContactId) || []).filter((message) =>
        this.isDisplayableMessage(message),
      ),
      this.messageSearchQuery,
      { starredOnly: this.starredOnly, starredLookup: this.starredMessages },
    );

    if (!this.currentContactId) {
      countEl.textContent = 'No chat selected';
    } else if (!this.messageSearchQuery) {
      countEl.textContent = this.starredOnly
        ? `${visibleMessages.length} starred shown`
        : `${visibleMessages.length} messages`;
    } else {
      countEl.textContent = `${count} match${count === 1 ? '' : 'es'}`;
    }
  },

  updateStarredToggleUI() {
    const button = document.getElementById('chat-starred-toggle');
    if (button) {
      button.title = this.starredOnly
        ? 'Show all messages'
        : 'Show starred messages only';
    }
    this.updateConversationMenuUI();
  },

  getVisibleMessagesForCurrentConversation() {
    const messages = (
      this.topics?.selectedId
        ? this.topics.messages
        : this.messages.get(this.currentContactId) || []
    ).filter((message) => this.isDisplayableMessage(message));
    return filterMessagesByQuery(messages, this.messageSearchQuery, {
      starredOnly: this.starredOnly,
      starredLookup: this.starredMessages,
    });
  },

  refreshCurrentConversationView() {
    if (!this.currentContactId) return;

    this.renderMessages(this.getVisibleMessagesForCurrentConversation());
    this.updateChatSearchUI();
  },
};
