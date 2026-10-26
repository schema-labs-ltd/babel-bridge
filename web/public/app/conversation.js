// Opening a conversation and loading, paginating and rendering its messages.

export const conversationMethods = {
  // Select a contact
  async selectContact(contactId) {
    try {
      this.closeCommandPalette();
      this.currentContactId = contactId;
      this.topics?.selectChat();
      void this.conversationSettingsClient
        .load(contactId)
        .catch((error) => console.warn(error.message));
      this.messageSearchQuery = '';
      this.starredOnly = false;

      // Clear any pending reply from previous chat
      this.clearReply();

      const contact = this.contacts.find((c) => c.id === contactId);

      // Update UI
      document.getElementById('no-chat-selected').classList.add('hidden');
      document.getElementById('chat-view').classList.remove('hidden');

      // Add chat-open class for mobile view
      document.getElementById('main-container').classList.add('chat-open');

      // Push history state for mobile back button
      if (this.isMobile()) {
        history.pushState(
          { chat: contactId },
          '',
          `?chat=${encodeURIComponent(contactId)}`,
        );
      }

      // Update chat header
      if (contact) {
        const displayName = this.getContactDisplayName(contact);
        document.getElementById('chat-name').textContent = displayName;

        const initial = (displayName || '?').charAt(0).toUpperCase();
        // Get avatar container - it's the .avatar element in .chat-header
        const avatarContainer = document.querySelector('.chat-header .avatar');
        const avatarUrl = this.avatarCache.get(contactId);

        if (avatarContainer) {
          if (avatarUrl) {
            avatarContainer.innerHTML = this.renderAvatarImage(
              avatarUrl,
              initial,
            );
          } else {
            avatarContainer.innerHTML = `<span>${this.escapeHtml(initial)}</span>`;
            // Fetch avatar if not cached
            this.fetchAvatar(contactId);
          }
        }
      }

      const cachedUsage = this.conversationUsageCache.get(contactId);
      this.updateConversationUsageDisplay(cachedUsage || { costUsd: 0 });

      const cachedMessages = this.messages.get(contactId);
      if (cachedMessages && cachedMessages.length > 0) {
        const normalizedMessages = this.normalizeLoadedMessages(
          contactId,
          cachedMessages,
        );
        this.messages.set(contactId, normalizedMessages);
        const visibleMessages =
          this.getRecentVisibleMessages(normalizedMessages);
        if (visibleMessages.length !== normalizedMessages.length) {
          this.messages.set(contactId, visibleMessages);
          this.messagesHasMore.set(contactId, true);
        }
        this.renderMessages(visibleMessages);
        this.setupScrollHandler();
      } else {
        this.showMessagesLoading();
        await this.loadMessages(contactId);
      }

      this.scheduleConversationUsageFetch(contactId);

      // Re-render contacts to update active state and unread
      this.scheduleRenderContacts();
      this.restoreDraftForCurrentContact();
      this.renderQuickReplies();
      this.closeConversationMenu();
      document.getElementById('chat-search-bar')?.classList.add('hidden');
      const chatSearchInput = document.getElementById('chat-search-input');
      if (chatSearchInput) chatSearchInput.value = '';
      this.updateChatSearchUI();
      this.updateStarredToggleUI();
      this.updateChatHeaderNote();
      this.updateWorkspaceUI();
      this.renderComposerAssist();

      // Update send button state and focus input (only on desktop)
      this.updateSendButton();
      if (!this.isMobile()) {
        document.getElementById('message-input').focus();
      }
    } catch (err) {
      console.error('Error selecting contact:', err);
    }
  },

  showMessagesLoading() {
    const container = document.getElementById('messages-list');
    if (!container) return;

    container.innerHTML = `
      <div class="messages-loading-state">
        <div class="loading-spinner"></div>
        <span>Loading messages...</span>
      </div>
    `;
  },

  getRecentVisibleMessages(messages) {
    if (!messages || messages.length <= this.initialMessageLimit) {
      return messages || [];
    }

    return messages.slice(-this.initialMessageLimit);
  },

  trimCachedMessages(contactId, preserveVisible = false) {
    const messages = this.messages.get(contactId);
    if (!messages || messages.length <= this.messageCacheLimit) {
      return;
    }

    if (preserveVisible && this.currentContactId === contactId) {
      return;
    }

    this.messages.set(contactId, messages.slice(-this.messageCacheLimit));
    this.messagesHasMore.set(contactId, true);
  },

  scheduleDeferredWork(callback, delay = 80) {
    const run = () => window.setTimeout(callback, delay);

    if (typeof window.requestIdleCallback === 'function') {
      window.requestIdleCallback(callback, { timeout: delay + 200 });
      return;
    }

    if (typeof window.requestAnimationFrame === 'function') {
      window.requestAnimationFrame(run);
      return;
    }

    window.setTimeout(callback, delay);
  },

  scheduleConversationUsageFetch(contactId) {
    if (this.conversationUsageTimer) {
      clearTimeout(this.conversationUsageTimer);
      this.conversationUsageTimer = null;
    }

    this.conversationUsageTimer = window.setTimeout(() => {
      this.conversationUsageTimer = null;
      this.scheduleDeferredWork(() => {
        if (contactId === this.currentContactId) {
          this.fetchConversationUsage(contactId);
        }
      }, 40);
    }, 0);
  },

  scheduleLinkPreviewLoad(root = document) {
    if (this.linkPreviewLoadTimer) {
      clearTimeout(this.linkPreviewLoadTimer);
      this.linkPreviewLoadTimer = null;
    }

    const activeContactId = this.currentContactId;
    this.linkPreviewLoadTimer = window.setTimeout(() => {
      this.linkPreviewLoadTimer = null;
      this.scheduleDeferredWork(() => {
        if (activeContactId !== this.currentContactId) {
          return;
        }

        const scope = root && root.isConnected ? root : document;
        this.loadVisibleLinkPreviews(scope);
      });
    }, 0);
  },

  // Load messages for a contact (initial load - most recent page)
  async loadMessages(contactId) {
    try {
      this.messagesLoading.set(contactId, true);
      if (this.demoMode) {
        this.messagesHasMore.set(contactId, false);
        this.fullyLoadedContacts.add(contactId);
        if (contactId === this.currentContactId) {
          this.renderMessages(this.getVisibleMessagesForCurrentConversation());
        }
        return;
      }

      const response = await this.apiFetch(
        `/api/messages/${encodeURIComponent(contactId)}?limit=${this.initialMessageLimit}`,
        {
          headers: this.getAuthHeaders(),
        },
      );
      const data = await response.json();

      // Handle both old format (array) and new format (object with messages/hasMore)
      const messages = Array.isArray(data) ? data : data.messages;
      const hasMore = Array.isArray(data) ? false : data.hasMore;

      const normalizedMessages = this.normalizeLoadedMessages(
        contactId,
        messages,
      );

      this.messages.set(contactId, normalizedMessages);
      this.messagesHasMore.set(contactId, hasMore);
      if (hasMore) {
        this.fullyLoadedContacts.delete(contactId);
      } else {
        this.fullyLoadedContacts.add(contactId);
      }
      if (contactId === this.currentContactId) {
        this.renderMessages(this.getVisibleMessagesForCurrentConversation());
      }

      // Set up scroll handler for infinite scroll
      if (contactId === this.currentContactId) {
        this.setupScrollHandler();
      }
    } catch (err) {
      console.error('Failed to load messages:', err);
    } finally {
      this.messagesLoading.set(contactId, false);
    }
  },

  async ensureFullConversationLoaded(contactId = this.currentContactId) {
    if (!contactId || this.fullyLoadedContacts.has(contactId)) {
      return;
    }

    try {
      this.messagesLoading.set(contactId, true);
      const response = await this.apiFetch(
        `/api/messages/${encodeURIComponent(contactId)}?limit=0`,
        {
          headers: this.getAuthHeaders(),
        },
      );
      const data = await response.json();
      const messages = Array.isArray(data) ? data : data.messages;

      const normalizedMessages = this.normalizeLoadedMessages(
        contactId,
        messages,
      );

      this.messages.set(contactId, normalizedMessages);
      this.messagesHasMore.set(contactId, false);
      this.fullyLoadedContacts.add(contactId);
    } catch (err) {
      console.error('Failed to load the full conversation:', err);
    } finally {
      this.messagesLoading.set(contactId, false);
    }
  },

  // Load older messages (for infinite scroll)
  async loadOlderMessages(contactId) {
    if (this.topics?.selectedId) {
      await this.topics.loadOlder();
      return;
    }
    // Don't load if already loading or no more messages
    if (this.messagesLoading.get(contactId)) return;
    if (!this.messagesHasMore.get(contactId)) return;

    const existingMessages = this.messages.get(contactId) || [];
    if (existingMessages.length === 0) return;

    // Get the oldest message cursor
    const oldestMessage = existingMessages[0];
    const oldestTimestamp = oldestMessage.timestamp;
    const olderParams = new URLSearchParams({
      before: String(oldestTimestamp),
      limit: '50',
    });
    if (oldestMessage.id) {
      olderParams.set('beforeId', oldestMessage.id);
    }

    try {
      this.messagesLoading.set(contactId, true);
      this.showLoadingIndicator();

      const response = await this.apiFetch(
        `/api/messages/${encodeURIComponent(contactId)}?${olderParams.toString()}`,
        {
          headers: this.getAuthHeaders(),
        },
      );
      const data = await response.json();

      const olderMessages = Array.isArray(data) ? data : data.messages;
      const hasMore = Array.isArray(data) ? false : data.hasMore;

      const normalizedOlderMessages = this.normalizeLoadedMessages(
        contactId,
        olderMessages,
        existingMessages,
      );

      if (normalizedOlderMessages.length > 0) {
        // Prepend older messages
        const allMessages = [...normalizedOlderMessages, ...existingMessages];
        this.messages.set(contactId, allMessages);
        this.messagesHasMore.set(contactId, hasMore);
        if (!hasMore) {
          this.fullyLoadedContacts.add(contactId);
        }

        // Re-render and maintain scroll position
        if (this.messageSearchQuery || this.starredOnly) {
          this.refreshCurrentConversationView();
        } else {
          this.prependMessages(normalizedOlderMessages);
        }
      } else if (olderMessages.length > 0) {
        this.messagesHasMore.set(contactId, hasMore);
        if (!hasMore) {
          this.fullyLoadedContacts.add(contactId);
        }
      } else {
        this.messagesHasMore.set(contactId, false);
        this.fullyLoadedContacts.add(contactId);
      }
    } catch (err) {
      console.error('Failed to load older messages:', err);
    } finally {
      this.messagesLoading.set(contactId, false);
      this.hideLoadingIndicator();
    }
  },

  // Set up scroll handler for infinite scroll
  setupScrollHandler() {
    const container = document.getElementById('messages-list');
    if (!container) return;

    // Remove existing handler if any
    container.removeEventListener('scroll', this.handleMessagesScroll);

    // Add scroll handler
    this.handleMessagesScroll = () => {
      // Load more when scrolled near the top (within 200px)
      if (container.scrollTop < 200 && this.currentContactId) {
        this.loadOlderMessages(this.currentContactId);
      }
      this.scheduleLinkPreviewLoad(container);
    };

    container.addEventListener('scroll', this.handleMessagesScroll);
  },

  // Show loading indicator at top of messages
  showLoadingIndicator() {
    const container = document.getElementById('messages-list');
    if (!container) return;

    // Remove existing indicator
    const existing = container.querySelector('.loading-more-indicator');
    if (existing) existing.remove();

    // Add new indicator at top
    const indicator = document.createElement('div');
    indicator.className = 'loading-more-indicator';
    indicator.innerHTML =
      '<div class="loading-spinner"></div><span>Loading older messages...</span>';
    container.insertBefore(indicator, container.firstChild);
  },

  // Hide loading indicator
  hideLoadingIndicator() {
    const indicator = document.querySelector('.loading-more-indicator');
    if (indicator) indicator.remove();
  },

  // Prepend older messages to the list (maintaining scroll position)
  prependMessages(olderMessages) {
    const container = document.getElementById('messages-list');
    const displayMessages = (olderMessages || []).filter((message) =>
      this.isDisplayableMessage(message),
    );
    if (!container || displayMessages.length === 0) return;

    // Remember scroll position from bottom
    const scrollHeightBefore = container.scrollHeight;
    const scrollTopBefore = container.scrollTop;

    // Build HTML for older messages
    let html = '';
    let lastDate = null;

    for (const message of displayMessages) {
      const messageDate = new Date(message.timestamp).toDateString();
      if (messageDate !== lastDate) {
        html += `<div class="date-separator"><span>${this.formatDate(message.timestamp)}</span></div>`;
        lastDate = messageDate;
      }
      html += this.renderMessage(message);
    }

    // Remove loading indicator and first date separator if it will be duplicated
    const firstDateSep = container.querySelector('.date-separator');
    if (firstDateSep && displayMessages.length > 0) {
      // Check if dates match (approximately)
      if (
        firstDateSep.textContent.includes(
          this.formatDate(
            displayMessages[displayMessages.length - 1].timestamp,
          ).split(',')[0],
        )
      ) {
        firstDateSep.remove();
      }
    }

    // Insert at the beginning
    container.insertAdjacentHTML('afterbegin', html);

    // Restore scroll position (keep viewing the same messages)
    const scrollHeightAfter = container.scrollHeight;
    container.scrollTop =
      scrollTopBefore + (scrollHeightAfter - scrollHeightBefore);

    // Load link previews after the DOM settles
    this.scheduleLinkPreviewLoad(container);
  },

  // Render messages
  renderMessages(messages) {
    const container = document.getElementById('messages-list');
    if (this.topics?.selectedId)
      messages = this.getVisibleMessagesForCurrentConversation();
    const displayMessages = (messages || []).filter((message) =>
      this.isDisplayableMessage(message),
    );

    if (displayMessages.length === 0) {
      const emptyMessage = this.messageSearchQuery
        ? 'No messages match your search yet'
        : this.starredOnly
          ? 'No starred messages in this conversation yet'
          : 'No messages yet';
      container.innerHTML = `<div class="empty-state"><p>${this.escapeHtml(emptyMessage)}</p></div>`;
      this.updateChatSearchUI();
      this.renderComposerAssist();
      return;
    }

    let html = '';
    let lastDate = null;

    for (const message of displayMessages) {
      // Add date separator if needed
      const messageDate = new Date(message.timestamp).toDateString();
      if (messageDate !== lastDate) {
        html += `<div class="date-separator"><span>${this.formatDate(message.timestamp)}</span></div>`;
        lastDate = messageDate;
      }

      html += this.renderMessage(message);
    }

    container.innerHTML = html;
    this.scrollToBottom();

    // Defer previews until after the first paint.
    this.scheduleLinkPreviewLoad(container);
    this.updateChatSearchUI();
    this.renderComposerAssist();
  },

  // Load link previews for messages near the viewport.
  loadVisibleLinkPreviews(root = document) {
    const containers = root.querySelectorAll(
      '.link-previews-container[data-urls]',
    );
    containers.forEach((container) => {
      if (
        container.dataset.previewLoaded === 'true' ||
        container.dataset.previewLoading === 'true'
      ) {
        return;
      }

      const rect = container.getBoundingClientRect();
      const viewportHeight =
        window.innerHeight || document.documentElement.clientHeight || 0;
      const isNearViewport =
        rect.bottom >= -300 && rect.top <= viewportHeight + 600;
      if (!isNearViewport) {
        return;
      }

      try {
        const urls = JSON.parse(container.dataset.urls);
        if (urls && urls.length > 0) {
          const messageEl = container.closest('.message');
          if (messageEl) {
            this.loadLinkPreviews(messageEl, urls);
          }
        }
      } catch (e) {
        console.error('Failed to parse URLs:', e);
      }
    });
  },
};
