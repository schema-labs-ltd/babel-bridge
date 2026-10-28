import {
  DEFAULT_FILTERS,
  messageSnippet,
  normalizeText,
} from './app-state-core.js';

export {
  getAppearanceThemeCatalog,
  getContrastRatio,
  resolveAppearanceTheme,
} from './app-state-appearance.js';
export {
  filterMessagesByQuery,
  getChecklistSummary,
  getContactDisplayName,
  getContactLabels,
  getDraftPreview,
  getDraftText,
  getPriorityInfo,
  getReminderStatus,
  getReplyState,
  getTimezoneInfo,
  getUntranslatedIncomingMessages,
  isContactSnoozed,
  isMessageStarred,
  parseLabelsInput,
} from './app-state-core.js';
export {
  buildVisitorDashboard,
  getContactActionSnapshot,
  getVisibleContacts,
} from './app-state-contacts.js';
export { createDemoWorkspace, suggestDemoReply } from './app-state-demo.js';
export {
  countMatchingMessages,
  getComposerAssistState,
  getComposerProfilePresets,
  getComposerReminderPresets,
  getComposerSendReadiness,
  getContextualReminderPresets,
  getSmartReplyOptions,
  simulateTranslation,
} from './app-state-composer.js';
export {
  buildConversationActionPlan,
  buildConversationBrief,
  buildConversationHandoffBrief,
} from './app-state-briefs.js';

export function normalizeInboxFilters(
  filters = {},
  { controlsAvailable = true } = {},
) {
  if (!controlsAvailable) {
    return { ...DEFAULT_FILTERS };
  }

  return Object.fromEntries(
    Object.keys(DEFAULT_FILTERS).map((key) => [key, Boolean(filters?.[key])]),
  );
}

function cloneObject(value) {
  return { ...(value || {}) };
}

function createChecklistId(text, updatedAt, index = 0) {
  const slug =
    String(text || '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 32) || 'task';
  return `${slug}-${updatedAt}-${index}`;
}

function normalizeChecklistText(value) {
  return String(value || '')
    .replace(/^[\-•*]\s*/, '')
    .trim();
}

function parseChecklistLine(line) {
  const raw = String(line || '').trim();
  if (!raw) return null;

  const match = raw.match(/^(?:[-*•]\s*)?\[( |x|X)\]\s*(.+)$/);
  if (match) {
    return {
      text: normalizeChecklistText(match[2]),
      done: match[1].toLowerCase() === 'x',
    };
  }

  const plainText = normalizeChecklistText(raw);
  if (!plainText) return null;
  return { text: plainText, done: false };
}

export function parseChecklistInput(value) {
  const seen = new Set();
  return String(value || '')
    .split(/\n+/)
    .map(parseChecklistLine)
    .filter(Boolean)
    .filter((item) => {
      const key = normalizeText(item.text);
      if (!key || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

export function upsertChecklistItems(
  existingItems,
  text,
  updatedAt = Date.now(),
) {
  const previous = Array.isArray(existingItems) ? existingItems : [];
  const existingByText = new Map(
    previous
      .map((item) => [normalizeText(item?.text), { ...item }])
      .filter(([key]) => Boolean(key)),
  );

  return parseChecklistInput(text).map((item, index) => {
    const key = normalizeText(item.text);
    const previousItem = existingByText.get(key);
    return {
      id: previousItem?.id || createChecklistId(item.text, updatedAt, index),
      text: item.text,
      done: item.done,
      updatedAt: previousItem?.updatedAt || updatedAt,
    };
  });
}

export function toggleChecklistItem(items, itemId, updatedAt = Date.now()) {
  return (Array.isArray(items) ? items : []).map((item) => {
    if (item?.id !== itemId) {
      return { ...item };
    }

    return {
      ...item,
      done: !item.done,
      updatedAt,
    };
  });
}

export function upsertDraft(drafts, contactId, text, updatedAt = Date.now()) {
  const nextDrafts = cloneObject(drafts);
  const trimmed = String(text || '').trim();

  if (!contactId) {
    return nextDrafts;
  }

  if (!trimmed) {
    delete nextDrafts[contactId];
    return nextDrafts;
  }

  nextDrafts[contactId] = {
    text: trimmed,
    updatedAt,
  };

  return nextDrafts;
}

function createQuickReplyId(text, updatedAt) {
  const slug =
    String(text || '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 24) || 'quick-reply';
  return `${slug}-${updatedAt}`;
}

export function upsertQuickReply(
  quickReplies,
  text,
  updatedAt = Date.now(),
  maxItems = 8,
) {
  const trimmed = String(text || '').trim();
  if (!trimmed) return Array.isArray(quickReplies) ? [...quickReplies] : [];

  const normalized = trimmed.toLowerCase();
  const nextReplies = (Array.isArray(quickReplies) ? quickReplies : [])
    .filter((reply) => normalizeText(reply?.text) !== normalized)
    .map((reply) => ({ ...reply }));

  nextReplies.unshift({
    id: createQuickReplyId(trimmed, updatedAt),
    text: trimmed,
    updatedAt,
  });

  return nextReplies.slice(0, Math.max(1, maxItems));
}

export function removeQuickReply(quickReplies, quickReplyId) {
  return (Array.isArray(quickReplies) ? quickReplies : [])
    .filter((reply) => reply?.id !== quickReplyId)
    .map((reply) => ({ ...reply }));
}

export function getMessageSnippet(message, maxLength = 140) {
  return messageSnippet(message, maxLength);
}

export function toggleStarredMessage(
  starredLookup,
  message,
  updatedAt = Date.now(),
) {
  const nextLookup = cloneObject(starredLookup);
  const id = message?.id;
  if (!id) return nextLookup;

  if (nextLookup[id]) {
    delete nextLookup[id];
    return nextLookup;
  }

  nextLookup[id] = {
    id,
    contactId: message.contactId || message.contact_id || '',
    timestamp: message.timestamp || Date.now(),
    updatedAt,
    snippet: messageSnippet(message),
  };

  return nextLookup;
}
