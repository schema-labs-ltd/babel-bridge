import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildConversationBrief,
  buildConversationActionPlan,
  buildConversationHandoffBrief,
  getComposerSendReadiness,
  getContextualReminderPresets,
  getComposerAssistState,
  getComposerProfilePresets,
  getComposerReminderPresets,
  getMessageSnippet,
  buildVisitorDashboard,
  createDemoWorkspace,
  simulateTranslation,
  getSmartReplyOptions,
  suggestDemoReply,
} from '../public/app-state.js';

test('createDemoWorkspace provides a usable visitor demo inbox', () => {
  const now = 1_710_000_000_000;
  const demo = createDemoWorkspace(now);
  const dashboard = buildVisitorDashboard({
    contacts: demo.contacts,
    drafts: demo.drafts,
    metadataByContact: demo.metadataByContact,
    messagesByContact: demo.messagesByContact,
    now,
  });

  assert.equal(demo.contacts.length, 4);
  assert.equal(demo.contacts[0].id, 'demo-casa-azul');
  assert.ok(demo.messagesByContact['demo-casa-azul'].length >= 3);
  assert.equal(dashboard.stats.needsReply, 3);
  assert.equal(dashboard.stats.dueReminders, 1);
  assert.equal(dashboard.stats.drafts, 1);
  assert.ok(dashboard.stats.openTasks >= 4);
  assert.equal(dashboard.snoozedContacts, 1);
});

test('demo reply and translation helpers make the product explorable without a backend', () => {
  const demo = createDemoWorkspace(1_710_000_000_000);
  const contact = demo.contacts.find((entry) => entry.id === 'demo-casa-azul');
  const metadata = demo.metadataByContact[contact.id];
  const message = demo.messagesByContact[contact.id].at(-1);
  const reply = suggestDemoReply({ message, metadata, contact });

  assert.match(reply, /18:30/);
  assert.match(reply, /taxi plate/);
  assert.equal(
    simulateTranslation(
      '18:30 works and I will send the taxi plate',
      'Spanish',
    ),
    '18:30 nos va bien. Te enviare la matricula del taxi antes de llegar.',
  );
  assert.equal(
    simulateTranslation(
      'Please share the venue pin and station exit',
      'Japanese',
    ),
    '会場のピンと駅の出口をすぐに共有します。',
  );
});

test('composer assist builds a translation preview and reply starter', () => {
  const incoming = {
    id: 'm1',
    isFromMe: false,
    content: { type: 'text', body: 'Puede llegar despues de las 18:00?' },
  };

  const state = getComposerAssistState({
    draftText: '18:30 works and I will send the taxi plate',
    metadata: {
      languageOverride: 'Spanish',
      translationStyle: 'friendly',
      translationEnabled: true,
    },
    contact: { id: 'host', name: 'Sofia' },
    latestIncomingMessage: incoming,
    demoMode: true,
  });

  assert.equal(
    getMessageSnippet(incoming),
    'Puede llegar despues de las 18:00?',
  );
  assert.equal(state.targetLanguage, 'Spanish');
  assert.equal(state.styleSummary, 'friendly tone');
  assert.equal(state.previewLabel, 'Demo translation preview');
  assert.match(state.translatedPreview, /18:30 nos va bien/);
  assert.match(state.suggestedReply, /18:30 still works/);
  assert.equal(state.canUseSuggestedReply, true);
  assert.deepEqual(
    state.smartReplies.map((reply) => reply.id),
    ['confirm', 'ask-detail', 'buy-time'],
  );
  assert.equal(
    state.profilePresets.languages.find((preset) => preset.value === 'Spanish')
      .active,
    true,
  );
  assert.equal(
    state.profilePresets.tones.find((preset) => preset.value === 'friendly')
      .active,
    true,
  );
  assert.equal(state.readiness.scoreLabel, 'Ready');
  assert.equal(
    state.conversationBrief.nextAction,
    'Review and send the saved draft.',
  );
});

test('composer assist still explains the route without a draft', () => {
  const state = getComposerAssistState({
    metadata: { targetLanguage: 'Japanese', translationEnabled: true },
    demoMode: false,
  });

  assert.equal(state.targetLanguage, 'Japanese');
  assert.equal(state.previewLabel, 'Translation route');
  assert.equal(state.hasDraft, false);
  assert.equal(state.canUseSuggestedReply, false);
  assert.equal(state.showPreview, true);
});

test('smart reply options give distinct one-tap composer choices', () => {
  const replies = getSmartReplyOptions({
    message: {
      content: { body: 'Please share the venue pin and station exit' },
    },
    metadata: { languageOverride: 'Japanese' },
    contact: { id: 'planner', name: 'Akari' },
  });

  assert.equal(replies.length, 3);
  assert.equal(replies[0].label, 'Confirm');
  assert.match(replies[1].text, /location details/);
  assert.match(replies[2].text, /Akari/);
});

test('composer profile and reminder presets expose fast setup choices', () => {
  const profile = getComposerProfilePresets({
    targetLanguage: 'French',
    translationStyle: 'formal',
  });
  const now = new Date('2026-04-27T10:20:00Z').getTime();
  const reminders = getComposerReminderPresets(now);

  assert.equal(
    profile.languages.find((preset) => preset.value === 'French').active,
    true,
  );
  assert.equal(
    profile.tones.find((preset) => preset.value === 'formal').active,
    true,
  );
  assert.deepEqual(
    reminders.map((preset) => preset.id),
    ['later-today', 'tomorrow', 'next-week'],
  );
  assert.equal(new Date(reminders[0].reminderAt).getMinutes(), 0);
  assert.ok(reminders[0].reminderAt > now);
  assert.ok(reminders[0].reminderAt <= now + 3 * 60 * 60 * 1000);
  assert.equal(new Date(reminders[1].reminderAt).getHours(), 9);
});

test('contextual reminders promote follow-up timing detected in the conversation', () => {
  const now = new Date('2026-04-27T10:20:00Z').getTime();
  const latestIncomingMessage = {
    id: 'incoming',
    isFromMe: false,
    content: {
      type: 'text',
      body: 'Can you confirm the quote tomorrow morning?',
    },
  };

  const presets = getContextualReminderPresets({ latestIncomingMessage, now });

  assert.equal(presets[0].id, 'context-tomorrow');
  assert.equal(presets[0].label, 'Tomorrow from chat');
  assert.equal(new Date(presets[0].reminderAt).getHours(), 9);
});

test('composer readiness warns about missed tasks and quiet-hour contacts', () => {
  const readiness = getComposerSendReadiness({
    draftText: 'Thanks, I will check.',
    metadata: {
      timezone: 'Asia/Tokyo',
      checklist: [{ id: 'task-1', text: 'Share venue pin', done: false }],
    },
    latestIncomingMessage: {
      id: 'incoming',
      isFromMe: false,
      content: { type: 'text', body: 'Which station exit should we use?' },
    },
    now: new Date('2026-04-27T18:30:00Z').getTime(),
  });

  assert.equal(readiness.warnings, 2);
  assert.equal(
    readiness.checks.some(
      (check) => check.id === 'tasks' && check.status === 'warning',
    ),
    true,
  );
  assert.equal(
    readiness.checks.some(
      (check) => check.id === 'timezone' && check.status === 'warning',
    ),
    true,
  );
});

test('conversation brief turns latest messages and tasks into a next action', () => {
  const brief = buildConversationBrief({
    contact: { id: 'trip', name: 'Trip group', unreadCount: 1 },
    metadata: {
      checklist: [{ id: 'task-1', text: 'Confirm venue pin', done: false }],
    },
    messages: [
      {
        id: 'outgoing',
        isFromMe: true,
        content: { type: 'text', body: 'I will check the location.' },
      },
      {
        id: 'incoming',
        isFromMe: false,
        content: { type: 'text', body: 'Which station exit should we use?' },
      },
    ],
  });

  assert.equal(brief.hasQuestion, true);
  assert.equal(brief.nextAction, 'Answer the latest question directly.');
  assert.equal(brief.openTasks[0].text, 'Confirm venue pin');
  assert.match(brief.summary, /Which station exit/);
});

test('conversation action plan prioritizes drafts, questions, tasks, and translation backlog', () => {
  const now = Date.parse('2026-04-28T10:00:00Z');
  const messages = [
    {
      id: 'incoming-1',
      isFromMe: false,
      content: { type: 'text', body: 'Which station exit should we use?' },
    },
    {
      id: 'incoming-2',
      isFromMe: false,
      content: { type: 'text', body: '駅の出口は何番ですか?' },
    },
  ];

  const plan = buildConversationActionPlan({
    contact: { id: 'trip', name: 'Trip group', unreadCount: 1 },
    metadata: {
      checklist: [{ id: 'task-1', text: 'Confirm venue pin', done: false }],
    },
    drafts: {
      trip: { text: 'I will confirm the venue pin shortly.', updatedAt: now },
    },
    messages,
    now,
  });

  assert.equal(plan.untranslatedCount, 2);
  assert.deepEqual(plan.actions.map((action) => action.id).slice(0, 4), [
    'review-draft',
    'answer-question',
    'translate-incoming',
    'task-task-1',
  ]);
});

test('conversation handoff brief is copy-ready and includes visitor context', () => {
  const handoff = buildConversationHandoffBrief({
    contact: { id: 'host', name: 'Sofia' },
    metadata: {
      labels: ['Travel', 'VIP'],
      notes: 'Prefers concise replies.',
      checklist: [{ id: 'task-1', text: 'Send taxi plate', done: false }],
      reminderText: 'Send taxi plate before arrival',
    },
    drafts: { host: { text: '18:30 works for us.', updatedAt: 123 } },
    messages: [
      {
        id: 'in',
        isFromMe: false,
        senderName: 'Sofia',
        content: { type: 'text', body: 'Puede llegar despues de las 18:00?' },
      },
      {
        id: 'out',
        isFromMe: true,
        content: { type: 'text', body: 'I will confirm.' },
      },
    ],
  });

  assert.match(handoff, /Conversation: Sofia/);
  assert.match(handoff, /Next action:/);
  assert.match(handoff, /Labels: Travel, VIP/);
  assert.match(handoff, /Open tasks:\n- Send taxi plate/);
  assert.match(handoff, /Saved draft:\n18:30 works for us./);
  assert.match(handoff, /Recent messages:\n- Sofia:/);
});
