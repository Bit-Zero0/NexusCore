import { createRouter, createWebHistory } from 'vue-router'

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    {
      path: '/',
      name: 'home',
      component: () => import('../views/HomeView.vue'),
    },
    {
      path: '/problems',
      name: 'problems',
      component: () => import('../views/ProblemsView.vue'),
    },
    {
      path: '/problems/:problemId',
      name: 'problem-detail',
      component: () => import('../views/ProblemDetailView.vue'),
    },
    {
      path: '/problems/:problemId/submit',
      name: 'problem-submit',
      component: () => import('../views/SubmitView.vue'),
    },
    {
      path: '/submissions',
      name: 'submissions',
      component: () => import('../views/SubmissionsView.vue'),
    },
    {
      path: '/submissions/:submissionId',
      name: 'submission-detail',
      component: () => import('../views/SubmissionDetailView.vue'),
    },
    {
      path: '/admin/problems',
      name: 'admin-problems',
      component: () => import('../views/AdminProblemsView.vue'),
    },
    {
      path: '/admin/problems/new',
      name: 'admin-problems-new',
      component: () => import('../views/AdminProblemEditorView.vue'),
    },
    {
      path: '/admin/problems/:problemId/edit',
      name: 'admin-problems-edit',
      component: () => import('../views/AdminProblemEditorView.vue'),
    },
    {
      path: '/admin/cluster',
      name: 'admin-cluster',
      component: () => import('../views/ClusterView.vue'),
    },
  ],
})

export default router
