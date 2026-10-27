import {
  COMPOSER_LANGUAGE_PRESETS,
  COMPOSER_REMINDER_PRESETS,
  COMPOSER_TONE_PRESETS,
  filterMessagesByQuery,
  getContactDisplayName,
  getTimezoneInfo,
  messageSnippet,
  normalizeText,
} from './app-state-core.js';
import { suggestDemoReply } from './app-state-demo.js';
import {
  buildConversationBrief,
  getOpenChecklistItems,
  textContainsQuestion,
  textMentionsAny,
} from './app-state-briefs.js';

export function getSmartReplyOptions({
  message = {},
  metadata = {},
  contact = {},
} = {}) {
  if (!message) return [];

  const displayName =
    getContactDisplayName(contact, metadata).split(' ')[0] || 'there';
  const targetLanguage = String(
    metadata?.targetLanguage || metadata?.languageOverride || '',
  ).trim();
  const languageNote = targetLanguage
    ? ` I will keep it clear in ${targetLanguage}.`
    : '';
  const primary = suggestDemoReply({
    message,
    metadata: { ...metadata, targetLanguage },
    contact,
  });

  const text = messageSnippet(message).toLowerCase();
  const detailPrompt =
    text.includes('address') || text.includes('venue') || text.includes('exit')
      ? 'Could you send the exact location details again so I can confirm the next step?'
      : 'Could you send the key detail again so I can confirm properly?';
  const delayReply =
    text.includes('tomorrow') || text.includes('quote')
      ? `Thanks ${displayName}, tomorrow works. I will confirm once I have checked it.${languageNote}`
      : `Thanks ${displayName}, I am checking this now and will confirm shortly.${languageNote}`;

  return [
    { id: 'confirm', label: 'Confirm', text: primary },
    { id: 'ask-detail', label: 'Ask detail', text: detailPrompt },
    { id: 'buy-time', label: 'Buy time', text: delayReply },
  ];
}

export function getComposerProfilePresets(metadata = {}) {
  const currentLanguage = normalizeText(
    metadata?.targetLanguage ||
      metadata?.languageOverride ||
      metadata?.language ||
      'Spanish',
  );
  const currentTone = normalizeText(metadata?.translationStyle || '');

  return {
    languages: COMPOSER_LANGUAGE_PRESETS.map((label) => ({
      label,
      value: label,
      active: normalizeText(label) === currentLanguage,
    })),
    tones: COMPOSER_TONE_PRESETS.map((label) => ({
      label,
      value: label.toLowerCase(),
      active: normalizeText(label) === currentTone,
    })),
  };
}

export function getComposerReminderPresets(now = Date.now()) {
  return COMPOSER_REMINDER_PRESETS.map((preset) => {
    const reminderAt = new Date(now);
    if (preset.days) {
      reminderAt.setDate(reminderAt.getDate() + preset.days);
      reminderAt.setHours(preset.hour, 0, 0, 0);
    } else {
      reminderAt.setHours(reminderAt.getHours() + preset.hours);
      reminderAt.setMinutes(0, 0, 0);
    }

    return {
      id: preset.id,
      label: preset.label,
      reminderAt: reminderAt.getTime(),
    };
  });
}

export function getContextualReminderPresets({
  draftText = '',
  latestIncomingMessage = null,
  metadata = {},
  now = Date.now(),
} = {}) {
  const presets = getComposerReminderPresets(now);
  const sourceText = `${draftText || ''} ${latestIncomingMessage ? messageSnippet(latestIncomingMessage, 120) : ''}`;
  const normalized = normalizeText(sourceText);
  const reminderText = String(metadata?.reminderText || '').trim();

  if (/\btomorrow\b|mañana|demain|明日/.test(normalized)) {
    const tomorrow = new Date(now);
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(9, 0, 0, 0);
    presets.unshift({
      id: 'context-tomorrow',
      label: 'Tomorrow from chat',
      reminderAt: tomorrow.getTime(),
      reason: 'Detected tomorrow in the conversation',
    });
  } else if (
    /\bquote\b|\bprice\b|\bconfirm\b|\bcheck\b|\bfollow up\b/.test(
      normalized,
    ) ||
    reminderText
  ) {
    const soon = new Date(now + 2 * 60 * 60 * 1000);
    soon.setMinutes(0, 0, 0);
    presets.unshift({
      id: 'context-follow-up',
      label: 'Follow-up soon',
      reminderAt: soon.getTime(),
      reason: reminderText || 'Detected a follow-up cue',
    });
  }

  return presets.filter(
    (preset, index, list) =>
      list.findIndex((candidate) => candidate.id === preset.id) === index,
  );
}

export function getComposerSendReadiness({
  draftText = '',
  metadata = {},
  latestIncomingMessage = null,
  now = Date.now(),
} = {}) {
  const trimmedDraft = String(draftText || '').trim();
  const latestIncomingSnippet = latestIncomingMessage
    ? messageSnippet(latestIncomingMessage, 160)
    : '';
  const openTasks = getOpenChecklistItems(metadata);
  const timezoneInfo = getTimezoneInfo(metadata, now);
  const checks = [];

  if (!trimmedDraft) {
    checks.push({
      id: 'draft',
      status: 'info',
      label: 'Type a draft to check reply coverage.',
    });
  } else {
    checks.push({
      id: 'draft',
      status: 'ready',
      label:
        metadata.translationEnabled === true
          ? 'Draft is ready to translate.'
          : 'Draft will send as written.',
    });
  }

  if (latestIncomingSnippet && textContainsQuestion(latestIncomingSnippet)) {
    checks.push({
      id: 'question',
      status:
        textContainsQuestion(trimmedDraft) || trimmedDraft.length > 12
          ? 'ready'
          : 'warning',
      label: textContainsQuestion(trimmedDraft)
        ? 'Draft asks a clarifying question.'
        : 'Latest incoming looks like a question; answer it directly.',
    });
  }

  if (openTasks.length > 0) {
    checks.push({
      id: 'tasks',
      status: textMentionsAny(
        trimmedDraft,
        openTasks.map((item) => item.text),
      )
        ? 'ready'
        : 'warning',
      label: textMentionsAny(
        trimmedDraft,
        openTasks.map((item) => item.text),
      )
        ? 'Draft references an open task.'
        : `Open task still visible: ${openTasks[0].text}`,
    });
  }

  if (timezoneInfo?.status === 'quiet-hours') {
    checks.push({
      id: 'timezone',
      status: 'warning',
      label: `${timezoneInfo.localTime} there. Consider scheduling or waiting.`,
    });
  } else if (timezoneInfo) {
    checks.push({
      id: 'timezone',
      status: 'ready',
      label: `${timezoneInfo.localTime} there. Timing looks reasonable.`,
    });
  }

  const warnings = checks.filter((check) => check.status === 'warning').length;
  return {
    checks,
    warnings,
    scoreLabel:
      warnings === 0
        ? 'Ready'
        : `${warnings} check${warnings === 1 ? '' : 's'}`,
  };
}

export function getComposerAssistState({
  draftText = '',
  metadata = {},
  contact = {},
  latestIncomingMessage = null,
  messages = [],
  demoMode = false,
  now = Date.now(),
} = {}) {
  const translationEnabled = metadata.translationEnabled === true;
  const targetLanguage =
    String(
      metadata?.targetLanguage ||
        metadata?.languageOverride ||
        metadata?.language ||
        (demoMode ? 'Spanish' : 'Automatic'),
    ).trim() || 'Automatic';
  const translationStyle = String(metadata?.translationStyle || '').trim();
  const trimmedDraft = String(draftText || '').trim();
  const latestIncomingSnippet = latestIncomingMessage
    ? messageSnippet(latestIncomingMessage, 96)
    : '';
  const suggestedReply = latestIncomingMessage
    ? suggestDemoReply({
        message: latestIncomingMessage,
        metadata: { ...metadata, targetLanguage },
        contact,
      })
    : '';
  const smartReplies = latestIncomingMessage
    ? getSmartReplyOptions({
        message: latestIncomingMessage,
        metadata: { ...metadata, targetLanguage },
        contact,
      })
    : [];
  const translatedPreview =
    trimmedDraft && translationEnabled && demoMode
      ? simulateTranslation(trimmedDraft, targetLanguage)
      : '';
  const styleSummary = translationStyle
    ? `${translationStyle} tone`
    : 'standard tone';
  const profilePresets = getComposerProfilePresets({
    ...metadata,
    targetLanguage,
    translationStyle,
  });
  const readiness = getComposerSendReadiness({
    draftText: trimmedDraft,
    metadata,
    latestIncomingMessage,
    now,
  });
  const conversationBrief = buildConversationBrief({
    contact,
    metadata,
    messages,
    drafts: contact?.id
      ? { [contact.id]: { text: trimmedDraft, updatedAt: now } }
      : {},
    now,
  });

  return {
    translationEnabled,
    targetLanguage,
    translationStyle,
    styleSummary,
    latestIncomingSnippet,
    suggestedReply,
    smartReplies,
    profilePresets,
    reminderPresets: getContextualReminderPresets({
      draftText: trimmedDraft,
      latestIncomingMessage,
      metadata,
      now,
    }),
    readiness,
    conversationBrief,
    translatedPreview,
    hasDraft: Boolean(trimmedDraft),
    hasIncomingContext: Boolean(latestIncomingMessage),
    canUseSuggestedReply: smartReplies.length > 0 || Boolean(suggestedReply),
    showPreview: Boolean(
      trimmedDraft ||
      latestIncomingMessage ||
      targetLanguage ||
      translationStyle ||
      demoMode,
    ),
    previewLabel: !translationEnabled
      ? 'Send as written'
      : demoMode
        ? 'Demo translation preview'
        : 'Translation route',
  };
}

export function simulateTranslation(text, targetLanguage = 'Spanish') {
  const trimmed = String(text || '').trim();
  if (!trimmed) return '';

  const language = String(targetLanguage || 'Spanish').trim() || 'Spanish';
  const normalizedLanguage = language.toLowerCase();
  const lowerText = trimmed.toLowerCase();
  const phraseMap = {
    spanish: [
      [
        /18:30|taxi|plate|station/,
        '18:30 nos va bien. Te enviare la matricula del taxi antes de llegar.',
      ],
      [/thank|thanks|confirm|shortly/, 'Gracias, lo confirmare en breve.'],
      [/address|send.*again/, 'Puedes enviarme la direccion otra vez?'],
    ],
    french: [
      [
        /deck|lesson|lunch|corrected/,
        'Merci, j enverrai la version corrigee apres le dejeuner et je confirmerai le prochain cours.',
      ],
      [/thank|thanks|confirm|shortly/, 'Merci, je confirmerai cela bientot.'],
    ],
    japanese: [
      [/venue|pin|station|exit/, '会場のピンと駅の出口をすぐに共有します。'],
      [
        /thank|thanks|confirm|shortly/,
        'ありがとうございます。すぐに確認します。',
      ],
    ],
  };

  const matches = phraseMap[normalizedLanguage] || phraseMap.spanish;
  const match = matches.find(([pattern]) => pattern.test(lowerText));
  if (match) return match[1];

  return `[${language} demo translation] ${trimmed}`;
}

export function countMatchingMessages(messages, query, options = {}) {
  return filterMessagesByQuery(messages, query, options).length;
}
