// Event bindings for pickers, menus, modals and keyboard shortcuts, plus mobile chat closing.

export const menuEventsMethods = {
  // Emoji picker, send menu, header and settings buttons, context menu,
  // modals and keyboard shortcuts. Called at the end of bindEvents.
  bindMenuAndModalEvents() {
    // Emoji picker button
    const emojiButton = document.getElementById('emoji-button');
    const emojiPicker = document.getElementById('emoji-picker');

    emojiButton?.addEventListener('click', (e) => {
      e.stopPropagation();
      emojiPicker?.classList.toggle('hidden');
      if (!emojiPicker?.classList.contains('hidden')) {
        this.renderEmojiCategory(this.currentEmojiCategory);
      }
    });

    // Emoji tab clicks
    document.querySelectorAll('.emoji-tab').forEach((tab) => {
      tab.addEventListener('click', (e) => {
        e.stopPropagation();
        const category = tab.dataset.category;
        this.currentEmojiCategory = category;
        document
          .querySelectorAll('.emoji-tab')
          .forEach((t) => t.classList.remove('active'));
        tab.classList.add('active');
        this.renderEmojiCategory(category);
      });
    });

    // Emoji selection (delegated)
    document
      .getElementById('emoji-picker-content')
      ?.addEventListener('click', (e) => {
        const emojiSpan = e.target.closest('.emoji-item');
        if (emojiSpan) {
          const emoji = emojiSpan.textContent;
          this.insertEmoji(emoji);
          this.addRecentEmoji(emoji);
        }
      });

    // Send dropdown toggle
    const dropdownToggle = document.getElementById('send-dropdown-toggle');
    const sendDropdown = document.getElementById('send-dropdown');

    dropdownToggle?.addEventListener('click', (e) => {
      e.stopPropagation();
      sendDropdown?.classList.toggle('hidden');
    });

    // Send with AI button
    const sendAiButton = document.getElementById('send-ai-button');
    sendAiButton?.addEventListener('click', () => {
      sendDropdown?.classList.add('hidden');
      this.sendWithAI();
    });

    document
      .getElementById('quick-reply-save')
      ?.addEventListener('click', () => {
        this.saveCurrentDraftAsQuickReply();
      });

    document
      .getElementById('quick-replies-list')
      ?.addEventListener('click', (event) => {
        const removeTarget = event.target.closest('[data-quick-reply-remove]');
        if (removeTarget) {
          event.stopPropagation();
          this.deleteQuickReply(removeTarget.dataset.quickReplyRemove);
          return;
        }

        const quickReplyButton = event.target.closest('[data-quick-reply-id]');
        if (quickReplyButton) {
          this.applyQuickReply(quickReplyButton.dataset.quickReplyId);
        }
      });

    document
      .getElementById('app-appearance-button')
      ?.addEventListener('click', () => {
        this.openAppearanceModal();
      });

    document
      .getElementById('chat-menu-button')
      ?.addEventListener('click', (event) => {
        event.stopPropagation();
        this.toggleConversationMenu();
      });

    document
      .getElementById('chat-search-toggle')
      ?.addEventListener('click', () => {
        this.toggleChatSearch();
        this.closeConversationMenu();
      });

    document
      .getElementById('chat-search-clear')
      ?.addEventListener('click', () => {
        this.clearChatSearch();
      });

    document
      .getElementById('chat-search-input')
      ?.addEventListener('input', (event) => {
        this.setChatSearchQuery(event.target.value);
      });

    document
      .getElementById('chat-starred-toggle')
      ?.addEventListener('click', () => {
        this.toggleStarredOnly();
        this.closeConversationMenu();
      });

    document
      .getElementById('chat-translate-all')
      ?.addEventListener('click', () => {
        this.translateUntranslatedIncomingMessages();
        this.closeConversationMenu();
      });

    // Chat settings button in header
    document
      .getElementById('chat-settings-button')
      ?.addEventListener('click', () => {
        this.openSettingsModal();
        this.closeConversationMenu();
      });

    document
      .getElementById('chat-workspace-toggle')
      ?.addEventListener('click', () => {
        this.toggleWorkspacePanel();
      });

    document
      .getElementById('workspace-backdrop')
      ?.addEventListener('click', () => {
        this.toggleWorkspacePanel(false);
      });

    document
      .getElementById('workspace-settings-button')
      ?.addEventListener('click', () => {
        this.openSettingsModal();
      });

    document
      .getElementById('workspace-copy-brief')
      ?.addEventListener('click', () => {
        this.copyConversationBrief();
      });

    document
      .getElementById('workspace-translate-all')
      ?.addEventListener('click', () => {
        this.translateUntranslatedIncomingMessages();
      });

    document
      .getElementById('workspace-checklist')
      ?.addEventListener('click', (event) => {
        const item = event.target.closest('[data-workspace-checklist]');
        if (item) {
          this.toggleWorkspaceChecklistItem(item.dataset.workspaceChecklist);
        }
      });

    // Context menu for contacts (right-click)
    document
      .getElementById('contacts-list')
      .addEventListener('contextmenu', (e) => {
        const contactItem = e.target.closest('.contact-item');
        if (contactItem) {
          e.preventDefault();
          const contactId = contactItem.dataset.contactId;
          this.showContactContextMenu(e, contactId);
        }
      });

    // Context menu items
    document
      .getElementById('contact-context-menu')
      ?.addEventListener('click', (e) => {
        const item = e.target.closest('.context-menu-item');
        if (!item) return;

        const action = item.dataset.action;
        const contactId = this.contextMenuContactId;

        if (action === 'pin') {
          this.togglePin(contactId);
        } else if (action === 'settings') {
          this.openSettingsModal(contactId);
        }

        this.hideContactContextMenu();
      });

    // Close context menu when clicking outside
    document.addEventListener('click', (e) => {
      if (!e.target.closest('#contact-context-menu')) {
        this.hideContactContextMenu();
      }
    });

    // Settings modal events
    document
      .querySelector('#settings-modal .modal-close')
      ?.addEventListener('click', () => {
        this.closeSettingsModal();
      });

    document
      .querySelector('#settings-modal .modal-backdrop')
      ?.addEventListener('click', () => {
        this.closeSettingsModal();
      });

    document
      .querySelector('#appearance-modal .modal-close')
      ?.addEventListener('click', () => {
        this.closeAppearanceModal();
      });

    document
      .querySelector('#appearance-modal .modal-backdrop')
      ?.addEventListener('click', () => {
        this.closeAppearanceModal();
      });

    document
      .getElementById('appearance-cancel')
      ?.addEventListener('click', () => {
        this.closeAppearanceModal();
      });

    document
      .getElementById('appearance-save')
      ?.addEventListener('click', () => {
        this.saveAppearanceSettings();
      });

    document
      .getElementById('workspace-export-button')
      ?.addEventListener('click', () => {
        this.downloadWorkspaceStateExport();
      });

    document
      .getElementById('workspace-import-button')
      ?.addEventListener('click', () => {
        document.getElementById('workspace-import-input')?.click();
      });

    document
      .getElementById('workspace-import-input')
      ?.addEventListener('change', async (event) => {
        const file = event.target.files?.[0];
        await this.importWorkspaceStateFromFile(file);
        event.target.value = '';
      });

    document
      .getElementById('appearance-theme-select')
      ?.addEventListener('change', () => {
        this.syncAppearanceControls({
          theme: document.getElementById('appearance-theme-select')?.value,
          mode: document.getElementById('appearance-mode-select')?.value,
        });
      });

    document
      .getElementById('appearance-mode-select')
      ?.addEventListener('change', () => {
        this.syncAppearanceControls({
          theme: document.getElementById('appearance-theme-select')?.value,
          mode: document.getElementById('appearance-mode-select')?.value,
        });
      });

    document
      .getElementById('settings-cancel')
      ?.addEventListener('click', () => {
        this.closeSettingsModal();
      });

    document.getElementById('settings-save')?.addEventListener('click', () => {
      this.saveConversationSettings();
    });
    document
      .getElementById('translation-enabled')
      ?.addEventListener('change', () =>
        this.updateTranslationSettingsControls(),
      );

    document
      .getElementById('command-palette-input')
      ?.addEventListener('input', (event) => {
        this.commandPaletteQuery = event.target.value || '';
        this.commandPaletteSelectionIndex = 0;
        this.renderCommandPalette();
      });

    document
      .getElementById('command-palette-results')
      ?.addEventListener('click', (event) => {
        const item = event.target.closest('[data-command-index]');
        if (!item) return;
        this.commandPaletteSelectionIndex = Number(
          item.dataset.commandIndex || 0,
        );
        this.activateCommandPaletteSelection();
      });

    document
      .querySelector('#command-palette .command-palette-backdrop')
      ?.addEventListener('click', () => {
        this.closeCommandPalette();
      });

    document.addEventListener('keydown', (e) => {
      const target = e.target;
      const isEditable =
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName));

      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        if (this.commandPaletteOpen) {
          this.closeCommandPalette();
        } else {
          this.openCommandPalette();
        }
        return;
      }

      if (this.commandPaletteOpen) {
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          this.moveCommandPaletteSelection(1);
        } else if (e.key === 'ArrowUp') {
          e.preventDefault();
          this.moveCommandPaletteSelection(-1);
        } else if (e.key === 'Enter') {
          e.preventDefault();
          this.activateCommandPaletteSelection();
        } else if (e.key === 'Escape') {
          e.preventDefault();
          this.closeCommandPalette();
        }
        return;
      }

      if (
        !isEditable &&
        e.key === '/' &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.altKey
      ) {
        e.preventDefault();
        document.getElementById('inbox-search-input')?.focus();
        document.getElementById('inbox-search-input')?.select();
        return;
      }

      if (!isEditable && e.key.toLowerCase() === 'w' && this.currentContactId) {
        e.preventDefault();
        this.toggleWorkspacePanel();
      }
    });

    // Close modal on Escape key
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        if (this.commandPaletteOpen) {
          this.closeCommandPalette();
          return;
        }
        this.closeSettingsModal();
        this.closeAppearanceModal();
        this.hideContactContextMenu();
        if (
          !document
            .getElementById('chat-search-bar')
            ?.classList.contains('hidden')
        ) {
          this.toggleChatSearch();
        }
      }
    });
  },

  // Check if on mobile device
  isMobile() {
    return window.innerWidth <= 768;
  },

  // Close chat view (mobile)
  closeChat() {
    this.currentContactId = null;
    this.messageSearchQuery = '';
    this.starredOnly = false;
    this.clearReply();
    document
      .getElementById('chat-view')
      .querySelector('#chat-search-bar')
      ?.classList.add('hidden');
    const chatSearchInput = document.getElementById('chat-search-input');
    if (chatSearchInput) chatSearchInput.value = '';
    document.getElementById('main-container').classList.remove('chat-open');
    document.getElementById('chat-view').classList.add('hidden');
    document.getElementById('no-chat-selected').classList.remove('hidden');
    this.closeConversationMenu();
    if (this.isMobile()) {
      this.workspaceExpanded = false;
    }
    this.renderQuickReplies();
    this.updateStarredToggleUI();
    this.updateChatSearchUI();
    this.updateChatHeaderNote();
    this.updateWorkspaceUI();
    this.renderContacts();

    // Update URL without chat parameter
    if (this.isMobile()) {
      history.replaceState({}, '', window.location.pathname);
    }
  },
};
