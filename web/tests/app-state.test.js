import test from 'node:test';
import assert from 'node:assert/strict';
import {
  upsertDraft,
  getDraftPreview,
  toggleStarredMessage,
  isMessageStarred,
  filterMessagesByQuery,
  getReplyState,
  parseChecklistInput,
  parseLabelsInput,
  removeQuickReply,
  toggleChecklistItem,
  upsertChecklistItems,
  upsertQuickReply,
  getChecklistSummary,
  getPriorityInfo,
  getTimezoneInfo,
  getUntranslatedIncomingMessages,
} from '../public/app-state.js';

test('upsertDraft stores trimmed text and removes empty drafts', () => {
  const withDraft = upsertDraft({}, 'alice', '  Need to reply later  ', 1000);
  assert.deepEqual(withDraft, {
    alice: {
      text: 'Need to reply later',
      updatedAt: 1000,
    },
  });

  const cleared = upsertDraft(withDraft, 'alice', '   ', 2000);
  assert.deepEqual(cleared, {});
});

test('getDraftPreview returns a user-facing summary', () => {
  const drafts = {
    chat_1: { text: 'This is a saved draft message', updatedAt: 123 },
  };

  assert.equal(
    getDraftPreview(drafts, 'chat_1'),
    'Draft: This is a saved draft message',
  );
  assert.equal(getDraftPreview(drafts, 'missing'), '');
});

test('toggleStarredMessage adds and removes starred messages', () => {
  const message = {
    id: 'msg-1',
    contactId: 'chat_1',
    timestamp: 111,
    content: { type: 'text', body: 'Remember the passport' },
  };

  const starred = toggleStarredMessage({}, message, 5000);
  assert.equal(isMessageStarred(starred, 'msg-1'), true);
  assert.equal(starred['msg-1'].snippet, 'Remember the passport');

  const unstarred = toggleStarredMessage(starred, message, 6000);
  assert.equal(isMessageStarred(unstarred, 'msg-1'), false);
  assert.deepEqual(unstarred, {});
});

test('filterMessagesByQuery supports text search and starred-only filtering', () => {
  const messages = [
    { id: '1', content: { type: 'text', body: 'Book the airport taxi' } },
    { id: '2', content: { type: 'text', body: 'Pick up groceries' } },
  ];

  assert.deepEqual(
    filterMessagesByQuery(messages, 'airport', { starredLookup: {} }).map(
      (message) => message.id,
    ),
    ['1'],
  );

  assert.deepEqual(
    filterMessagesByQuery(messages, '', {
      starredOnly: true,
      starredLookup: { 2: { id: '2' } },
    }).map((message) => message.id),
    ['2'],
  );
});

test('parseLabelsInput trims, deduplicates, and preserves visitor-defined labels', () => {
  assert.deepEqual(
    parseLabelsInput(' VIP, travel plans, vip,   Family  , , follow-up '),
    ['VIP', 'travel plans', 'Family', 'follow-up'],
  );
});

test('getReplyState identifies reply queues, drafts, and waiting states', () => {
  const now = 1_710_000_000_000;
  const incomingMessage = {
    id: 'm1',
    timestamp: now - 10_000,
    isFromMe: false,
    content: { type: 'text', body: 'Can you confirm?' },
  };
  const outgoingMessage = {
    id: 'm2',
    timestamp: now - 5_000,
    isFromMe: true,
    content: { type: 'text', body: 'On it' },
  };

  assert.equal(
    getReplyState({
      contact: { id: 'vip', unreadCount: 0 },
      messages: [incomingMessage],
      drafts: {},
    }),
    'needs-reply',
  );
  assert.equal(
    getReplyState({
      contact: { id: 'vip', unreadCount: 0 },
      messages: [outgoingMessage],
      drafts: {},
    }),
    'waiting',
  );
  assert.equal(
    getReplyState({
      contact: { id: 'vip', unreadCount: 0 },
      messages: [incomingMessage],
      drafts: { vip: { text: 'Draft reply', updatedAt: now } },
    }),
    'drafting',
  );
  assert.equal(
    getReplyState({
      contact: { id: 'vip', unreadCount: 4 },
      messages: [],
      drafts: {},
    }),
    'needs-reply',
  );
  assert.equal(
    getReplyState({
      contact: { id: 'vip', unreadCount: 4 },
      messages: [],
      drafts: {},
      metadata: { snoozedUntil: now + 60_000 },
      now,
    }),
    'snoozed',
  );
});

test('upsertQuickReply deduplicates snippets and removeQuickReply deletes by id', () => {
  const initial = upsertQuickReply([], '  On my way now  ', 100);
  assert.equal(initial.length, 1);
  assert.equal(initial[0].text, 'On my way now');

  const deduped = upsertQuickReply(initial, 'on my way now', 200);
  assert.equal(deduped.length, 1);
  assert.equal(deduped[0].updatedAt, 200);

  const expanded = upsertQuickReply(
    deduped,
    'Thanks — I will send it shortly.',
    300,
    2,
  );
  const trimmed = upsertQuickReply(expanded, 'Got it, thank you!', 400, 2);
  assert.deepEqual(
    trimmed.map((reply) => reply.text),
    ['Got it, thank you!', 'Thanks — I will send it shortly.'],
  );

  assert.deepEqual(removeQuickReply(trimmed, trimmed[0].id), [trimmed[1]]);
});

test('priority info normalizes visitor triage preferences and surfaces important conversations', () => {
  assert.deepEqual(getPriorityInfo({ priority: 'URGENT' }), {
    value: 'urgent',
    label: 'Urgent',
    rank: 0,
    isImportant: true,
  });

  assert.deepEqual(getPriorityInfo({ priority: 'low' }), {
    value: 'low',
    label: 'Low',
    rank: 3,
    isImportant: false,
  });

  assert.deepEqual(getPriorityInfo({}), {
    value: 'normal',
    label: 'Normal',
    rank: 2,
    isImportant: false,
  });
});

test('checklists can be parsed from visitor notes, updated, and summarized', () => {
  const parsed = parseChecklistInput(
    '- [ ] Confirm check-in code\n- [x] Book taxi\nBring passport',
  );
  assert.deepEqual(
    parsed.map((item) => ({ text: item.text, done: item.done })),
    [
      { text: 'Confirm check-in code', done: false },
      { text: 'Book taxi', done: true },
      { text: 'Bring passport', done: false },
    ],
  );

  const initial = upsertChecklistItems(
    [],
    '- [ ] Confirm check-in code\n- [x] Book taxi',
    100,
  );
  assert.equal(initial.length, 2);
  assert.equal(initial[0].done, false);
  assert.equal(initial[1].done, true);

  const toggled = toggleChecklistItem(initial, initial[0].id, 200);
  assert.equal(toggled[0].done, true);
  assert.equal(toggled[0].updatedAt, 200);

  const updated = upsertChecklistItems(
    toggled,
    '- [ ] Confirm check-in code\nBring passport',
    300,
  );
  assert.deepEqual(
    updated.map((item) => ({ text: item.text, done: item.done })),
    [
      { text: 'Confirm check-in code', done: false },
      { text: 'Bring passport', done: false },
    ],
  );

  assert.deepEqual(getChecklistSummary({ checklist: updated }), {
    total: 2,
    completed: 0,
    open: 2,
    label: '2 open tasks',
  });
});

test('untranslated incoming messages excludes outgoing, translated, and media-only entries', () => {
  const messages = [
    { id: 'incoming', isFromMe: false, content: { body: 'Hola' } },
    {
      id: 'translated',
      isFromMe: false,
      isTranslated: true,
      content: { body: 'Bonjour' },
    },
    { id: 'outgoing', isFromMe: true, content: { body: 'Hello' } },
    { id: 'media', isFromMe: false, content: { type: 'image' } },
  ];

  assert.deepEqual(
    getUntranslatedIncomingMessages(messages).map((message) => message.id),
    ['incoming'],
  );
});

test('timezone info surfaces visitor-friendly local time and quiet hours context', () => {
  const morningUtc = Date.parse('2026-04-16T07:30:00Z');
  const quietUtc = Date.parse('2026-04-16T23:30:00Z');

  assert.deepEqual(getTimezoneInfo({ timezone: 'Europe/Madrid' }, morningUtc), {
    timezone: 'Europe/Madrid',
    localTime: '09:30',
    label: '09:30 local time',
    status: 'working-hours',
    statusLabel: 'Working hours',
  });

  assert.deepEqual(
    getTimezoneInfo({ timezone: 'America/Los_Angeles' }, quietUtc),
    {
      timezone: 'America/Los_Angeles',
      localTime: '16:30',
      label: '16:30 local time',
      status: 'working-hours',
      statusLabel: 'Working hours',
    },
  );

  assert.deepEqual(getTimezoneInfo({ timezone: 'Asia/Tokyo' }, quietUtc), {
    timezone: 'Asia/Tokyo',
    localTime: '08:30',
    label: '08:30 local time',
    status: 'morning',
    statusLabel: 'Morning',
  });
});
