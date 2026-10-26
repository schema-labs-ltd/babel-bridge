// Sending messages, images and reactions.

import {
  simulateTranslation,
  upsertDraft,
} from '../app-state.js?v=20260912-conversation-translation';

export const sendingMethods = {
  // Send a message
  async sendMessage() {
    const input = document.getElementById('message-input');
    const text = input.value.trim();

    if (!text || !this.currentContactId || this.replyOnlyBlocked()) return;

    const contactId = this.currentContactId;
    const capturedReply = this.replyingTo ? { ...this.replyingTo } : null;
    const sendButton = document.getElementById('send-button');
    sendButton.disabled = true;

    // Capture reply state before clearing
    const replyTo = capturedReply ? capturedReply.messageId : null;
    const replyToSender = capturedReply ? capturedReply.senderJid : null;

    if (this.demoMode) {
      const replyContext = capturedReply ? { ...capturedReply } : null;
      const metadata = this.getContactMetadata(contactId);
      const targetLanguage = metadata.targetLanguage || 'Spanish';
      const translatedText =
        metadata.translationEnabled === true
          ? simulateTranslation(text, targetLanguage)
          : null;
      const sendOriginalFollowUp = Boolean(
        metadata.sendOriginalFollowUp && translatedText,
      );
      const localMessage = {
        id: `demo-out-${Date.now()}`,
        timestamp: Date.now(),
        contactId: contactId,
        isFromMe: true,
        isForwarded: false,
        senderJid: this.getMessageSenderJid({ isFromMe: true }),
        content: {
          type: 'text',
          body: text,
          showTranslatedPrimary: sendOriginalFollowUp,
        },
        isTranslated: Boolean(translatedText),
        originalText: text,
        translatedText,
        sourceLanguage: targetLanguage,
        replyContext,
      };

      input.value = '';
      this.drafts = upsertDraft(this.drafts, contactId, '');
      this.persistDrafts();
      this.renderQuickReplies();
      this.clearReply();
      this.updateSendButton();
      this.autoResizeTextarea(input);

      if (!this.messages.has(contactId)) {
        this.messages.set(contactId, []);
      }
      this.messages.get(contactId).push(localMessage);
      if (sendOriginalFollowUp) {
        this.messages.get(contactId).push({
          id: `demo-original-${Date.now()}`,
          timestamp: Date.now() + 1,
          contactId: contactId,
          isFromMe: true,
          isForwarded: false,
          senderJid: this.getMessageSenderJid({ isFromMe: true }),
          content: { type: 'text', body: text },
          isTranslated: false,
        });
      }
      this.refreshCurrentConversationView();
      this.scrollToBottom();
      this.updateContactInList(localMessage);
      this.renderConversationWorkspace();
      sendButton.disabled = this.replyOnlyBlocked();
      this.updateSendButton();
      return;
    }

    try {
      const requestBody = {
        contactId: contactId,
        text: text,
      };

      // Add reply params if replying
      if (replyTo) {
        requestBody.replyTo = replyTo;
        if (replyToSender) {
          requestBody.replyToSender = replyToSender;
        }
        requestBody.replyToText = capturedReply?.text || null;
        requestBody.replyToSenderName = capturedReply?.senderName || null;
      }

      const response = await this.apiFetch('/api/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(requestBody),
      });

      const result = await response.json();

      if (!response.ok) {
        throw new Error(result.error || 'Failed to send message');
      }

      // Save reply context before clearing (for local message display)
      const replyContext = capturedReply ? { ...capturedReply } : null;

      // Only clear the draft that was actually sent; a different chat or newly typed draft stays intact.
      if (this.currentContactId === contactId && input.value.trim() === text) {
        input.value = '';
        this.drafts = upsertDraft(this.drafts, contactId, '');
        this.persistDrafts();
        this.renderQuickReplies();
        this.clearReply();
        this.updateSendButton();
        this.autoResizeTextarea(input);
      }
      this.scheduleRenderContacts();
      this.updateChatHeaderNote();
      this.renderConversationWorkspace();

      // Create a local message representation with translation info from response
      const localMessage = {
        id: result.messageId || 'temp-' + Date.now(),
        timestamp: result.timestamp || Date.now(),
        contactId: contactId,
        isFromMe: true,
        isForwarded: false,
        senderJid: this.getMessageSenderJid({ isFromMe: true }),
        content: {
          type: 'text',
          body: text,
          showTranslatedPrimary:
            result.originalFollowUpSent ||
            Boolean(result.originalFollowUpError),
        },
        // Include translation info if the message was translated
        isTranslated: result.isTranslated || false,
        originalText: result.isTranslated ? text : null, // What user typed (English)
        translatedText: result.translatedText || null, // What was sent (foreign language)
        sourceLanguage: result.sourceLanguage || null, // Target language
        // Include reply context if this was a reply
        replyContext: replyContext,
      };

      // Add to local store and display
      if (!this.messages.has(contactId)) {
        this.messages.set(contactId, []);
      }

      const messages = this.messages.get(contactId);
      if (!messages.some((m) => m.id === localMessage.id)) {
        messages.push(localMessage);
        if (result.originalFollowUpSent && result.originalMessageId) {
          messages.push({
            id: result.originalMessageId,
            timestamp: result.originalTimestamp || Date.now(),
            contactId: contactId,
            isFromMe: true,
            isForwarded: false,
            senderJid: this.getMessageSenderJid({ isFromMe: true }),
            content: { type: 'text', body: text },
            isTranslated: false,
          });
        }
        this.refreshCurrentConversationView();
        this.scrollToBottom();
        this.updateChatHeaderNote();
        this.renderConversationWorkspace();
      }

      // Update contact list
      this.updateContactInList(localMessage);

      // Refresh usage stats if translation occurred
      if (result.isTranslated) {
        this.fetchGlobalUsage();
        this.fetchConversationUsage(contactId);
      }

      if (result.originalFollowUpError) {
        alert(
          `The translation was sent, but the original follow-up failed: ${result.originalFollowUpError}`,
        );
      }
    } catch (err) {
      console.error('Failed to send message:', err);
      alert('Failed to send message: ' + err.message);
    } finally {
      sendButton.disabled = this.replyOnlyBlocked();
      this.updateSendButton();
    }
  },

  // Send an image
  async sendImage(file) {
    if (!file || !this.currentContactId || this.replyOnlyBlocked()) return;

    // Check file size (limit to 16MB)
    if (file.size > 16 * 1024 * 1024) {
      alert('Image is too large. Maximum size is 16MB.');
      return;
    }

    // Check file type
    const allowedImageTypes = new Set([
      'image/jpeg',
      'image/png',
      'image/gif',
      'image/webp',
    ]);
    if (!allowedImageTypes.has(String(file.type || '').toLowerCase())) {
      alert('Please select a JPEG, PNG, GIF, or WebP image file.');
      return;
    }

    const attachButton = document.getElementById('attach-button');
    attachButton.disabled = true;

    // Capture reply state before clearing
    const replyTo = this.replyingTo ? this.replyingTo.messageId : null;
    const replyToSender = this.replyingTo ? this.replyingTo.senderJid : null;

    try {
      // Read file as base64
      const mediaData = await this.fileToBase64(file);

      const requestBody = {
        contactId: this.currentContactId,
        mediaData: mediaData,
        mimeType: file.type,
        caption: null,
      };

      // Add reply params if replying
      if (replyTo) {
        requestBody.replyTo = replyTo;
        if (replyToSender) {
          requestBody.replyToSender = replyToSender;
        }
        requestBody.replyToText = this.replyingTo?.text || null;
        requestBody.replyToSenderName = this.replyingTo?.senderName || null;
      }

      const response = await this.apiFetch('/api/send-image', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(requestBody),
      });

      const result = await response.json();

      if (!response.ok) {
        throw new Error(result.error || 'Failed to send image');
      }

      // Save reply context before clearing (for local message display)
      const replyContext = this.replyingTo ? { ...this.replyingTo } : null;

      // Clear reply state on success
      this.clearReply();

      // Create a local message representation
      const localMessage = {
        id: result.messageId || 'temp-img-' + Date.now(),
        timestamp: result.timestamp || Date.now(),
        contactId: this.currentContactId,
        isFromMe: true,
        isForwarded: false,
        senderJid: this.getMessageSenderJid({ isFromMe: true }),
        content: {
          type: 'image',
          mime_type: file.type,
          media_data: mediaData,
        },
        // Include reply context if this was a reply
        replyContext: replyContext,
      };

      // Add to local store and display
      if (!this.messages.has(this.currentContactId)) {
        this.messages.set(this.currentContactId, []);
      }

      const messages = this.messages.get(this.currentContactId);
      if (!messages.some((m) => m.id === localMessage.id)) {
        messages.push(localMessage);
        this.refreshCurrentConversationView();
        this.scrollToBottom();
      }

      // Update contact list
      this.updateContactInList(localMessage);
    } catch (err) {
      console.error('Failed to send image:', err);
      alert('Failed to send image: ' + err.message);
    } finally {
      attachButton.disabled = this.replyOnlyBlocked();
    }
  },

  // Convert file to base64
  fileToBase64(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        // Remove the data URL prefix (e.g., "data:image/jpeg;base64,")
        const base64 = reader.result.split(',')[1];
        resolve(base64);
      };
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  },

  // Send a reaction to a message
  async sendReaction(messageId, contactId, senderJid, emoji) {
    // Close any open reaction pickers
    document
      .querySelectorAll('.reaction-picker.show')
      .forEach((el) => el.classList.remove('show'));

    if (!messageId || !contactId) {
      console.error('Missing messageId or contactId for reaction');
      return;
    }

    try {
      const response = await this.apiFetch('/api/react', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          contactId: contactId,
          messageId: messageId,
          senderJid: senderJid || null,
          emoji: emoji,
        }),
      });

      const result = await response.json();

      if (!response.ok) {
        throw new Error(result.error || 'Failed to send reaction');
      }

      if (result.success === false)
        throw new Error(result.error || 'Failed to send reaction');
      this.handleReactionMessage(
        result.reaction || {
          id: `confirmed-reaction-${crypto.randomUUID()}`,
          contactId,
          timestamp: Date.now(),
          isFromMe: true,
          contentType: 'Reaction',
          content: { type: 'reaction', target_message_id: messageId, emoji },
        },
      );

      console.log('Reaction sent successfully');
    } catch (err) {
      console.error('Failed to send reaction:', err);
      alert('Failed to send reaction: ' + err.message);
    }
  },
};
