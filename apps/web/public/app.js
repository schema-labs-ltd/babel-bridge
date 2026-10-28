import { setupVoiceNotes } from './voice-notes.js';
import { setupMessageTones } from './message-tones.js';
import { setupTopics } from './topics.js';
import { setupChatGptSignIn } from './chatgpt-signin.js';
// WhatsApp Translator Web Client

import { ConversationSettingsClient } from './conversation-settings.js';
import { resolveAppearanceTheme } from './app-state.js?v=20260912-conversation-translation';
import { workspaceStateMethods } from './app/workspace-state.js';
import { composerAssistMethods } from './app/composer-assist.js';
import { inboxMethods } from './app/inbox.js';
import { commandPaletteMethods } from './app/command-palette.js';
import { workspacePanelMethods } from './app/workspace-panel.js';
import { connectionMethods } from './app/connection.js';
import { liveMessagesMethods } from './app/live-messages.js';
import { contactsMethods } from './app/contacts.js';
import { conversationMethods } from './app/conversation.js';
import { messageRenderMethods } from './app/message-render.js';
import { messageContentMethods } from './app/message-content.js';
import { conversationToolsMethods } from './app/conversation-tools.js';
import { sendingMethods } from './app/sending.js';
import { translationMethods } from './app/translation.js';
import { repliesAiMethods } from './app/replies-ai.js';
import { eventsMethods } from './app/events.js';
import { menuEventsMethods } from './app/menu-events.js';
import { formattingMethods } from './app/formatting.js';
import { messageMediaMethods } from './app/message-media.js';
import { settingsModalMethods } from './app/settings-modal.js';

class WhatsAppClient {
  // Track contact ID for context menu
  contextMenuContactId = null;

  constructor() {
    this.ws = null;
    this.connected = false;
    this.qrData = null;
    this.demoMode = false;
    this.contacts = [];
    this.currentContactId = null;
    this.settingsContactId = null;
    this.initialMessageLimit = 30;
    this.messageCacheLimit = 200;
    this.notificationsReadyAt = 0;
    this.notificationPermissionRequested = false;
    this.notificationPromptEl = null;
    this.messages = new Map();
    this.contactsRenderTimer = null;
    this.metadataRefreshTimer = null;
    this.messagesHasMore = new Map(); // contactId -> boolean (whether more messages exist)
    this.messagesLoading = new Map(); // contactId -> boolean (whether currently loading)
    this.avatarCache = new Map(); // JID -> URL
    this.avatarFetching = new Set(); // JIDs currently being fetched
    this.globalUsage = { inputTokens: 0, outputTokens: 0, costUsd: 0 };
    this.conversationUsageCache = new Map(); // contactId -> usage
    this.conversationUsageTimer = null;
    this.linkPreviewCache = new Map(); // URL -> LinkPreview
    this.linkPreviewFetching = new Set(); // URLs currently being fetched
    this.linkPreviewLoadTimer = null;
    this.typingState = new Map(); // chatId -> { userId, state, timestamp }
    this.typingTimeouts = new Map(); // chatId -> timeoutId (auto-clear after 10s)
    this.replyingTo = null; // { messageId, senderJid, senderName, text, isFromMe }
    this.authToken = localStorage.getItem('wa_auth_token'); // Auth token for API requests
    this.recentEmojis = JSON.parse(
      localStorage.getItem('wa_recent_emojis') || '[]',
    );
    this.currentEmojiCategory = 'recent';
    this.draftsStorageKey = 'wa_chat_drafts';
    this.starredStorageKey = 'wa_starred_messages';
    this.contactMetadataStorageKey = 'wa_contact_metadata';
    this.inboxPreferencesStorageKey = 'wa_inbox_preferences';
    this.quickRepliesStorageKey = 'wa_quick_replies';
    this.appearanceStorageKey = 'wa_appearance_preferences';
    this.drafts = this.loadStoredJson(this.draftsStorageKey);
    this.starredMessages = this.loadStoredJson(this.starredStorageKey);
    this.contactMetadata = this.loadStoredJson(this.contactMetadataStorageKey);
    this.conversationSettingsClient = new ConversationSettingsClient({
      request: (path, options) => this.apiFetch(path, options),
      isDemo: () => this.demoMode,
      onChange: (contactId) => {
        if (contactId === this.currentContactId) {
          this.updateChatHeaderNote();
          this.renderComposerAssist();
        }
      },
    });
    this.quickReplies = this.loadStoredArray(this.quickRepliesStorageKey);
    this.appearancePreferences = this.loadStoredJson(this.appearanceStorageKey);
    this.appearanceState = resolveAppearanceTheme(this.appearancePreferences, {
      systemMode: this.getSystemAppearanceMode(),
    });
    this.systemAppearanceQuery = null;
    this.handleSystemAppearanceChange =
      this.handleSystemAppearanceChange.bind(this);
    const storedInboxPreferences = this.loadStoredJson(
      this.inboxPreferencesStorageKey,
    );
    this.sidebarSearchQuery =
      typeof storedInboxPreferences.searchQuery === 'string'
        ? storedInboxPreferences.searchQuery
        : '';
    this.contactFilters = {
      unreadOnly: Boolean(storedInboxPreferences.unreadOnly),
      groupsOnly: Boolean(storedInboxPreferences.groupsOnly),
      draftsOnly: Boolean(storedInboxPreferences.draftsOnly),
      importantOnly: Boolean(storedInboxPreferences.importantOnly),
      tasksOnly: Boolean(storedInboxPreferences.tasksOnly),
      pinnedOnly: Boolean(storedInboxPreferences.pinnedOnly),
      notesOnly: Boolean(storedInboxPreferences.notesOnly),
      dueRemindersOnly: Boolean(storedInboxPreferences.dueRemindersOnly),
      labelsOnly: Boolean(storedInboxPreferences.labelsOnly),
      snoozedOnly: Boolean(storedInboxPreferences.snoozedOnly),
      needsReplyOnly: Boolean(storedInboxPreferences.needsReplyOnly),
    };
    this.messageSearchQuery = '';
    this.starredOnly = false;
    this.fullyLoadedContacts = new Set();
    this.commandPaletteOpen = false;
    this.commandPaletteQuery = '';
    this.commandPaletteSelectionIndex = 0;
    this.commandPaletteItems = [];
    this.workspaceExpanded = !this.isMobile();
    this.mobileLayoutActive = this.isMobile();
    this.handleResponsiveLayoutChange =
      this.handleResponsiveLayoutChange.bind(this);

    // Emoji data organized by category
    this.emojiData = {
      smileys: [
        '😀',
        '😃',
        '😄',
        '😁',
        '😅',
        '😂',
        '🤣',
        '😊',
        '😇',
        '🙂',
        '🙃',
        '😉',
        '😌',
        '😍',
        '🥰',
        '😘',
        '😗',
        '😙',
        '😚',
        '😋',
        '😛',
        '😜',
        '🤪',
        '😝',
        '🤑',
        '🤗',
        '🤭',
        '🤫',
        '🤔',
        '🤐',
        '🤨',
        '😐',
        '😑',
        '😶',
        '😏',
        '😒',
        '🙄',
        '😬',
        '🤥',
        '😔',
        '😪',
        '🤤',
        '😴',
        '😷',
        '🤒',
        '🤕',
        '🤢',
        '🤮',
        '🤧',
        '🥵',
        '🥶',
        '🥴',
        '😵',
        '🤯',
        '🤠',
        '🥳',
        '😎',
        '🤓',
        '🧐',
        '😕',
        '😟',
        '🙁',
        '☹️',
        '😮',
        '😯',
        '😲',
        '😳',
        '🥺',
        '😦',
        '😧',
        '😨',
        '😰',
        '😥',
        '😢',
        '😭',
        '😱',
        '😖',
        '😣',
        '😞',
        '😓',
        '😩',
        '😫',
        '🥱',
        '😤',
        '😡',
        '😠',
        '🤬',
        '😈',
        '👿',
        '💀',
        '☠️',
        '💩',
        '🤡',
        '👹',
        '👺',
        '👻',
        '👽',
        '👾',
        '🤖',
      ],
      gestures: [
        '👋',
        '🤚',
        '🖐️',
        '✋',
        '🖖',
        '👌',
        '🤌',
        '🤏',
        '✌️',
        '🤞',
        '🤟',
        '🤘',
        '🤙',
        '👈',
        '👉',
        '👆',
        '🖕',
        '👇',
        '☝️',
        '👍',
        '👎',
        '✊',
        '👊',
        '🤛',
        '🤜',
        '👏',
        '🙌',
        '👐',
        '🤲',
        '🤝',
        '🙏',
        '✍️',
        '💅',
        '🤳',
        '💪',
        '🦾',
        '🦿',
        '🦵',
        '🦶',
        '👂',
        '🦻',
        '👃',
        '🧠',
        '🫀',
        '🫁',
        '🦷',
        '🦴',
        '👀',
        '👁️',
        '👅',
        '👄',
        '👶',
        '🧒',
        '👦',
        '👧',
        '🧑',
        '👱',
        '👨',
        '🧔',
        '👩',
        '🧓',
        '👴',
        '👵',
      ],
      hearts: [
        '❤️',
        '🧡',
        '💛',
        '💚',
        '💙',
        '💜',
        '🖤',
        '🤍',
        '🤎',
        '💔',
        '❣️',
        '💕',
        '💞',
        '💓',
        '💗',
        '💖',
        '💘',
        '💝',
        '💟',
        '♥️',
        '💌',
        '💋',
        '👄',
        '🫂',
        '👥',
        '👤',
      ],
      animals: [
        '🐶',
        '🐱',
        '🐭',
        '🐹',
        '🐰',
        '🦊',
        '🐻',
        '🐼',
        '🐨',
        '🐯',
        '🦁',
        '🐮',
        '🐷',
        '🐸',
        '🐵',
        '🙈',
        '🙉',
        '🙊',
        '🐒',
        '🐔',
        '🐧',
        '🐦',
        '🐤',
        '🦆',
        '🦅',
        '🦉',
        '🦇',
        '🐺',
        '🐗',
        '🐴',
        '🦄',
        '🐝',
        '🪱',
        '🐛',
        '🦋',
        '🐌',
        '🐞',
        '🐜',
        '🪳',
        '🦂',
        '🐢',
        '🐍',
        '🦎',
        '🦖',
        '🦕',
        '🐙',
        '🦑',
        '🦐',
        '🦞',
        '🦀',
        '🐡',
        '🐠',
        '🐟',
        '🐬',
        '🐳',
        '🐋',
        '🦈',
        '🐊',
        '🐅',
        '🐆',
        '🦓',
        '🦍',
        '🦧',
        '🐘',
        '🦛',
        '🦏',
        '🐪',
        '🐫',
        '🦒',
        '🦘',
        '🐃',
        '🐂',
        '🐄',
        '🐎',
        '🐖',
        '🐏',
        '🐑',
        '🦙',
        '🐐',
        '🦌',
        '🐕',
        '🐩',
        '🦮',
        '🐕‍🦺',
        '🐈',
        '🐓',
        '🦃',
        '🦚',
        '🦜',
        '🦢',
        '🦩',
        '🐇',
        '🦝',
        '🦨',
        '🦡',
        '🦫',
        '🦦',
        '🦥',
        '🐁',
        '🐀',
        '🐿️',
        '🦔',
      ],
      food: [
        '🍕',
        '🍔',
        '🍟',
        '🌭',
        '🍿',
        '🧂',
        '🥓',
        '🥚',
        '🍳',
        '🧇',
        '🥞',
        '🧈',
        '🍞',
        '🥐',
        '🥖',
        '🥨',
        '🧀',
        '🥗',
        '🥙',
        '🥪',
        '🌮',
        '🌯',
        '🫔',
        '🥫',
        '🍝',
        '🍜',
        '🍲',
        '🍛',
        '🍣',
        '🍱',
        '🥟',
        '🦪',
        '🍤',
        '🍙',
        '🍚',
        '🍘',
        '🍥',
        '🥠',
        '🥮',
        '🍢',
        '🍡',
        '🍧',
        '🍨',
        '🍦',
        '🥧',
        '🧁',
        '🍰',
        '🎂',
        '🍮',
        '🍭',
        '🍬',
        '🍫',
        '🍩',
        '🍪',
        '🌰',
        '🥜',
        '🍯',
        '🥛',
        '🍼',
        '☕',
        '🫖',
        '🍵',
        '🧃',
        '🥤',
        '🧋',
        '🍶',
        '🍺',
        '🍻',
        '🥂',
        '🍷',
        '🥃',
        '🍸',
        '🍹',
        '🧉',
        '🍾',
        '🧊',
        '🥄',
        '🍴',
        '🍽️',
        '🥣',
        '🥡',
        '🥢',
        '🧆',
      ],
      travel: [
        '✈️',
        '🚀',
        '🛸',
        '🚁',
        '🛶',
        '⛵',
        '🚤',
        '🛥️',
        '🛳️',
        '⛴️',
        '🚢',
        '🚂',
        '🚃',
        '🚄',
        '🚅',
        '🚆',
        '🚇',
        '🚈',
        '🚉',
        '🚊',
        '🚝',
        '🚞',
        '🚋',
        '🚌',
        '🚍',
        '🚎',
        '🚐',
        '🚑',
        '🚒',
        '🚓',
        '🚔',
        '🚕',
        '🚖',
        '🚗',
        '🚘',
        '🚙',
        '🛻',
        '🚚',
        '🚛',
        '🚜',
        '🏎️',
        '🏍️',
        '🛵',
        '🦽',
        '🦼',
        '🛺',
        '🚲',
        '🛴',
        '🛹',
        '🛼',
        '🚏',
        '🛣️',
        '🛤️',
        '🛢️',
        '⛽',
        '🚨',
        '🚥',
        '🚦',
        '🛑',
        '🚧',
        '⚓',
        '🗺️',
        '🗿',
        '🗽',
        '🗼',
        '🏰',
        '🏯',
        '🏟️',
        '🎡',
        '🎢',
        '🎠',
        '⛲',
        '⛱️',
        '🏖️',
        '🏝️',
        '🏜️',
        '🌋',
        '⛰️',
        '🏔️',
        '🗻',
        '🏕️',
        '⛺',
        '🛖',
        '🏠',
        '🏡',
        '🏘️',
        '🏚️',
        '🏗️',
        '🏢',
        '🏬',
        '🏣',
        '🏤',
        '🏥',
        '🏦',
        '🏨',
        '🏪',
        '🏫',
        '🏩',
        '💒',
        '🏛️',
        '⛪',
        '🕌',
        '🕍',
        '🛕',
        '🕋',
        '⛩️',
      ],
      objects: [
        '💡',
        '🔦',
        '🏮',
        '🪔',
        '📱',
        '📲',
        '💻',
        '🖥️',
        '🖨️',
        '⌨️',
        '🖱️',
        '🖲️',
        '💽',
        '💾',
        '💿',
        '📀',
        '🧮',
        '🎥',
        '🎞️',
        '📽️',
        '🎬',
        '📺',
        '📷',
        '📸',
        '📹',
        '📼',
        '🔍',
        '🔎',
        '🕯️',
        '💵',
        '💴',
        '💶',
        '💷',
        '💰',
        '💳',
        '💎',
        '⚖️',
        '🪜',
        '🧰',
        '🪛',
        '🔧',
        '🔨',
        '⚒️',
        '🛠️',
        '⛏️',
        '🪚',
        '🔩',
        '⚙️',
        '🪤',
        '🧱',
        '⛓️',
        '🧲',
        '🔫',
        '💣',
        '🧨',
        '🪓',
        '🔪',
        '🗡️',
        '⚔️',
        '🛡️',
        '🚬',
        '⚰️',
        '🪦',
        '⚱️',
        '🏺',
        '🔮',
        '📿',
        '🧿',
        '💈',
        '⚗️',
        '🔭',
        '🔬',
        '🕳️',
        '🩹',
        '🩺',
        '💊',
        '💉',
        '🩸',
        '🧬',
        '🦠',
        '🧫',
        '🧪',
      ],
      symbols: [
        '✨',
        '⭐',
        '🌟',
        '💫',
        '⚡',
        '🔥',
        '💥',
        '☄️',
        '🌈',
        '☀️',
        '🌤️',
        '⛅',
        '🌥️',
        '☁️',
        '🌦️',
        '🌧️',
        '⛈️',
        '🌩️',
        '🌨️',
        '❄️',
        '☃️',
        '⛄',
        '🌬️',
        '💨',
        '🌪️',
        '🌫️',
        '🌊',
        '💧',
        '💦',
        '☔',
        '🎵',
        '🎶',
        '🎼',
        '🎤',
        '🎧',
        '📻',
        '🎷',
        '🪗',
        '🎸',
        '🎹',
        '🎺',
        '🎻',
        '🪕',
        '🥁',
        '🪘',
        '⚽',
        '🏀',
        '🏈',
        '⚾',
        '🥎',
        '🎾',
        '🏐',
        '🏉',
        '🥏',
        '🎱',
        '🪀',
        '🏓',
        '🏸',
        '🏒',
        '🏑',
        '🥍',
        '🏏',
        '🪃',
        '🥅',
        '⛳',
        '🪁',
        '🏹',
        '🎣',
        '🤿',
        '🥊',
        '🥋',
        '🎽',
        '🛷',
        '⛸️',
        '🥌',
        '🎿',
        '⛷️',
        '🏂',
        '🪂',
        '🏋️',
        '🎯',
        '🎮',
        '🕹️',
        '🎰',
        '🧩',
        '♠️',
        '♥️',
        '♦️',
        '♣️',
        '🃏',
        '🀄',
        '🎴',
        '🎭',
        '🎨',
      ],
    };

    this.setupAppearanceSync();
    this.applyAppearanceTheme();
    this.init();
  }

  async init() {
    // Check if authentication is required
    const authRequired = await this.checkAuth();

    if (authRequired && !this.authToken) {
      this.showPasswordOverlay();
      this.bindPasswordEvents();
      return;
    }

    // If we have a token, verify it's still valid
    if (authRequired && this.authToken) {
      const valid = await this.verifyToken();
      if (!valid) {
        this.authToken = null;
        localStorage.removeItem('wa_auth_token');
        this.showPasswordOverlay();
        this.bindPasswordEvents();
        return;
      }
    }

    // Auth passed, continue with normal initialization
    this.startApp();
  }

  startApp() {
    document.getElementById('password-overlay')?.classList.add('hidden');
    this.connectWebSocket();
    this.bindEvents();
    this.updateInputPlaceholder();
    this.setupVisualViewport();
    this.updateNotificationPrompt();
    this.setupNotificationPermissionRequest();
    this.renderQuickReplies();
    this.updateConversationMenuUI();
    this.updateStarredToggleUI();
    this.updateChatSearchUI();
    this.syncInboxControls();
    this.syncAppearanceControls();
    this.updateWorkspaceUI();
    this.renderVisitorDashboard();
  }
}

// Methods live in ./app/*.js by area. Install them as non-enumerable
// prototype methods, in source order, exactly as class methods behave.
for (const methods of [
  workspaceStateMethods,
  composerAssistMethods,
  inboxMethods,
  commandPaletteMethods,
  workspacePanelMethods,
  connectionMethods,
  liveMessagesMethods,
  contactsMethods,
  conversationMethods,
  messageRenderMethods,
  messageContentMethods,
  conversationToolsMethods,
  sendingMethods,
  translationMethods,
  repliesAiMethods,
  eventsMethods,
  menuEventsMethods,
  formattingMethods,
  messageMediaMethods,
  settingsModalMethods,
]) {
  for (const [name, descriptor] of Object.entries(
    Object.getOwnPropertyDescriptors(methods),
  )) {
    Object.defineProperty(WhatsAppClient.prototype, name, {
      ...descriptor,
      enumerable: false,
    });
  }
}

/* Initialize app */
window.app = new WhatsAppClient();
setupVoiceNotes(window.app);
window.app.messageTones = setupMessageTones(window.app);
window.app.topics = setupTopics(window.app);
window.app.chatGpt = setupChatGptSignIn(window.app);
