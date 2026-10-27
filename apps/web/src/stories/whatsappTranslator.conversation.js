import { ATTACH_ICON, CHECK_ICON, CLOSE_ICON, EMOJI_ICON, MENU_ICON, SEND_ICON, WORKSPACE_ICON, joinClasses, renderAppFrame, renderBadge, renderIconButton } from './whatsappTranslator.shared.js';
import { renderContactsPanel, renderEmptyChatState, renderMessagesList } from './whatsappTranslator.panels.js';

export function renderComposer({ showReply = true, showDraft = true } = {}) {
  return `
    <footer class="message-input-area">
      <div class="reply-preview ${showReply ? '' : 'hidden'}">
        <div class="reply-preview-content">
          <span class="reply-preview-sender">Sofia at Casa Azul</span>
          <span class="reply-preview-text">Can I arrive after 18:00?</span>
        </div>
        <button class="reply-preview-close" type="button" aria-label="Cancel reply">
          ${CLOSE_ICON}
        </button>
      </div>

      <div class="draft-banner ${showDraft ? '' : 'hidden'}">
        <span id="draft-banner-text">Draft saved for this conversation</span>
        <button class="draft-clear-button" type="button">Discard draft</button>
      </div>

      <div class="quick-replies-bar">
        <div class="quick-replies-header">
          <span class="quick-replies-label">Quick replies</span>
          <button class="quick-reply-save" type="button">Save draft</button>
        </div>
        <div class="quick-replies-list">
          <button class="quick-reply-chip" type="button">
            <span class="quick-reply-text">Happy to confirm once the taxi is booked.</span>
            <span class="quick-reply-remove">x</span>
          </button>
          <button class="quick-reply-chip" type="button">
            <span class="quick-reply-text">I will send the corrected deck before lunch.</span>
            <span class="quick-reply-remove">x</span>
          </button>
        </div>
      </div>

      <div class="input-container">
        <div class="emoji-button-container">
          ${renderIconButton('emoji-button', 'Insert emoji', EMOJI_ICON)}
        </div>
        ${renderIconButton('attach-button', 'Attach image', ATTACH_ICON)}
        <textarea id="storybook-message-input" rows="1" maxlength="65536" placeholder="Type a message">18:30 works on our side. Please send the taxi plate when you have it.</textarea>
        <div class="send-button-group">
          <button class="send-button" type="button" title="Send message" aria-label="Send message">
            ${SEND_ICON}
          </button>
          <button class="send-dropdown-toggle" type="button" title="More send options" aria-label="More send options">
            <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true">
              <path fill="currentColor" d="M7 10l5 5 5-5z"></path>
            </svg>
          </button>
        </div>
      </div>
    </footer>
  `;
}

export function renderWorkspacePanel({ collapsed = false, reminderDue = true } = {}) {
  const workspaceClasses = joinClasses(
    'conversation-workspace',
    collapsed && 'collapsed',
    reminderDue && 'has-reminder-due',
  );

  return `
    <aside id="conversation-workspace" class="${workspaceClasses}">
      <div class="conversation-workspace-frame">
        <div class="workspace-header">
          <div>
            <span class="workspace-kicker">Workspace</span>
            <h3>Conversation context</h3>
          </div>
          <button class="workspace-edit-button" type="button">Edit</button>
        </div>

        <div class="workspace-summary">
          ${renderBadge({ type: 'priority urgent', label: 'Urgent' })}
          ${renderBadge({ type: 'reply', label: 'Reply' })}
          ${renderBadge({ type: 'label', label: 'Travel' })}
        </div>

        <div class="workspace-section">
          <span class="workspace-section-label">Right now</span>
          <p class="workspace-primary-status">Awaiting the final arrival time and the taxi plate before 18:30.</p>
        </div>

        <div class="workspace-section">
          <span class="workspace-section-label">Notes</span>
          <p class="workspace-notes">Guest is landing from Madrid. Keep replies short, friendly, and practical. Mention the blue gate rather than the street name.</p>
        </div>

        <div class="workspace-section">
          <div class="workspace-section-heading">
            <span class="workspace-section-label">Checklist</span>
            <span class="workspace-section-meta">1 open task</span>
          </div>
          <div class="workspace-checklist">
            <button class="workspace-checklist-item" type="button">
              <span class="workspace-checklist-mark"></span>
              <span class="workspace-checklist-text">Save the taxi plate when it arrives</span>
            </button>
            <button class="workspace-checklist-item done" type="button">
              <span class="workspace-checklist-mark">${CHECK_ICON}</span>
              <span class="workspace-checklist-text">Confirm the lockbox instructions</span>
            </button>
          </div>
        </div>

        <div class="workspace-section">
          <div class="workspace-section-heading">
            <span class="workspace-section-label">Labels</span>
            <span class="workspace-section-meta">18:07 local time</span>
          </div>
          <div class="workspace-labels">
            <span class="workspace-label-chip">Travel</span>
            <span class="workspace-label-chip">Arrival</span>
            <span class="workspace-label-chip">Host</span>
          </div>
        </div>

        <div class="workspace-section">
          <span class="workspace-section-label">Reminder</span>
          <p class="workspace-reminder ${reminderDue ? 'due' : ''}">Reminder due: follow up if the taxi plate has not arrived by 17:45.</p>
        </div>
      </div>
    </aside>
  `;
}

export function renderChatHeader() {
  return `
    <header class="chat-header">
      <button class="back-button" aria-label="Back to contacts" type="button">
        <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true">
          <path fill="currentColor" d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"></path>
        </svg>
      </button>
      <div class="avatar">
        <span>S</span>
      </div>
      <div class="chat-info">
        <span class="chat-name">Sofia at Casa Azul</span>
        <span class="chat-note-indicator">Travel host · arrival reminder due</span>
      </div>
      <div class="chat-actions">
        ${renderIconButton('chat-settings-button chat-workspace-button', 'Toggle workspace', WORKSPACE_ICON)}
        <span class="cost-badge chat-cost-badge">$2.38</span>
        ${renderIconButton('chat-settings-button chat-menu-button', 'Conversation actions', MENU_ICON)}
      </div>
    </header>
  `;
}

export function renderConversationLayout({ mobileWorkspaceOpen = false } = {}) {
  return renderAppFrame(`
    <div id="main-container" class="chat-open">
      ${renderContactsPanel({ activeId: 'sofia' })}
      <main id="messages-panel">
        <div id="chat-view" class="chat-view ${mobileWorkspaceOpen ? 'workspace-open' : ''}">
          ${renderChatHeader()}
          <button class="workspace-backdrop ${mobileWorkspaceOpen ? '' : 'hidden'}" type="button" aria-label="Close conversation context"></button>
          <div class="chat-shell">
            <div class="chat-main-column">
              ${renderMessagesList()}
              ${renderComposer()}
            </div>
            ${renderWorkspacePanel()}
          </div>
        </div>
      </main>
    </div>
  `, { page: true });
}

export function renderInboxLayout() {
  return renderAppFrame(`
    <div id="main-container">
      ${renderContactsPanel({ activeId: null })}
      <main id="messages-panel">
        ${renderEmptyChatState()}
      </main>
    </div>
  `, { page: true });
}
