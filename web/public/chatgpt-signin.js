// Sign in with ChatGPT: text AI draws on the user's ChatGPT plan.
// ChatGPT redirects to 127.0.0.1, which reaches this browser's computer rather
// than the server, so the user pastes that address back to finish sign-in.

export function chatGptStatusText(status) {
  if (!status?.available) return 'Sign in with ChatGPT isn’t available on this server.';
  if (!status.signedIn) {
    return status.apiKeyConfigured
      ? 'Not signed in. Text AI uses the server’s OpenAI API key.'
      : 'Not signed in. Sign in to use your ChatGPT plan for translation and AI replies.';
  }
  const who = status.email || status.name || 'your ChatGPT account';
  const voice = status.apiKeyConfigured
    ? 'Voice notes still use the server’s API key.'
    : 'Voice notes need an OpenAI API key on the server.';
  return `Signed in as ${who}. Translation, AI replies and topics use your ChatGPT plan. ${voice}`;
}

export function setupChatGptSignIn(app) {
  const $ = id => document.getElementById(id);
  let status = null;
  let busy = false;

  async function request(path, options = {}) {
    const response = await app.apiFetch(path, {
      headers: {'Content-Type': 'application/json'},
      ...options,
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
    return body;
  }

  function setError(message) {
    const node = $('chatgpt-error');
    if (!node) return;
    node.textContent = message || '';
    node.hidden = !message;
  }

  function render() {
    const section = $('chatgpt-section');
    if (!section) return;
    section.hidden = !status?.available;
    $('chatgpt-status').textContent = chatGptStatusText(status);
    const signedIn = Boolean(status?.signedIn);
    $('chatgpt-sign-in').hidden = signedIn;
    $('chatgpt-signed-in').hidden = !signedIn;
    $('chatgpt-sign-in').disabled = busy;
    $('chatgpt-sign-out').disabled = busy;
    if (signedIn) $('chatgpt-callback-step').hidden = true;
    const select = $('chatgpt-model-select');
    select.replaceChildren(...(status?.models || []).map(model => {
      const option = document.createElement('option');
      option.value = model.slug;
      option.textContent = model.displayName;
      return option;
    }));
    select.disabled = busy || !status?.models?.length;
    if (status?.model) select.value = status.model;
  }

  async function refresh() {
    try {
      status = await request('/api/chatgpt');
    } catch (error) {
      status = null;
      console.error('Failed to load ChatGPT sign-in status:', error);
    }
    render();
  }

  async function run(action) {
    busy = true;
    setError('');
    render();
    try {
      await action();
    } catch (error) {
      setError(error.message);
    } finally {
      busy = false;
      render();
    }
  }

  $('chatgpt-sign-in')?.addEventListener('click', () => {
    // Open the tab during the click so popup blockers allow it.
    const tab = window.open('about:blank', '_blank');
    run(async () => {
      try {
        const {authorizeUrl} = await request('/api/chatgpt/sign-in', {method: 'POST', body: '{}'});
        if (tab) tab.location.href = authorizeUrl;
        else window.open(authorizeUrl, '_blank', 'noopener');
        $('chatgpt-callback-url').value = '';
        $('chatgpt-callback-step').hidden = false;
      } catch (error) {
        tab?.close();
        throw error;
      }
    });
  });

  $('chatgpt-finish')?.addEventListener('click', () => run(async () => {
    const callbackUrl = $('chatgpt-callback-url').value.trim();
    if (!callbackUrl) throw new Error('Paste the address your browser showed after you approved.');
    status = await request('/api/chatgpt/callback', {method: 'POST', body: JSON.stringify({callbackUrl})});
  }));

  $('chatgpt-model-select')?.addEventListener('change', event => run(async () => {
    status = await request('/api/chatgpt/model', {method: 'PUT', body: JSON.stringify({model: event.target.value})});
  }));

  $('chatgpt-sign-out')?.addEventListener('click', () => run(async () => {
    status = await request('/api/chatgpt/sign-out', {method: 'POST', body: '{}'});
    if (!status.revocationConfirmed) {
      setError('Signed out here, but ChatGPT didn’t confirm it. You can disconnect Babel Bridge in ChatGPT Settings.');
    }
  }));

  return {refresh};
}
