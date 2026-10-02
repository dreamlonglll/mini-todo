import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import './styles/main.scss'

// 生产模式下禁用浏览器默认行为
if (import.meta.env.PROD) {
  // 禁用右键菜单
  document.addEventListener('contextmenu', (e) => {
    e.preventDefault()
  })

  // 禁用浏览器快捷键
  document.addEventListener('keydown', (e) => {
    // 禁用 F5 刷新
    if (e.key === 'F5') {
      e.preventDefault()
      return
    }

    // 禁用 F12 开发者工具
    if (e.key === 'F12') {
      e.preventDefault()
      return
    }

    // 禁用 Ctrl/Cmd 组合键
    if (e.ctrlKey || e.metaKey) {
      const blockedKeys = [
        'r', 'R',     // 刷新
        'u', 'U',     // 查看源码
        's', 'S',     // 保存
        'p', 'P',     // 打印
        'g', 'G',     // 查找
        'f', 'F',     // 查找
        'j', 'J',     // 下载
        'h', 'H',     // 历史记录
        'd', 'D',     // 书签
        'e', 'E',     // 搜索
      ]

      // Ctrl+Shift 组合键
      if (e.shiftKey) {
        const blockedShiftKeys = [
          'i', 'I',   // 开发者工具
          'j', 'J',   // 开发者工具
          'c', 'C',   // 开发者工具
          'r', 'R',   // 强制刷新
          'n', 'N',   // 无痕窗口
        ]
        if (blockedShiftKeys.includes(e.key)) {
          e.preventDefault()
          return
        }
      }

      if (blockedKeys.includes(e.key)) {
        e.preventDefault()
        return
      }
    }
  })
}

// Element Plus 组件与样式按需引入（vite.config.ts 的 unplugin-vue-components），
// 图标在各 SFC 里显式 import，函数式 API 走 @/plugins/element：这里不再全量注册
const app = createApp(App)

app.use(createPinia())
app.use(router)
app.mount('#app')
