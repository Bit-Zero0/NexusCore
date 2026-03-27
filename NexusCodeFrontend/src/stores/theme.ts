import { computed, ref } from 'vue'
import { defineStore } from 'pinia'

type ThemeMode = 'light' | 'dark'

const THEME_KEY = 'nexus-theme'

export const useThemeStore = defineStore('theme', () => {
  const mode = ref<ThemeMode>('dark')

  const applyTheme = (value: ThemeMode) => {
    mode.value = value
    document.documentElement.dataset.theme = value
    localStorage.setItem(THEME_KEY, value)
  }

  const initialize = () => {
    const saved = localStorage.getItem(THEME_KEY)
    if (saved === 'light' || saved === 'dark') {
      applyTheme(saved)
      return
    }

    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    applyTheme(prefersDark ? 'dark' : 'light')
  }

  const toggleTheme = () => {
    applyTheme(mode.value === 'dark' ? 'light' : 'dark')
  }

  return {
    mode,
    isDark: computed(() => mode.value === 'dark'),
    initialize,
    toggleTheme,
  }
})
