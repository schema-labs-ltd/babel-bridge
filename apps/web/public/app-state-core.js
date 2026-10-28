export const DEFAULT_FILTERS = {
  unreadOnly: false,
  groupsOnly: false,
  draftsOnly: false,
  pinnedOnly: false,
  notesOnly: false,
  dueRemindersOnly: false,
  labelsOnly: false,
  snoozedOnly: false,
  needsReplyOnly: false,
  importantOnly: false,
  tasksOnly: false,
};

const PRIORITY_CONFIG = {
  urgent: { value: 'urgent', label: 'Urgent', rank: 0, isImportant: true },
  high: { value: 'high', label: 'High', rank: 1, isImportant: true },
  normal: { value: 'normal', label: 'Normal', rank: 2, isImportant: false },
  low: { value: 'low', label: 'Low', rank: 3, isImportant: false },
};

export const COMPOSER_LANGUAGE_PRESETS = ['Spanish', 'French', 'Japanese'];

export const COMPOSER_TONE_PRESETS = ['Friendly', 'Formal', 'Concise'];

export const COMPOSER_REMINDER_PRESETS = [
  { id: 'later-today', label: 'Later today', hours: 3 },
  { id: 'tomorrow', label: 'Tomorrow 9 AM', days: 1, hour: 9 },
  { id: 'next-week', label: 'Next week', days: 7, hour: 9 },
];

export function normalizeText(value) {
  return String(value || '')
    .trim()
    .toLowerCase();
}

export function normalizeTimestamp(value) {
  if (value === null || value === undefined || value === '') return null;
  const timestamp = Number(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

function normalizePriority(value) {
  const normalized = normalizeText(value);
  return PRIORITY_CONFIG[normalized] ? normalized : 'normal';
}

export function getChecklistItems(metadata = {}) {
  return Array.isArray(metadata?.checklist) ? metadata.checklist : [];
}

export function getContactDisplayName(contact = {}, metadata = {}) {
  const alias = String(metadata?.alias || '').trim();
  if (alias) return alias;

  if (contact?.name) return contact.name;
  if (contact?.phone) return `+${contact.phone}`;
  if (contact?.type === 'group') return 'Group Chat';

  const phoneFromJid = String(contact?.id || '').split('@')[0];
  return phoneFromJid ? `+${phoneFromJid}` : 'Unknown';
}

export function parseLabelsInput(value) {
  const seen = new Set();
  return String(value || '')
    .split(/[\n,]/)
    .map((label) => label.trim())
    .filter(Boolean)
    .filter((label) => {
      const key = label.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

export function getPriorityInfo(metadata = {}) {
  const value = normalizePriority(metadata?.priority);
  return { ...PRIORITY_CONFIG[value] };
}

export function getChecklistSummary(metadata = {}) {
  const checklist = getChecklistItems(metadata);
  const total = checklist.length;
  const completed = checklist.filter((item) => item?.done).length;
  const open = Math.max(0, total - completed);
  let label = 'No tasks';
  if (open > 0) {
    label = `${open} open task${open === 1 ? '' : 's'}`;
  } else if (total > 0) {
    label = 'All tasks done';
  }

  return { total, completed, open, label };
}

export function getTimezoneInfo(metadata = {}, now = Date.now()) {
  const timezone = String(metadata?.timezone || '').trim();
  if (!timezone) return null;

  try {
    const parts = new Intl.DateTimeFormat('en-GB', {
      timeZone: timezone,
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    }).formatToParts(new Date(now));
    const hour = Number(
      parts.find((part) => part.type === 'hour')?.value ?? '0',
    );
    const minute = parts.find((part) => part.type === 'minute')?.value ?? '00';
    const localTime = `${String(hour).padStart(2, '0')}:${minute}`;

    let status = 'daytime';
    let statusLabel = 'Daytime';
    if (hour >= 9 && hour < 18) {
      status = 'working-hours';
      statusLabel = 'Working hours';
    } else if (hour >= 6 && hour < 9) {
      status = 'morning';
      statusLabel = 'Morning';
    } else if (hour >= 18 && hour < 22) {
      status = 'evening';
      statusLabel = 'Evening';
    } else {
      status = 'quiet-hours';
      statusLabel = 'Quiet hours';
    }

    return {
      timezone,
      localTime,
      label: `${localTime} local time`,
      status,
      statusLabel,
    };
  } catch {
    return null;
  }
}

export function getContactLabels(metadata = {}) {
  if (Array.isArray(metadata?.labels)) {
    return parseLabelsInput(metadata.labels.join(','));
  }
  return parseLabelsInput(metadata?.labelsText || metadata?.labels || '');
}

export function isContactSnoozed(metadata = {}, now = Date.now()) {
  const snoozedUntil = normalizeTimestamp(metadata?.snoozedUntil);
  return Boolean(snoozedUntil && snoozedUntil > now);
}

export function getReminderStatus(metadata = {}, now = Date.now()) {
  const reminderAt = normalizeTimestamp(metadata?.reminderAt);
  const reminderText = normalizeText(metadata?.reminderText);
  if (!reminderAt || !reminderText) return 'none';
  return reminderAt <= now ? 'due' : 'upcoming';
}

export function getDraftText(drafts, contactId) {
  return drafts?.[contactId]?.text || '';
}

export function getDraftPreview(drafts, contactId, maxLength = 48) {
  const draft = getDraftText(drafts, contactId);
  if (!draft) return '';

  const preview =
    draft.length > maxLength ? `${draft.slice(0, maxLength - 1)}…` : draft;
  return `Draft: ${preview}`;
}

export function getReplyState({
  contact = {},
  messages = [],
  drafts = {},
  metadata = {},
  contactId = contact?.id,
  now = Date.now(),
} = {}) {
  if (isContactSnoozed(metadata, now)) {
    return 'snoozed';
  }

  if (getDraftText(drafts, contactId)) {
    return 'drafting';
  }

  const latestMessage =
    Array.isArray(messages) && messages.length > 0
      ? messages[messages.length - 1]
      : null;

  if (latestMessage) {
    return latestMessage.isFromMe || latestMessage.is_from_me
      ? 'waiting'
      : 'needs-reply';
  }

  if ((contact?.unreadCount || 0) > 0) {
    return 'needs-reply';
  }

  return 'idle';
}

export function messageSnippet(message, maxLength = 140) {
  const content = message?.content || {};
  const parts = [
    content.body,
    content.text,
    content.caption,
    message?.originalText,
    message?.original_text,
    message?.translatedText,
    message?.translated_text,
  ].filter(Boolean);

  const text = String(parts[0] || '').trim();
  if (!text) return '[Message]';
  return text.length > maxLength ? `${text.slice(0, maxLength - 1)}…` : text;
}

export function getUntranslatedIncomingMessages(messages = []) {
  return (Array.isArray(messages) ? messages : []).filter((message) => {
    const isOutgoing = Boolean(message?.isFromMe || message?.is_from_me);
    const isTranslated = Boolean(
      message?.isTranslated || message?.is_translated,
    );
    const content = message?.content || {};
    const hasText = Boolean(content.body || content.caption || content.text);
    return !isOutgoing && !isTranslated && hasText;
  });
}

export function isMessageStarred(starredLookup, messageId) {
  return Boolean(messageId && starredLookup?.[messageId]);
}

function messageSearchText(message) {
  const content = message?.content || {};
  return [
    content.body,
    content.text,
    content.caption,
    message?.originalText,
    message?.original_text,
    message?.translatedText,
    message?.translated_text,
    message?.senderName,
    message?.sender_name,
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
}

export function filterMessagesByQuery(messages, query, options = {}) {
  const { starredOnly = false, starredLookup = {} } = options;
  const normalizedQuery = normalizeText(query);

  return (messages || []).filter((message) => {
    if (starredOnly && !isMessageStarred(starredLookup, message.id)) {
      return false;
    }

    if (!normalizedQuery) {
      return true;
    }

    return messageSearchText(message).includes(normalizedQuery);
  });
}
