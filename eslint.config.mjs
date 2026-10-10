import withNuxt from './apps/desktop/.nuxt/eslint.config.mjs'
import prettier from 'eslint-config-prettier'
import architecture from './tooling/eslint/architecture.mjs'
import logging from './tooling/eslint/logging.mjs'

export default withNuxt(
  {
    ignores: [
      '.dev/**',
      '**/out/**',
      '**/dist/**',
      '**/.output/**',
      '**/.nuxt/**',
      'coverage/**',
      'test-results/**',
      'playwright-report/**',
      'apps/desktop/electron/renderer/*.d.ts',
      'core/src/contracts/generated/**',
    ],
  },
  {
    files: [
      'apps/desktop/app/**/*.{ts,vue}',
      'core/src/**/*.ts',
      'hosts/node/src/**/*.ts',
      'apps/cli/src/**/*.ts',
      'apps/desktop/electron/**/*.ts',
      'apps/desktop/contracts/**/*.ts',
    ],
    plugins: { architecture, logging },
    languageOptions: {
      parserOptions: {
        project: [
          './apps/desktop/.nuxt/tsconfig.app.json',
          './apps/desktop/tsconfig.electron.json',
          './core/tsconfig.json',
          './hosts/node/tsconfig.json',
          './apps/cli/tsconfig.json',
        ],
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      'architecture/boundaries': 'error',
      'logging/no-silent-catch': 'error',
      'logging/no-silent-promise-catch': 'error',
      'logging/no-raw-console': 'error',
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': ['error', { checksVoidReturn: false }],
      '@typescript-eslint/no-explicit-any': 'error',
    },
  },
  {
    rules: {
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
      // Existing components intentionally use concise names and optional props.
      'vue/multi-word-component-names': 'off',
      'vue/require-default-prop': 'off',
      // Reactive dictionaries intentionally remove entries by key.
      '@typescript-eslint/no-dynamic-delete': 'off',
    },
  },
  {
    files: ['**/*.ts', 'apps/desktop/app/**/*.vue'],
    // TypeScript checks names, including generated Nuxt auto-imports.
    rules: { 'no-undef': 'off' },
  },
  prettier,
)
