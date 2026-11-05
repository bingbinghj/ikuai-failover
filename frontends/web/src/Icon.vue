<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ name: string; size?: number }>()
const names: Record<string, string> = {
  network: 'mdi-lan',
  dashboard: 'mdi-view-dashboard-outline',
  bell: 'mdi-bell-outline',
  refresh: 'mdi-reload',
  pause: 'mdi-pause',
  play: 'mdi-play-outline',
  arrow: 'mdi-arrow-right',
  check: 'mdi-check',
  clock: 'mdi-clock-outline',
  alert: 'mdi-alert-circle-outline',
  search: 'mdi-magnify',
  logout: 'mdi-logout',
  lock: 'mdi-lock-outline',
  eye: 'mdi-eye-outline',
  shield: 'mdi-shield-check-outline',
  settings: 'mdi-cog-outline',
  sun: 'mdi-white-balance-sunny',
  moon: 'mdi-weather-night',
  monitor: 'mdi-monitor',
  menu: 'mdi-menu',
  close: 'mdi-close',
  logs: 'mdi-text-box-outline',
  download: 'mdi-download',
  'check-all': 'mdi-check-all',
  'chevron-down': 'mdi-chevron-down',
}
const icon = computed(() => props.name.startsWith('mdi-') ? props.name : names[props.name] ?? 'mdi-lan')
</script>

<template>
  <VIcon :icon="icon" :size="size ?? 20" aria-hidden="true" />
</template>
