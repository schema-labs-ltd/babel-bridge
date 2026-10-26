import test from 'node:test';
import assert from 'node:assert/strict';
import {
  getVisibleContacts,
  normalizeInboxFilters,
  getReminderStatus,
  getContactDisplayName,
  isContactSnoozed,
  getContactActionSnapshot,
  buildVisitorDashboard,
} from '../public/app-state.js';

test('getVisibleContacts prioritizes drafts, unread filter, and search matching', () => {
  const contacts = [
    {
      id: 'alice',
      name: 'Alice',
      phone: '111',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 10,
    },
    {
      id: 'project',
      name: 'Project Team',
      phone: null,
      type: 'group',
      unreadCount: 3,
      lastMessageTime: 20,
    },
  ];

  const drafts = {
    alice: { text: 'Send travel plan', updatedAt: 999 },
  };

  const visibleWithSearch = getVisibleContacts({
    contacts,
    drafts,
    searchQuery: 'travel',
    filters: { unreadOnly: false, groupsOnly: false, draftsOnly: false },
    messagePreviewByContact: {},
    metadataByContact: {},
  }).map((contact) => contact.id);

  assert.deepEqual(visibleWithSearch, ['alice']);

  const unreadOnly = getVisibleContacts({
    contacts,
    drafts,
    searchQuery: '',
    filters: { unreadOnly: true, groupsOnly: false, draftsOnly: false },
    messagePreviewByContact: {},
    metadataByContact: {},
  }).map((contact) => contact.id);

  assert.deepEqual(unreadOnly, ['project']);

  const draftsOnly = getVisibleContacts({
    contacts,
    drafts,
    searchQuery: '',
    filters: { unreadOnly: false, groupsOnly: false, draftsOnly: true },
    messagePreviewByContact: {},
    metadataByContact: {},
  }).map((contact) => contact.id);

  assert.deepEqual(draftsOnly, ['alice']);
});

test('sidebar search ignores active inbox filters and snoozing', () => {
  const contacts = [
    {
      id: 'alice',
      name: 'Alice',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 10,
    },
  ];

  const visible = getVisibleContacts({
    contacts,
    searchQuery: 'alice',
    filters: { unreadOnly: true },
    metadataByContact: { alice: { snoozedUntil: 2_000 } },
    now: 1_000,
  }).map((contact) => contact.id);

  assert.deepEqual(visible, ['alice']);
});

test('hidden inbox controls cannot leave an unread-only filter active', () => {
  const normalized = normalizeInboxFilters(
    { unreadOnly: true, groupsOnly: true },
    { controlsAvailable: false },
  );

  assert.equal(normalized.unreadOnly, false);
  assert.equal(normalized.groupsOnly, false);
});

test('getVisibleContacts supports pinned and notes-only inbox filters', () => {
  const contacts = [
    {
      id: 'vip',
      name: 'VIP Client',
      phone: '555',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 30,
    },
    {
      id: 'family',
      name: 'Family',
      phone: '777',
      type: 'group',
      unreadCount: 2,
      lastMessageTime: 40,
    },
  ];

  const metadataByContact = {
    vip: { pinnedAt: 100, notes: 'Needs the pricing recap before Friday' },
    family: { pinnedAt: null, notes: '' },
  };

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { pinnedOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['vip'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { notesOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['vip'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: 'pricing recap',
      filters: {},
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['vip'],
  );
});

test('snoozed conversations stay out of the default inbox until the snooze expires', () => {
  const now = 1_710_000_000_000;
  const contacts = [
    {
      id: 'snoozed',
      name: 'Later',
      phone: '111',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 20,
    },
    {
      id: 'active',
      name: 'Now',
      phone: '222',
      type: 'private',
      unreadCount: 1,
      lastMessageTime: 30,
    },
  ];
  const metadataByContact = {
    snoozed: { snoozedUntil: now + 60_000 },
  };

  assert.equal(isContactSnoozed(metadataByContact.snoozed, now), true);

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: {},
      messagePreviewByContact: {},
      metadataByContact,
      now,
    }).map((contact) => contact.id),
    ['active'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { snoozedOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
      now,
    }).map((contact) => contact.id),
    ['snoozed'],
  );
});

test('getVisibleContacts supports reminder and label-focused triage views', () => {
  const now = 1_710_000_000_000;
  const contacts = [
    {
      id: 'vip',
      name: 'VIP Client',
      phone: '555',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 30,
    },
    {
      id: 'trip',
      name: 'Trip Planner',
      phone: '777',
      type: 'private',
      unreadCount: 2,
      lastMessageTime: 40,
    },
  ];
  const metadataByContact = {
    vip: {
      reminderText: 'Send the pricing breakdown',
      reminderAt: now - 5_000,
      labels: ['VIP', 'follow-up'],
    },
    trip: {
      reminderText: 'Share the itinerary tomorrow',
      reminderAt: now + 86_400_000,
      labels: ['travel'],
    },
  };

  assert.equal(getReminderStatus(metadataByContact.vip, now), 'due');
  assert.equal(getReminderStatus(metadataByContact.trip, now), 'upcoming');

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { dueRemindersOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
      now,
    }).map((contact) => contact.id),
    ['vip'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { labelsOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
      now,
    }).map((contact) => contact.id),
    ['vip', 'trip'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: 'pricing breakdown vip',
      filters: {},
      messagePreviewByContact: {},
      metadataByContact,
      now,
    }).map((contact) => contact.id),
    ['vip'],
  );
});

test('getVisibleContacts supports alias search and reply queue filters', () => {
  const now = 1_710_000_000_000;
  const contacts = [
    {
      id: 'host',
      name: 'Maria Lopez',
      phone: '111',
      type: 'private',
      unreadCount: 1,
      lastMessageTime: 30,
    },
    {
      id: 'vendor',
      name: 'Office Vendor',
      phone: '222',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 40,
    },
  ];
  const metadataByContact = {
    host: { alias: 'Madrid Airbnb host' },
    vendor: { alias: 'Printer supplier' },
  };

  assert.equal(
    getContactDisplayName(contacts[0], metadataByContact.host),
    'Madrid Airbnb host',
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: 'airbnb madrid',
      filters: {},
      messagePreviewByContact: {},
      metadataByContact,
      messagesByContact: {
        host: [
          {
            id: 'host-1',
            isFromMe: false,
            timestamp: now - 1_000,
            content: { type: 'text', body: 'Can you send the check-in code?' },
          },
        ],
        vendor: [
          {
            id: 'vendor-1',
            isFromMe: true,
            timestamp: now - 2_000,
            content: { type: 'text', body: 'Invoice paid, thanks' },
          },
        ],
      },
      now,
    }).map((contact) => contact.id),
    ['host'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { needsReplyOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
      messagesByContact: {
        host: [
          {
            id: 'host-1',
            isFromMe: false,
            timestamp: now - 1_000,
            content: { type: 'text', body: 'Can you send the check-in code?' },
          },
        ],
        vendor: [
          {
            id: 'vendor-1',
            isFromMe: true,
            timestamp: now - 2_000,
            content: { type: 'text', body: 'Invoice paid, thanks' },
          },
        ],
      },
      now,
    }).map((contact) => contact.id),
    ['host'],
  );
});

test('getVisibleContacts supports important-only inbox triage and priority-aware search', () => {
  const contacts = [
    {
      id: 'host',
      name: 'Airport host',
      phone: '111',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 20,
    },
    {
      id: 'friend',
      name: 'Dinner plans',
      phone: '222',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 10,
    },
  ];

  const metadataByContact = {
    host: { priority: 'urgent' },
    friend: { priority: 'low' },
  };

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { importantOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['host'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: 'urgent airport',
      filters: {},
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['host'],
  );
});

test('getVisibleContacts supports open-task inbox triage and checklist search', () => {
  const contacts = [
    {
      id: 'trip',
      name: 'Trip planner',
      phone: '111',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 20,
    },
    {
      id: 'done',
      name: 'Handled',
      phone: '222',
      type: 'private',
      unreadCount: 0,
      lastMessageTime: 10,
    },
  ];

  const metadataByContact = {
    trip: {
      checklist: [
        { id: 'a', text: 'Send passport details', done: false },
        { id: 'b', text: 'Confirm airport pickup', done: true },
      ],
    },
    done: {
      checklist: [{ id: 'c', text: 'Archive receipt', done: true }],
    },
  };

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: '',
      filters: { tasksOnly: true },
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['trip'],
  );

  assert.deepEqual(
    getVisibleContacts({
      contacts,
      drafts: {},
      searchQuery: 'passport details',
      filters: {},
      messagePreviewByContact: {},
      metadataByContact,
    }).map((contact) => contact.id),
    ['trip'],
  );
});

test('getContactActionSnapshot highlights the most urgent visitor context first', () => {
  const now = 1_710_000_000_000;
  const contact = {
    id: 'vip',
    name: 'VIP Client',
    unreadCount: 2,
    lastMessageTime: now - 5_000,
  };

  const snapshot = getContactActionSnapshot({
    contact,
    drafts: {
      vip: { text: 'Need to send the contract recap', updatedAt: now - 2_000 },
    },
    metadata: {
      priority: 'urgent',
      reminderText: 'Send the contract recap',
      reminderAt: now - 60_000,
      checklist: [{ id: 'task-1', text: 'Share contract recap', done: false }],
      labels: ['VIP'],
    },
    messages: [
      {
        id: 'm1',
        timestamp: now - 5_000,
        isFromMe: false,
        content: { type: 'text', body: 'Can you send the recap?' },
      },
    ],
    now,
  });

  assert.equal(snapshot.headline, 'Reminder due');
  assert.equal(snapshot.replyState, 'drafting');
  assert.equal(snapshot.checklist.open, 1);
  assert.equal(snapshot.priority.value, 'urgent');
  assert.match(snapshot.summary, /Reminder due/);
  assert.ok(snapshot.attentionScore > 150);
});

test('buildVisitorDashboard summarizes focus work, reminders, drafts, and open tasks', () => {
  const now = 1_710_000_000_000;
  const contacts = [
    {
      id: 'vip',
      name: 'VIP Client',
      unreadCount: 1,
      lastMessageTime: now - 1_000,
    },
    {
      id: 'trip',
      name: 'Trip Planner',
      unreadCount: 0,
      lastMessageTime: now - 10_000,
    },
    {
      id: 'later',
      name: 'Later',
      unreadCount: 0,
      lastMessageTime: now - 20_000,
    },
  ];

  const dashboard = buildVisitorDashboard({
    contacts,
    drafts: {
      vip: { text: 'Need to send the contract recap', updatedAt: now - 2_000 },
    },
    metadataByContact: {
      vip: {
        priority: 'urgent',
        reminderText: 'Send the contract recap',
        reminderAt: now - 60_000,
        checklist: [
          { id: 'task-1', text: 'Share contract recap', done: false },
        ],
      },
      trip: {
        reminderText: 'Share the itinerary',
        reminderAt: now + 3_600_000,
        checklist: [
          { id: 'task-2', text: 'Book the taxi', done: false },
          { id: 'task-3', text: 'Save passport copy', done: true },
        ],
      },
      later: {
        snoozedUntil: now + 86_400_000,
        checklist: [
          { id: 'task-4', text: 'Ignore while snoozed', done: false },
        ],
      },
    },
    messagesByContact: {
      vip: [
        {
          id: 'vip-1',
          timestamp: now - 1_000,
          isFromMe: false,
          content: { type: 'text', body: 'Can you send the recap?' },
        },
      ],
      trip: [
        {
          id: 'trip-1',
          timestamp: now - 10_000,
          isFromMe: true,
          content: { type: 'text', body: 'Working on the itinerary now.' },
        },
      ],
    },
    now,
  });

  assert.deepEqual(dashboard.stats, {
    needsReply: 1,
    dueReminders: 1,
    drafts: 1,
    openTasks: 2,
  });
  assert.equal(dashboard.focusContacts[0].contact.id, 'vip');
  assert.deepEqual(
    dashboard.upcomingReminders.map((entry) => entry.contact.id),
    ['trip'],
  );
  assert.equal(dashboard.snoozedContacts, 1);
});
