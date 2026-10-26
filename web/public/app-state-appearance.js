import { normalizeText } from './app-state-core.js';

const APPEARANCE_THEME_CONFIG = {
  whatsapp: {
    label: 'WhatsApp',
    light: {
      label: 'WhatsApp',
      themeColor: '#f7f5f2',
      accentColor: '#128c7e',
      surfaceColor: '#ffffff',
      textColor: '#111b21',
    },
    dark: {
      label: 'WhatsApp',
      themeColor: '#111b21',
      accentColor: '#00a884',
      surfaceColor: '#202c33',
      textColor: '#e9edef',
    },
  },
  ocean: {
    label: 'Ocean',
    light: {
      label: 'Ocean',
      themeColor: '#f4f7fb',
      accentColor: '#0f6cbd',
      surfaceColor: '#ffffff',
      textColor: '#102a43',
    },
    dark: {
      label: 'Ocean',
      themeColor: '#0f172a',
      accentColor: '#38bdf8',
      surfaceColor: '#162033',
      textColor: '#e0f2fe',
    },
  },
  sunset: {
    label: 'Sunset',
    light: {
      label: 'Sunset',
      themeColor: '#fff6ef',
      accentColor: '#dd6b20',
      surfaceColor: '#ffffff',
      textColor: '#4a2c1d',
    },
    dark: {
      label: 'Sunset',
      themeColor: '#1f1720',
      accentColor: '#f97316',
      surfaceColor: '#2a1f2d',
      textColor: '#fde7d8',
    },
  },
  github: {
    label: 'GitHub',
    light: {
      label: 'GitHub',
      themeColor: '#f6f8fa',
      accentColor: '#0969da',
      surfaceColor: '#ffffff',
      textColor: '#1f2328',
    },
    dark: {
      label: 'GitHub',
      themeColor: '#0d1117',
      accentColor: '#2f81f7',
      surfaceColor: '#161b22',
      textColor: '#e6edf3',
    },
  },
  dracula: {
    label: 'Dracula',
    light: {
      label: 'Dracula',
      themeColor: '#f7f4ff',
      accentColor: '#7c3aed',
      surfaceColor: '#ffffff',
      textColor: '#2d2140',
    },
    dark: {
      label: 'Dracula',
      themeColor: '#282a36',
      accentColor: '#bd93f9',
      surfaceColor: '#303341',
      textColor: '#f8f8f2',
    },
  },
  nord: {
    label: 'Nord',
    light: {
      label: 'Nord',
      themeColor: '#eceff4',
      accentColor: '#5e81ac',
      surfaceColor: '#ffffff',
      textColor: '#2e3440',
    },
    dark: {
      label: 'Nord',
      themeColor: '#2e3440',
      accentColor: '#88c0d0',
      surfaceColor: '#3b4252',
      textColor: '#eceff4',
    },
  },
  linear: {
    label: 'Linear',
    light: {
      label: 'Linear',
      themeColor: '#f7f8f8',
      accentColor: '#5e6ad2',
      surfaceColor: '#ffffff',
      textColor: '#171717',
    },
    dark: {
      label: 'Linear',
      themeColor: '#08090a',
      accentColor: '#7170ff',
      surfaceColor: '#191a1b',
      textColor: '#f7f8f8',
    },
  },
  vercel: {
    label: 'Vercel',
    light: {
      label: 'Vercel',
      themeColor: '#ffffff',
      accentColor: '#0070f3',
      surfaceColor: '#ffffff',
      textColor: '#171717',
    },
    dark: {
      label: 'Vercel',
      themeColor: '#000000',
      accentColor: '#3291ff',
      surfaceColor: '#111111',
      textColor: '#fafafa',
    },
  },
};

function normalizeHexColor(value) {
  const color = String(value || '').trim();
  const shortMatch = color.match(/^#([\da-f]{3})$/i);
  if (shortMatch) {
    return `#${shortMatch[1]
      .split('')
      .map((char) => char + char)
      .join('')}`.toLowerCase();
  }
  const longMatch = color.match(/^#([\da-f]{6})$/i);
  return longMatch ? `#${longMatch[1].toLowerCase()}` : null;
}

function hexToRgb(color) {
  const normalized = normalizeHexColor(color);
  if (!normalized) return null;
  return {
    r: Number.parseInt(normalized.slice(1, 3), 16),
    g: Number.parseInt(normalized.slice(3, 5), 16),
    b: Number.parseInt(normalized.slice(5, 7), 16),
  };
}

function getRelativeLuminance(color) {
  const rgb = hexToRgb(color);
  if (!rgb) return null;
  const toLinear = (channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return (
    0.2126 * toLinear(rgb.r) +
    0.7152 * toLinear(rgb.g) +
    0.0722 * toLinear(rgb.b)
  );
}

export function getContrastRatio(foreground, background) {
  const fg = getRelativeLuminance(foreground);
  const bg = getRelativeLuminance(background);
  if (fg == null || bg == null) return 1;
  const lighter = Math.max(fg, bg);
  const darker = Math.min(fg, bg);
  return Number(((lighter + 0.05) / (darker + 0.05)).toFixed(2));
}

export function getAppearanceThemeCatalog() {
  return Object.entries(APPEARANCE_THEME_CONFIG).map(([id, theme]) => ({
    id,
    label: theme.label,
    light: { ...theme.light },
    dark: { ...theme.dark },
  }));
}

export function resolveAppearanceTheme(preferences = {}, options = {}) {
  const requestedTheme = normalizeText(preferences?.theme);
  const theme = APPEARANCE_THEME_CONFIG[requestedTheme]
    ? requestedTheme
    : 'whatsapp';
  const requestedMode = normalizeText(preferences?.mode);
  const mode = ['light', 'dark', 'system'].includes(requestedMode)
    ? requestedMode
    : 'system';
  const systemMode =
    normalizeText(options?.systemMode) === 'dark' ? 'dark' : 'light';
  const resolvedMode = mode === 'system' ? systemMode : mode;
  const definition = { ...APPEARANCE_THEME_CONFIG[theme][resolvedMode] };

  return {
    theme,
    mode,
    resolvedMode,
    definition,
    dataTheme: `${theme}-${resolvedMode}`,
  };
}
