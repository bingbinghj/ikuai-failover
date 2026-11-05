import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useTheme, type ThemeDefinition } from 'vuetify'

export type ThemePreference = 'system' | 'light' | 'dark'
export type ThemeColorScheme = 'light' | 'dark'
const key = 'ikuai-failover-theme'

export function readAppearancePreference(): ThemePreference {
  try {
    const value = localStorage.getItem(key)
    return value === 'light' || value === 'dark' ? value : 'system'
  } catch { return 'system' }
}

export function initialColorScheme(): ThemeColorScheme {
  const preference = readAppearancePreference()
  return preference === 'system' ? window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light' : preference
}

const preference = ref<ThemePreference>(readAppearancePreference())

export function useAppearance() {
  const theme = useTheme()
  const media = window.matchMedia('(prefers-color-scheme: dark)')
  const systemDark = ref(media.matches)
  const scheme = computed(() => preference.value === 'system' ? systemDark.value ? 'dark' : 'light' : preference.value)
  const choices: { value: ThemePreference; label: string; icon: string }[] = [
    { value: 'light', label: '浅色主题', icon: 'sun' },
    { value: 'dark', label: '深色主题', icon: 'moon' },
    { value: 'system', label: '跟随系统', icon: 'monitor' },
  ]
  const label = computed(() => choices.find(choice => choice.value === preference.value)?.label ?? '跟随系统')
  const change = () => { systemDark.value = media.matches }
  watch([preference, scheme], () => {
    theme.global.name.value = scheme.value
    document.documentElement.dataset.theme = scheme.value
    document.documentElement.dataset.themePreference = preference.value
    document.documentElement.style.colorScheme = scheme.value
    const color = getMoviePilotThemePalette(scheme.value).colors?.background ?? '#F2F2F7'
    let meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')
    if (!meta) { meta = document.createElement('meta'); meta.name = 'theme-color'; document.head.append(meta) }
    meta.content = color
    try { localStorage.setItem(key, preference.value) } catch { /* Theme remains available without browser storage. */ }
  }, { immediate: true })
  onMounted(() => media.addEventListener('change', change))
  onUnmounted(() => media.removeEventListener('change', change))
  return { preference, scheme, label, choices }
}

export function getMoviePilotThemePalette(colorScheme: ThemeColorScheme): ThemeDefinition {
  const darkPalette = {
    dark: true,
    colors: {
      'primary': '#8D51F9',
      'secondary': '#7C9CC4',
      'on-secondary': '#07101C',
      'success': '#69C59A',
      'info': '#7C9CC4',
      'warning': '#E4B863',
      'error': '#FF7B7B',
      'on-primary': '#FFFFFF',
      'on-success': '#07150F',
      'on-warning': '#171106',
      'background': '#0B1322',
      'on-background': '#F2F5FA',
      'surface': '#182235',
      'on-surface': '#F2F5FA',
      'surface-variant': '#202C42',
      'on-surface-variant': 'rgba(242, 245, 250, 0.72)',
      'grey-50': '#121D30',
      'grey-100': '#18253A',
      'grey-200': '#233149',
      'grey-300': '#34435D',
      'grey-400': '#8794A8',
      'grey-500': '#A1ACBD',
      'grey-600': '#BBC3D0',
      'grey-700': '#CDD3DD',
      'grey-800': '#E0E4EA',
      'grey-900': '#F2F5FA',
      'perfect-scrollbar-thumb': 'rgba(255, 255, 255, 0.24)',
      'skin-bordered-background': '#111B2B',
      'skin-bordered-surface': '#182235',
      'card-background': '#182235',
    },
    variables: {
      'code-color': '#F3DCA8',
      'overlay-scrim-background': '#030712',
      'overlay-scrim-opacity': 0.66,
      'hover-opacity': 0.08,
      'focus-opacity': 0.14,
      'selected-opacity': 0.16,
      'activated-opacity': 0.14,
      'pressed-opacity': 0.18,
      'dragged-opacity': 0.14,
      'border-color': '#FFFFFF',
      'table-header-background': 'rgba(255, 255, 255, 0.06)',
      'custom-background': 'rgba(255, 255, 255, 0.06)',
      'card-background': '#182235',

      'shadow-key-umbra-opacity': 'rgba(3, 7, 18, 0.34)',
      'shadow-key-penumbra-opacity': 'rgba(3, 7, 18, 0.24)',
      'shadow-key-ambient-opacity': 'rgba(3, 7, 18, 0.16)',
    },
  }
  if (colorScheme === 'dark') return darkPalette

  return {
    dark: false,
    colors: {
      ...darkPalette.colors,
      'background': '#F5F5FB',
      'on-background': '#5D596C',
      'surface': '#FFFFFF',
      'on-surface': '#5D596C',
      'surface-variant': '#ECEBEF',
      'on-surface-variant': 'rgba(60, 60, 67, 0.72)',
      'secondary': '#636366',
      'on-secondary': '#FFFFFF',
      'success': '#248A3D',
      'info': '#007AFF',
      'warning': '#FF9500',
      'error': '#FF3B30',
      'on-success': '#FFFFFF',
      'on-warning': '#000000',
      'grey-50': '#F2F2F7',
      'grey-100': '#E5E5EA',
      'grey-200': '#D1D1D6',
      'grey-300': '#C7C7CC',
      'grey-400': '#AEAEB2',
      'grey-500': '#8E8E93',
      'grey-600': '#636366',
      'grey-700': '#48484A',
      'grey-800': '#3A3A3C',
      'grey-900': '#1C1C1E',
      'perfect-scrollbar-thumb': 'rgba(60, 60, 67, 0.24)',
      'skin-bordered-background': '#F5F5FB',
      'skin-bordered-surface': '#FFFFFF',
      'card-background': '#FFFFFF',
    },
    variables: {
      ...darkPalette.variables,
      'overlay-scrim-background': '#000000',
      'border-color': '#3C3C43',
      'card-background': '#FFFFFF',
      'shadow-key-umbra-opacity': 'rgba(0, 0, 0, 0.1)',
      'shadow-key-penumbra-opacity': 'rgba(0, 0, 0, 0.07)',
      'shadow-key-ambient-opacity': 'rgba(0, 0, 0, 0.04)',
    },
  }
}
