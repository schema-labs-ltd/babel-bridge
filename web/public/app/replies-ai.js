// Reply targeting, the send button and AI compose and reply.

import { suggestDemoReply } from '../app-state.js?v=20260912-conversation-translation';

export const repliesAiMethods = {
  // Set reply state for a message
  setReplyTo(message) {
    const content = message.content;
    const isFromMe = Boolean(message.isFromMe || message.is_from_me);

    // Get message preview text for display
    let previewText = '';
    switch (content.type) {
      case 'text':
        previewText = content.body || content.text || '';
        break;
      case 'image':
        previewText = content.caption || '[ Image ]';
        break;
      case 'video':
        previewText = content.caption || '[ Video ]';
        break;
      case 'audio':
        previewText = content.isVoiceNote ? '[ Voice Note ]' : '[ Audio ]';
        break;
      case 'document':
        previewText = '[ Document: ' + (content.fileName || 'file') + ' ]';
        break;
      case 'sticker':
        previewText = '[ Sticker ]';
        break;
      default:
        previewText = '[ Message ]';
    }

    // Truncate long messages
    if (previewText.length > 100) {
      previewText = previewText.substring(0, 100) + '...';
    }

    // Get sender name
    let senderName = 'You';
    if (!isFromMe) {
      senderName =
        message.senderName ||
        message.sender_name ||
        message.senderPhone ||
        message.sender_phone ||
        'Unknown';
    }

    // Get sender JID for the reply
    const senderJid = this.getMessageSenderJid(message);

    // Capture image data if replying to an image (for AI compose)
    let imageData = null;
    let imageType = null;
    if (content.type === 'image') {
      const mediaData = content.media_data || content.mediaData;
      if (mediaData) {
        imageData = mediaData;
        imageType = content.mime_type || content.mimeType || 'image/jpeg';
      }
    }

    this.replyingTo = {
      messageId: message.id,
      senderJid: senderJid,
      senderName: senderName,
      text: previewText,
      isFromMe: isFromMe,
      imageData: imageData,
      imageType: imageType,
    };

    this.updateReplyPreview();

    // Focus the input
    document.getElementById('message-input').focus();
  },

  // Clear reply state
  clearReply() {
    this.replyingTo = null;
    this.updateReplyPreview();
  },

  // Update the reply preview UI
  updateReplyPreview() {
    this.updateReplyOnlyControls();
    const previewContainer = document.getElementById('reply-preview');
    if (!previewContainer) return;

    if (this.replyingTo) {
      const senderEl = previewContainer.querySelector('.reply-preview-sender');
      const textEl = previewContainer.querySelector('.reply-preview-text');

      if (senderEl) senderEl.textContent = this.replyingTo.senderName;
      if (textEl) textEl.textContent = this.replyingTo.text;

      previewContainer.classList.remove('hidden');
    } else {
      previewContainer.classList.add('hidden');
    }
  },

  // Handle reply button click
  handleReplyClick(messageId) {
    // Close any open reaction pickers
    document
      .querySelectorAll('.reaction-picker.show')
      .forEach((el) => el.classList.remove('show'));

    // Find the message in our cache
    if (!this.currentContactId) return;

    const messages = this.messages.get(this.currentContactId);
    if (!messages) return;

    const message = messages.find((m) => m.id === messageId);
    if (message) {
      this.setReplyTo(message);
    }
  },

  // Update send button state
  updateSendButton() {
    const input = document.getElementById('message-input');
    const sendButton = document.getElementById('send-button');
    const dropdownToggle = document.getElementById('send-dropdown-toggle');
    const hasContent =
      input.value.trim() && this.currentContactId && this.connected;

    sendButton.disabled = !hasContent || this.replyOnlyBlocked();
    if (dropdownToggle) {
      dropdownToggle.disabled = !hasContent;
    }
  },

  // Send message composed by AI
  async sendWithAI() {
    const input = document.getElementById('message-input');
    const prompt = input.value.trim();

    if (!prompt || !this.currentContactId) return;

    // Show AI composing indicator
    this.showAIComposing(true);

    try {
      // Build request with optional reply context
      const requestBody = { prompt };

      if (this.replyingTo) {
        requestBody.replyToText = this.replyingTo.text;
        requestBody.replyToSender = this.replyingTo.senderName;

        // Include image data if replying to an image
        if (this.replyingTo.imageData) {
          requestBody.replyToImage = this.replyingTo.imageData;
          requestBody.replyToImageType = this.replyingTo.imageType;
        }
      }

      const response = await this.apiFetch('/api/ai-compose', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(requestBody),
      });

      const result = await response.json();

      if (!result.success) {
        throw new Error(result.error || 'AI compose failed');
      }

      const aiMessage = String(result.message || '').trim();
      if (!aiMessage) {
        throw new Error('AI compose returned an empty draft');
      }

      input.value = aiMessage;
      this.autoResizeTextarea(input);
      this.updateSendButton();
      this.handleDraftInput();
      input.focus();

      // Log cost if available
      if (result.costUsd) {
        console.log(`AI compose cost: $${result.costUsd.toFixed(6)}`);
      }
    } catch (err) {
      console.error('Failed to compose with AI:', err);
      alert('Failed to compose message with AI: ' + err.message);
    } finally {
      this.showAIComposing(false);
    }
  },

  // Generate AI reply for a received message (styled to sound like the user)
  async generateAIReply(messageId) {
    if (!this.currentContactId) return;

    // Find the message in cache
    const messages = this.messages.get(this.currentContactId);
    const message = messages?.find((m) => m.id === messageId);
    if (!message) {
      console.error('Message not found:', messageId);
      return;
    }

    // Show loading state on the button
    const msgEl = document.querySelector(`[data-message-id="${messageId}"]`);
    const btn = msgEl?.querySelector('.ai-reply-btn');
    if (btn) {
      btn.classList.add('loading');
      btn.disabled = true;
    }

    try {
      if (this.demoMode) {
        const contact =
          this.contacts.find((entry) => entry.id === this.currentContactId) ||
          {};
        const replyText = suggestDemoReply({
          message,
          metadata: this.getContactMetadata(this.currentContactId),
          contact,
        });

        this.setReplyTo(message);
        const input = document.getElementById('message-input');
        input.value = replyText;
        this.autoResizeTextarea(input);
        this.updateSendButton();
        this.handleDraftInput();
        input.focus();
        return;
      }

      const response = await this.apiFetch('/api/ai-reply', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.getAuthHeaders(),
        },
        body: JSON.stringify({
          contactId: this.currentContactId,
          messageId: messageId,
        }),
      });

      const result = await response.json();

      if (!result.success) {
        throw new Error(result.error || 'AI reply generation failed');
      }
      if (!result.replyText || !result.replyText.trim()) {
        throw new Error('AI reply was empty');
      }

      // Set reply context to this message (reuses existing setReplyTo)
      this.setReplyTo(message);

      // Populate input with AI-generated reply (user can edit before sending)
      const input = document.getElementById('message-input');
      input.value = result.replyText;
      this.autoResizeTextarea(input);
      this.updateSendButton();
      this.handleDraftInput();
      input.focus();

      // Log cost for debugging
      if (result.costUsd) {
        console.log(`AI reply generated, cost: $${result.costUsd.toFixed(6)}`);
      }
    } catch (err) {
      console.error('Failed to generate AI reply:', err);
      alert('Failed to generate AI reply: ' + err.message);
    } finally {
      if (btn) {
        btn.classList.remove('loading');
        btn.disabled = false;
      }
    }
  },

  // Show/hide AI composing indicator
  showAIComposing(show) {
    let indicator = document.getElementById('ai-composing-indicator');

    if (show) {
      if (!indicator) {
        indicator = document.createElement('div');
        indicator.id = 'ai-composing-indicator';
        indicator.className = 'ai-composing';
        indicator.innerHTML = `
          <div class="ai-composing-spinner"></div>
          <span>AI is composing your message...</span>
        `;

        const inputArea = document.querySelector('.message-input-area');
        const inputContainer = document.querySelector('.input-container');
        if (inputArea && inputContainer) {
          inputArea.insertBefore(indicator, inputContainer);
        }
      }
      indicator.style.display = 'flex';

      // Disable input while composing
      document.getElementById('message-input').disabled = true;
      document.getElementById('send-button').disabled = true;
      document.getElementById('send-dropdown-toggle').disabled = true;
    } else {
      if (indicator) {
        indicator.style.display = 'none';
      }
      // Re-enable input
      document.getElementById('message-input').disabled = false;
      this.updateSendButton();
    }
  },

  // Auto-resize textarea (expands up to max-height)
  autoResizeTextarea(textarea) {
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 150) + 'px';
  },
};
