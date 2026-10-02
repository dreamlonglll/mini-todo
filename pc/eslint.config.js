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

  // 开发辅助脚本（如 scripts/e2e/cdp.mjs）跑在 Node 里
  {
    files: ['scripts/**/*.{js,mjs}'],
    languageOptions: {
      globals: {
        ...globals.node,
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

  // Element Plus 按需引入后，ElMessage / ElMessageBox 等函数式 API 的样式不会被自动带上：
  // 一律经 @/plugins/element 导入（那里同时引入样式）。仅类型导入不受限制
  {
    files: ['src/**/*.{ts,vue}'],
    ignores: ['src/plugins/element.ts'],
    rules: {
      '@typescript-eslint/no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: 'element-plus',
              message: '请从 @/plugins/element 导入（该模块同时引入对应样式）；新的函数式 API 先在那里补上样式再导出',
              allowTypeImports: true,
            },
          ],
          patterns: [
            {
              group: ['element-plus/*'],
              message: '组件由 unplugin-vue-components 按需引入，函数式 API 请从 @/plugins/element 导入',
              allowTypeImports: true,
            },
          ],
        },
      ],
    },
  },

  // 关闭与 Prettier 冲突的格式类规则（必须放最后）
  eslintConfigPrettier
)
