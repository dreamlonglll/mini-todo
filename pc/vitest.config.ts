import { defineConfig } from 'vitest/config'
import { fileURLToPath } from 'node:url'

// 单测只覆盖纯函数工具（src/**/*.test.ts），用不到 Vue 插件与 Element Plus 按需引入：
// 与 vite.config.ts 分开，也免得跑测试时改写 src/components.d.ts。

// 日期时间工具按本机墙钟解析 / 格式化，固定时区让断言与运行机器无关
// （Asia/Shanghai 无夏令时；子进程 / worker 继承这里的环境变量，test.env 再兜底一次）
process.env.TZ = 'Asia/Shanghai'

export default defineConfig({
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
    env: {
      TZ: 'Asia/Shanghai',
    },
  },
})
