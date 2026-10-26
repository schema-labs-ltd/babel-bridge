// Message list clicks, linkify, markdown, link previews, media and emoji.

export const messageMediaMethods = {
  handleMessagesListClick(event) {
    const target = event.target;
    if (!(target instanceof Element)) {
      return;
    }

    const tooltipClose = target.closest(
      '[data-message-action="close-translation-tooltip"]',
    );
    if (tooltipClose) {
      event.stopPropagation();
      tooltipClose
        .closest('.translation-indicator')
        ?.classList.remove('show-tooltip');
      return;
    }

    const translationIndicator = target.closest('.translation-indicator');
    if (translationIndicator) {
      event.stopPropagation();
      translationIndicator.classList.toggle('show-tooltip');
      return;
    }

    const fullscreenImage = target.closest('img[data-fullscreen-toggle]');
    if (fullscreenImage) {
      event.stopPropagation();
      fullscreenImage.classList.toggle('fullscreen');
      return;
    }

    const mediaPlaceholder = target.closest('.media-placeholder');
    if (mediaPlaceholder) {
      event.stopPropagation();
      const mediaContainer = mediaPlaceholder.closest('.lazy-media');
      const messageId =
        mediaContainer?.dataset.messageId ||
        mediaPlaceholder.closest('.message')?.dataset.messageId;
      if (messageId) {
        this.loadMedia(messageId, mediaPlaceholder);
      }
      return;
    }

    const actionTarget = target.closest('[data-message-action]');
    if (!actionTarget) {
      return;
    }

    const messageId =
      actionTarget.dataset.messageId ||
      actionTarget.closest('.message')?.dataset.messageId;

    switch (actionTarget.dataset.messageAction) {
      case 'translate':
        event.stopPropagation();
        if (messageId) this.translateMessage(messageId);
        break;
      case 'reply':
        event.stopPropagation();
        if (messageId) this.handleReplyClick(messageId);
        break;
      case 'ai-reply':
        event.stopPropagation();
        if (messageId) this.generateAIReply(messageId);
        break;
      case 'star':
        event.stopPropagation();
        if (messageId) this.toggleMessageStar(messageId);
        break;
      case 'toggle-reactions':
        event.stopPropagation();
        actionTarget.parentElement
          ?.querySelector('.reaction-picker')
          ?.classList.toggle('show');
        break;
      case 'react':
        event.stopPropagation();
        if (
          messageId &&
          actionTarget.dataset.contactId &&
          actionTarget.dataset.senderJid
        ) {
          this.sendReaction(
            messageId,
            actionTarget.dataset.contactId,
            actionTarget.dataset.senderJid,
            actionTarget.dataset.reaction || '',
          );
        }
        actionTarget.closest('.reaction-picker')?.classList.remove('show');
        break;
      default:
        break;
    }
  },

  // Convert URLs in text to clickable links and apply WhatsApp markdown formatting
  linkifyText(text) {
    if (!text) return '';

    const urlRegex = /(https?:\/\/[^\s<>\[\](){}|\\^`\x00-\x1f\x7f]+)/gi;
    const parts = text.split(urlRegex);

    return parts
      .map((part, index) => {
        // Even indices are non-URL text, odd indices are URLs (due to capture group)
        if (index % 2 === 0) {
          // Non-URL text - escape it and apply WhatsApp formatting
          return this.formatWhatsAppMarkdown(this.escapeHtml(part));
        } else {
          // URL - clean trailing punctuation and create link
          let cleanUrl = part;
          let trailing = '';
          while (cleanUrl.match(/[.,!?)\]};:'"]+$/)) {
            trailing = cleanUrl.slice(-1) + trailing;
            cleanUrl = cleanUrl.slice(0, -1);
          }
          return `<a href="${this.escapeHtml(cleanUrl)}" target="_blank" rel="noopener noreferrer" class="message-link">${this.escapeHtml(cleanUrl)}</a>${this.escapeHtml(trailing)}`;
        }
      })
      .join('');
  },

  // Apply WhatsApp-style markdown formatting
  // *bold* _italic_ ~strikethrough~ ```code block``` `inline code`
  formatWhatsAppMarkdown(text) {
    if (!text) return '';

    // Process code blocks first (```text```) - these should not have other formatting inside
    text = text.replace(
      /```([\s\S]*?)```/g,
      '<code class="wa-code-block">$1</code>',
    );

    // Process inline code (`text`)
    text = text.replace(
      /`([^`\n]+)`/g,
      '<code class="wa-code-inline">$1</code>',
    );

    // Process bold (*text*) - must not be inside a code block
    // Match * followed by non-whitespace, any chars, non-whitespace, then *
    text = text.replace(/\*([^\s*](?:[^*]*[^\s*])?)\*/g, '<strong>$1</strong>');

    // Process italic (_text_) - must not be inside a code block
    text = text.replace(
      /(?<![a-zA-Z0-9])_([^\s_](?:[^_]*[^\s_])?)_(?![a-zA-Z0-9])/g,
      '<em>$1</em>',
    );

    // Process strikethrough (~text~)
    text = text.replace(/~([^\s~](?:[^~]*[^\s~])?)~/g, '<s>$1</s>');

    // Convert newlines to <br>
    text = text.replace(/\n/g, '<br>');

    return text;
  },

  // Load link previews for a message element
  async loadLinkPreviews(messageEl, urls) {
    const container = messageEl.querySelector('.link-previews-container');
    if (!container || urls.length === 0) return;
    if (
      container.dataset.previewLoaded === 'true' ||
      container.dataset.previewLoading === 'true'
    )
      return;

    container.dataset.previewLoading = 'true';

    for (const url of urls) {
      const preview = await this.fetchLinkPreview(url);
      if (preview && !preview.error) {
        const cardHtml = this.renderLinkPreviewCard(preview, url);
        if (cardHtml) {
          container.insertAdjacentHTML('beforeend', cardHtml);
        }
      }
    }

    container.dataset.previewLoading = 'false';
    container.dataset.previewLoaded = 'true';
  },

  // Load media on demand (lazy loading)
  async loadMedia(messageId, placeholderEl) {
    // Find the lazy-media container
    const container = placeholderEl.closest('.lazy-media');
    if (!container) {
      console.error('Could not find lazy-media container');
      return;
    }

    // Get media type info from the container
    const mediaType = container.dataset.mediaType;
    const mimeType = container.dataset.mimeType;
    const fileName = container.dataset.fileName;

    // Show loading state
    placeholderEl.classList.add('loading');
    placeholderEl.innerHTML = `
      <div class="media-loading-spinner"></div>
      <span>Loading...</span>
    `;

    try {
      // Fetch media from the API
      const response = await this.apiFetch(
        `/api/media/${encodeURIComponent(messageId)}`,
      );

      if (!response.ok) {
        throw new Error('Failed to load media');
      }

      const data = await response.json();

      if (!data.media_data) {
        throw new Error('No media data received');
      }

      // Build the data URL
      const actualMimeType = data.mime_type || mimeType;
      const mediaSrc = data.media_data.startsWith('data:')
        ? data.media_data
        : `data:${actualMimeType};base64,${data.media_data}`;

      // Replace placeholder with actual media based on type
      let mediaHtml = '';

      switch (mediaType) {
        case 'image':
          mediaHtml = `<img src="${mediaSrc}" alt="Image" loading="lazy" data-fullscreen-toggle="true">`;
          break;

        case 'video':
          mediaHtml = `
            <video controls preload="metadata">
              <source src="${mediaSrc}" type="${actualMimeType}">
              Your browser does not support video playback.
            </video>
          `;
          break;

        case 'audio':
          const isVoiceNote = container.classList.contains('voice-note');
          mediaHtml = `
            <audio controls preload="metadata">
              <source src="${mediaSrc}" type="${actualMimeType}">
              Your browser does not support audio playback.
            </audio>
            ${isVoiceNote ? '<span class="voice-note-label">Voice Note</span>' : ''}
          `;
          break;

        case 'document':
          const docFileName = fileName || 'document';
          mediaHtml = `
            <a href="${mediaSrc}" download="${this.escapeHtml(docFileName)}" class="document-download">
              <svg viewBox="0 0 24 24" width="24" height="24">
                <path fill="currentColor" d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zm4 18H6V4h7v5h5v11z"/>
              </svg>
              <div class="document-info">
                <span class="document-name">${this.escapeHtml(docFileName)}</span>
                <span class="document-size">Click to download</span>
              </div>
            </a>
          `;
          break;

        case 'sticker':
          mediaHtml = `<img src="${mediaSrc}" alt="Sticker" loading="lazy">`;
          break;

        default:
          mediaHtml = `<img src="${mediaSrc}" alt="Media" loading="lazy">`;
      }

      // Remove lazy-media class and replace content
      container.classList.remove('lazy-media');
      container.innerHTML = mediaHtml;
      const audioPlayer = container.querySelector('audio');
      if (audioPlayer)
        audioPlayer.onerror = () => {
          const status = document.createElement('p');
          status.textContent =
            'Unable to play this voice note. Reload to retry downloading it.';
          status.setAttribute('role', 'alert');
          container.append(status);
        };

      // Also update the message cache so re-renders show the media
      this.updateMessageMediaCache(messageId, data.media_data, actualMimeType);
    } catch (err) {
      console.error('Failed to load media:', err);
      // Show error state
      placeholderEl.classList.remove('loading');
      placeholderEl.classList.add('error');
      placeholderEl.innerHTML = `
        <svg viewBox="0 0 24 24" width="32" height="32">
          <path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z"/>
        </svg>
        <span>Failed to load. Click to retry.</span>
      `;
    }
  },

  // Update message cache with loaded media data
  updateMessageMediaCache(messageId, mediaData, mimeType) {
    // Find the message in the current contact's messages
    if (!this.currentContactId) return;

    const messages = this.messages.get(this.currentContactId);
    if (!messages) return;

    const message = messages.find((m) => m.id === messageId);
    if (message && message.content) {
      message.content.media_data = mediaData;
      message.content.mediaData = mediaData;
      message.content.mime_type = mimeType;
      message.content.mimeType = mimeType;
      // Remove has_media flag since we now have the data
      delete message.content.has_media;
      delete message.content.hasMedia;
    }
  },

  // Render emojis for a category
  renderEmojiCategory(category) {
    const container = document.getElementById('emoji-picker-content');
    if (!container) return;

    let emojis;
    if (category === 'recent') {
      emojis =
        this.recentEmojis.length > 0
          ? this.recentEmojis
          : ['😀', '😊', '😂', '❤️', '👍', '🎉', '🔥', '✨']; // Default if no recent
    } else {
      emojis = this.emojiData[category] || [];
    }

    container.innerHTML = emojis
      .map((emoji) => `<span class="emoji-item">${emoji}</span>`)
      .join('');
  },

  // Insert emoji at cursor position in message input
  insertEmoji(emoji) {
    const input = document.getElementById('message-input');
    if (!input) return;

    const start = input.selectionStart;
    const end = input.selectionEnd;
    const text = input.value;

    // Insert emoji at cursor position
    input.value = text.substring(0, start) + emoji + text.substring(end);

    // Move cursor after the inserted emoji
    const newCursorPos = start + emoji.length;
    input.setSelectionRange(newCursorPos, newCursorPos);

    // Focus the input
    input.focus();

    // Trigger input event to update send button state
    input.dispatchEvent(new Event('input', { bubbles: true }));

    // Close the picker
    document.getElementById('emoji-picker')?.classList.add('hidden');
  },

  // Add emoji to recent list
  addRecentEmoji(emoji) {
    // Remove if already exists
    this.recentEmojis = this.recentEmojis.filter((e) => e !== emoji);
    // Add to front
    this.recentEmojis.unshift(emoji);
    // Keep only last 32 emojis
    this.recentEmojis = this.recentEmojis.slice(0, 32);
    // Save to localStorage
    this.persistRecentEmojis();
  },
};
