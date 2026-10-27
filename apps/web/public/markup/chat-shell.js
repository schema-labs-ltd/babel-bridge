// The open conversation's body: search bar, messages, composer and the context workspace.
// markup.js puts it in place of its <template> in index.html.
export default `
            <div class="chat-shell">
              <div class="chat-main-column">
                <div id="chat-search-bar" class="chat-search-bar hidden">
                  <input
                    id="chat-search-input"
                    type="search"
                    placeholder="Search messages in this conversation"
                  />
                  <span id="chat-search-count" class="chat-search-count"
                    >0 matches</span
                  >
                  <button
                    id="chat-search-clear"
                    class="chat-search-clear"
                    title="Clear search"
                  >
                    Clear
                  </button>
                </div>

                <div id="messages-list" class="messages-list">
                  <!-- Messages will be inserted here -->
                </div>

                <footer class="message-input-area">
                  <p
                    id="reply-only-hint"
                    class="form-hint"
                    role="status"
                    hidden
                  >
                    Reply only · Choose an incoming message and tap Reply before
                    sending.
                  </p>
                  <div id="reply-preview" class="reply-preview hidden">
                    <div class="reply-preview-content">
                      <span class="reply-preview-sender">Name</span>
                      <span class="reply-preview-text">Message text...</span>
                    </div>
                    <button
                      id="reply-preview-close"
                      class="reply-preview-close"
                      aria-label="Cancel reply"
                    >
                      <svg viewBox="0 0 24 24" width="20" height="20">
                        <path
                          fill="currentColor"
                          d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
                        />
                      </svg>
                    </button>
                  </div>
                  <div id="composer-assist" class="composer-assist hidden">
                    <div class="composer-assist-main">
                      <span
                        id="composer-assist-kicker"
                        class="composer-assist-kicker"
                        >Translation route</span
                      >
                      <strong id="composer-assist-title"
                        >Spanish · standard tone</strong
                      >
                      <span
                        id="composer-assist-preview"
                        class="composer-assist-preview"
                        >Type a message to preview the translation route.</span
                      >
                      <div
                        id="composer-profile-presets"
                        class="composer-profile-presets"
                      ></div>
                      <div
                        id="composer-smart-replies"
                        class="composer-smart-replies"
                      ></div>
                      <div
                        id="composer-readiness"
                        class="composer-readiness"
                        aria-live="polite"
                      ></div>
                    </div>
                    <div class="composer-assist-actions">
                      <button
                        id="composer-use-suggestion"
                        class="composer-assist-button"
                        type="button"
                      >
                        Use smart reply
                      </button>
                      <button
                        id="composer-remind-tomorrow"
                        class="composer-assist-button"
                        type="button"
                      >
                        Remind tomorrow
                      </button>
                      <div
                        id="composer-reminder-presets"
                        class="composer-reminder-presets"
                      ></div>
                      <button
                        id="composer-edit-profile"
                        class="composer-assist-button subtle"
                        type="button"
                      >
                        Edit profile
                      </button>
                    </div>
                  </div>
                  <div class="quick-replies-bar">
                    <div class="quick-replies-header">
                      <span class="quick-replies-label">Quick replies</span>
                      <button
                        id="quick-reply-save"
                        class="quick-reply-save"
                        type="button"
                      >
                        Save draft
                      </button>
                    </div>
                    <div
                      id="quick-replies-list"
                      class="quick-replies-list"
                    ></div>
                  </div>
                  <div class="input-container">
                    <input
                      type="file"
                      id="image-input"
                      accept="image/*"
                      style="display: none"
                    />
                    <div class="emoji-button-container">
                      <button
                        id="emoji-button"
                        class="emoji-button"
                        title="Insert emoji"
                      >
                        <svg viewBox="0 0 24 24" width="24" height="24">
                          <path
                            fill="currentColor"
                            d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm-5-6c.78 2.34 2.72 4 5 4s4.22-1.66 5-4H7zm2-3c.55 0 1-.45 1-1s-.45-1-1-1-1 .45-1 1 .45 1 1 1zm6 0c.55 0 1-.45 1-1s-.45-1-1-1-1 .45-1 1 .45 1 1 1z"
                          />
                        </svg>
                      </button>
                      <div id="emoji-picker" class="emoji-picker hidden">
                        <div class="emoji-picker-header">
                          <div class="emoji-tabs">
                            <button
                              class="emoji-tab active"
                              data-category="recent"
                              title="Recent"
                            >
                              🕐
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="smileys"
                              title="Smileys"
                            >
                              😀
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="gestures"
                              title="Gestures"
                            >
                              👋
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="hearts"
                              title="Hearts"
                            >
                              ❤️
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="animals"
                              title="Animals"
                            >
                              🐶
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="food"
                              title="Food"
                            >
                              🍕
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="travel"
                              title="Travel"
                            >
                              ✈️
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="objects"
                              title="Objects"
                            >
                              💡
                            </button>
                            <button
                              class="emoji-tab"
                              data-category="symbols"
                              title="Symbols"
                            >
                              ✨
                            </button>
                          </div>
                        </div>
                        <div
                          class="emoji-picker-content"
                          id="emoji-picker-content"
                        >
                          <!-- Emojis loaded by JavaScript -->
                        </div>
                      </div>
                    </div>
                    <button
                      id="attach-button"
                      class="attach-button"
                      title="Send image"
                    >
                      <svg viewBox="0 0 24 24" width="24" height="24">
                        <path
                          fill="currentColor"
                          d="M16.5 6.5v8.25a4.25 4.25 0 1 1-8.5 0V5.75a2.75 2.75 0 1 1 5.5 0v8.5a1.25 1.25 0 1 1-2.5 0V7h-1.5v7.25a2.75 2.75 0 0 0 5.5 0v-8.5a4.25 4.25 0 1 0-8.5 0v9a5.75 5.75 0 1 0 11.5 0V6.5z"
                        />
                      </svg>
                    </button>
                    <textarea
                      id="message-input"
                      placeholder="Type a message"
                      rows="1"
                      maxlength="65536"
                    ></textarea>
                    <div class="send-button-group">
                      <button
                        id="send-button"
                        class="send-button"
                        disabled
                        title="Send message"
                      >
                        <svg viewBox="0 0 24 24" width="24" height="24">
                          <path
                            fill="currentColor"
                            d="M1.101 21.757L23.8 12.028 1.101 2.3l.011 7.912 13.623 1.816-13.623 1.817-.011 7.912z"
                          />
                        </svg>
                      </button>
                      <button
                        id="send-dropdown-toggle"
                        class="send-dropdown-toggle"
                        disabled
                        title="More send options"
                      >
                        <svg viewBox="0 0 24 24" width="12" height="12">
                          <path fill="currentColor" d="M7 10l5 5 5-5z" />
                        </svg>
                      </button>
                      <div id="send-dropdown" class="send-dropdown hidden">
                        <button id="send-ai-button" class="send-dropdown-item">
                          <svg viewBox="0 0 24 24" width="16" height="16">
                            <path
                              fill="currentColor"
                              d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z"
                            />
                          </svg>
                          <span>Send with AI</span>
                        </button>
                      </div>
                    </div>
                  </div>
                </footer>
              </div>

              <aside id="conversation-workspace" class="conversation-workspace">
                <div class="conversation-workspace-frame">
                  <div class="workspace-header">
                    <div>
                      <span class="workspace-kicker">Workspace</span>
                      <h3>Conversation context</h3>
                    </div>
                    <button
                      id="workspace-settings-button"
                      class="workspace-edit-button"
                      type="button"
                    >
                      Edit
                    </button>
                  </div>
                  <div
                    id="conversation-workspace-summary"
                    class="workspace-summary"
                  ></div>
                  <div id="workspace-brief" class="workspace-brief"></div>
                  <div class="workspace-actions">
                    <button
                      id="workspace-copy-brief"
                      class="workspace-action-button"
                      type="button"
                    >
                      Copy brief
                    </button>
                    <button
                      id="workspace-translate-all"
                      class="workspace-action-button"
                      type="button"
                    >
                      Translate incoming
                    </button>
                  </div>
                  <div
                    id="workspace-action-plan"
                    class="workspace-action-plan"
                  ></div>
                  <div class="workspace-section">
                    <span class="workspace-section-label">Right now</span>
                    <p
                      id="workspace-primary-status"
                      class="workspace-primary-status"
                    >
                      No local context yet.
                    </p>
                  </div>
                  <div class="workspace-section">
                    <span class="workspace-section-label">Notes</span>
                    <p id="workspace-notes" class="workspace-notes">
                      No notes yet.
                    </p>
                  </div>
                  <div class="workspace-section">
                    <div class="workspace-section-heading">
                      <span class="workspace-section-label">Checklist</span>
                      <span
                        id="workspace-checklist-summary"
                        class="workspace-section-meta"
                        >No tasks</span
                      >
                    </div>
                    <div
                      id="workspace-checklist"
                      class="workspace-checklist"
                    ></div>
                  </div>
                  <div class="workspace-section">
                    <div class="workspace-section-heading">
                      <span class="workspace-section-label">Labels</span>
                      <span
                        id="workspace-timezone"
                        class="workspace-section-meta"
                      ></span>
                    </div>
                    <div id="workspace-labels" class="workspace-labels"></div>
                  </div>
                  <div class="workspace-section">
                    <span class="workspace-section-label">Reminder</span>
                    <p id="workspace-reminder" class="workspace-reminder">
                      No reminder set.
                    </p>
                  </div>
                </div>
              </aside>
            </div>
`;
