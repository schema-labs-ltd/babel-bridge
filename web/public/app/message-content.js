// Rendering message content by type, appending messages and drafts.

import {
  getDraftText,
  upsertDraft,
} from '../app-state.js?v=20260912-conversation-translation';

export const messageContentMethods = {
  // Render message content
  renderContent(message) {
    const content = message.content;
    const isTranslated = message.is_translated || message.isTranslated;
    const isFromMe = message.isFromMe || message.is_from_me;
    const showTranslatedPrimary = Boolean(
      content?.showTranslatedPrimary || content?.show_translated_primary,
    );

    // Display logic - always show MY language (English) in the bubble:
    // - Outgoing translated: show original_text (English - what I typed)
    // - Outgoing non-translated: show content.body (English - what I typed)
    // - Incoming translated: show translated_text (English translation of what they sent)
    // - Incoming non-translated: show content.body/caption (already English)
    let displayText;
    let displayCaption;

    if (isTranslated && isFromMe && showTranslatedPrimary) {
      displayText =
        message.translated_text ||
        message.translatedText ||
        content.body ||
        content.text ||
        '';
      displayCaption = content.caption
        ? message.translated_text || message.translatedText || content.caption
        : null;
    } else if (isTranslated && isFromMe) {
      // Outgoing translated: show original_text (English - what I typed)
      displayText =
        message.original_text ||
        message.originalText ||
        content.body ||
        content.text ||
        '';
      displayCaption = content.caption
        ? message.original_text || message.originalText || content.caption
        : null;
    } else if (isTranslated && !isFromMe) {
      // Incoming translated: show the English translation
      // translated_text contains the translated body or caption
      const translatedContent =
        message.translated_text || message.translatedText || '';
      displayText = translatedContent || content.body || content.text || '';
      // For media with captions, the translated_text IS the translated caption
      displayCaption = content.caption ? translatedContent : null;
    } else {
      // Non-translated: show content.body/caption as-is
      displayText = content.body || content.text || '';
      displayCaption = content.caption || null;
    }

    // Extract URLs for link previews
    const urls = this.extractUrls(displayText);
    const hasUrls = urls.length > 0;

    switch (content.type) {
      case 'text':
        // Use single quotes for data-urls attribute since JSON contains double quotes
        const urlsJson = hasUrls
          ? JSON.stringify(urls).replace(/'/g, '&#39;')
          : '';
        return `
          <div class="message-text">${this.linkifyText(displayText)}</div>
          ${hasUrls ? `<div class="link-previews-container" data-urls='${urlsJson}'></div>` : ''}
        `;

      case 'image':
        // Check if we have the actual image data or if it needs to be lazy loaded
        const mediaData = content.media_data || content.mediaData;
        const hasMedia = content.has_media || content.hasMedia;
        const messageId = message.id;
        const mimeType = content.mime_type || content.mimeType || 'image/jpeg';

        if (mediaData) {
          const imgSrc = mediaData.startsWith('data:')
            ? mediaData
            : `data:${mimeType};base64,${mediaData}`;
          return `
            <div class="message-image">
              <img src="${imgSrc}" alt="Image" loading="lazy" data-fullscreen-toggle="true">
            </div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        } else if (hasMedia) {
          // Media needs to be lazy loaded - show placeholder
          return `
            <div class="message-image lazy-media" data-message-id="${this.escapeHtml(messageId)}" data-mime-type="${this.escapeHtml(mimeType)}" data-media-type="image">
              <div class="media-placeholder">
                <svg viewBox="0 0 24 24" width="48" height="48">
                  <path fill="currentColor" d="M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z"/>
                </svg>
                <span>Click to load image</span>
              </div>
            </div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        } else {
          // No media available
          return `
            <div class="message-media image">[ Image unavailable ]${content.file_size ? ' - ' + this.formatSize(content.file_size) : ''}</div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        }

      case 'video':
        const videoData = content.media_data || content.mediaData;
        const videoHasMedia = content.has_media || content.hasMedia;
        const videoMsgId = message.id;
        const videoMime = content.mime_type || content.mimeType || 'video/mp4';

        if (videoData) {
          const videoSrc = videoData.startsWith('data:')
            ? videoData
            : `data:${videoMime};base64,${videoData}`;
          return `
            <div class="message-video">
              <video controls preload="metadata">
                <source src="${videoSrc}" type="${videoMime}">
                Your browser does not support video playback.
              </video>
            </div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        } else if (videoHasMedia) {
          // Media needs to be lazy loaded - show placeholder
          const durationText = content.duration_seconds
            ? this.formatDuration(content.duration_seconds)
            : '';
          return `
            <div class="message-video lazy-media" data-message-id="${this.escapeHtml(videoMsgId)}" data-mime-type="${this.escapeHtml(videoMime)}" data-media-type="video">
              <div class="media-placeholder">
                <svg viewBox="0 0 24 24" width="48" height="48">
                  <path fill="currentColor" d="M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z"/>
                </svg>
                <span>Click to load video${durationText ? ` (${durationText})` : ''}</span>
              </div>
            </div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        } else {
          return `
            <div class="message-media video">[ Video unavailable ]${content.duration_seconds ? ' - ' + this.formatDuration(content.duration_seconds) : ''}${content.file_size ? ' - ' + this.formatSize(content.file_size) : ''}</div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        }

      case 'audio':
        const audioData = content.media_data || content.mediaData;
        const audioHasMedia = content.has_media || content.hasMedia;
        const audioMsgId = message.id;
        const audioMime = content.mime_type || content.mimeType || 'audio/ogg';
        const isVoiceNote = content.is_voice_note || content.isVoiceNote;

        if (audioData && audioMime === 'audio/mpeg') {
          const src = audioData.startsWith('data:')
            ? audioData
            : `data:audio/mpeg;base64,${audioData}`;
          return `<div class="message-audio ${isVoiceNote ? 'voice-note' : ''}"><audio controls preload="metadata" src="${this.escapeHtml(src)}"></audio></div>`;
        } else if (audioHasMedia || audioData) {
          // Media needs to be lazy loaded - show placeholder
          const audioType = isVoiceNote ? 'voice note' : 'audio';
          const durationText = content.duration_seconds
            ? this.formatDuration(content.duration_seconds)
            : '';
          return `
            <div class="message-audio lazy-media ${isVoiceNote ? 'voice-note' : ''}" data-message-id="${this.escapeHtml(audioMsgId)}" data-mime-type="${this.escapeHtml(audioMime)}" data-media-type="audio">
              <div class="media-placeholder">
                <svg viewBox="0 0 24 24" width="32" height="32">
                  <path fill="currentColor" d="M12 3v9.28c-.47-.17-.97-.28-1.5-.28C8.01 12 6 14.01 6 16.5S8.01 21 10.5 21c2.31 0 4.2-1.75 4.45-4H15V6h4V3h-7z"/>
                </svg>
                <span>Click to load ${audioType}${durationText ? ` (${durationText})` : ''}</span>
              </div>
            </div>
          `;
        } else {
          const audioType = isVoiceNote ? 'Voice Note' : 'Audio';
          return `<div class="message-media audio">[ ${audioType} unavailable ]${content.duration_seconds ? ' - ' + this.formatDuration(content.duration_seconds) : ''}</div>`;
        }

      case 'document':
        const docData = content.media_data || content.mediaData;
        const docHasMedia = content.has_media || content.hasMedia;
        const docMsgId = message.id;
        const fileName = content.file_name || content.fileName || 'document';
        const docMime =
          content.mime_type || content.mimeType || 'application/octet-stream';

        if (docData) {
          const docSrc = docData.startsWith('data:')
            ? docData
            : `data:${docMime};base64,${docData}`;
          return `
            <div class="message-document">
              <a href="${docSrc}" download="${this.escapeHtml(fileName)}" class="document-download">
                <svg viewBox="0 0 24 24" width="24" height="24">
                  <path fill="currentColor" d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zm4 18H6V4h7v5h5v11z"/>
                </svg>
                <div class="document-info">
                  <span class="document-name">${this.escapeHtml(fileName)}</span>
                  <span class="document-size">${content.file_size ? this.formatSize(content.file_size) : ''}</span>
                </div>
              </a>
            </div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        } else if (docHasMedia) {
          // Media needs to be lazy loaded - show placeholder
          const sizeText = content.file_size
            ? this.formatSize(content.file_size)
            : '';
          return `
            <div class="message-document lazy-media" data-message-id="${this.escapeHtml(docMsgId)}" data-mime-type="${this.escapeHtml(docMime)}" data-media-type="document" data-file-name="${this.escapeHtml(fileName)}">
              <div class="media-placeholder document-placeholder">
                <svg viewBox="0 0 24 24" width="24" height="24">
                  <path fill="currentColor" d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zm4 18H6V4h7v5h5v11z"/>
                </svg>
                <div class="document-info">
                  <span class="document-name">${this.escapeHtml(fileName)}</span>
                  <span class="document-size">${sizeText ? sizeText : 'Click to download'}</span>
                </div>
              </div>
            </div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        } else {
          return `
            <div class="message-media document">[ Document unavailable: ${this.escapeHtml(fileName)} ]${content.file_size ? ' - ' + this.formatSize(content.file_size) : ''}</div>
            ${displayCaption ? `<div class="message-caption">${this.escapeHtml(displayCaption)}</div>` : ''}
          `;
        }

      case 'sticker':
        const stickerData = content.media_data || content.mediaData;
        const stickerHasMedia = content.has_media || content.hasMedia;
        const stickerMsgId = message.id;
        const stickerMime =
          content.mime_type || content.mimeType || 'image/webp';
        const isAnimated = content.is_animated || content.isAnimated;

        if (stickerData) {
          const stickerSrc = stickerData.startsWith('data:')
            ? stickerData
            : `data:${stickerMime};base64,${stickerData}`;
          return `
            <div class="message-sticker">
              <img src="${stickerSrc}" alt="Sticker" loading="lazy">
            </div>
          `;
        } else if (stickerHasMedia) {
          // Media needs to be lazy loaded - show placeholder
          return `
            <div class="message-sticker lazy-media" data-message-id="${this.escapeHtml(stickerMsgId)}" data-mime-type="${this.escapeHtml(stickerMime)}" data-media-type="sticker">
              <div class="media-placeholder sticker-placeholder">
                <svg viewBox="0 0 24 24" width="48" height="48">
                  <path fill="currentColor" d="M21.99 4c0-1.1-.89-2-1.99-2H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h14l4 4-.01-18zM17 11h-4v4h-2v-4H7V9h4V5h2v4h4v2z"/>
                </svg>
                <span>${isAnimated ? 'Animated sticker' : 'Sticker'}</span>
              </div>
            </div>
          `;
        } else {
          return `<div class="message-media">[ ${isAnimated ? 'Animated ' : ''}Sticker unavailable ]</div>`;
        }

      case 'location':
        const lat = content.latitude;
        const lng = content.longitude;
        const locName = content.name || content.address || 'Shared Location';
        const locAddress =
          content.address && content.address !== content.name
            ? content.address
            : '';
        const mapsUrl =
          lat && lng ? `https://www.google.com/maps?q=${lat},${lng}` : null;

        if (mapsUrl) {
          return `
            <div class="message-location">
              <a href="${mapsUrl}" target="_blank" rel="noopener" class="location-link">
                <div class="location-preview">
                  <img src="https://maps.googleapis.com/maps/api/staticmap?center=${lat},${lng}&zoom=15&size=300x150&markers=color:red%7C${lat},${lng}&key=" 
                       data-show-next-on-error="true"
                       alt="Map">
                  <div class="location-placeholder" style="display:none;">
                    <svg viewBox="0 0 24 24" width="32" height="32">
                      <path fill="currentColor" d="M12 2C8.13 2 5 5.13 5 9c0 5.25 7 13 7 13s7-7.75 7-13c0-3.87-3.13-7-7-7zm0 9.5c-1.38 0-2.5-1.12-2.5-2.5s1.12-2.5 2.5-2.5 2.5 1.12 2.5 2.5-1.12 2.5-2.5 2.5z"/>
                    </svg>
                  </div>
                </div>
                <div class="location-info">
                  <span class="location-name">${this.escapeHtml(locName)}</span>
                  ${locAddress ? `<span class="location-address">${this.escapeHtml(locAddress)}</span>` : ''}
                  <span class="location-coords">${lat.toFixed(6)}, ${lng.toFixed(6)}</span>
                </div>
              </a>
            </div>
          `;
        } else {
          return `<div class="message-media">[ Location: ${this.escapeHtml(locName)} ]</div>`;
        }

      case 'contact':
        return `<div class="message-media">[ Contact: ${this.escapeHtml(content.name)} ]</div>`;

      case 'revoked':
        return `<div class="message-text" style="font-style: italic; opacity: 0.7;">This message was deleted</div>`;

      case 'poll':
        const options = (content.options || [])
          .map((o) => `  - ${this.escapeHtml(o)}`)
          .join('\n');
        return `
          <div class="message-text">
            <strong>Poll: ${this.escapeHtml(content.question)}</strong>
            <pre style="margin-top: 8px; font-family: inherit;">${options}</pre>
          </div>
        `;

      default:
        return `<div class="message-media">[ ${content.rawType || 'Unknown message type'} ]</div>`;
    }
  },

  // Append a single message to the list
  appendMessage(message) {
    if (!this.isDisplayableMessage(message)) return;

    const container = document.getElementById('messages-list');

    // Check if we need a date separator
    const messages = this.messages.get(message.contactId) || [];
    const prevMessage = messages[messages.length - 2];

    let html = '';
    if (prevMessage) {
      const prevDate = new Date(prevMessage.timestamp).toDateString();
      const currDate = new Date(message.timestamp).toDateString();
      if (prevDate !== currDate) {
        html += `<div class="date-separator"><span>${this.formatDate(message.timestamp)}</span></div>`;
      }
    }

    html += this.renderMessage(message);
    container.insertAdjacentHTML('beforeend', html);

    const newMessage = container.lastElementChild;
    if (newMessage) {
      this.scheduleLinkPreviewLoad(newMessage);
    }
  },

  // Scroll to bottom of messages
  scrollToBottom() {
    const container = document.getElementById('messages-list');
    container.scrollTop = container.scrollHeight;
  },

  restoreDraftForCurrentContact() {
    const input = document.getElementById('message-input');
    if (!input || !this.currentContactId) return;

    input.value = getDraftText(this.drafts, this.currentContactId);
    this.autoResizeTextarea(input);
  },

  handleDraftInput() {
    if (!this.currentContactId) return;

    const input = document.getElementById('message-input');
    this.drafts = upsertDraft(
      this.drafts,
      this.currentContactId,
      input?.value || '',
    );
    this.persistDrafts();
    this.renderQuickReplies();
    this.renderComposerAssist();
    this.scheduleRenderContacts();
    this.updateChatHeaderNote();
    this.renderConversationWorkspace();
  },
};
