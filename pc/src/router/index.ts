import { createRouter, createWebHashHistory, RouteRecordRaw } from 'vue-router'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    name: 'main',
    component: () => import('@/views/MainView.vue')
  },
  {
    path: '/editor',
    name: 'editor',
    component: () => import('@/views/EditorView.vue')
  },
  {
    path: '/settings',
    name: 'settings',
    component: () => import('@/views/SettingsView.vue')
  },
  {
    path: '/notification',
    name: 'notification',
    component: () => import('@/views/NotificationView.vue')
  },
  {
    path: '/subtask-editor',
    name: 'subtask-editor',
    component: () => import('@/views/SubtaskEditorView.vue')
  },
  {
    path: '/completed',
    name: 'completed',
    component: () => import('@/views/CompletedView.vue')
  },
  {
    // 桌面收纳箱窗口（label fence-{id}，由后端 commands/fence.rs 建窗）
    path: '/fence',
    name: 'fence',
    component: () => import('@/views/FenceView.vue')
  }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

export default router
