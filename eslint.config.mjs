import withNuxt from './.nuxt/eslint.config.mjs'
import prettier from 'eslint-config-prettier'
import architecture from './scripts/eslint/architecture.mjs'

export default withNuxt(
  {
    ignores: [
      '.dev/**',
      'out/**',
      'dist/**',
      '.output/**',
      '.nuxt/**',
      'coverage/**',
      'test-results/**',
      'playwright-report/**',
      'src/renderer/*.d.ts',
    ],
  },
  {
    files: ['app/**/*.{ts,vue}', 'src/**/*.ts'],
    plugins: { architecture },
    languageOptions: {
      parserOptions: {
        project: ['./.nuxt/tsconfig.app.json', './tsconfig.electron.json'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      'architecture/boundaries': 'error',
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
    files: ['**/*.ts', 'app/**/*.vue'],
    // TypeScript checks names, including generated Nuxt auto-imports.
    rules: { 'no-undef': 'off' },
  },
  prettier,
)
