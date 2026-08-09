import js from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'
import pluginVue from 'eslint-plugin-vue'
import eslintConfigPrettier from 'eslint-config-prettier'

export default tseslint.config(
  // 全局忽略：构建产物 / Rust 侧 / 依赖
  { ignores: ['dist/**', 'src-tauri/**', 'node_modules/**'] },

  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/recommended'],

  {
    languageOptions: {
      globals: {
        ...globals.browser,
      },
    },
  },

  // .vue SFC 的 <script lang="ts"> 交给 typescript-eslint 解析
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
        extraFileExtensions: ['.vue'],
        sourceType: 'module',
      },
    },
  },

  {
    rules: {
      // vuedraggable / ProseMirror 事件对象等少数场景仍需 any，降级为警告
      '@typescript-eslint/no-explicit-any': 'warn',
      // 与 vue-tsc 的 noUnusedLocals 重复，且 catch(e) 等惯用法常误报；
      // 保留下划线前缀豁免
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrors: 'none' },
      ],
    },
  },

  // 关闭与 Prettier 冲突的格式类规则（必须放最后）
  eslintConfigPrettier
)
