import { getContactDisplayName, messageSnippet } from './app-state-core.js';

export function createDemoWorkspace(now = Date.now()) {
  const minutesAgo = (minutes) => now - minutes * 60_000;
  const minutesFromNow = (minutes) => now + minutes * 60_000;

  const contacts = [
    {
      id: 'demo-casa-azul',
      name: 'Sofia at Casa Azul',
      phone: '34600111222',
      type: 'private',
      unreadCount: 2,
      pinnedAt: minutesAgo(50),
      lastMessageTime: minutesAgo(4),
      lastMessagePreview: 'Perfect, can you arrive after 18:00?',
    },
    {
      id: 'demo-tutor',
      name: 'Jules (French tutor)',
      phone: '33677889900',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: minutesAgo(42),
      lastMessagePreview: 'Draft saved: send the corrected deck after lunch.',
    },
    {
      id: 'demo-osaka-group',
      name: 'Osaka arrival group',
      phone: null,
      type: 'group',
      unreadCount: 1,
      lastMessageTime: minutesAgo(95),
      lastMessagePreview: 'Venue pin still needs confirming.',
    },
    {
      id: 'demo-plumber',
      name: 'Theo the plumber',
      phone: '447700900123',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: minutesAgo(180),
      lastMessagePreview: 'Snoozed until tomorrow morning.',
    },
  ];

  const messagesByContact = {
    'demo-casa-azul': [
      {
        id: 'demo-casa-1',
        contactId: 'demo-casa-azul',
        timestamp: minutesAgo(21),
        isFromMe: false,
        senderName: 'Sofia',
        senderJid: '34600111222@s.whatsapp.net',
        content: {
          type: 'text',
          body: 'Las llaves estan en la caja junto a la puerta azul.',
        },
        originalText: 'Las llaves estan en la caja junto a la puerta azul.',
        translatedText: 'The keys are in the box next to the blue door.',
        sourceLanguage: 'Spanish',
        isTranslated: true,
        reactions: { '🙏': ['me'] },
      },
      {
        id: 'demo-casa-2',
        contactId: 'demo-casa-azul',
        timestamp: minutesAgo(14),
        isFromMe: true,
        senderJid: 'demo@s.whatsapp.net',
        content: {
          type: 'text',
          body: '18:30 works for us. I will send the taxi plate when we leave the station.',
        },
        translatedText:
          '18:30 nos va bien. Te enviare la matricula del taxi cuando salgamos de la estacion.',
        sourceLanguage: 'Spanish',
        isTranslated: true,
      },
      {
        id: 'demo-casa-3',
        contactId: 'demo-casa-azul',
        timestamp: minutesAgo(4),
        isFromMe: false,
        senderName: 'Sofia',
        senderJid: '34600111222@s.whatsapp.net',
        content: {
          type: 'text',
          body: 'Perfecto, puede llegar despues de las 18:00?',
        },
        originalText: 'Perfecto, puede llegar despues de las 18:00?',
        translatedText: 'Perfect, can you arrive after 18:00?',
        sourceLanguage: 'Spanish',
        isTranslated: true,
      },
    ],
    'demo-tutor': [
      {
        id: 'demo-tutor-1',
        contactId: 'demo-tutor',
        timestamp: minutesAgo(52),
        isFromMe: false,
        senderName: 'Jules',
        senderJid: '33677889900@s.whatsapp.net',
        content: {
          type: 'text',
          body: 'Peux-tu envoyer la version corrigee apres le dejeuner ?',
        },
      },
      {
        id: 'demo-tutor-2',
        contactId: 'demo-tutor',
        timestamp: minutesAgo(42),
        isFromMe: true,
        senderJid: 'demo@s.whatsapp.net',
        content: {
          type: 'text',
          body: 'Yes, I will send the corrected version after lunch.',
        },
        translatedText:
          'Oui, je vais envoyer la version corrigee apres le dejeuner.',
        sourceLanguage: 'French',
        isTranslated: true,
      },
    ],
    'demo-osaka-group': [
      {
        id: 'demo-osaka-1',
        contactId: 'demo-osaka-group',
        timestamp: minutesAgo(105),
        isFromMe: false,
        senderName: 'Mika',
        senderJid: '819012345678@s.whatsapp.net',
        chatType: 'group',
        content: {
          type: 'text',
          body: 'Venue pin still needs confirming before everyone heads out.',
        },
      },
      {
        id: 'demo-osaka-2',
        contactId: 'demo-osaka-group',
        timestamp: minutesAgo(95),
        isFromMe: false,
        senderName: 'Ken',
        senderJid: '819098765432@s.whatsapp.net',
        chatType: 'group',
        content: { type: 'text', body: '駅の出口は何番ですか?' },
        originalText: '駅の出口は何番ですか?',
        translatedText: 'Which station exit should we use?',
        sourceLanguage: 'Japanese',
        isTranslated: true,
      },
    ],
    'demo-plumber': [
      {
        id: 'demo-plumber-1',
        contactId: 'demo-plumber',
        timestamp: minutesAgo(180),
        isFromMe: false,
        senderName: 'Theo',
        senderJid: '447700900123@s.whatsapp.net',
        content: {
          type: 'text',
          body: 'I will send the boiler quote tomorrow morning.',
        },
      },
    ],
  };

  const metadataByContact = {
    'demo-casa-azul': {
      alias: 'Casa Azul host',
      priority: 'urgent',
      labels: ['Travel', 'VIP'],
      notes:
        'Prefers concise Spanish replies. Confirm timings and transport details clearly.',
      timezone: 'Europe/Madrid',
      targetLanguage: 'Spanish',
      reminderText: 'Send the taxi plate before arrival',
      reminderAt: minutesAgo(1),
      checklist: [
        {
          id: 'demo-casa-task-1',
          text: 'Confirm check-in time',
          done: false,
          updatedAt: minutesAgo(30),
        },
        {
          id: 'demo-casa-task-2',
          text: 'Send taxi plate',
          done: false,
          updatedAt: minutesAgo(30),
        },
      ],
    },
    'demo-tutor': {
      priority: 'normal',
      labels: ['French', 'Learning'],
      notes: 'Friendly tone. He likes short context and bullet points.',
      timezone: 'Europe/Paris',
      targetLanguage: 'French',
      checklist: [
        {
          id: 'demo-tutor-task-1',
          text: 'Attach corrected deck',
          done: false,
          updatedAt: minutesAgo(60),
        },
        {
          id: 'demo-tutor-task-2',
          text: 'Confirm next lesson time',
          done: true,
          updatedAt: minutesAgo(60),
        },
      ],
    },
    'demo-osaka-group': {
      priority: 'high',
      labels: ['Japan', 'Group'],
      notes:
        'Group chat for arrival logistics. Keep messages practical and avoid long explanations.',
      timezone: 'Asia/Tokyo',
      targetLanguage: 'Japanese',
      reminderText: 'Share the venue pin and train exit',
      reminderAt: minutesFromNow(75),
      checklist: [
        {
          id: 'demo-osaka-task-1',
          text: 'Confirm venue pin',
          done: false,
          updatedAt: minutesAgo(90),
        },
        {
          id: 'demo-osaka-task-2',
          text: 'Share local train exit',
          done: false,
          updatedAt: minutesAgo(90),
        },
      ],
    },
    'demo-plumber': {
      priority: 'low',
      labels: ['Home'],
      notes: 'Snoozed until the quote is expected.',
      timezone: 'Europe/London',
      snoozedUntil: minutesFromNow(16 * 60),
    },
  };

  const drafts = {
    'demo-tutor': {
      text: 'Thanks Jules, I will send the corrected deck just after lunch and confirm the next lesson time.',
      updatedAt: minutesAgo(10),
    },
  };

  const quickReplies = [
    {
      id: `demo-reply-1-${now}`,
      text: 'Thanks, I will confirm shortly.',
      updatedAt: minutesAgo(15),
    },
    {
      id: `demo-reply-2-${now}`,
      text: 'Could you send the address again?',
      updatedAt: minutesAgo(16),
    },
    {
      id: `demo-reply-3-${now}`,
      text: 'I will send the details before we leave.',
      updatedAt: minutesAgo(17),
    },
  ];

  return {
    contacts,
    messagesByContact,
    metadataByContact,
    drafts,
    quickReplies,
    globalUsage: { inputTokens: 1840, outputTokens: 720, costUsd: 0.03 },
  };
}

export function suggestDemoReply({
  message = {},
  metadata = {},
  contact = {},
} = {}) {
  const text = messageSnippet(message).toLowerCase();
  const displayName = getContactDisplayName(contact, metadata);
  const targetLanguage = String(metadata?.targetLanguage || '').trim();
  const languageNote = targetLanguage
    ? ` I will keep it clear in ${targetLanguage}.`
    : '';

  if (
    text.includes('18:00') ||
    text.includes('check') ||
    text.includes('llave') ||
    text.includes('key')
  ) {
    return `Thanks ${displayName.split(' ')[0]}, 18:30 still works. I will send the taxi plate before we arrive.${languageNote}`;
  }
  if (
    text.includes('venue') ||
    text.includes('exit') ||
    text.includes('出口')
  ) {
    return `I am confirming the venue pin now and will share the correct station exit with everyone shortly.${languageNote}`;
  }
  if (text.includes('quote') || text.includes('tomorrow')) {
    return `Thanks, tomorrow morning works. I will keep the chat snoozed until the quote is ready.`;
  }

  return `Thanks, I have this. I will reply with the next clear step shortly.${languageNote}`;
}
