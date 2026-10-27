import { APPEARANCE_ICON, CHAT_GLYPH, LOGOUT_ICON, PIN_ICON, SAMPLE_CONTACTS, SAMPLE_FOCUS_ITEMS, SAMPLE_MESSAGES, SAMPLE_REMINDERS, SEARCH_ICON, escapeHtml, joinClasses, renderAppFrame, renderBadge, renderDashboardListItem, renderIconButton, renderMessage, renderStatCard } from './whatsappTranslator.shared.js';

export function renderAuthOverlay({ showError = false } = {}) {
  return renderAppFrame(`
    <div class="overlay">
      <div class="overlay-content overlay-card auth-card">
        <div class="overlay-badge" aria-hidden="true">WT</div>
        <span class="overlay-kicker">Protected workspace</span>
        <h2>Unlock WhatsApp Translator</h2>
        <p>Sign in to open your inbox, follow-ups, and translation tools.</p>
        <form class="password-form">
          <input type="password" id="password-input" placeholder="Enter password" autocomplete="current-password">
          <button class="password-button" type="submit">Continue</button>
        </form>
        <p class="password-error ${showError ? '' : 'hidden'}">Incorrect password. Try again.</p>
      </div>
    </div>
  `);
}

export function renderContactItem(contact, { activeId = null } = {}) {
  const isActive = contact.active || contact.id === activeId;
  const classes = joinClasses(
    'contact-item',
    isActive && 'active',
    contact.group && 'is-group',
    contact.pinned && 'is-pinned',
    contact.important && 'is-important',
    contact.due && 'has-due-reminder',
  );
  const previewClass = contact.previewClass || 'preview-text';

  return `
    <div class="${classes}" data-contact-id="${escapeHtml(contact.id)}">
      <div class="avatar-container">
        <div class="avatar">
          <span>${escapeHtml(contact.initial || contact.name.charAt(0))}</span>
          ${contact.group ? '<div class="group-indicator"></div>' : ''}
        </div>
        <button class="pin-button ${contact.pinned ? 'pinned' : ''}" type="button" title="${contact.pinned ? 'Unpin' : 'Pin'}" aria-label="${contact.pinned ? 'Unpin' : 'Pin'}">
          ${PIN_ICON}
        </button>
      </div>
      <div class="contact-details">
        <div class="contact-header">
          <div class="contact-title">
            <span class="contact-name">${escapeHtml(contact.name)}</span>
            ${(contact.badges || []).map(renderBadge).join('')}
          </div>
          <span class="contact-time">${escapeHtml(contact.time)}</span>
        </div>
        <div class="contact-preview">
          <span class="${previewClass}">${escapeHtml(contact.preview)}</span>
          ${contact.unread ? `<span class="unread-badge">${escapeHtml(contact.unread)}</span>` : ''}
        </div>
      </div>
    </div>
  `;
}

export function renderContactsPanel({ activeId = 'jules', contacts = SAMPLE_CONTACTS } = {}) {
  return `
    <aside id="contacts-panel">
      <header class="panel-header">
        <div class="user-info">
          <div class="avatar">
            <span>W</span>
          </div>
          <div class="user-details">
            <span id="user-name">Workspace owner</span>
            <span id="user-phone" class="phone">+44 20 5555 0199</span>
          </div>
        </div>
        <div class="header-actions">
          <div class="status-indicator">
            <span class="status-dot connected"></span>
            <span>Connected</span>
          </div>
          ${renderIconButton('header-icon-button', 'Command palette', SEARCH_ICON)}
          ${renderIconButton('header-icon-button', 'Appearance', APPEARANCE_ICON)}
          ${renderIconButton('logout-button', 'Logout', LOGOUT_ICON)}
        </div>
      </header>

      <section class="inbox-toolbar">
        <label class="inbox-search" for="storybook-inbox-search">
          ${SEARCH_ICON}
          <input id="storybook-inbox-search" type="search" value="" placeholder="Search chats, aliases, drafts, or notes">
        </label>
        <div class="inbox-filter-row">
          <button class="inbox-filter-chip active" type="button">Needs reply</button>
          <button class="inbox-filter-chip" type="button">Drafts</button>
          <button class="inbox-filter-chip" type="button">Tasks</button>
          <button class="inbox-filter-chip" type="button">Pinned</button>
          <button class="inbox-filter-clear" type="button">Clear</button>
        </div>
      </section>

      <div class="contacts-list">
        ${contacts.map((contact) => renderContactItem(contact, { activeId })).join('')}
      </div>

      <footer class="sidebar-footer">
        <div class="usage-info">
          <span class="usage-label">Translation Cost</span>
          <span class="usage-cost">$14.82</span>
        </div>
      </footer>
    </aside>
  `;
}

export function renderVisitorDashboard() {
  const stats = [
    { value: 4, label: 'Needs reply', description: 'Chats waiting on you', active: true },
    { value: 2, label: 'Due reminders', description: 'Follow-ups due now' },
    { value: 3, label: 'Saved drafts', description: 'Replies already started' },
    { value: 5, label: 'Open tasks', description: 'Checklist items still open' },
  ];

  return `
    <section id="visitor-dashboard" class="visitor-dashboard">
      <div class="visitor-dashboard-hero">
        <div>
          <span class="visitor-dashboard-kicker">Today</span>
          <h3>Start with the conversations that matter most</h3>
          <p>Jump straight into replies, reminders, drafts, and open tasks without digging through the inbox.</p>
        </div>
        <button class="visitor-dashboard-command-button" type="button">Open command palette</button>
      </div>

      <div class="visitor-dashboard-stats">
        ${stats.map(renderStatCard).join('')}
      </div>

      <div class="visitor-dashboard-grid">
        <section class="visitor-dashboard-panel">
          <div class="visitor-dashboard-panel-header">
            <h3>Focus queue</h3>
            <span class="visitor-dashboard-count">${SAMPLE_FOCUS_ITEMS.length} active</span>
          </div>
          <div class="visitor-dashboard-list">
            ${SAMPLE_FOCUS_ITEMS.map(renderDashboardListItem).join('')}
          </div>
        </section>

        <section class="visitor-dashboard-panel">
          <div class="visitor-dashboard-panel-header">
            <h3>Upcoming reminders</h3>
            <span class="visitor-dashboard-count">${SAMPLE_REMINDERS.length} scheduled</span>
          </div>
          <div class="visitor-dashboard-list">
            ${SAMPLE_REMINDERS.map(renderDashboardListItem).join('')}
          </div>
        </section>
      </div>
    </section>
  `;
}

export function renderEmptyChatState() {
  return `
    <div class="empty-chat">
      <div class="empty-chat-content">
        <div class="empty-chat-intro">
          <div class="empty-icon">
            ${CHAT_GLYPH}
          </div>
          <h2>WhatsApp Translator</h2>
          <p>Select a conversation to view messages</p>
        </div>
        ${renderVisitorDashboard()}
      </div>
    </div>
  `;
}

export function renderMessagesList(messages = SAMPLE_MESSAGES) {
  return `
    <div id="messages-list" class="messages-list">
      ${messages.map(renderMessage).join('')}
    </div>
  `;
}
