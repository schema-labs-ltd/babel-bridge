// Text, time, size and cost formatting, usage display and link preview fetching.

export const formattingMethods = {
  // Utility functions
  escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  },

  formatTime(timestamp) {
    if (!timestamp) return '';
    const date = new Date(timestamp);
    const now = new Date();
    const diff = now - date;

    // Today
    if (date.toDateString() === now.toDateString()) {
      return date.toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit',
      });
    }

    // Yesterday
    const yesterday = new Date(now);
    yesterday.setDate(yesterday.getDate() - 1);
    if (date.toDateString() === yesterday.toDateString()) {
      return 'Yesterday';
    }

    // This week
    if (diff < 7 * 24 * 60 * 60 * 1000) {
      return date.toLocaleDateString([], { weekday: 'short' });
    }

    // Older
    return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
  },

  formatMetadataTime(timestamp) {
    if (!timestamp) return '';
    const date = new Date(timestamp);
    const now = new Date();
    const diff = timestamp - now.getTime();
    const absDiff = Math.abs(diff);

    if (date.toDateString() === now.toDateString()) {
      return `today at ${date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`;
    }

    const tomorrow = new Date(now);
    tomorrow.setDate(tomorrow.getDate() + 1);
    if (date.toDateString() === tomorrow.toDateString()) {
      return `tomorrow at ${date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`;
    }

    if (absDiff < 7 * 24 * 60 * 60 * 1000) {
      return date.toLocaleString([], {
        weekday: 'short',
        hour: 'numeric',
        minute: '2-digit',
      });
    }

    return date.toLocaleString([], {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    });
  },

  toDateTimeLocalValue(timestamp) {
    if (!timestamp) return '';
    const date = new Date(timestamp);
    const offset = date.getTimezoneOffset();
    const localDate = new Date(date.getTime() - offset * 60_000);
    return localDate.toISOString().slice(0, 16);
  },

  parseDateTimeLocalValue(value) {
    if (!value) return null;
    const timestamp = new Date(value).getTime();
    return Number.isFinite(timestamp) ? timestamp : null;
  },

  formatDate(timestamp) {
    const date = new Date(timestamp);
    const now = new Date();

    if (date.toDateString() === now.toDateString()) {
      return 'Today';
    }

    const yesterday = new Date(now);
    yesterday.setDate(yesterday.getDate() - 1);
    if (date.toDateString() === yesterday.toDateString()) {
      return 'Yesterday';
    }

    return date.toLocaleDateString([], {
      weekday: 'long',
      month: 'long',
      day: 'numeric',
      year: date.getFullYear() !== now.getFullYear() ? 'numeric' : undefined,
    });
  },

  formatMessageTime(timestamp) {
    return new Date(timestamp).toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
    });
  },

  formatSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  },

  formatDuration(seconds) {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  },

  // Format cost for display
  formatCost(costUsd) {
    if (costUsd < 0.01) {
      return '$' + costUsd.toFixed(4);
    }
    return '$' + costUsd.toFixed(2);
  },

  // Fetch and display global usage
  async fetchGlobalUsage() {
    if (this.demoMode) {
      this.updateGlobalUsageDisplay();
      return;
    }

    try {
      const response = await this.apiFetch('/api/usage');
      const usage = await response.json();
      this.globalUsage = usage;
      this.updateGlobalUsageDisplay();
    } catch (err) {
      console.error('Failed to fetch global usage:', err);
    }
  },

  // Update global usage display in sidebar
  updateGlobalUsageDisplay() {
    const costEl = document.getElementById('global-cost');
    if (costEl) {
      costEl.textContent = this.formatCost(this.globalUsage.costUsd || 0);
    }
  },

  // Fetch and display conversation usage
  async fetchConversationUsage(contactId) {
    if (this.demoMode) {
      const messages = this.messages.get(contactId) || [];
      const translatedCount = messages.filter(
        (message) => message.isTranslated || message.is_translated,
      ).length;
      const usage = { costUsd: translatedCount * 0.004 };
      this.conversationUsageCache.set(contactId, usage);
      if (contactId === this.currentContactId) {
        this.updateConversationUsageDisplay(usage);
      }
      return;
    }

    try {
      const response = await this.apiFetch(
        `/api/usage/${encodeURIComponent(contactId)}`,
        {
          headers: this.getAuthHeaders(),
        },
      );
      const usage = await response.json();
      this.conversationUsageCache.set(contactId, usage);
      if (contactId === this.currentContactId) {
        this.updateConversationUsageDisplay(usage);
      }
    } catch (err) {
      console.error('Failed to fetch conversation usage:', err);
    }
  },

  // Update conversation usage display in chat header
  updateConversationUsageDisplay(usage) {
    document.querySelectorAll('[data-chat-cost]').forEach((costEl) => {
      costEl.textContent = this.formatCost(usage.costUsd || 0);
    });
  },

  // Extract URLs from text
  extractUrls(text) {
    if (!text) return [];
    const urlRegex = /https?:\/\/[^\s<>\[\](){}|\\^`\x00-\x1f\x7f]+/gi;
    const matches = text.match(urlRegex) || [];

    // Clean trailing punctuation
    return matches.map((url) => {
      while (url.match(/[.,!?)\]};:'"]+$/)) {
        url = url.slice(0, -1);
      }
      return url;
    });
  },

  // Fetch link preview from API
  async fetchLinkPreview(url) {
    // Check memory cache first
    if (this.linkPreviewCache.has(url)) {
      return this.linkPreviewCache.get(url);
    }

    // Skip if already fetching
    if (this.linkPreviewFetching.has(url)) {
      return null;
    }

    this.linkPreviewFetching.add(url);

    try {
      const response = await this.apiFetch(
        `/api/link-preview?url=${encodeURIComponent(url)}`,
      );
      const preview = await response.json();

      // Cache the result
      this.linkPreviewCache.set(url, preview);

      return preview;
    } catch (err) {
      console.error('Failed to fetch link preview:', err);
      return null;
    } finally {
      this.linkPreviewFetching.delete(url);
    }
  },

  // Render link preview card HTML
  renderLinkPreviewCard(preview, url) {
    if (!preview || preview.error) {
      return ''; // Don't show card for errors
    }

    const hasImage =
      preview.imageUrl && !preview.imageUrl.includes('undefined');
    const title = preview.title || this.getDomainFromUrl(url);
    const description = preview.description || '';
    const siteName = preview.siteName || this.getDomainFromUrl(url);

    return `
      <a href="${this.escapeHtml(url)}" target="_blank" rel="noopener noreferrer" class="link-preview-card">
        ${
          hasImage
            ? `
          <div class="link-preview-image">
            <img src="${this.escapeHtml(preview.imageUrl)}" alt="" loading="lazy" data-hide-on-error="true">
          </div>
        `
            : ''
        }
        <div class="link-preview-content">
          <div class="link-preview-site">${this.escapeHtml(siteName)}</div>
          <div class="link-preview-title">${this.escapeHtml(title)}</div>
          ${description ? `<div class="link-preview-description">${this.escapeHtml(description)}</div>` : ''}
        </div>
      </a>
    `;
  },

  // Get domain from URL for display
  getDomainFromUrl(url) {
    try {
      const urlObj = new URL(url);
      return urlObj.hostname.replace(/^www\./, '');
    } catch {
      return url;
    }
  },
};
