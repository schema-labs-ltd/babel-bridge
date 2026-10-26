// Command palette actions, search and keyboard selection.

export const commandPaletteMethods = {
  getCommandPaletteActions() {
    const actions = [
      {
        id: 'action-dashboard',
        kind: 'action',
        title: 'Go to Today dashboard',
        subtitle: 'Return to the visitor overview and action queue',
        keywords: 'today dashboard home overview',
        run: () => {
          this.closeCommandPalette();
          this.closeChat();
        },
      },
      {
        id: 'action-inbox-search',
        kind: 'action',
        title: 'Search inbox',
        subtitle: 'Focus the left-side inbox search box',
        keywords: 'search inbox chats filters',
        run: () => {
          this.closeCommandPalette();
          document.getElementById('inbox-search-input')?.focus();
          document.getElementById('inbox-search-input')?.select();
        },
      },
      {
        id: 'action-needs-reply',
        kind: 'action',
        title: 'Show chats that need a reply',
        subtitle: 'Apply the reply queue inbox preset',
        keywords: 'reply queue unread pending',
        run: () => {
          this.closeCommandPalette();
          this.applyInboxPreset('needsReply');
          this.closeChat();
        },
      },
      {
        id: 'action-reminders',
        kind: 'action',
        title: 'Show due reminders',
        subtitle: 'Filter the inbox to follow-ups that are due now',
        keywords: 'reminders due follow-up',
        run: () => {
          this.closeCommandPalette();
          this.applyInboxPreset('dueReminders');
          this.closeChat();
        },
      },
      {
        id: 'action-clear-filters',
        kind: 'action',
        title: 'Clear inbox filters',
        subtitle: 'Reset search and show every visible conversation again',
        keywords: 'clear reset filters inbox',
        run: () => {
          this.closeCommandPalette();
          this.clearInboxFilters();
        },
      },
      {
        id: 'action-appearance',
        kind: 'action',
        title: 'Open appearance settings',
        subtitle: 'Change theme and light/dark mode',
        keywords: 'appearance theme dark light',
        run: () => {
          this.closeCommandPalette();
          this.openAppearanceModal();
        },
      },
    ];

    if (this.currentContactId) {
      actions.unshift(
        {
          id: 'action-workspace',
          kind: 'action',
          title: this.workspaceExpanded
            ? 'Collapse workspace panel'
            : 'Expand workspace panel',
          subtitle: 'Show or hide the conversation context card',
          keywords: 'workspace panel context notes checklist',
          run: () => {
            this.closeCommandPalette();
            this.toggleWorkspacePanel();
          },
        },
        {
          id: 'action-chat-search',
          kind: 'action',
          title: 'Search current conversation',
          subtitle: 'Open in-chat search and focus it immediately',
          keywords: 'search messages current chat',
          run: () => {
            this.closeCommandPalette();
            if (
              document
                .getElementById('chat-search-bar')
                ?.classList.contains('hidden')
            ) {
              this.toggleChatSearch();
            } else {
              document.getElementById('chat-search-input')?.focus();
              document.getElementById('chat-search-input')?.select();
            }
          },
        },
        {
          id: 'action-copy-brief',
          kind: 'action',
          title: 'Copy current conversation brief',
          subtitle:
            'Copy next action, context, tasks, draft, and recent messages',
          keywords: 'copy brief handoff summary context',
          run: () => {
            this.closeCommandPalette();
            this.copyConversationBrief();
          },
        },
        {
          id: 'action-translate-all',
          kind: 'action',
          title: 'Translate incoming messages',
          subtitle:
            'Translate every untranslated incoming message in this chat',
          keywords: 'translate incoming unread messages batch',
          run: () => {
            this.closeCommandPalette();
            this.translateUntranslatedIncomingMessages();
          },
        },
        {
          id: 'action-chat-settings',
          kind: 'action',
          title: 'Edit current conversation settings',
          subtitle: 'Open nickname, timezone, and translation settings',
          keywords:
            'settings edit nickname timezone translation language style',
          run: () => {
            this.closeCommandPalette();
            this.openSettingsModal();
          },
        },
      );
    }

    return actions;
  },

  getCommandPaletteItems(query = '') {
    const normalizedQuery = String(query || '')
      .trim()
      .toLowerCase();
    const tokens = normalizedQuery.split(/\s+/).filter(Boolean);
    const matchesTokens = (value) => {
      if (!tokens.length) return true;
      const haystack = String(value || '').toLowerCase();
      return tokens.every((token) => haystack.includes(token));
    };

    const actions = this.getCommandPaletteActions()
      .filter((action) =>
        matchesTokens(`${action.title} ${action.subtitle} ${action.keywords}`),
      )
      .map((action) => ({ ...action }));

    const contacts = (this.contacts || [])
      .map((contact) => {
        const snapshot = this.getContactActionSnapshot(contact.id);
        const searchText = [
          this.getContactDisplayName(contact),
          contact.phone,
          snapshot.summary,
          snapshot.headline,
          this.getContactNotePreview(contact.id),
          snapshot.labels.join(' '),
        ]
          .filter(Boolean)
          .join(' ');
        const matches = matchesTokens(searchText);
        if (!matches) return null;

        let rank =
          snapshot.attentionScore +
          Math.min(20, Math.max(0, Number(contact.unreadCount || 0) * 2));
        if (!tokens.length) {
          rank += 8;
        } else {
          const displayName = this.getContactDisplayName(contact).toLowerCase();
          if (tokens.some((token) => displayName.startsWith(token))) {
            rank += 20;
          }
        }

        return {
          id: `contact-${contact.id}`,
          kind: 'contact',
          contactId: contact.id,
          title: this.getContactDisplayName(contact),
          subtitle: snapshot.summary || snapshot.headline,
          rank,
          snapshot,
          run: () => {
            this.closeCommandPalette();
            this.selectContact(contact.id);
          },
        };
      })
      .filter(Boolean)
      .sort((a, b) => {
        if (b.rank !== a.rank) return b.rank - a.rank;
        return (
          (b.snapshot?.lastMessageTime || 0) -
          (a.snapshot?.lastMessageTime || 0)
        );
      })
      .slice(0, tokens.length ? 8 : 6);

    return [...actions.slice(0, tokens.length ? 6 : 4), ...contacts];
  },

  renderCommandPalette() {
    const container = document.getElementById('command-palette-results');
    const input = document.getElementById('command-palette-input');
    if (!container || !input) return;

    input.value = this.commandPaletteQuery;
    this.commandPaletteItems = this.getCommandPaletteItems(
      this.commandPaletteQuery,
    );
    if (this.commandPaletteItems.length === 0) {
      this.commandPaletteSelectionIndex = 0;
      container.innerHTML = `
        <div class="command-palette-empty">
          <strong>No matching chats or actions.</strong>
          <span>Try a contact name, “reply”, “theme”, or “reminder”.</span>
        </div>
      `;
      return;
    }

    this.commandPaletteSelectionIndex = Math.min(
      this.commandPaletteSelectionIndex,
      this.commandPaletteItems.length - 1,
    );

    container.innerHTML = this.commandPaletteItems
      .map((item, index) => {
        const snapshot = item.snapshot;
        const badge =
          item.kind === 'contact' && snapshot?.headline
            ? `<span class="command-palette-item-badge">${this.escapeHtml(snapshot.headline)}</span>`
            : `<span class="command-palette-item-badge action">Action</span>`;
        const meta =
          item.kind === 'contact'
            ? `<span class="command-palette-item-meta">${this.escapeHtml(snapshot?.lastMessageTime ? this.formatTime(snapshot.lastMessageTime) : 'No messages yet')}</span>`
            : '<span class="command-palette-item-meta">Shortcut</span>';

        return `
        <button
          class="command-palette-item ${index === this.commandPaletteSelectionIndex ? 'active' : ''}"
          type="button"
          data-command-index="${index}"
        >
          <div class="command-palette-item-copy">
            <span class="command-palette-item-title">${this.escapeHtml(item.title)}</span>
            <span class="command-palette-item-subtitle">${this.escapeHtml(item.subtitle || '')}</span>
          </div>
          <div class="command-palette-item-meta-group">
            ${badge}
            ${meta}
          </div>
        </button>
      `;
      })
      .join('');

    container
      .querySelector('.command-palette-item.active')
      ?.scrollIntoView({ block: 'nearest' });
  },

  openCommandPalette(initialQuery = '') {
    const palette = document.getElementById('command-palette');
    const input = document.getElementById('command-palette-input');
    if (!palette || !input) return;

    this.commandPaletteOpen = true;
    this.commandPaletteQuery = initialQuery;
    this.commandPaletteSelectionIndex = 0;
    palette.classList.remove('hidden');
    this.renderCommandPalette();
    input.focus();
    input.select();
  },

  closeCommandPalette() {
    this.commandPaletteOpen = false;
    this.commandPaletteQuery = '';
    this.commandPaletteSelectionIndex = 0;
    this.commandPaletteItems = [];
    document.getElementById('command-palette')?.classList.add('hidden');
  },

  moveCommandPaletteSelection(direction = 1) {
    if (!this.commandPaletteOpen || this.commandPaletteItems.length === 0)
      return;
    const maxIndex = this.commandPaletteItems.length - 1;
    this.commandPaletteSelectionIndex =
      this.commandPaletteSelectionIndex + direction;
    if (this.commandPaletteSelectionIndex < 0) {
      this.commandPaletteSelectionIndex = maxIndex;
    } else if (this.commandPaletteSelectionIndex > maxIndex) {
      this.commandPaletteSelectionIndex = 0;
    }
    this.renderCommandPalette();
  },

  activateCommandPaletteSelection(index = this.commandPaletteSelectionIndex) {
    const item = this.commandPaletteItems[index];
    if (!item) return;
    item.run();
  },
};
