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
  }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

export default router
