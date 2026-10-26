// Rendering a single message bubble.

import { isMessageStarred } from '../app-state.js?v=20260912-conversation-translation';

export const messageRenderMethods = {
  // Render a single message
  renderMessage(message) {
    const isOutgoing = message.isFromMe || message.is_from_me;
    const isTranslated = message.isTranslated || message.is_translated;
    const time = `${message.content?.edited_at_ms ? 'Edited ' : ''}${this.formatMessageTime(message.timestamp)}`;
    const content = this.renderContent(message);

    let forwarded = '';
    if (message.isForwarded || message.is_forwarded) {
      forwarded = '<div class="message-forwarded">Forwarded</div>';
    }

    let sender = '';
    if (
      !isOutgoing &&
      (message.chatType === 'group' || message.chat_type === 'group')
    ) {
      sender = `<div class="message-sender">${this.escapeHtml(message.senderName || message.sender_name || message.senderPhone || message.sender_phone)}</div>`;
    }

    // Translation indicator
    let translationIndicator = '';
    if (isTranslated) {
      const sourceLanguage =
        message.sourceLanguage || message.source_language || 'Unknown';
      const showTranslatedPrimary = Boolean(
        message.content?.showTranslatedPrimary ||
        message.content?.show_translated_primary,
      );

      // Tooltip shows the "other" version:
      // - Outgoing: show translated_text (what was sent to them in foreign language)
      // - Incoming: show original_text (what they sent in foreign language)
      let tooltipText, tooltipHeader, languageLabel;

      if (isOutgoing && showTranslatedPrimary) {
        tooltipText = message.originalText || message.original_text || '';
        tooltipHeader = 'Original draft';
        languageLabel = 'your text';
      } else if (isOutgoing) {
        // Outgoing: show what was sent (translated foreign text)
        tooltipText = message.translatedText || message.translated_text || '';
        tooltipHeader = 'Sent as';
        languageLabel = sourceLanguage;
      } else {
        // Incoming: show original (foreign text they sent)
        tooltipText = message.originalText || message.original_text || '';
        tooltipHeader = 'Original message';
        languageLabel = sourceLanguage;
      }

      translationIndicator = `
        <span class="translation-indicator" data-message-action="toggle-translation-tooltip">
          <span class="info-icon">i</span>
          <span>Translated</span>
          <div class="original-tooltip">
            <button class="tooltip-close" type="button" data-message-action="close-translation-tooltip">&times;</button>
            <div class="tooltip-header">${tooltipHeader} (${this.escapeHtml(languageLabel)})</div>
            <div class="tooltip-text">${this.escapeHtml(tooltipText)}</div>
          </div>
        </span>
      `;
    }

    // Get message metadata for reactions
    const messageId = message.id;
    const senderJid = this.getMessageSenderJid(message);
    const contactId =
      message.contactId || message.contact_id || this.currentContactId;
    const starred = isMessageStarred(this.starredMessages, messageId);
    const messageIdAttr = this.escapeHtml(messageId);
    const senderJidAttr = this.escapeHtml(senderJid);
    const contactIdAttr = this.escapeHtml(contactId);

    // Check if message can be translated (incoming, has text, not already translated)
    const hasText =
      message.content &&
      (message.content.body || message.content.caption || message.content.text);
    const canTranslate = !isOutgoing && hasText && !isTranslated;

    // Translate button (for untranslated incoming messages)
    const translateButton = `
      <button class="message-action-btn translate-button ${canTranslate ? 'can-translate' : ''}" 
              type="button"
              data-message-action="translate"
              data-message-id="${messageIdAttr}"
              title="${isTranslated ? 'Already translated' : canTranslate ? 'Translate' : 'No text to translate'}">
        <svg viewBox="0 0 24 24"><path fill="currentColor" d="M12.87 15.07l-2.54-2.51.03-.03c1.74-1.94 2.98-4.17 3.71-6.53H17V4h-7V2H8v2H1v1.99h11.17C11.5 7.92 10.44 9.75 9 11.35 8.07 10.32 7.3 9.19 6.69 8h-2c.73 1.63 1.73 3.17 2.98 4.56l-5.09 5.02L4 19l5-5 3.11 3.11.76-2.04zM18.5 10h-2L12 22h2l1.12-3h4.75L21 22h2l-4.5-12zm-2.62 7l1.62-4.33L19.12 17h-3.24z"/></svg>
      </button>
    `;

    // Reply button
    const replyButton = `
      <button class="message-action-btn" type="button" data-message-action="reply" data-message-id="${messageIdAttr}" title="Reply">
        <svg viewBox="0 0 24 24"><path fill="currentColor" d="M10 9V5l-7 7 7 7v-4.1c5 0 8.5 1.6 11 5.1-1-5-4-10-11-11z"/></svg>
      </button>
    `;

    // AI Reply button (only for incoming messages with text content)
    const canAIReply = !isOutgoing && hasText;
    const aiReplyButton = canAIReply
      ? `
      <button class="message-action-btn ai-reply-btn" 
              type="button"
              data-message-action="ai-reply"
              data-message-id="${messageIdAttr}"
              title="Generate AI reply that sounds like you">
        <svg viewBox="0 0 24 24" width="18" height="18">
          <path fill="currentColor" d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>
        </svg>
        <span class="ai-sparkle">✨</span>
      </button>
    `
      : '';

    // Reaction button with quick emoji picker
    const reactionButton = `
      <div class="reaction-button-container">
        <button class="message-action-btn" type="button" data-message-action="toggle-reactions" title="React">
          <svg viewBox="0 0 24 24"><path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm-5-6c.78 2.34 2.72 4 5 4s4.22-1.66 5-4H7zm2-3c.55 0 1-.45 1-1s-.45-1-1-1-1 .45-1 1 .45 1 1 1zm6 0c.55 0 1-.45 1-1s-.45-1-1-1-1 .45-1 1 .45 1 1 1z"/></svg>
        </button>
        <div class="reaction-picker">
          <span class="reaction-emoji" role="button" tabindex="0" data-message-action="react" data-message-id="${messageIdAttr}" data-contact-id="${contactIdAttr}" data-sender-jid="${senderJidAttr}" data-reaction="👍">👍</span>
          <span class="reaction-emoji" role="button" tabindex="0" data-message-action="react" data-message-id="${messageIdAttr}" data-contact-id="${contactIdAttr}" data-sender-jid="${senderJidAttr}" data-reaction="❤️">❤️</span>
          <span class="reaction-emoji" role="button" tabindex="0" data-message-action="react" data-message-id="${messageIdAttr}" data-contact-id="${contactIdAttr}" data-sender-jid="${senderJidAttr}" data-reaction="😂">😂</span>
          <span class="reaction-emoji" role="button" tabindex="0" data-message-action="react" data-message-id="${messageIdAttr}" data-contact-id="${contactIdAttr}" data-sender-jid="${senderJidAttr}" data-reaction="😮">😮</span>
          <span class="reaction-emoji" role="button" tabindex="0" data-message-action="react" data-message-id="${messageIdAttr}" data-contact-id="${contactIdAttr}" data-sender-jid="${senderJidAttr}" data-reaction="😢">😢</span>
          <span class="reaction-emoji" role="button" tabindex="0" data-message-action="react" data-message-id="${messageIdAttr}" data-contact-id="${contactIdAttr}" data-sender-jid="${senderJidAttr}" data-reaction="🙏">🙏</span>
        </div>
      </div>
    `;

    const starButton = `
      <button class="message-action-btn star-toggle ${starred ? 'active' : ''}"
              type="button"
              data-message-action="star"
              data-message-id="${messageIdAttr}"
              title="${starred ? 'Remove star' : 'Star message'}">
        <svg viewBox="0 0 24 24"><path fill="currentColor" d="m12 17.27 6.18 3.73-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"/></svg>
      </button>
    `;

    // Quoted message (reply context)
    let quotedMessage = '';
    if (message.replyContext) {
      const ctx = message.replyContext;
      quotedMessage = `
        <div class="quoted-message">
          <div class="quoted-sender">${this.escapeHtml(ctx.senderName || 'Unknown')}</div>
          <div class="quoted-text">${this.escapeHtml(ctx.text || '')}</div>
        </div>
      `;
    }

    // Reactions display
    const reactionsHtml = this.renderReactions(message.reactions);
    const starredBadge = starred
      ? '<span class="message-starred-badge" title="Starred">★</span>'
      : '';

    return `
      <div class="message ${isOutgoing ? 'outgoing' : 'incoming'}" data-message-id="${messageIdAttr}">
        ${forwarded}
        ${sender}
        ${quotedMessage}
        ${content}
        ${reactionsHtml}
        <div class="message-footer">
          <span class="message-time">${time}</span>
          ${isOutgoing && message.deliveryStatus === 'uncertain' ? '<span class="delivery-uncertain">Delivery uncertain</span>' : ''}
          ${isOutgoing && message.deliveryStatus === 'sending' ? '<span class="delivery-pending">Sending…</span>' : ''}
          ${starredBadge}
          ${translationIndicator}
          <div class="message-actions">
            ${translateButton}
            ${replyButton}
            ${aiReplyButton}
            ${starButton}
            ${reactionButton}
          </div>
        </div>
      </div>
    `;
  },
};
