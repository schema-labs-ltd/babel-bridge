import test from 'node:test';
import assert from 'node:assert/strict';
import { chatGptStatusText } from '../public/chatgpt-signin.js';

test('signed-out status says which credential text AI uses', () => {
  assert.match(
    chatGptStatusText({
      available: true,
      signedIn: false,
      apiKeyConfigured: true,
    }),
    /API key/,
  );
  assert.match(
    chatGptStatusText({
      available: true,
      signedIn: false,
      apiKeyConfigured: false,
    }),
    /Sign in/,
  );
  assert.match(chatGptStatusText({ available: false }), /isn’t available/);
});

test('signed-in status names the account and the voice note limit', () => {
  const withKey = chatGptStatusText({
    available: true,
    signedIn: true,
    email: 'a@example.com',
    apiKeyConfigured: true,
  });
  assert.match(withKey, /a@example\.com/);
  assert.match(withKey, /still use the server’s API key/);
  const withoutKey = chatGptStatusText({
    available: true,
    signedIn: true,
    apiKeyConfigured: false,
  });
  assert.match(withoutKey, /need an OpenAI API key/);
});

test('choosing a ChatGPT model saves the chosen model, not the previous one', async () => {
  const elements = new Map();
  const element = (id) => {
    if (!elements.has(id)) {
      const listeners = {};
      elements.set(id, {
        id,
        hidden: false,
        disabled: false,
        value: '',
        textContent: '',
        children: [],
        addEventListener: (type, fn) => {
          listeners[type] = fn;
        },
        dispatch: (type, event) => listeners[type]?.(event),
        replaceChildren(...children) {
          this.children = children;
        },
      });
    }
    return elements.get(id);
  };
  globalThis.document = {
    getElementById: element,
    createElement: () => ({ value: '', textContent: '' }),
  };
  const models = [
    { slug: 'gpt-6.1-sol', displayName: 'Sol' },
    { slug: 'gpt-6.1-luna', displayName: 'Luna' },
  ];
  let saved = 'gpt-6.1-sol';
  const sent = [];
  const app = {
    apiFetch: async (path, options = {}) => {
      if (options.method === 'PUT') {
        const { model } = JSON.parse(options.body);
        sent.push(model);
        saved = model;
      }
      const body = { available: true, signedIn: true, model: saved, models };
      return { ok: true, json: async () => body };
    },
  };
  try {
    const { setupChatGptSignIn } = await import('../public/chatgpt-signin.js');
    const chatGpt = setupChatGptSignIn(app);
    await chatGpt.refresh();
    const select = element('chatgpt-model-select');
    assert.equal(select.value, 'gpt-6.1-sol');
    select.value = 'gpt-6.1-luna';
    select.dispatch('change', { target: select });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.deepEqual(sent, ['gpt-6.1-luna']);
    assert.equal(select.value, 'gpt-6.1-luna');
  } finally {
    delete globalThis.document;
  }
});
