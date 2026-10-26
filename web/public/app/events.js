// Core DOM event bindings for the inbox, composer, attachments and message list.

export const eventsMethods = {
  // Bind UI events
  bindEvents() {
    // Logout button
    document.getElementById('logout-button')?.addEventListener('click', () => {
      this.handleLogout();
    });

    document
      .getElementById('inbox-search-input')
      ?.addEventListener('input', (event) => {
        this.setInboxSearchQuery(event.target.value);
      });

    document
      .getElementById('inbox-clear-button')
      ?.addEventListener('click', () => {
        this.clearInboxFilters();
      });

    document.querySelectorAll('[data-inbox-filter]').forEach((button) => {
      button.addEventListener('click', () =>
        this.toggleInboxFilter(button.dataset.inboxFilter),
      );
    });

    document
      .getElementById('command-palette-button')
      ?.addEventListener('click', () => {
        this.openCommandPalette();
      });

    document
      .getElementById('demo-mode-button')
      ?.addEventListener('click', () => {
        this.startDemoWorkspace();
      });

    document
      .getElementById('visitor-dashboard-open-palette')
      ?.addEventListener('click', () => {
        this.openCommandPalette();
      });

    document
      .getElementById('visitor-dashboard')
      ?.addEventListener('click', (event) => {
        const presetTarget = event.target.closest('[data-dashboard-preset]');
        if (presetTarget) {
          this.applyInboxPreset(presetTarget.dataset.dashboardPreset);
          return;
        }

        const contactTarget = event.target.closest('[data-dashboard-contact]');
        if (contactTarget) {
          this.selectContact(contactTarget.dataset.dashboardContact);
        }
      });

    // Contact click
    document.getElementById('contacts-list').addEventListener('click', (e) => {
      const target = e.target;
      if (!(target instanceof Element)) {
        return;
      }

      const pinButton = target.closest('[data-contact-action="toggle-pin"]');
      if (pinButton) {
        e.stopPropagation();
        this.togglePin(pinButton.dataset.contactId);
        return;
      }

      const contactItem = target.closest('.contact-item');
      if (contactItem) {
        const contactId = contactItem.dataset.contactId;
        this.selectContact(contactId);
      }
    });

    // Back button (mobile)
    document.getElementById('back-button').addEventListener('click', () => {
      this.closeChat();
    });

    // Handle browser back button on mobile
    window.addEventListener('popstate', (e) => {
      if (this.currentContactId && this.isMobile()) {
        e.preventDefault();
        this.closeChat();
      }
    });

    window.addEventListener('resize', this.handleResponsiveLayoutChange);
    window.addEventListener(
      'orientationchange',
      this.handleResponsiveLayoutChange,
    );
    this.handleResponsiveLayoutChange();

    // Message input
    const input = document.getElementById('message-input');
    const sendButton = document.getElementById('send-button');

    // Update send button state on input
    input.addEventListener('input', () => {
      this.updateSendButton();
      this.autoResizeTextarea(input);
      this.handleDraftInput();
    });

    // Send on Cmd+Enter (Mac) or Ctrl+Enter (Windows/Linux)
    // Plain Enter creates newlines (like WhatsApp desktop)
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        if (!sendButton.disabled) {
          this.sendMessage();
        }
      }
    });

    // Send button click
    sendButton.addEventListener('click', () => {
      this.sendMessage();
    });

    document
      .getElementById('composer-use-suggestion')
      ?.addEventListener('click', () => {
        this.useComposerSuggestedReply();
      });

    document
      .getElementById('reply-preview-close')
      ?.addEventListener('click', () => {
        this.clearReply();
      });

    document
      .getElementById('composer-remind-tomorrow')
      ?.addEventListener('click', () => {
        this.setComposerReminderTomorrow();
      });

    document
      .getElementById('composer-profile-presets')
      ?.addEventListener('click', (event) => {
        const languageTarget = event.target.closest('[data-composer-language]');
        if (languageTarget) {
          this.applyComposerProfilePreset({
            language: languageTarget.dataset.composerLanguage,
          });
          return;
        }

        const toneTarget = event.target.closest('[data-composer-tone]');
        if (toneTarget) {
          this.applyComposerProfilePreset({
            tone: toneTarget.dataset.composerTone,
          });
        }
      });

    document
      .getElementById('composer-smart-replies')
      ?.addEventListener('click', (event) => {
        const replyTarget = event.target.closest('[data-smart-reply-id]');
        if (replyTarget) {
          this.useComposerSuggestedReply(replyTarget.dataset.smartReplyId);
        }
      });

    document
      .getElementById('composer-reminder-presets')
      ?.addEventListener('click', (event) => {
        const presetTarget = event.target.closest('[data-reminder-preset]');
        if (presetTarget) {
          this.setComposerReminderPreset(presetTarget.dataset.reminderPreset);
        }
      });

    document
      .getElementById('composer-edit-profile')
      ?.addEventListener('click', () => {
        this.openSettingsModal();
      });

    // Image attach button
    const attachButton = document.getElementById('attach-button');
    const imageInput = document.getElementById('image-input');

    attachButton.addEventListener('click', () => {
      if (this.currentContactId) {
        imageInput.click();
      }
    });

    // Handle image selection
    imageInput.addEventListener('change', async (e) => {
      const file = e.target.files[0];
      if (file && this.currentContactId) {
        await this.sendImage(file);
      }
      // Reset input so the same file can be selected again
      imageInput.value = '';
    });

    this.startViewedMessageTracking();

    // Handle visibility change (for reconnecting on mobile)
    document.addEventListener('visibilitychange', () => {
      void this.refreshAfterBecomingVisible();
    });
    window.addEventListener('online', () => this.connectWebSocket());

    // Prevent pull-to-refresh on mobile when scrolling messages
    const messagesList = document.getElementById('messages-list');
    messagesList.addEventListener('click', (e) => {
      this.handleMessagesListClick(e);
    });

    messagesList.addEventListener('keydown', (e) => {
      const target = e.target;
      if (!(target instanceof Element)) {
        return;
      }

      if (
        (e.key === 'Enter' || e.key === ' ') &&
        target.closest('.reaction-emoji')
      ) {
        e.preventDefault();
        this.handleMessagesListClick(e);
      }
    });

    messagesList.addEventListener(
      'touchstart',
      (e) => {
        if (messagesList.scrollTop === 0) {
          messagesList.scrollTop = 1;
        }
      },
      { passive: true },
    );

    document.addEventListener(
      'error',
      (event) => {
        const target = event.target;
        if (!(target instanceof HTMLImageElement)) {
          return;
        }

        if (target.dataset.avatarFallback !== undefined) {
          const fallback = document.createElement('span');
          fallback.textContent = target.dataset.avatarFallback || '?';
          target.parentElement?.replaceChildren(fallback);
          return;
        }

        if (target.dataset.hideOnError !== undefined) {
          if (target.parentElement) {
            target.parentElement.style.display = 'none';
          }
          return;
        }

        if (target.dataset.showNextOnError !== undefined) {
          target.style.display = 'none';
          if (target.nextElementSibling instanceof HTMLElement) {
            target.nextElementSibling.style.display = 'flex';
          }
        }
      },
      true,
    );

    // Close reaction pickers and emoji picker when clicking outside
    document.addEventListener('click', (e) => {
      if (!e.target.closest('.reaction-button-container')) {
        document
          .querySelectorAll('.reaction-picker.show')
          .forEach((el) => el.classList.remove('show'));
      }
      // Close send dropdown when clicking outside
      if (!e.target.closest('.send-button-group')) {
        document.getElementById('send-dropdown')?.classList.add('hidden');
      }
      // Close emoji picker when clicking outside
      if (!e.target.closest('.emoji-button-container')) {
        document.getElementById('emoji-picker')?.classList.add('hidden');
      }
      // Close conversation actions menu when clicking outside
      if (!e.target.closest('.chat-actions')) {
        this.closeConversationMenu();
      }
    });

    this.bindMenuAndModalEvents();
  },
};
