import test from 'node:test';
import assert from 'node:assert/strict';
import {
  getAppearanceThemeCatalog,
  getContrastRatio,
  resolveAppearanceTheme,
} from '../public/app-state.js';

test('resolveAppearanceTheme falls back safely and honors system dark mode', () => {
  assert.deepEqual(
    resolveAppearanceTheme(
      { theme: 'unknown', mode: 'maybe' },
      { systemMode: 'dark' },
    ),
    {
      theme: 'whatsapp',
      mode: 'system',
      resolvedMode: 'dark',
      definition: {
        label: 'WhatsApp',
        themeColor: '#111b21',
        accentColor: '#00a884',
        surfaceColor: '#202c33',
        textColor: '#e9edef',
      },
      dataTheme: 'whatsapp-dark',
    },
  );
});

test('resolveAppearanceTheme returns the selected light theme variant explicitly', () => {
  assert.deepEqual(
    resolveAppearanceTheme(
      { theme: 'ocean', mode: 'light' },
      { systemMode: 'dark' },
    ),
    {
      theme: 'ocean',
      mode: 'light',
      resolvedMode: 'light',
      definition: {
        label: 'Ocean',
        themeColor: '#f4f7fb',
        accentColor: '#0f6cbd',
        surfaceColor: '#ffffff',
        textColor: '#102a43',
      },
      dataTheme: 'ocean-light',
    },
  );
});

test('appearance theme catalog includes additional recognizable developer themes', () => {
  const catalog = getAppearanceThemeCatalog();
  assert.deepEqual(
    catalog.map((theme) => theme.id),
    [
      'whatsapp',
      'ocean',
      'sunset',
      'github',
      'dracula',
      'nord',
      'linear',
      'vercel',
    ],
  );
});

test('all appearance theme variants meet a readable text contrast threshold', () => {
  const catalog = getAppearanceThemeCatalog();

  for (const theme of catalog) {
    for (const mode of ['light', 'dark']) {
      const variant = theme[mode];
      assert.ok(
        getContrastRatio(variant.textColor, variant.surfaceColor) >= 4.5,
        `${theme.id}-${mode} surface contrast should be at least 4.5`,
      );
      assert.ok(
        getContrastRatio(variant.textColor, variant.themeColor) >= 4.5,
        `${theme.id}-${mode} page contrast should be at least 4.5`,
      );
    }
  }
});
