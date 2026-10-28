// index.html keeps the larger parts of its markup in markup/*.js so that
// no file grows past the repository's size limit. This module puts each
// part in place of its <template data-markup="name"> placeholder. It is
// loaded as a module just before app.js, and module scripts run in order,
// so the elements exist by the time the app starts.
import appearanceModal from './markup/appearance-modal.js?v=20261026-split';
import chatHeader from './markup/chat-header.js?v=20261026-split';
import chatShell from './markup/chat-shell.js?v=20261026-split';
import settingsModal from './markup/settings-modal.js?v=20261026-split';

const parts = {
  'appearance-modal': appearanceModal,
  'chat-header': chatHeader,
  'chat-shell': chatShell,
  'settings-modal': settingsModal,
};

for (const slot of document.querySelectorAll('template[data-markup]')) {
  slot.outerHTML = parts[slot.dataset.markup];
}
