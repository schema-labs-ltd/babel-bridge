import { INITIAL_VIEWPORTS } from 'storybook/viewport';

// styles.css only @imports these with cache-busting queries, which Vite
// does not bundle, so load them directly in their cascade order.
import '../public/styles-base.css';
import '../public/styles-contacts.css';
import '../public/styles-messages.css';
import '../public/styles-simplified.css';
import '../public/styles-responsive.css';
import '../public/styles-menus.css';
import '../public/styles-workspace.css';
import '../public/styles-features.css';
import './storybook.css';

/** @type { import('@storybook/html-vite').Preview } */
const preview = {
  parameters: {
    layout: 'fullscreen',
    backgrounds: {
      disable: true,
    },
    viewport: {
      options: INITIAL_VIEWPORTS,
    },
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    options: {
      storySort: {
        order: ['Layouts', 'Components'],
      },
    },
  },
  initialGlobals: {
    appearanceTheme: 'whatsapp',
    appearanceMode: 'light',
    viewport: { value: 'desktop', isRotated: false },
  },
  globalTypes: {
    appearanceTheme: {
      name: 'Theme',
      description: 'Preview the different palette options used by the app.',
      toolbar: {
        icon: 'paintbrush',
        dynamicTitle: true,
        items: [
          { value: 'whatsapp', title: 'WhatsApp' },
          { value: 'ocean', title: 'Ocean' },
          { value: 'sunset', title: 'Sunset' },
          { value: 'github', title: 'GitHub' },
          { value: 'nord', title: 'Nord' },
          { value: 'linear', title: 'Linear' },
          { value: 'vercel', title: 'Vercel' },
        ],
      },
    },
    appearanceMode: {
      name: 'Mode',
      description: 'Switch between the light and dark surface modes.',
      toolbar: {
        icon: 'contrast',
        dynamicTitle: true,
        items: [
          { value: 'light', title: 'Light' },
          { value: 'dark', title: 'Dark' },
        ],
      },
    },
  },
  decorators: [
    (Story, context) => {
      const theme = context.globals.appearanceTheme || 'whatsapp';
      const mode = context.globals.appearanceMode || 'light';
      const pageLayout = context.parameters.pageLayout || 'canvas';
      const rendered = Story();

      document.documentElement.dataset.theme = `${theme}-${mode}`;
      document.documentElement.style.colorScheme = mode;
      document.body.classList.add('sb-storybook-body');

      return `<div class="sb-stage sb-stage--${pageLayout}">${rendered}</div>`;
    },
  ],
};

export default preview;
