/**
 * Element Plus 函数式 API 的唯一入口
 *
 * 模板里的 <el-*> 组件由 unplugin-vue-components 按需引入，连同样式（见 vite.config.ts）；
 * ElMessage / ElMessageBox 这类在脚本里调用的 API 不经过模板，插件不会替它们引入样式。
 * 统一从这里导入，样式随本模块一起加载，不会出现"弹出来了却没有样式"的消息框。
 *
 * 需要新的函数式 API（如 ElNotification、ElLoading.service）时：先在这里补上对应组件的
 * style/css，再导出。eslint 的 no-restricted-imports 禁止其它文件直接从 'element-plus' 导入值。
 */
import 'element-plus/es/components/message/style/css'
import 'element-plus/es/components/message-box/style/css'

export { ElMessage, ElMessageBox } from 'element-plus'
