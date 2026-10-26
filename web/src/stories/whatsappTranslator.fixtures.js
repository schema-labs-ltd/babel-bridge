import { escapeHtml, renderAppFrame } from './whatsappTranslator.shared.js';

export { escapeHtml, renderAppFrame } from './whatsappTranslator.shared.js';
export { renderAuthOverlay, renderContactItem, renderContactsPanel, renderEmptyChatState, renderMessagesList, renderVisitorDashboard } from './whatsappTranslator.panels.js';
export { renderChatHeader, renderComposer, renderConversationLayout, renderInboxLayout, renderWorkspacePanel } from './whatsappTranslator.conversation.js';

export function renderComponentPreview(content, variant = 'chat') {
  return renderAppFrame(`
    <div class="sb-component-card sb-component-card--${escapeHtml(variant)}">
      ${content}
    </div>
  `);
}
