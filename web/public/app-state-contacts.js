import {
  DEFAULT_FILTERS,
  getChecklistItems,
  getChecklistSummary,
  getContactLabels,
  getDraftPreview,
  getDraftText,
  getPriorityInfo,
  getReminderStatus,
  getReplyState,
  getTimezoneInfo,
  isContactSnoozed,
  normalizeText,
  normalizeTimestamp,
} from './app-state-core.js';

function getMetadata(metadataByContact, contactId) {
  return metadataByContact?.[contactId] || {};
}

function contactMatchesSearch(
  contact,
  draftPreview,
  messagePreview,
  metadata,
  normalizedQuery,
) {
  if (!normalizedQuery) return true;

  const priority = getPriorityInfo(metadata);
  const checklistSummary = getChecklistSummary(metadata);
  const timezoneInfo = getTimezoneInfo(metadata);
  const checklistSearchText = getChecklistItems(metadata)
    .map((item) => item?.text)
    .filter(Boolean)
    .join(' ');

  const haystack = [
    contact?.name,
    contact?.phone,
    contact?.id,
    metadata?.alias,
    draftPreview,
    messagePreview,
    metadata?.notes,
    metadata?.notePreview,
    metadata?.reminderText,
    getContactLabels(metadata).join(' '),
    priority.label,
    checklistSummary.label,
    checklistSearchText,
    timezoneInfo?.timezone,
    timezoneInfo?.label,
    timezoneInfo?.statusLabel,
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();

  const tokens = normalizedQuery.split(/\s+/).filter(Boolean);
  return tokens.every((token) => haystack.includes(token));
}

export function getVisibleContacts({
  contacts,
  drafts = {},
  searchQuery = '',
  filters = DEFAULT_FILTERS,
  messagePreviewByContact = {},
  metadataByContact = {},
  messagesByContact = {},
  now = Date.now(),
}) {
  const normalizedQuery = normalizeText(searchQuery);
  const mergedFilters = { ...DEFAULT_FILTERS, ...(filters || {}) };

  return (contacts || []).filter((contact) => {
    const draftPreview = getDraftPreview(drafts, contact.id);
    const hasDraft = Boolean(draftPreview);
    const metadata = getMetadata(metadataByContact, contact.id);
    const priority = getPriorityInfo(metadata);
    const checklistSummary = getChecklistSummary(metadata);
    const isPinned = metadata?.pinnedAt != null || contact?.pinnedAt != null;
    const hasNotes = Boolean(
      normalizeText(metadata?.notes || metadata?.notePreview),
    );
    const labels = getContactLabels(metadata);
    const hasLabels = labels.length > 0;
    const reminderStatus = getReminderStatus(metadata, now);
    const isSnoozed = isContactSnoozed(metadata, now);
    const replyState = getReplyState({
      contact,
      messages: messagesByContact?.[contact.id] || [],
      drafts,
      metadata,
      contactId: contact.id,
      now,
    });

    // A typed search is global: inbox filters and snoozing must not hide a matching chat.
    if (normalizedQuery) {
      return contactMatchesSearch(
        contact,
        draftPreview,
        messagePreviewByContact?.[contact.id] || '',
        metadata,
        normalizedQuery,
      );
    }

    if (isSnoozed && !mergedFilters.snoozedOnly) {
      return false;
    }

    if (mergedFilters.unreadOnly && !(contact.unreadCount > 0)) {
      return false;
    }

    if (mergedFilters.groupsOnly && contact.type !== 'group') {
      return false;
    }

    if (mergedFilters.draftsOnly && !hasDraft) {
      return false;
    }

    if (mergedFilters.pinnedOnly && !isPinned) {
      return false;
    }

    if (mergedFilters.notesOnly && !hasNotes) {
      return false;
    }

    if (mergedFilters.dueRemindersOnly && reminderStatus !== 'due') {
      return false;
    }

    if (mergedFilters.labelsOnly && !hasLabels) {
      return false;
    }

    if (mergedFilters.snoozedOnly && !isSnoozed) {
      return false;
    }

    if (
      mergedFilters.needsReplyOnly &&
      !['needs-reply', 'drafting'].includes(replyState)
    ) {
      return false;
    }

    if (mergedFilters.importantOnly && !priority.isImportant) {
      return false;
    }

    if (mergedFilters.tasksOnly && checklistSummary.open === 0) {
      return false;
    }

    return contactMatchesSearch(
      contact,
      draftPreview,
      messagePreviewByContact?.[contact.id] || '',
      metadata,
      normalizedQuery,
    );
  });
}

export function getContactActionSnapshot({
  contact = {},
  drafts = {},
  metadata = {},
  messages = [],
  now = Date.now(),
} = {}) {
  const contactId = contact?.id || '';
  const priority = getPriorityInfo(metadata);
  const checklist = getChecklistSummary(metadata);
  const reminderStatus = getReminderStatus(metadata, now);
  const replyState = getReplyState({
    contact,
    messages,
    drafts,
    metadata,
    contactId,
    now,
  });
  const draft = drafts?.[contactId] || null;
  const hasDraft = Boolean(getDraftText(drafts, contactId));
  const isPinned = metadata?.pinnedAt != null || contact?.pinnedAt != null;
  const isSnoozed = isContactSnoozed(metadata, now);
  const labels = getContactLabels(metadata);
  const timezoneInfo = getTimezoneInfo(metadata, now);
  const unreadCount = Math.max(0, Number(contact?.unreadCount || 0));
  const lastMessageTime =
    normalizeTimestamp(contact?.lastMessageTime) ||
    normalizeTimestamp(messages?.[messages.length - 1]?.timestamp) ||
    0;
  const reminderAt = normalizeTimestamp(metadata?.reminderAt);
  const draftUpdatedAt = normalizeTimestamp(draft?.updatedAt);

  const reasons = [];
  if (reminderStatus === 'due') {
    reasons.push('Reminder due');
  } else if (replyState === 'drafting') {
    reasons.push('Draft reply saved');
  } else if (replyState === 'needs-reply') {
    reasons.push('Needs your reply');
  } else if (replyState === 'waiting') {
    reasons.push('Waiting for them');
  }

  if (checklist.open > 0) {
    reasons.push(
      `${checklist.open} open task${checklist.open === 1 ? '' : 's'}`,
    );
  }

  if (priority.value !== 'normal') {
    reasons.push(`${priority.label} priority`);
  }

  if (hasDraft && !reasons.includes('Draft reply saved')) {
    reasons.push('Draft saved');
  }

  if (timezoneInfo && reasons.length < 3) {
    reasons.push(`${timezoneInfo.localTime} local time`);
  }

  if (labels.length > 0 && reasons.length < 3) {
    reasons.push(labels.slice(0, 2).join(' · '));
  }

  let attentionScore = 0;
  if (reminderStatus === 'due') {
    attentionScore += 130;
  } else if (reminderStatus === 'upcoming') {
    attentionScore += 48;
  }

  if (replyState === 'drafting') {
    attentionScore += 112;
  } else if (replyState === 'needs-reply') {
    attentionScore += 96;
  } else if (replyState === 'waiting') {
    attentionScore += 18;
  }

  attentionScore += checklist.open * 14;
  attentionScore += unreadCount * 4;
  if (priority.value === 'urgent') {
    attentionScore += 52;
  } else if (priority.value === 'high') {
    attentionScore += 36;
  } else if (priority.value === 'low') {
    attentionScore -= 10;
  }

  if (isPinned) {
    attentionScore += 14;
  }
  if (hasDraft) {
    attentionScore += 10;
  }
  if (labels.length > 0) {
    attentionScore += Math.min(10, labels.length * 2);
  }
  if (isSnoozed) {
    attentionScore -= 160;
  }

  return {
    contactId,
    unreadCount,
    hasDraft,
    draftUpdatedAt,
    isPinned,
    isSnoozed,
    priority,
    checklist,
    labels,
    replyState,
    reminderStatus,
    reminderAt,
    timezoneInfo,
    lastMessageTime,
    attentionScore,
    headline: reasons[0] || 'Recent activity',
    summary: reasons.slice(0, 3).join(' • '),
  };
}

export function buildVisitorDashboard({
  contacts = [],
  drafts = {},
  metadataByContact = {},
  messagesByContact = {},
  now = Date.now(),
  maxFocusContacts = 5,
  maxUpcomingReminders = 4,
} = {}) {
  const actionSnapshots = (contacts || []).map((contact) => {
    const metadata = getMetadata(metadataByContact, contact?.id);
    return {
      contact,
      metadata,
      snapshot: getContactActionSnapshot({
        contact,
        drafts,
        metadata,
        messages: messagesByContact?.[contact?.id] || [],
        now,
      }),
    };
  });

  const activeSnapshots = actionSnapshots.filter(
    (entry) => !entry.snapshot.isSnoozed,
  );

  const focusContacts = activeSnapshots
    .filter((entry) => {
      const snapshot = entry.snapshot;
      return (
        snapshot.attentionScore > 0 &&
        (snapshot.replyState === 'drafting' ||
          snapshot.replyState === 'needs-reply' ||
          snapshot.reminderStatus === 'due' ||
          snapshot.checklist.open > 0 ||
          snapshot.priority.isImportant ||
          snapshot.hasDraft)
      );
    })
    .sort((a, b) => {
      if (b.snapshot.attentionScore !== a.snapshot.attentionScore) {
        return b.snapshot.attentionScore - a.snapshot.attentionScore;
      }
      return (
        (b.snapshot.lastMessageTime || 0) - (a.snapshot.lastMessageTime || 0)
      );
    })
    .slice(0, Math.max(1, maxFocusContacts));

  const upcomingReminders = activeSnapshots
    .filter(
      (entry) =>
        entry.snapshot.reminderStatus === 'upcoming' &&
        entry.snapshot.reminderAt,
    )
    .sort((a, b) => {
      if (a.snapshot.reminderAt !== b.snapshot.reminderAt) {
        return a.snapshot.reminderAt - b.snapshot.reminderAt;
      }
      return b.snapshot.attentionScore - a.snapshot.attentionScore;
    })
    .slice(0, Math.max(1, maxUpcomingReminders));

  const stats = {
    needsReply: activeSnapshots.filter((entry) =>
      ['needs-reply', 'drafting'].includes(entry.snapshot.replyState),
    ).length,
    dueReminders: activeSnapshots.filter(
      (entry) => entry.snapshot.reminderStatus === 'due',
    ).length,
    drafts: activeSnapshots.filter((entry) => entry.snapshot.hasDraft).length,
    openTasks: activeSnapshots.reduce(
      (sum, entry) => sum + entry.snapshot.checklist.open,
      0,
    ),
  };

  return {
    stats,
    focusContacts,
    upcomingReminders,
    totalContacts: (contacts || []).length,
    snoozedContacts: actionSnapshots.filter((entry) => entry.snapshot.isSnoozed)
      .length,
  };
}
