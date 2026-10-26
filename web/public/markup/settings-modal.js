// The conversation settings dialog.
// markup.js puts it in place of its <template> in index.html.
export default `
      <div id="settings-modal" class="modal hidden">
        <div class="modal-backdrop"></div>
        <div class="modal-content settings-modal-content">
          <div class="modal-header">
            <h3>Chat settings</h3>
            <button class="modal-close" aria-label="Close">
              <svg viewBox="0 0 24 24" width="20" height="20">
                <path
                  fill="currentColor"
                  d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
                />
              </svg>
            </button>
          </div>
          <div class="modal-body settings-modal-body">
            <section
              class="settings-section"
              aria-label="Message notifications"
            >
              <span class="settings-section-label">Notifications</span>
              <button
                type="button"
                id="conversation-message-tone"
                class="message-tone-setting"
              >
                <span>Message ringtone</span
                ><span id="conversation-message-tone-summary">Choose…</span
                ><span aria-hidden="true">›</span>
              </button>
            </section>
            <section
              class="settings-section"
              aria-label="Chat display settings"
            >
              <span class="settings-section-label">Display</span>
              <div class="form-group">
                <label for="contact-alias">Nickname</label>
                <input
                  type="text"
                  id="contact-alias"
                  placeholder="e.g., Madrid Airbnb host"
                />
                <p class="form-hint">Only changes the name in this app.</p>
              </div>
              <div class="form-group">
                <label for="conversation-timezone">Contact timezone</label>
                <input
                  type="text"
                  id="conversation-timezone"
                  placeholder="e.g., Europe/Madrid"
                />
                <p class="form-hint">Used to show their local time.</p>
              </div>
            </section>
            <section class="settings-section" aria-label="Sending settings">
              <span class="settings-section-label">Sending</span>
              <label class="checkbox-row" for="reply-only">
                <input
                  type="checkbox"
                  id="reply-only"
                  aria-describedby="reply-only-description"
                />
                <span>Reply only</span>
              </label>
              <p class="form-hint" id="reply-only-description">
                Choose an incoming message and tap Reply before sending. Applies
                on every connected device.
              </p>
            </section>
            <section class="settings-section" aria-label="Translation settings">
              <span class="settings-section-label">Translation</span>
              <div class="form-group">
                <label class="checkbox-row" for="translation-enabled">
                  <input
                    type="checkbox"
                    id="translation-enabled"
                    aria-describedby="translation-enabled-hint"
                  />
                  <span>Translate messages</span>
                </label>
                <p class="form-hint" id="translation-enabled-hint">
                  Off by default for every person and group.
                </p>
              </div>
              <div class="form-group">
                <label for="language-override">Translate this chat to</label>
                <input
                  type="text"
                  id="language-override"
                  placeholder="Default: English"
                />
                <p class="form-hint">
                  Set a different target language for this chat.
                </p>
              </div>
              <div class="form-group">
                <label for="translation-style">Translation style</label>
                <input
                  type="text"
                  id="translation-style"
                  placeholder="e.g., informal, formal, family"
                />
                <p class="form-hint">
                  Optional tone instruction for translations.
                </p>
              </div>
              <div class="form-group">
                <label class="checkbox-row" for="send-original-follow-up">
                  <input type="checkbox" id="send-original-follow-up" />
                  <span>Send my original message too</span>
                </label>
                <p class="form-hint">
                  Sends the translation first, then your original text as a
                  separate follow-up.
                </p>
              </div>
            </section>
          </div>
          <div class="modal-footer">
            <button class="modal-button secondary" id="settings-cancel">
              Cancel
            </button>
            <button class="modal-button primary" id="settings-save">
              Save
            </button>
          </div>
        </div>
      </div>

`;
