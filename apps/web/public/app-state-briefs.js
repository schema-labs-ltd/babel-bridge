import {
  getChecklistItems,
  getContactDisplayName,
  getContactLabels,
  getDraftText,
  getUntranslatedIncomingMessages,
  messageSnippet,
  normalizeText,
} from './app-state-core.js';
import { getContactActionSnapshot } from './app-state-contacts.js';

function getLatestDirectionalMessage(messages = [], isFromMe) {
  for (let index = (messages || []).length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (Boolean(message?.isFromMe || message?.is_from_me) === isFromMe) {
      return message;
    }
  }
  return null;
}

export function textContainsQuestion(text) {
  const normalized = normalizeText(text);
  return (
    /\?/.test(String(text || '')) ||
    /\b(can|could|would|will|when|where|what|which|who|how|is|are|do|does|did|peux|puedes|puede|何|どこ|いつ)\b/i.test(
      normalized,
    )
  );
}

export function textMentionsAny(text, values = []) {
  const normalized = normalizeText(text);
  return values.some((value) => {
    const token = normalizeText(value);
    return token && normalized.includes(token);
  });
}

export function getOpenChecklistItems(metadata = {}) {
  return getChecklistItems(metadata).filter((item) => item && !item.done);
}

export function buildConversationBrief({
  contact = {},
  metadata = {},
  messages = [],
  drafts = {},
  now = Date.now(),
} = {}) {
  const latestIncoming = getLatestDirectionalMessage(messages, false);
  const latestOutgoing = getLatestDirectionalMessage(messages, true);
  const snapshot = getContactActionSnapshot({
    contact,
    drafts,
    metadata,
    messages,
    now,
  });
  const openTasks = getOpenChecklistItems(metadata);
  const latestIncomingSnippet = latestIncoming
    ? messageSnippet(latestIncoming, 92)
    : '';
  const latestOutgoingSnippet = latestOutgoing
    ? messageSnippet(latestOutgoing, 92)
    : '';
  const hasQuestion = textContainsQuestion(latestIncomingSnippet);

  let nextAction = 'No immediate action suggested.';
  if (snapshot.reminderStatus === 'due') {
    nextAction = metadata.reminderText || 'Follow up on the due reminder.';
  } else if (snapshot.replyState === 'drafting') {
    nextAction = 'Review and send the saved draft.';
  } else if (hasQuestion) {
    nextAction = 'Answer the latest question directly.';
  } else if (openTasks.length > 0) {
    nextAction = `Handle: ${openTasks[0].text}`;
  } else if (snapshot.replyState === 'needs-reply') {
    nextAction = 'Send a short acknowledgement or next step.';
  }

  const contextLines = [
    latestIncomingSnippet ? `Incoming: ${latestIncomingSnippet}` : '',
    latestOutgoingSnippet ? `Last sent: ${latestOutgoingSnippet}` : '',
    snapshot.timezoneInfo
      ? `${snapshot.timezoneInfo.label} · ${snapshot.timezoneInfo.statusLabel}`
      : '',
  ].filter(Boolean);

  return {
    latestIncomingSnippet,
    latestOutgoingSnippet,
    hasQuestion,
    openTasks: openTasks.map((item) => ({ ...item })),
    nextAction,
    contextLines,
    summary:
      contextLines[0] ||
      snapshot.summary ||
      'Add messages or local context to build a brief.',
  };
}

export function buildConversationActionPlan({
  contact = {},
  metadata = {},
  messages = [],
  drafts = {},
  now = Date.now(),
  maxItems = 4,
} = {}) {
  const contactId = contact?.id || '';
  const brief = buildConversationBrief({
    contact,
    metadata,
    messages,
    drafts,
    now,
  });
  const snapshot = getContactActionSnapshot({
    contact,
    drafts,
    metadata,
    messages,
    now,
  });
  const openTasks = getOpenChecklistItems(metadata);
  const untranslated = getUntranslatedIncomingMessages(messages);
  const draftText = getDraftText(drafts, contactId);
  const actions = [];
  const addAction = (action) => {
    if (!action?.id || actions.some((entry) => entry.id === action.id)) return;
    actions.push(action);
  };

  if (snapshot.reminderStatus === 'due') {
    addAction({
      id: 'due-reminder',
      label: 'Handle due reminder',
      detail: String(metadata?.reminderText || 'Follow up now.').trim(),
      priority: 'high',
    });
  }

  if (draftText) {
    addAction({
      id: 'review-draft',
      label: 'Review saved draft',
      detail: messageSnippet({ content: { body: draftText } }, 110),
      priority: 'high',
    });
  }

  if (brief.hasQuestion) {
    addAction({
      id: 'answer-question',
      label: 'Answer latest question',
      detail: brief.latestIncomingSnippet,
      priority: 'high',
    });
  }

  if (untranslated.length > 0) {
    addAction({
      id: 'translate-incoming',
      label: `Translate ${untranslated.length} incoming message${untranslated.length === 1 ? '' : 's'}`,
      detail: messageSnippet(untranslated[untranslated.length - 1], 110),
      priority: 'normal',
    });
  }

  openTasks.slice(0, 3).forEach((task, index) => {
    addAction({
      id: `task-${task.id || index}`,
      label: index === 0 ? 'Work next open task' : 'Keep task visible',
      detail: task.text,
      priority: index === 0 ? 'normal' : 'low',
    });
  });

  if (snapshot.timezoneInfo?.status === 'quiet-hours') {
    addAction({
      id: 'quiet-hours',
      label: 'Consider waiting',
      detail: `${snapshot.timezoneInfo.localTime} is quiet hours for this contact.`,
      priority: 'normal',
    });
  }

  if (actions.length === 0) {
    actions.push({
      id: 'no-action',
      label: 'No immediate action',
      detail: brief.summary,
      priority: 'low',
    });
  }

  return {
    nextAction: brief.nextAction,
    actions: actions.slice(0, Math.max(1, maxItems)),
    untranslatedCount: untranslated.length,
  };
}

export function buildConversationHandoffBrief({
  contact = {},
  metadata = {},
  messages = [],
  drafts = {},
  now = Date.now(),
  maxMessages = 4,
} = {}) {
  const contactId = contact?.id || '';
  const displayName = getContactDisplayName(contact, metadata);
  const brief = buildConversationBrief({
    contact,
    metadata,
    messages,
    drafts,
    now,
  });
  const snapshot = getContactActionSnapshot({
    contact,
    drafts,
    metadata,
    messages,
    now,
  });
  const labels = getContactLabels(metadata);
  const notes = String(metadata?.notes || '').trim();
  const draftText = getDraftText(drafts, contactId);
  const reminder = String(metadata?.reminderText || '').trim();
  const openTasks = getOpenChecklistItems(metadata);
  const recentMessages = (Array.isArray(messages) ? messages : [])
    .slice(-Math.max(1, maxMessages))
    .map((message) => {
      const sender =
        message?.isFromMe || message?.is_from_me
          ? 'Me'
          : message?.senderName || message?.sender_name || displayName;
      return `- ${sender}: ${messageSnippet(message, 140)}`;
    });

  const sections = [
    `Conversation: ${displayName}`,
    `Next action: ${brief.nextAction}`,
    snapshot.summary ? `Status: ${snapshot.summary}` : '',
    labels.length ? `Labels: ${labels.join(', ')}` : '',
    snapshot.timezoneInfo
      ? `Local time: ${snapshot.timezoneInfo.localTime} (${snapshot.timezoneInfo.statusLabel})`
      : '',
    reminder ? `Reminder: ${reminder}` : '',
    openTasks.length
      ? `Open tasks:\n${openTasks.map((task) => `- ${task.text}`).join('\n')}`
      : '',
    draftText ? `Saved draft:\n${draftText}` : '',
    notes ? `Private notes:\n${notes}` : '',
    recentMessages.length
      ? `Recent messages:\n${recentMessages.join('\n')}`
      : '',
  ].filter(Boolean);

  return sections.join('\n\n');
}
