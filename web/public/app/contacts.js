// Contact list, avatars, previews and viewed-message tracking.

import { ViewedMessageTracker, isMessageVisible } from '../read-receipts.js';
import { getDraftPreview } from '../app-state.js?v=20260912-conversation-translation';

export const contactsMethods = {
  // Load contacts from server
  async loadContacts() {
    try {
      const response = await this.apiFetch('/api/contacts', {
        headers: this.getAuthHeaders(),
      });
      this.contacts = (await response.json()).map((contact) =>
        this.applyStoredContactMetadata(contact),
      );
      this.syncInboxControls();
      this.renderContacts();
      return true;
    } catch (err) {
      console.error('Failed to load contacts:', err);
      return false;
    }
  },

  scheduleRenderContacts() {
    if (this.contactsRenderTimer) return;

    this.contactsRenderTimer = setTimeout(() => {
      this.contactsRenderTimer = null;
      this.renderContacts();
    }, 50);
  },

  // Fetch avatar for a contact
  async fetchAvatar(jid) {
    if (this.demoMode) return;

    // Skip if already cached or being fetched
    if (this.avatarCache.has(jid) || this.avatarFetching.has(jid)) {
      return;
    }

    this.avatarFetching.add(jid);

    try {
      const response = await this.apiFetch(
        `/api/avatar/${encodeURIComponent(jid)}`,
        {
          headers: this.getAuthHeaders(),
        },
      );
      const data = await response.json();

      if (data.url) {
        this.avatarCache.set(jid, data.url);
        // Update any visible avatars for this contact
        this.updateAvatarDisplay(jid, data.url);
      }
    } catch (err) {
      console.error('Failed to fetch avatar:', err);
    } finally {
      this.avatarFetching.delete(jid);
    }
  },

  // Update avatar display for a specific JID
  updateAvatarDisplay(jid, url) {
    const initial = this.getInitial(jid);

    // Update in contacts list
    const contactItem = document.querySelector(
      `.contact-item[data-contact-id="${jid}"] .avatar`,
    );
    if (contactItem) {
      contactItem.innerHTML = this.renderAvatarImage(url, initial);
    }

    // Update in chat header if this is the current contact
    if (this.currentContactId === jid) {
      const chatAvatar = document.querySelector('.chat-header .avatar');
      if (chatAvatar) {
        chatAvatar.innerHTML = this.renderAvatarImage(url, initial);
      }
    }
  },

  // Get initial for a contact by JID
  getInitial(jid) {
    const contact = this.contacts.find((c) => c.id === jid);
    return (contact?.name || contact?.phone || '?').charAt(0).toUpperCase();
  },

  renderAvatarImage(url, initial) {
    return `<img src="${this.escapeHtml(url)}" alt="" data-avatar-fallback="${this.escapeHtml(initial)}">`;
  },

  // Render contacts list
  renderContacts() {
    const container = document.getElementById('contacts-list');
    this.renderVisitorDashboard();

    if (this.contacts.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <p>No conversations yet</p>
          <p class="hint">Messages will appear here</p>
        </div>
      `;
      return;
    }

    this.syncInboxControls();
    const sorted = this.getFilteredContacts();
    if (sorted.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <p>No conversations match this inbox view</p>
          <p class="hint">Try a different search or clear a few filters.</p>
          <button id="inbox-reset-empty" class="sidebar-reset-button">Reset inbox filters</button>
        </div>
      `;
      document
        .getElementById('inbox-reset-empty')
        ?.addEventListener('click', () => this.clearInboxFilters());
      return;
    }

    container.innerHTML = sorted
      .map((contact) => {
        const isGroup = contact.type === 'group';
        const isPinned = contact.pinnedAt != null;
        const displayName = this.getContactDisplayName(contact);
        const initial = (displayName || '?').charAt(0).toUpperCase();
        const time = this.formatTime(contact.lastMessageTime);
        const isActive = contact.id === this.currentContactId;
        const unread =
          contact.unreadCount > 0
            ? `<span class="unread-badge">${contact.unreadCount}</span>`
            : '';

        // Get last message preview - prefer saved drafts, then cached messages, then backend preview
        const messages = this.messages.get(contact.id) || [];
        const lastMessage = this.getLastDisplayableMessage(messages);
        const draftPreview = getDraftPreview(this.drafts, contact.id);
        let preview = '';
        let previewClass = draftPreview
          ? 'preview-text draft-preview'
          : 'preview-text';

        if (draftPreview) {
          preview = draftPreview;
        } else if (lastMessage) {
          // Use locally cached message for preview
          preview = this.getMessagePreview(lastMessage);
          // For groups, prefix with sender name if not from me
          if (isGroup && !lastMessage.isFromMe && lastMessage.senderName) {
            const senderFirst = lastMessage.senderName.split(' ')[0];
            if (!preview.startsWith('You: ')) {
              preview = `${senderFirst}: ${preview}`;
            }
          }
        } else if (contact.lastMessagePreview) {
          // Fall back to server-provided preview (used before messages are loaded)
          preview = contact.lastMessagePreview;
        }

        // Check for cached avatar
        const avatarUrl = this.avatarCache.get(contact.id);
        const avatarContent = avatarUrl
          ? this.renderAvatarImage(avatarUrl, initial)
          : `<span>${this.escapeHtml(initial)}</span>`;

        // Group indicator (fold mark in corner)
        const groupIndicator = isGroup
          ? '<div class="group-indicator"></div>'
          : '';

        // Pin button (shows on hover, filled when pinned)
        const pinButton = `
        <button class="pin-button ${isPinned ? 'pinned' : ''}" 
                type="button"
                data-contact-action="toggle-pin"
                data-contact-id="${this.escapeHtml(contact.id)}"
                title="${isPinned ? 'Unpin' : 'Pin'}">
          <svg viewBox="0 0 24 24" width="14" height="14">
            <path fill="currentColor" d="M16 12V4h1V2H7v2h1v8l-2 2v2h5.2v6h1.6v-6H18v-2l-2-2z"/>
          </svg>
        </button>
      `;

        return `
        <div class="contact-item ${isActive ? 'active' : ''} ${isGroup ? 'is-group' : ''} ${isPinned ? 'is-pinned' : ''}" data-contact-id="${contact.id}">
          <div class="avatar-container">
            <div class="avatar">
              ${avatarContent}
              ${groupIndicator}
            </div>
            ${pinButton}
          </div>
          <div class="contact-details">
            <div class="contact-header">
              <div class="contact-title">
                <span class="contact-name">${this.escapeHtml(displayName)}</span>
              </div>
              <span class="contact-time">${time}</span>
            </div>
            <div class="contact-preview">
              <span class="${previewClass}">${this.escapeHtml(preview)}</span>
              ${unread}
            </div>
          </div>
        </div>
      `;
      })
      .join('');

    // Fetch avatars lazily for just the visible contact rows after the list renders.
    const visibleContacts = Array.from(
      container.querySelectorAll('.contact-item[data-contact-id]'),
    )
      .slice(0, 24)
      .map((el) => el.dataset.contactId)
      .filter(Boolean);

    visibleContacts.forEach((contactId) => this.fetchAvatar(contactId));
    this.scheduleMetadataRefresh();
  },

  // Get message preview text
  getMessagePreview(message) {
    const isFromMe = message.isFromMe || message.is_from_me;
    const isTranslated = message.isTranslated || message.is_translated;
    const translatedText =
      message.translatedText || message.translated_text || '';
    const originalText = message.originalText || message.original_text || '';
    const prefix = isFromMe ? 'You: ' : '';
    const content = message.content || {};
    const bodyText = () => {
      if (isTranslated && isFromMe && originalText) return originalText;
      if (isTranslated && !isFromMe && translatedText) return translatedText;
      return content.body || content.text || '';
    };
    const captionText = () => {
      if (isTranslated && isFromMe && originalText) return originalText;
      if (isTranslated && !isFromMe && translatedText) return translatedText;
      return content.caption || '';
    };

    switch (content.type) {
      case 'text':
        return prefix + bodyText().substring(0, 50);
      case 'image':
        return (
          prefix +
          '[ Image ]' +
          (captionText() ? ' ' + captionText().substring(0, 30) : '')
        );
      case 'video':
        return (
          prefix +
          '[ Video ]' +
          (captionText() ? ' ' + captionText().substring(0, 30) : '')
        );
      case 'audio':
        return prefix + (content.isVoiceNote ? '[ Voice Note ]' : '[ Audio ]');
      case 'document':
        return prefix + '[ Document: ' + (content.fileName || 'file') + ' ]';
      case 'sticker':
        return prefix + '[ Sticker ]';
      case 'location':
        return prefix + '[ Location ]';
      case 'contact':
        return prefix + '[ Contact: ' + content.name + ' ]';
      case 'revoked':
        return '[ Message deleted ]';
      case 'poll':
        return prefix + '[ Poll: ' + content.question + ' ]';
      default:
        return prefix + '[ Message ]';
    }
  },

  getMessageSenderJid(message) {
    const isFromMe = message.isFromMe || message.is_from_me;

    if (isFromMe) {
      const myPhone =
        document.getElementById('user-phone')?.textContent?.replace('+', '') ||
        '';
      return myPhone ? `${myPhone}@s.whatsapp.net` : '';
    }

    const senderJid = message.senderJid || message.sender_jid;
    if (senderJid) return senderJid;

    const senderPhone = message.senderPhone || message.sender_phone || '';
    if (!senderPhone) return '';
    return senderPhone.includes('@')
      ? senderPhone
      : `${senderPhone}@s.whatsapp.net`;
  },

  getLatestIncomingMessage(contactId) {
    const messages = this.messages.get(contactId) || [];
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      const message = messages[i];
      if (!message.isFromMe && !message.is_from_me) {
        return message;
      }
    }
    return null;
  },

  async markViewedMessages(contactId, messageIds) {
    if (this.demoMode) return;
    const response = await this.apiFetch(
      `/api/contacts/${encodeURIComponent(contactId)}/read`,
      {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.getAuthHeaders(),
        },
        body: JSON.stringify({ messageIds }),
      },
    );
    if (!response.ok) throw new Error('Unable to record viewed messages');
    const result = await response.json();
    if (!result.success) throw new Error('Unable to record viewed messages');
    const contact = this.contacts.find((c) => c.id === contactId);
    if (contact) {
      contact.unreadCount = result.unreadCount;
      this.scheduleRenderContacts();
    }
  },

  startViewedMessageTracking() {
    const tracker = new ViewedMessageTracker({
      send: (chat, ids) => this.markViewedMessages(chat, ids),
      pending: this.loadStoredJson('wa_pending_viewed_messages_v1'),
      save: (pending) => {
        if (!this.demoMode)
          localStorage.setItem(
            'wa_pending_viewed_messages_v1',
            JSON.stringify(pending),
          );
      },
    });
    this.viewedMessageTracker = tracker;
    let retryAt = 0;
    this.viewedMessagesTimer = setInterval(() => {
      const root = document.getElementById('messages-list');
      const chatView = document.getElementById('chat-view');
      const covered = [...document.querySelectorAll('.modal')].some(
        (el) =>
          !el.classList.contains('hidden') &&
          el.getBoundingClientRect().height > 0,
      );
      const active =
        document.visibilityState === 'visible' &&
        document.hasFocus() &&
        !covered &&
        root &&
        chatView &&
        !chatView.classList.contains('hidden') &&
        !!this.currentContactId;
      const visible = [];
      if (active) {
        const clip = root.getBoundingClientRect();
        const viewport = {
          top: Math.max(0, clip.top),
          bottom: Math.min(window.innerHeight, clip.bottom),
          left: Math.max(0, clip.left),
          right: Math.min(window.innerWidth, clip.right),
        };
        for (const node of root.querySelectorAll(
          '.message.incoming[data-message-id]',
        )) {
          if (!isMessageVisible(node.getBoundingClientRect(), viewport))
            continue;
          const message = (this.messages.get(this.currentContactId) || []).find(
            (m) => m.id === node.dataset.messageId,
          );
          if (message) visible.push(message);
        }
      }
      const now = Date.now();
      tracker.sample(visible, { active, now });
      if (!this.demoMode && now >= retryAt) {
        retryAt = now + 1000;
        tracker.flush().catch(() => {
          retryAt = Date.now() + 10000;
        });
      }
    }, 200);
  },
};
