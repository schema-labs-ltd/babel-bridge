// Viewport, sign-in, WebSocket connection, live status and QR linking.

import {
  createReliableFetch,
  reconnectDelay,
  mergeMessageUpdate,
} from '../send-recovery.js';
import { calculateViewportLayout } from '../viewport.js?v=20260616-reactions-ui';

export const connectionMethods = {
  // Keep the app height aligned with iOS Safari's real usable screen area.
  // We only apply a JS viewport override while the keyboard is open. When the
  // keyboard is closed we fall back to CSS large-viewport sizing so the app can
  // occupy the full screen instead of stopping above Safari's bottom chrome.
  setupVisualViewport() {
    const root = document.documentElement;
    let frame = null;

    const updateViewportHeight = () => {
      if (frame !== null) {
        cancelAnimationFrame(frame);
      }

      frame = requestAnimationFrame(() => {
        const viewport = window.visualViewport;
        const layout = calculateViewportLayout({
          innerHeight: window.innerHeight,
          viewportHeight: viewport?.height,
          viewportOffsetTop: viewport?.offsetTop,
        });

        if (layout.effectiveHeight === null) {
          root.style.removeProperty('--viewport-height');
        } else {
          root.style.setProperty(
            '--viewport-height',
            `${layout.effectiveHeight}px`,
          );
        }
        root.style.setProperty(
          '--keyboard-offset',
          `${layout.keyboardOffset}px`,
        );
        frame = null;
      });
    };

    updateViewportHeight();

    if (window.visualViewport) {
      window.visualViewport.addEventListener('resize', updateViewportHeight);
      window.visualViewport.addEventListener('scroll', updateViewportHeight);
    }

    window.addEventListener('resize', updateViewportHeight);
    window.addEventListener('orientationchange', updateViewportHeight);
    window.addEventListener('pageshow', updateViewportHeight);
  },

  async checkAuth() {
    try {
      const response = await fetch('/api/auth/check');
      const data = await response.json();
      return data.required;
    } catch (err) {
      console.error('Failed to check auth:', err);
      return false;
    }
  },

  async verifyToken() {
    try {
      // Try to make an authenticated request to verify token is valid
      const response = await this.apiFetch('/api/status', {
        headers: this.getAuthHeaders(),
      });
      return response.ok;
    } catch (err) {
      return false;
    }
  },

  showPasswordOverlay() {
    document.getElementById('password-overlay')?.classList.remove('hidden');
    document.getElementById('connecting-overlay')?.classList.add('hidden');
    document.getElementById('password-input')?.focus();
  },

  bindPasswordEvents() {
    const form = document.getElementById('password-form');
    const input = document.getElementById('password-input');
    const error = document.getElementById('password-error');

    form?.addEventListener('submit', (event) => {
      event.preventDefault();
      this.handleLogin();
    });

    input?.addEventListener('input', () => {
      error?.classList.add('hidden');
    });
  },

  async handleLogin() {
    const input = document.getElementById('password-input');
    const submit = document.getElementById('password-submit');
    const error = document.getElementById('password-error');

    const password = input?.value;
    if (!password) return;

    submit.disabled = true;
    error?.classList.add('hidden');

    try {
      const response = await fetch('/api/auth', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ password }),
      });

      const result = await response.json();

      if (result.success) {
        this.authExpired = false;
        // Store token
        if (result.token) {
          this.authToken = result.token;
          localStorage.setItem('wa_auth_token', result.token);
        }

        // Clear password input
        if (input) input.value = '';

        // Start the app
        this.startApp();
      } else {
        if (error) error.textContent = result.error || 'Unable to sign in.';
        error?.classList.remove('hidden');
        input?.focus();
        input?.select();
      }
    } catch (err) {
      console.error('Login failed:', err);
      error?.classList.remove('hidden');
    } finally {
      submit.disabled = false;
    }
  },

  // Get auth headers for API requests
  getAuthHeaders(baseHeaders = {}) {
    const headers = { ...baseHeaders };
    if (this.authToken) {
      headers['Authorization'] = `Bearer ${this.authToken}`;
    }
    return headers;
  },

  async apiFetch(url, options = {}) {
    this.reliableFetch ||= createReliableFetch();
    const response = await this.reliableFetch(url, {
      ...options,
      headers: this.getAuthHeaders(options.headers || {}),
    });
    if (response.status === 401) {
      this.authExpired = true;
      this.showPasswordOverlay();
    }
    return response;
  },

  // Handle logout
  async handleLogout() {
    const confirmed = confirm(
      'Are you sure you want to logout?\n\n' +
        'This will:\n' +
        '- Disconnect from WhatsApp\n' +
        '- Delete all messages and contacts\n' +
        '- Clear the session (require new QR scan)\n\n' +
        'This action cannot be undone.',
    );

    if (!confirmed) return;

    try {
      const response = await this.apiFetch('/api/logout', {
        method: 'POST',
        headers: this.getAuthHeaders(),
      });

      const result = await response.json();

      if (result.success) {
        clearInterval(this.viewedMessagesTimer);
        localStorage.removeItem('wa_pending_viewed_messages_v1');
        this.viewedMessageTracker = null;
        // Clear local storage
        localStorage.removeItem('wa_auth_token');
        this.authToken = null;

        // Clear local data
        this.contacts = [];
        this.messages.clear();
        this.topics?.reset();
        this.avatarCache.clear();
        this.currentContactId = null;
        this.qrData = null;

        // Clear the UI
        document.getElementById('contacts-list').innerHTML = `
          <div class="empty-state">
            <p>No conversations yet</p>
            <p class="hint">Messages will appear here</p>
          </div>
        `;

        // Show connecting overlay - bridge will restart and send new QR
        this.showConnecting();

        // The WebSocket will receive the new QR code when bridge restarts
      } else {
        alert('Logout failed: ' + (result.error || 'Unknown error'));
      }
    } catch (err) {
      console.error('Logout failed:', err);
      alert('Logout failed: ' + err.message);
    }
  },

  // Update placeholder to show correct keyboard shortcut for OS
  updateInputPlaceholder() {
    const input = document.getElementById('message-input');
    if (input) {
      const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0;
      const shortcut = isMac ? '⌘+Enter' : 'Ctrl+Enter';
      input.placeholder = this.isMobile()
        ? 'Type a message'
        : `Type a message (${shortcut} to send)`;
    }
  },

  handleResponsiveLayoutChange() {
    const isMobile = this.isMobile();

    if (isMobile !== this.mobileLayoutActive) {
      this.mobileLayoutActive = isMobile;
      this.workspaceExpanded = !isMobile;
    }

    if (!isMobile) {
      document.getElementById('workspace-backdrop')?.classList.add('hidden');
    }

    this.updateInputPlaceholder();
    this.updateWorkspaceUI();
  },

  // WebSocket connection
  connectWebSocket() {
    if (this.demoMode || this.authExpired) return;
    if (
      this.ws &&
      [WebSocket.OPEN, WebSocket.CONNECTING].includes(this.ws.readyState)
    )
      return;
    clearTimeout(this.reconnectTimer);

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const tokenQuery = this.authToken
      ? `?token=${encodeURIComponent(this.authToken)}`
      : '';
    const wsUrl = `${protocol}//${window.location.host}/ws${tokenQuery}`;

    const socket = new WebSocket(wsUrl);
    this.ws = socket;
    this.lastLiveMessage = Date.now();

    this.ws.onopen = () => {
      clearInterval(this.liveWatchdog);
      this.liveWatchdog = setInterval(() => {
        if (this.ws === socket && Date.now() - this.lastLiveMessage > 50000)
          socket.close();
      }, 10000);
      this.setLiveReconnectBanner(false);
      void this.recoverLiveMessages();
    };

    this.ws.onmessage = (event) => {
      if (this.ws !== socket) return;
      this.lastLiveMessage = Date.now();
      this.reconnectAttempt = 0;
      const data = JSON.parse(event.data);
      this.handleMessage(data);
    };

    this.ws.onclose = async () => {
      if (this.demoMode || this.ws !== socket) return;
      clearInterval(this.liveWatchdog);
      this.setLiveReconnectBanner(true);
      try {
        await this.apiFetch('/api/status');
      } catch {
        /* Network still unavailable. */
      }
      if (this.authExpired || this.ws !== socket) return;
      this.reconnectAttempt = (this.reconnectAttempt || 0) + 1;
      this.reconnectTimer = setTimeout(
        () => this.connectWebSocket(),
        reconnectDelay(this.reconnectAttempt),
      );
    };

    this.ws.onerror = (err) => {
      console.error('WebSocket error:', err);
    };
  },

  setLiveReconnectBanner(reconnecting) {
    let banner = document.getElementById('live-reconnect-banner');
    if (!banner) {
      banner = document.createElement('div');
      banner.id = 'live-reconnect-banner';
      banner.setAttribute('role', 'status');
      banner.textContent =
        'Reconnecting live updates… Your messages are saved.';
      document.getElementById('main-container')?.prepend(banner);
    }
    banner.hidden = !reconnecting;
  },

  async recoverLiveMessages() {
    if (this.demoMode) return;
    if (this.liveRecoveryRunning) {
      this.liveRecoveryPending = true;
      return;
    }
    this.liveRecoveryRunning = true;
    try {
      do {
        this.liveRecoveryPending = false;
        await this.loadContacts();
        if (this.currentContactId)
          await this.loadMessages(this.currentContactId);
        await this.topics?.load();
      } while (this.liveRecoveryPending && !this.authExpired);
    } finally {
      this.liveRecoveryRunning = false;
    }
  },

  async refreshAfterBecomingVisible() {
    if (
      document.visibilityState !== 'visible' ||
      this.demoMode ||
      this.authExpired
    )
      return;
    if (!this.ws || this.ws.readyState === WebSocket.CLOSED)
      this.connectWebSocket();
    // A topic can finish while the page is suspended, even if its socket still
    // appears open. Reload the catalog and selected page independently.
    await this.topics?.load();
  },

  handleMessageUpdate(message) {
    if (this.liveRecoveryRunning) this.liveRecoveryPending = true;
    this.prepareMessageForCache(message);
    this.topics?.update(message);
    this.messages.set(
      message.contactId,
      mergeMessageUpdate(this.messages.get(message.contactId) || [], message),
    );
    this.scheduleRenderContacts();
    if (this.currentContactId === message.contactId)
      this.refreshCurrentConversationView();
  },

  // Handle incoming WebSocket messages
  handleMessage(data) {
    switch (data.type) {
      case 'topics_updated':
        void this.topics?.load();
        break;
      case 'conversation_settings_updated':
        this.conversationSettingsClient.apply(data.chat_id, data.settings);
        break;
      case 'status':
        void this.handleStatus(data);
        break;

      case 'qr':
        this.showQRCode(data.data);
        break;

      case 'connected':
        this.handleConnected(data);
        break;

      case 'disconnected':
        void this.handleDisconnected();
        break;

      case 'voice_ready':
        window.dispatchEvent(
          new CustomEvent('voice-ready', { detail: data.message_id }),
        );
        break;
      case 'message_updated':
        this.handleMessageUpdate(data.message);
        break;
      case 'resync':
      case 'send_result':
        void this.recoverLiveMessages();
        break;
      case 'message':
        this.handleNewMessage(data.message);
        break;

      case 'reaction':
        this.handleReactionMessage(data.message);
        break;

      case 'typing':
        this.handleTyping(data);
        break;

      case 'messages_viewed': {
        const contact = this.contacts.find((c) => c.id === data.chat_id);
        if (contact) {
          contact.unreadCount = data.unread_count;
          this.scheduleRenderContacts();
        }
        break;
      }

      case 'mark_as_read':
        this.handleMarkAsRead(data.chat_id);
        break;

      case 'error':
        console.error('Error:', data.error);
        break;
    }
  },

  // Handle mark-as-read event from another device
  handleMarkAsRead(chatId) {
    const contact = this.contacts.find((c) => c.id === chatId);
    if (contact) {
      contact.unreadCount = 0;
      this.scheduleRenderContacts();
    }
  },

  // Toggle pin status for a contact
  async togglePin(contactId) {
    const contact = this.contacts.find((c) => c.id === contactId);
    if (!contact) return;

    const nextPinnedAt = contact.pinnedAt != null ? null : Date.now();
    contact.pinnedAt = nextPinnedAt;
    this.updateContactMetadata(contactId, { pinnedAt: nextPinnedAt });
    this.scheduleRenderContacts();
    if (contactId === this.currentContactId) {
      this.updateChatHeaderNote();
      this.renderConversationWorkspace();
    }
  },

  // Handle status update
  async handleStatus(data) {
    if (data.connection_state) {
      this.connectionState = data.connection_state;
      this.qrData = data.qr || null;
    }
    if (data.connected) {
      this.handleConnected(data);
    } else {
      await this.handleDisconnected();
    }
  },

  // Show connecting overlay
  showConnecting() {
    if (this.demoMode) return;
    if (this.qrData) {
      this.showQRCode(this.qrData);
      return;
    }

    document.getElementById('qr-overlay').classList.add('hidden');
    document.getElementById('connecting-overlay').classList.remove('hidden');
    document.getElementById('main-container').classList.add('hidden');
  },

  showCachedWorkspaceDisconnected() {
    document.getElementById('qr-overlay')?.classList.add('hidden');
    document.getElementById('connecting-overlay')?.classList.add('hidden');
    document.getElementById('main-container')?.classList.remove('hidden');
    const userName = document.getElementById('user-name');
    const userPhone = document.getElementById('user-phone');
    if (userName && userName.textContent === 'Loading...') {
      userName.textContent = 'WhatsApp disconnected';
    }
    if (userPhone && !userPhone.textContent) {
      userPhone.textContent = 'Cached inbox available';
    }

    if (!this.currentContactId) {
      document.getElementById('no-chat-selected')?.classList.remove('hidden');
      document.getElementById('chat-view')?.classList.add('hidden');
      document.getElementById('main-container')?.classList.remove('chat-open');
    }

    this.updateSendButton();
    this.renderVisitorDashboard();
  },

  // Show QR code
  showQRCode(qrData) {
    this.connectionRevision = (this.connectionRevision || 0) + 1;
    this.connected = false;
    this.qrData = qrData;
    document.getElementById('connecting-overlay').classList.add('hidden');
    document.getElementById('qr-overlay').classList.remove('hidden');
    document.getElementById('main-container').classList.add('hidden');

    // Generate QR code
    const container = document.getElementById('qr-container');
    container.innerHTML = '';

    // Use QRCode library if available, otherwise show text
    if (typeof QRCode !== 'undefined') {
      new QRCode(container, {
        text: qrData,
        width: 264,
        height: 264,
        colorDark: '#000000',
        colorLight: '#ffffff',
        correctLevel: QRCode.CorrectLevel.L,
      });
    } else {
      // Fallback: create canvas QR code manually using simple approach
      this.renderQRCode(container, qrData);
    }
  },

  // Fallback QR code renderer - displays text if library not available
  renderQRCode(container, data) {
    // If QRCode library is available, use it
    if (typeof QRCode !== 'undefined') {
      container.innerHTML = '';
      new QRCode(container, {
        text: data,
        width: 264,
        height: 264,
        colorDark: '#000000',
        colorLight: '#ffffff',
        correctLevel: QRCode.CorrectLevel.L,
      });
    } else {
      // Fallback: show the QR data as text for manual entry
      container.innerHTML = `<div style="word-break: break-all; font-size: 10px; max-width: 264px;">${data}</div>`;
    }
  },

  // Handle connected state
  handleConnected(data) {
    this.connectionRevision = (this.connectionRevision || 0) + 1;
    this.connectionState = 'connected';
    this.connected = true;
    this.qrData = null;
    this.notificationsReadyAt = Date.now() + 10000;

    document.getElementById('qr-overlay').classList.add('hidden');
    document.getElementById('connecting-overlay').classList.add('hidden');
    document.getElementById('main-container').classList.remove('hidden');

    // Update user info
    if (data.name) {
      document.getElementById('user-name').textContent = data.name;
    }
    if (data.phone) {
      document.getElementById('user-phone').textContent = '+' + data.phone;
    }

    // Load contacts
    this.loadContacts();

    // Load global usage
    this.fetchGlobalUsage();
  },

  // Handle disconnected state
  async handleDisconnected() {
    if (this.demoMode) return;

    this.connected = false;
    const revision = (this.connectionRevision =
      (this.connectionRevision || 0) + 1);
    this.qrData = null;

    const qrData = await this.fetchCurrentQRCode();
    if (this.connected || revision !== this.connectionRevision) return;
    if (qrData) {
      this.showQRCode(qrData);
      return;
    }
    if (this.connectionState === 'linking_required') {
      this.showConnecting();
      return;
    }

    const loadedContacts = await this.loadContacts();
    if (this.connected || revision !== this.connectionRevision) return;
    if (this.qrData) {
      this.showQRCode(this.qrData);
      return;
    }

    if (loadedContacts && this.contacts.length > 0) {
      this.showCachedWorkspaceDisconnected();
    } else {
      this.showConnecting();
    }
  },

  async fetchCurrentQRCode() {
    try {
      const response = await this.apiFetch('/api/qr');
      if (!response.ok) {
        return null;
      }

      const data = await response.json();
      return data.qr || null;
    } catch (err) {
      console.warn('Failed to check current QR code:', err);
      return null;
    }
  },
};
