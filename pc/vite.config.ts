import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import Components from "unplugin-vue-components/vite";
import { ElementPlusResolver } from "unplugin-vue-components/resolvers";
import { resolve } from "path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    vue(),
    // Element Plus 按需引入：模板里用到的 <el-*> 组件连同其样式由插件自动 import，
    // 不再全量注册组件、加载整份 index.css（每个窗口首包都要背上它）。
    // - dirs: [] —— 只解析 Element Plus；项目内组件与 @element-plus/icons-vue 图标一律在 SFC 里显式 import
    // - ElMessage / ElMessageBox 等函数式 API 不经过模板，插件不会替它们引样式，
    //   统一从 @/plugins/element 导入（eslint no-restricted-imports 兜底）
    // - dts：让 vue-tsc 认识模板里的 <el-*> 组件类型；构建时自动按实际用到的组件重写，需随代码提交
    Components({
      dirs: [],
      resolvers: [ElementPlusResolver({ importStyle: "css" })],
      dts: "src/components.d.ts",
    }),
  ],

  // 开发服务器：按需引入的 Element Plus 组件 / 样式入口是转换模板时才发现的，冷缓存下首次打开
  // 用到新组件的窗口会触发依赖重新预构建、所有窗口整页刷新。预先声明，启动时一次构建完
  optimizeDeps: {
    include: ["element-plus/es", "element-plus/es/components/*/style/css"],
  },

  resolve: {
    alias: [
      { find: "@", replacement: resolve(__dirname, "src") },
      // vuedraggable 4.1.0 只发布了 UMD 包，其中的 require("vue") 走 CommonJS 条件会解析到
      // vue/index.js → vue.cjs.prod.js：完整版 Vue，连带打进模板编译器 @vue/compiler-core /
      // compiler-dom（含会被 CSP 拦下的 new Function）。精确匹配裸 "vue"，让它与 ESM 的
      // import 'vue' 落到同一个运行时文件（同一份 Vue 实例，模板已在构建期编译，不需要编译器）
      { find: /^vue$/, replacement: "vue/dist/vue.runtime.esm-bundler.js" },
    ],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    // 显式绑 127.0.0.1：默认的 false 会让 Vite 绑到 "localhost"，而 Windows 上
    // localhost 优先解析成 ::1，Node 17+ 按首个解析结果绑定，结果只监听 IPv6。
    // tauri dev 探测 http://localhost:1420/ 走 IPv4 会被拒，卡在
    // "Waiting for your frontend dev server to start" 不动
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
