// Incoming message handling: reactions, notifications, typing and contact list updates.

export const liveMessagesMethods = {
  getMessageContentType(message) {
    return String(
      message?.content?.type ||
        message?.contentType ||
        message?.content_type ||
        '',
    ).toLowerCase();
  },

  isReactionMessage(message) {
    const declaredType = String(
      message?.contentType || message?.content_type || '',
    ).toLowerCase();
    const embeddedType = String(message?.content?.type || '').toLowerCase();
    return declaredType === 'reaction' || embeddedType === 'reaction';
  },

  isDisplayableMessage(message) {
    const contentType = this.getMessageContentType(message);
    return (
      Boolean(contentType) &&
      !['reaction', 'protocol', 'unknown'].includes(contentType)
    );
  },

  prepareMessageForCache(message) {
    if (!message || typeof message !== 'object') return message;

    if (!message.senderJid) {
      message.senderJid = this.getMessageSenderJid(message);
    }
    if (!message.replyContext && message.content?.reply_context) {
      message.replyContext = message.content.reply_context;
    }
    return message;
  },

  getReactionTargetMessageId(reactionMsg) {
    const content = reactionMsg?.content || {};
    return (
      content.target_message_id ||
      content.targetMessageId ||
      content.targetMessageID ||
      reactionMsg?.target_message_id ||
      reactionMsg?.targetMessageId ||
      reactionMsg?.targetMessageID ||
      ''
    );
  },

  getReactionActor(reactionMsg) {
    const isFromMe = reactionMsg?.isFromMe || reactionMsg?.is_from_me;
    if (isFromMe) {
      return 'me';
    }
    return (
      reactionMsg?.senderPhone ||
      reactionMsg?.sender_phone ||
      reactionMsg?.senderJid ||
      'unknown'
    );
  },

  applyReactionToMessage(targetMessage, reactionMsg) {
    const content = reactionMsg?.content || {};
    const emoji = content.emoji || '';
    const reactor = this.getReactionActor(reactionMsg);

    const state = {
      id: reactionMsg.id || '',
      timestamp: reactionMsg.timestamp || 0,
      emoji,
    };
    const previous = targetMessage.reactionStates?.[reactor];
    if (
      previous &&
      (previous.timestamp > state.timestamp ||
        (previous.timestamp === state.timestamp && previous.id >= state.id))
    )
      return;
    targetMessage.reactionStates = {
      ...targetMessage.reactionStates,
      [reactor]: state,
    };

    if (!targetMessage.reactions) {
      targetMessage.reactions = {};
    }

    for (const [existingEmoji, reactors] of Object.entries(
      targetMessage.reactions,
    )) {
      targetMessage.reactions[existingEmoji] = (reactors || []).filter(
        (entry) => entry !== reactor,
      );
      if (targetMessage.reactions[existingEmoji].length === 0) {
        delete targetMessage.reactions[existingEmoji];
      }
    }

    if (emoji) {
      if (!targetMessage.reactions[emoji]) {
        targetMessage.reactions[emoji] = [];
      }
      if (!targetMessage.reactions[emoji].includes(reactor)) {
        targetMessage.reactions[emoji].push(reactor);
      }
    }
  },

  applyReactionToMessages(messages = [], reactionMsg) {
    const targetMessageId = this.getReactionTargetMessageId(reactionMsg);
    if (!targetMessageId) return false;

    const targetMessage = messages.find(
      (message) => message.id === targetMessageId,
    );
    if (!targetMessage) return false;

    this.applyReactionToMessage(targetMessage, reactionMsg);
    return true;
  },

  normalizeLoadedMessages(
    contactId,
    rawMessages = [],
    additionalReactionTargets = [],
  ) {
    const displayMessages = [];
    const reactionMessages = [];

    for (let message of rawMessages || []) {
      const current = this.messages
        .get(contactId)
        ?.find((value) => value.id === message.id);
      if (
        (current?.content?.edited_at_ms || 0) >
        (message.content?.edited_at_ms || 0)
      )
        message = current;
      this.prepareMessageForCache(message);
      if (this.isReactionMessage(message)) {
        reactionMessages.push(message);
        continue;
      }
      if (this.isDisplayableMessage(message)) {
        displayMessages.push(message);
      }
    }

    const reactionTargets =
      additionalReactionTargets.length > 0
        ? [...displayMessages, ...additionalReactionTargets]
        : displayMessages;

    for (const message of reactionMessages) {
      if (!this.applyReactionToMessages(reactionTargets, message)) {
        console.log('Skipping reaction whose target is not loaded');
      }
    }

    return displayMessages;
  },

  getLastDisplayableMessage(messages = []) {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index];
      if (this.isDisplayableMessage(message)) {
        return message;
      }
    }
    return null;
  },

  // Handle new message
  handleNewMessage(message) {
    if (this.liveRecoveryRunning) this.liveRecoveryPending = true;
    this.prepareMessageForCache(message);

    // Check if this is a reaction message
    if (this.isReactionMessage(message)) {
      this.handleReactionMessage(message);
      return;
    }

    // Skip protocol and unknown messages - these shouldn't be displayed
    if (!this.isDisplayableMessage(message)) {
      console.log(
        'Skipping non-displayable message type:',
        this.getMessageContentType(message),
      );
      return;
    }

    // Add to local cache
    if (!this.messages.has(message.contactId)) {
      this.messages.set(message.contactId, []);
    }

    const messages = this.messages.get(message.contactId);
    if (!messages.some((m) => m.id === message.id)) {
      messages.push(message);
      messages.sort((a, b) => a.timestamp - b.timestamp);
    }

    this.trimCachedMessages(message.contactId, true);

    // Update contact in list
    this.updateContactInList(message);

    this.maybeShowNotification(message);

    // If this contact is currently selected, show the message
    if (this.currentContactId === message.contactId) {
      this.refreshCurrentConversationView();
      this.scrollToBottom();
      this.updateChatHeaderNote();
      this.renderConversationWorkspace();
    }

    // Refresh usage stats if this was a translated message
    if (message.isTranslated || message.is_translated) {
      this.fetchGlobalUsage();
      if (this.currentContactId === message.contactId) {
        this.fetchConversationUsage(message.contactId);
      }
    }

    // Clear typing indicator when message arrives from that user
    if (!message.isFromMe && !message.is_from_me) {
      this.clearTypingState(message.contactId);
    }
  },

  notificationsSupported() {
    return (
      typeof window !== 'undefined' &&
      'Notification' in window &&
      window.isSecureContext
    );
  },

  updateNotificationPrompt() {
    const host = document.getElementById('contacts-panel');
    if (!host) return;

    if (!this.notificationPromptEl) {
      this.notificationPromptEl = document.createElement('div');
      this.notificationPromptEl.id = 'notification-prompt';
      this.notificationPromptEl.className = 'notification-prompt hidden';
      host.insertBefore(
        this.notificationPromptEl,
        document.getElementById('contacts-list'),
      );
    }

    if (!this.notificationsSupported()) {
      this.notificationPromptEl.innerHTML = `
        <span>Browser notifications are not available in this context.</span>
      `;
      this.notificationPromptEl.classList.remove('hidden');
      return;
    }

    if (Notification.permission === 'default') {
      this.notificationPromptEl.innerHTML = `
        <span>Enable browser notifications for new translated messages.</span>
        <button id="notification-enable-button" type="button">Enable</button>
      `;
      this.notificationPromptEl.classList.remove('hidden');
      this.notificationPromptEl
        .querySelector('#notification-enable-button')
        ?.addEventListener('click', () => this.requestNotificationPermission());
      return;
    }

    if (Notification.permission === 'denied') {
      this.notificationPromptEl.innerHTML = `
        <span>Notifications are blocked in your browser settings.</span>
      `;
      this.notificationPromptEl.classList.remove('hidden');
      return;
    }

    this.notificationPromptEl.classList.add('hidden');
  },

  async requestNotificationPermission() {
    if (!this.notificationsSupported()) {
      this.updateNotificationPrompt();
      return;
    }

    this.notificationPermissionRequested = true;

    try {
      await Notification.requestPermission();
    } catch (err) {
      console.error('Failed to request notification permission:', err);
    } finally {
      this.updateNotificationPrompt();
    }
  },

  setupNotificationPermissionRequest() {
    if (!this.notificationsSupported()) return;
    if (Notification.permission !== 'default') return;
    if (this.notificationPermissionRequested) return;

    const requestPermission = async () => {
      if (this.notificationPermissionRequested) return;
      this.notificationPermissionRequested = true;

      try {
        await Notification.requestPermission();
      } catch (err) {
        console.error('Failed to request notification permission:', err);
      } finally {
        this.updateNotificationPrompt();
        window.removeEventListener('click', requestPermission, true);
        window.removeEventListener('keydown', requestPermission, true);
        window.removeEventListener('touchstart', requestPermission, true);
      }
    };

    window.addEventListener('click', requestPermission, true);
    window.addEventListener('keydown', requestPermission, true);
    window.addEventListener('touchstart', requestPermission, true);
  },

  shouldNotifyForMessage(message) {
    if (!this.notificationsSupported()) return false;
    if (Notification.permission !== 'granted') return false;
    if (Date.now() < this.notificationsReadyAt) return false;
    if (message.isFromMe || message.is_from_me) return false;

    const contentType =
      message.content?.type || message.contentType || message.content_type;
    if (
      !contentType ||
      ['reaction', 'protocol', 'unknown'].includes(contentType)
    ) {
      return false;
    }

    const isCurrentThreadVisible =
      this.currentContactId === message.contactId &&
      document.visibilityState === 'visible' &&
      document.hasFocus();

    return !isCurrentThreadVisible;
  },

  getThreadDisplayName(message) {
    const contact = this.contacts.find((c) => c.id === message.contactId);
    return (
      contact?.name ||
      message.contactName ||
      contact?.phone ||
      message.contactPhone ||
      message.senderName ||
      message.senderPhone ||
      'WhatsApp'
    );
  },

  getNotificationText(message) {
    const translated = message.translatedText || message.translated_text;
    if (translated) {
      return translated;
    }

    const content = message.content || {};
    switch (content.type) {
      case 'text':
        return content.body || content.text || '';
      case 'image':
        return content.caption || '[ Image ]';
      case 'video':
        return content.caption || '[ Video ]';
      case 'audio':
        return content.isVoiceNote || content.is_voice_note
          ? '[ Voice note ]'
          : '[ Audio ]';
      case 'document':
        return content.fileName || content.file_name || '[ Document ]';
      case 'sticker':
        return '[ Sticker ]';
      case 'location':
        return '[ Location ]';
      case 'contact':
        return content.name || '[ Contact ]';
      case 'poll':
        return content.question || '[ Poll ]';
      default:
        return this.getMessagePreview(message).replace(/^You:\s*/, '');
    }
  },

  async maybeShowNotification(message) {
    if (!this.shouldNotifyForMessage(message)) {
      return;
    }

    const title = this.getThreadDisplayName(message);
    let body = this.getNotificationText(message);

    const isGroup = (message.chatType || message.chat_type) === 'group';
    const sender =
      message.senderName ||
      message.sender_name ||
      message.senderPhone ||
      message.sender_phone;
    if (isGroup && sender) {
      body = `${sender}: ${body}`;
    }

    if (!body) {
      body = 'New message';
    }

    const sound = (await this.messageTones?.notificationSound(
      message.contactId,
    )) ?? { silent: false, filename: null };
    if (!this.shouldNotifyForMessage(message)) return;

    const notification = new Notification(title, {
      body,
      icon: '/icon-192.png',
      badge: '/icon-192.png',
      tag: `chat:${message.contactId}`,
      renotify: false,
      silent: sound.silent,
    });
    this.messageTones?.playNotification(sound.filename);

    notification.addEventListener('click', () => {
      window.focus();
      if (message.contactId) {
        this.selectContact(message.contactId);
      }
      notification.close();
    });
  },

  // Handle incoming reaction message
  handleReactionMessage(reactionMsg) {
    const contactId = reactionMsg.contactId;
    const targetMessageId = this.getReactionTargetMessageId(reactionMsg);

    if (!targetMessageId) {
      console.warn('Reaction message missing target_message_id');
      return;
    }

    // Find the target message in our cache
    const messages = this.messages.get(contactId);
    if (!messages) return;

    if (!this.applyReactionToMessages(messages, reactionMsg)) {
      console.log('Target message not found for reaction');
      return;
    }

    // Update display if viewing this chat
    if (this.currentContactId === contactId) {
      this.updateMessageReactions(targetMessageId);
    }

    console.log('Reaction updated');
  },

  // Handle typing indicator
  handleTyping(data) {
    const { chat_id, user_id, state } = data;
    console.log('Typing event received');

    // Clear existing timeout for this chat
    if (this.typingTimeouts.has(chat_id)) {
      clearTimeout(this.typingTimeouts.get(chat_id));
      this.typingTimeouts.delete(chat_id);
    }

    if (state === 'paused') {
      // Remove typing state
      this.typingState.delete(chat_id);
    } else {
      // Set typing or recording state
      this.typingState.set(chat_id, {
        userId: user_id,
        state: state, // 'typing' or 'recording'
        timestamp: Date.now(),
      });

      // Auto-clear after 10 seconds (in case paused event is missed)
      const timeoutId = setTimeout(() => {
        this.clearTypingState(chat_id);
      }, 10000);
      this.typingTimeouts.set(chat_id, timeoutId);
    }

    // Update UI if this is the current chat
    if (this.currentContactId === chat_id) {
      this.updateTypingIndicator();
    }

    // Also update the contact list preview
    this.updateContactTypingPreview(chat_id);
  },

  // Clear typing state for a chat
  clearTypingState(chatId) {
    this.typingState.delete(chatId);
    if (this.typingTimeouts.has(chatId)) {
      clearTimeout(this.typingTimeouts.get(chatId));
      this.typingTimeouts.delete(chatId);
    }

    if (this.currentContactId === chatId) {
      this.updateTypingIndicator();
    }
    this.updateContactTypingPreview(chatId);
  },

  // Update typing indicator in chat header
  updateTypingIndicator() {
    const indicatorEl = document.getElementById('typing-indicator');
    console.log('updateTypingIndicator called');
    if (!indicatorEl) {
      console.warn('typing-indicator element not found!');
      return;
    }

    const typingInfo = this.typingState.get(this.currentContactId);
    console.log(
      'typing indicator state',
      typingInfo ? typingInfo.state : 'idle',
    );

    if (typingInfo) {
      const text =
        typingInfo.state === 'recording' ? 'recording audio...' : 'typing...';
      indicatorEl.textContent = text;
      indicatorEl.classList.remove('hidden');
      console.log('Showing typing indicator:', text);
    } else {
      indicatorEl.classList.add('hidden');
      console.log('Hiding typing indicator');
    }
  },

  // Update contact list to show typing preview
  updateContactTypingPreview(chatId) {
    const contactItem = document.querySelector(
      `.contact-item[data-contact-id="${chatId}"]`,
    );
    if (!contactItem) return;

    const previewEl = contactItem.querySelector('.preview-text');
    if (!previewEl) return;

    const typingInfo = this.typingState.get(chatId);

    if (typingInfo) {
      const text =
        typingInfo.state === 'recording' ? 'recording audio...' : 'typing...';
      previewEl.innerHTML = `<span class="typing-preview">${text}</span>`;
    } else {
      // Restore the original preview
      const contact = this.contacts.find((c) => c.id === chatId);
      if (contact) {
        const messages = this.messages.get(chatId) || [];
        const lastMessage = this.getLastDisplayableMessage(messages);
        const preview = lastMessage ? this.getMessagePreview(lastMessage) : '';
        previewEl.textContent = preview;
      }
    }
  },

  // Update contact in the list
  updateContactInList(message) {
    // Find or create contact
    let contact = this.contacts.find((c) => c.id === message.contactId);

    if (!contact) {
      // Use contactName (the chat/contact name) not senderName
      contact = {
        id: message.contactId,
        name: message.contactName || message.contactPhone,
        phone: message.contactPhone,
        type: message.chatType,
        lastMessageTime: message.timestamp,
        unreadCount: 0,
      };
      this.contacts.push(contact);
    } else {
      contact.lastMessageTime = Math.max(
        contact.lastMessageTime,
        message.timestamp,
      );
      // Only update name if we have a better one from contactName (not senderName!)
      // contactName is the stable chat name (other person for DMs, group name for groups)
      if (
        message.contactName &&
        message.contactName !== message.contactPhone &&
        !contact.name
      ) {
        contact.name = message.contactName;
      }
    }

    // Increment unread if not from me and not currently viewing
    if (!message.isFromMe && !message.is_from_me) {
      contact.unreadCount = (contact.unreadCount || 0) + 1;
    }

    // Batch contact list updates during reconnect/history sync.
    this.scheduleRenderContacts();
  },
};
