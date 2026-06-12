import js from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import jsxA11y from 'eslint-plugin-jsx-a11y';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  { ignores: ['dist', 'node_modules'] },
  {
    extends: [
      js.configs.recommended,
      ...tseslint.configs.recommended,
    ],
    files: ['**/*.{ts,tsx,js}'],
    languageOptions: {
      ecmaVersion: 2020,
      globals: {
        window: 'readonly',
        document: 'readonly',
        navigator: 'readonly',
        console: 'readonly',
        process: 'readonly',
        describe: 'readonly',
        test: 'readonly',
        it: 'readonly',
        expect: 'readonly',
        vi: 'readonly',
        beforeEach: 'readonly',
        afterEach: 'readonly',
      },
    },
    plugins: {
      'react-hooks': reactHooks,
      'jsx-a11y': jsxA11y,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: {
      'jsx-a11y': jsxA11y,
    },
    rules: {
      ...jsxA11y.flatConfigs.recommended.rules,
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: 'Literal[value=/system-ui/]',
          message: 'Banned string literal "system-ui". Use the --ui or --display tokens instead.',
        },
        {
          selector: 'TemplateElement[value.raw=/system-ui/]',
          message: 'Banned template string containing "system-ui". Use the --ui or --display tokens instead.',
        },
        {
          selector: 'Literal[value=/\\b(text|bg|border)-(gray|slate|zinc|neutral|stone|red|green|blue|amber|yellow)-\\d{2,3}\\b/]',
          message: 'Banned Tailwind default-palette color class. Use token-mapped palette names instead.',
        },
        {
          selector: 'TemplateElement[value.raw=/\\b(text|bg|border)-(gray|slate|zinc|neutral|stone|red|green|blue|amber|yellow)-\\d{2,3}\\b/]',
          message: 'Banned Tailwind default-palette color class in template string. Use token-mapped palette names instead.',
        },
        {
          selector: 'Literal[value=/\\b(animate-(pulse|spin|bounce))\\b/]',
          message: 'Banned fake-live/decorative animation classes. Use brief interaction transitions only.',
        },
        {
          selector: 'TemplateElement[value.raw=/\\b(animate-(pulse|spin|bounce))\\b/]',
          message: 'Banned fake-live/decorative animation classes in template string. Use brief interaction transitions only.',
        },
        {
          selector: 'Literal[value=/\\banimate-\\[[^\\]]*infinite/]',
          message: 'Banned arbitrary infinite animations. Use brief interaction transitions only.',
        },
        {
          selector: 'TemplateElement[value.raw=/\\banimate-\\[[^\\]]*infinite/]',
          message: 'Banned arbitrary infinite animations in template string. Use brief interaction transitions only.',
        },
        {
          selector: 'Literal[value=/\\bbg-gradient-/]',
          message: 'Banned decorative background gradient (design.md §9). Use solid background colors instead.',
        },
        {
          selector: 'TemplateElement[value.raw=/\\bbg-gradient-/]',
          message: 'Banned decorative background gradient in template string (design.md §9). Use solid background colors instead.',
        },
        {
          selector: 'Literal[value=/\\bshadow(?!-modal-seat\\b)(?:-[a-zA-Z0-9_-]+)?\\b/]',
          message: 'Banned shadow class. Only "shadow-modal-seat" is allowed.',
        },
        {
          selector: 'TemplateElement[value.raw=/\\bshadow(?!-modal-seat\\b)(?:-[a-zA-Z0-9_-]+)?\\b/]',
          message: 'Banned shadow class in template string. Only "shadow-modal-seat" is allowed.',
        },
      ],
    },
  },
  {
    files: ['src/components/status/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-syntax': [
        'warn',
        {
          selector: 'Literal[value=/\\bamber\\b/]',
          message: 'Amber is brand-only, never a status (design.md §2)',
        },
        {
          selector: 'TemplateElement[value.raw=/\\bamber\\b/]',
          message: 'Amber is brand-only, never a status (design.md §2)',
        },
      ],
    },
  }
);
