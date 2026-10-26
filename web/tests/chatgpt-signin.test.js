import test from 'node:test';
import assert from 'node:assert/strict';
import { chatGptStatusText } from '../public/chatgpt-signin.js';

test('signed-out status says which credential text AI uses', () => {
  assert.match(chatGptStatusText({available: true, signedIn: false, apiKeyConfigured: true}), /API key/);
  assert.match(chatGptStatusText({available: true, signedIn: false, apiKeyConfigured: false}), /Sign in/);
  assert.match(chatGptStatusText({available: false}), /isn’t available/);
});

test('signed-in status names the account and the voice note limit', () => {
  const withKey = chatGptStatusText({available: true, signedIn: true, email: 'a@example.com', apiKeyConfigured: true});
  assert.match(withKey, /a@example\.com/);
  assert.match(withKey, /still use the server’s API key/);
  const withoutKey = chatGptStatusText({available: true, signedIn: true, apiKeyConfigured: false});
  assert.match(withoutKey, /need an OpenAI API key/);
});
