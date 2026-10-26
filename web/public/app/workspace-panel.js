// Conversation workspace panel and checklist.

import {
  buildConversationBrief,
  buildConversationActionPlan,
  toggleChecklistItem,
} from '../app-state.js?v=20260912-conversation-translation';

export const workspacePanelMethods = {
  toggleWorkspacePanel(force) {
    this.workspaceExpanded =
      typeof force === 'boolean' ? force : !this.workspaceExpanded;
    this.updateWorkspaceUI();
  },

  updateWorkspaceUI() {
    const workspace = document.getElementById('conversation-workspace');
    const toggleButton = document.getElementById('chat-workspace-toggle');
    const backdrop = document.getElementById('workspace-backdrop');
    const chatView = document.getElementById('chat-view');
    const showWorkspaceOverlay =
      this.isMobile() &&
      this.workspaceExpanded &&
      Boolean(this.currentContactId);
    if (workspace) {
      workspace.classList.toggle('collapsed', !this.workspaceExpanded);
    }
    if (toggleButton) {
      toggleButton.classList.toggle('active', this.workspaceExpanded);
      toggleButton.setAttribute(
        'aria-expanded',
        String(this.workspaceExpanded),
      );
      toggleButton.title = this.workspaceExpanded
        ? this.isMobile()
          ? 'Hide conversation context'
          : 'Hide workspace'
        : this.isMobile()
          ? 'Show conversation context'
          : 'Show workspace';
      toggleButton.setAttribute(
        'aria-label',
        this.workspaceExpanded
          ? this.isMobile()
            ? 'Hide conversation context'
            : 'Hide workspace'
          : this.isMobile()
            ? 'Show conversation context'
            : 'Show workspace',
      );
    }
    if (backdrop) {
      backdrop.classList.toggle('hidden', !showWorkspaceOverlay);
    }
    if (chatView) {
      chatView.classList.toggle('workspace-open', showWorkspaceOverlay);
    }
    this.renderConversationWorkspace();
  },

  renderConversationWorkspace() {
    const summaryEl = document.getElementById('conversation-workspace-summary');
    const statusEl = document.getElementById('workspace-primary-status');
    const notesEl = document.getElementById('workspace-notes');
    const checklistEl = document.getElementById('workspace-checklist');
    const checklistSummaryEl = document.getElementById(
      'workspace-checklist-summary',
    );
    const labelsEl = document.getElementById('workspace-labels');
    const reminderEl = document.getElementById('workspace-reminder');
    const timezoneEl = document.getElementById('workspace-timezone');
    const briefEl = document.getElementById('workspace-brief');
    const actionPlanEl = document.getElementById('workspace-action-plan');
    const copyBriefButton = document.getElementById('workspace-copy-brief');
    const translateAllButton = document.getElementById(
      'workspace-translate-all',
    );
    const workspace = document.getElementById('conversation-workspace');
    if (
      !summaryEl ||
      !statusEl ||
      !notesEl ||
      !checklistEl ||
      !labelsEl ||
      !reminderEl ||
      !timezoneEl ||
      !briefEl ||
      !actionPlanEl ||
      !workspace
    ) {
      return;
    }

    if (!this.currentContactId) {
      summaryEl.innerHTML = '';
      statusEl.textContent = 'No local context yet.';
      notesEl.textContent = 'No notes yet.';
      checklistEl.innerHTML = '';
      checklistSummaryEl.textContent = 'No tasks';
      labelsEl.innerHTML = '';
      reminderEl.textContent = 'No reminder set.';
      timezoneEl.textContent = '';
      briefEl.innerHTML = '';
      actionPlanEl.innerHTML = '';
      if (copyBriefButton) copyBriefButton.disabled = true;
      if (translateAllButton) translateAllButton.disabled = true;
      return;
    }

    const contact = this.contacts.find(
      (entry) => entry.id === this.currentContactId,
    ) || { id: this.currentContactId };
    const snapshot = this.getContactActionSnapshot(this.currentContactId);
    const metadata = this.getContactMetadata(this.currentContactId);
    const checklist = Array.isArray(metadata.checklist)
      ? metadata.checklist
      : [];
    const labels = snapshot.labels;
    const notes = String(metadata.notes || '').trim();
    const brief = buildConversationBrief({
      contact,
      metadata,
      messages: this.messages.get(this.currentContactId) || [],
      drafts: this.drafts,
    });
    const actionPlan = buildConversationActionPlan({
      contact,
      metadata,
      messages: this.messages.get(this.currentContactId) || [],
      drafts: this.drafts,
    });

    summaryEl.innerHTML =
      this.buildContactBadgeMarkup(this.currentContactId) ||
      '<span class="workspace-empty-chip">No visitor context yet</span>';
    statusEl.textContent =
      snapshot.summary ||
      'Add notes, labels, reminders, or tasks to keep this conversation organized.';
    briefEl.innerHTML = `
      <div class="workspace-brief-primary">${this.escapeHtml(brief.nextAction)}</div>
      ${
        brief.contextLines.length > 0
          ? `
        <div class="workspace-brief-lines">
          ${brief.contextLines.map((line) => `<span>${this.escapeHtml(line)}</span>`).join('')}
        </div>
      `
          : ''
      }
    `;
    actionPlanEl.innerHTML = `
      <span class="workspace-section-label">Action plan</span>
      <div class="workspace-action-list">
        ${actionPlan.actions
          .map(
            (action) => `
          <div class="workspace-action-item ${this.escapeHtml(action.priority || 'normal')}">
            <span class="workspace-action-title">${this.escapeHtml(action.label)}</span>
            <span class="workspace-action-detail">${this.escapeHtml(action.detail)}</span>
          </div>
        `,
          )
          .join('')}
      </div>
    `;

    if (copyBriefButton) {
      copyBriefButton.disabled = false;
      copyBriefButton.textContent = 'Copy brief';
    }
    if (translateAllButton) {
      translateAllButton.disabled = actionPlan.untranslatedCount === 0;
      translateAllButton.textContent =
        actionPlan.untranslatedCount === 0
          ? 'Translated'
          : `Translate ${actionPlan.untranslatedCount}`;
    }

    notesEl.textContent =
      notes ||
      'No notes yet. Add private context in conversation settings so you can keep tone, follow-ups, and reminders visible while replying.';
    notesEl.classList.toggle('empty', !notes);

    checklistSummaryEl.textContent = snapshot.checklist.label;
    if (checklist.length === 0) {
      checklistEl.innerHTML =
        '<span class="workspace-empty-chip">No checklist items yet</span>';
    } else {
      checklistEl.innerHTML = checklist
        .map(
          (item) => `
        <button
          class="workspace-checklist-item ${item.done ? 'done' : ''}"
          type="button"
          data-workspace-checklist="${item.id}"
        >
          <span class="workspace-checklist-mark">${item.done ? '✓' : ''}</span>
          <span class="workspace-checklist-text">${this.escapeHtml(item.text)}</span>
        </button>
      `,
        )
        .join('');
    }

    if (labels.length === 0) {
      labelsEl.innerHTML =
        '<span class="workspace-empty-chip">No labels yet</span>';
    } else {
      labelsEl.innerHTML = labels
        .map(
          (label) =>
            `<span class="workspace-label-chip">${this.escapeHtml(label)}</span>`,
        )
        .join('');
    }

    reminderEl.textContent =
      this.getReminderSummary(this.currentContactId) || 'No reminder set.';
    reminderEl.classList.toggle('due', snapshot.reminderStatus === 'due');
    timezoneEl.textContent = snapshot.timezoneInfo
      ? `${snapshot.timezoneInfo.localTime} · ${snapshot.timezoneInfo.statusLabel}`
      : '';

    workspace.classList.toggle(
      'has-reminder-due',
      snapshot.reminderStatus === 'due',
    );
    workspace.classList.toggle('collapsed', !this.workspaceExpanded);
  },

  toggleWorkspaceChecklistItem(itemId) {
    if (!this.currentContactId || !itemId) return;

    const metadata = this.getContactMetadata(this.currentContactId);
    const checklist = toggleChecklistItem(metadata.checklist || [], itemId);
    this.updateContactMetadata(this.currentContactId, { checklist });

    const contact = this.contacts.find(
      (entry) => entry.id === this.currentContactId,
    );
    if (contact) {
      contact.checklist = checklist;
    }

    this.renderContacts();
    this.updateChatHeaderNote();
    this.renderConversationWorkspace();
  },
};
