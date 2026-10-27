// The open conversation's header: contact details, search, translation and menu controls.
// markup.js puts it in place of its <template> in index.html.
export default `
            <header class="chat-header">
              <button
                id="back-button"
                class="back-button"
                aria-label="Back to contacts"
              >
                <svg viewBox="0 0 24 24" width="24" height="24">
                  <path
                    fill="currentColor"
                    d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"
                  />
                </svg>
              </button>
              <div class="avatar">
                <span id="chat-avatar-initial">?</span>
              </div>
              <div class="chat-info">
                <span id="chat-name" class="chat-name">Contact Name</span>
                <span
                  id="chat-note-indicator"
                  class="chat-note-indicator hidden"
                ></span>
                <span id="typing-indicator" class="typing-indicator hidden"
                  >typing...</span
                >
              </div>
              <div class="chat-actions">
                <button
                  id="chat-workspace-toggle"
                  class="chat-settings-button chat-workspace-button"
                  type="button"
                  title="Toggle workspace"
                  aria-label="Toggle workspace"
                >
                  <svg viewBox="0 0 24 24" width="20" height="20">
                    <path
                      fill="currentColor"
                      d="M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5zm6 0H5v14h4V5zm2 14h8V5h-8v14z"
                    />
                  </svg>
                </button>
                <span class="cost-badge chat-cost-badge" data-chat-cost
                  >$0.00</span
                >
                <button
                  id="chat-menu-button"
                  class="chat-settings-button chat-menu-button"
                  type="button"
                  title="Conversation actions"
                  aria-label="Conversation actions"
                  aria-haspopup="menu"
                  aria-expanded="false"
                >
                  <svg viewBox="0 0 24 24" width="20" height="20">
                    <path
                      fill="currentColor"
                      d="M12 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm0 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm0 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4z"
                    />
                  </svg>
                </button>
                <div
                  id="chat-actions-menu"
                  class="chat-actions-menu hidden"
                  role="menu"
                  aria-label="Conversation actions"
                >
                  <button
                    id="chat-search-toggle"
                    class="chat-menu-item"
                    type="button"
                    role="menuitem"
                  >
                    Search messages
                  </button>
                  <button
                    id="chat-starred-toggle"
                    class="chat-menu-item"
                    type="button"
                    role="menuitem"
                  >
                    Show starred messages only
                  </button>
                  <button
                    id="chat-translate-all"
                    class="chat-menu-item"
                    type="button"
                    role="menuitem"
                  >
                    Translate incoming messages
                  </button>
                  <button
                    id="chat-settings-button"
                    class="chat-menu-item"
                    type="button"
                    role="menuitem"
                  >
                    Conversation settings
                  </button>
                  <div class="chat-menu-divider" aria-hidden="true"></div>
                  <div class="chat-menu-meta">
                    <span class="chat-menu-meta-label">Conversation cost</span>
                    <span class="chat-menu-cost" data-chat-cost>$0.00</span>
                  </div>
                </div>
              </div>
            </header>
`;
