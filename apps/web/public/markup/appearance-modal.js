// The global settings dialog: appearance, ChatGPT sign-in and workspace backup.
// markup.js puts it in place of its <template> in index.html.
export default `
      <div id="appearance-modal" class="modal hidden">
        <div class="modal-backdrop"></div>
        <div class="modal-content appearance-modal-content">
          <div class="modal-header">
            <h3>Global settings</h3>
            <button class="modal-close" aria-label="Close global settings">
              <svg viewBox="0 0 24 24" width="20" height="20">
                <path
                  fill="currentColor"
                  d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
                />
              </svg>
            </button>
          </div>
          <div class="modal-body">
            <section
              class="settings-section"
              aria-label="Message notifications"
            >
              <span class="settings-section-label">Notifications</span>
              <button
                type="button"
                id="global-message-tone"
                class="message-tone-setting"
              >
                <span>Message ringtone</span
                ><span id="global-message-tone-summary">Choose…</span
                ><span aria-hidden="true">›</span>
              </button>
            </section>
            <div class="settings-section">
              <span class="settings-section-label">OpenAI</span>
              <div class="form-group">
                <label for="openai-model-select">GPT model</label>
                <select id="openai-model-select">
                  <option value="">App defaults</option>
                  <option value="gpt-6-astra">GPT-6 Astra</option>
                  <option value="gpt-5.6-sol">
                    GPT-5.6 Sol · most capable
                  </option>
                  <option value="gpt-5.6-terra">
                    GPT-5.6 Terra · balanced
                  </option>
                  <option value="gpt-5.6-luna">
                    GPT-5.6 Luna · fastest value
                  </option>
                </select>
                <p class="form-hint">
                  Overrides the model for translation, detection, compose, and
                  AI replies.
                </p>
              </div>
              <div class="form-group">
                <label for="openai-reasoning-select">Reasoning level</label>
                <select id="openai-reasoning-select">
                  <option value="">Task default</option>
                  <option value="none">None</option>
                  <option value="low">Low</option>
                  <option value="medium">Medium</option>
                  <option value="high">High</option>
                  <option value="xhigh">Extra high</option>
                  <option value="max">Maximum</option>
                </select>
                <p class="form-hint">
                  Higher levels can improve difficult replies but take longer
                  and use more output tokens.
                </p>
              </div>
            </div>
            <div class="settings-section" id="chatgpt-section" hidden>
              <span class="settings-section-label">ChatGPT account</span>
              <p class="form-hint" id="chatgpt-status" role="status"></p>
              <button type="button" class="modal-button" id="chatgpt-sign-in">
                Sign in with ChatGPT
              </button>
              <div id="chatgpt-callback-step" class="form-group" hidden>
                <label for="chatgpt-callback-url">Finish sign-in</label>
                <p class="form-hint">
                  After you approve in the new tab, it opens an address starting
                  with http://127.0.0.1 that won’t load. Copy that whole address
                  and paste it here.
                </p>
                <input
                  type="url"
                  id="chatgpt-callback-url"
                  placeholder="http://127.0.0.1:1455/auth/callback?code=…"
                  autocomplete="off"
                  spellcheck="false"
                />
                <button type="button" class="modal-button" id="chatgpt-finish">
                  Finish sign-in
                </button>
              </div>
              <div id="chatgpt-signed-in" hidden>
                <div class="form-group">
                  <label for="chatgpt-model-select">ChatGPT model</label>
                  <select id="chatgpt-model-select"></select>
                  <p class="form-hint">
                    Used instead of the GPT model above while you’re signed in.
                    Usage counts toward your ChatGPT plan limits.
                  </p>
                </div>
                <button
                  type="button"
                  class="modal-button secondary"
                  id="chatgpt-sign-out"
                >
                  Sign out of ChatGPT
                </button>
              </div>
              <p class="form-hint" id="chatgpt-error" role="alert" hidden></p>
            </div>
            <div class="settings-section">
              <span class="settings-section-label">Appearance</span>
              <div class="form-group">
                <label for="appearance-theme-select">Theme</label>
                <select id="appearance-theme-select">
                  <option value="whatsapp">WhatsApp</option>
                  <option value="ocean">Ocean</option>
                  <option value="sunset">Sunset</option>
                  <option value="github">GitHub</option>
                  <option value="dracula">Dracula</option>
                  <option value="nord">Nord</option>
                  <option value="linear">Linear</option>
                  <option value="vercel">Vercel</option>
                </select>
                <p class="form-hint">
                  Every theme includes both light and dark variants, and each
                  palette is tuned to keep text readable on both the page and
                  elevated surfaces.
                </p>
              </div>
              <div class="form-group">
                <label for="appearance-mode-select">Mode</label>
                <select id="appearance-mode-select">
                  <option value="system">Follow system</option>
                  <option value="light">Always light</option>
                  <option value="dark">Always dark</option>
                </select>
                <p class="form-hint">
                  Choose whether the app should always stay light, always stay
                  dark, or switch with your device.
                </p>
              </div>
              <div class="appearance-preview-card">
                <span class="appearance-preview-label">Preview</span>
                <strong id="appearance-preview-text"
                  >WhatsApp · Dark variant (following system)</strong
                >
              </div>
            </div>
            <div class="workspace-data-section">
              <span class="workspace-data-label">Workspace data</span>
              <p class="form-hint">
                Drafts, stars, notes, labels, reminders, filters, quick replies,
                recents, and theme. Sign-in stays on this browser.
              </p>
              <div class="workspace-data-actions">
                <button
                  class="modal-button secondary"
                  id="workspace-export-button"
                  type="button"
                >
                  Export workspace
                </button>
                <button
                  class="modal-button secondary"
                  id="workspace-import-button"
                  type="button"
                >
                  Import workspace
                </button>
                <input
                  id="workspace-import-input"
                  type="file"
                  accept="application/json"
                  hidden
                />
              </div>
            </div>
          </div>
          <div class="modal-footer">
            <button class="modal-button secondary" id="appearance-cancel">
              Cancel
            </button>
            <button class="modal-button primary" id="appearance-save">
              Apply
            </button>
          </div>
        </div>
      </div>

`;
