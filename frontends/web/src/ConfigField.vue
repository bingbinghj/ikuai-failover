<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { Field } from './configSchema'
const props = defineProps<{ field: Field; value: unknown; id: string; showPasswords?: boolean }>()
const emit = defineEmits<{ update: [value: unknown]; invalid: [message: string] }>()
const error = ref('')
const revealed = ref(false)
watch(() => props.id, () => { revealed.value = false })
const current = computed(() => props.value ?? props.field.default ?? (props.field.kind === 'boolean' ? false : ''))
const displayValue = computed(() => props.field.kind === 'json' ? JSON.stringify(current.value, null, 2) ?? '' : props.field.kind === 'list' ? Array.isArray(current.value) ? current.value.join('\n') : '' : typeof current.value === 'number' || typeof current.value === 'string' ? current.value : '')
const choices = computed(() => {
  const values = props.field.choices ?? props.field.options?.map(value => ({ value, label: ({ text: '文本', markdown: 'Markdown', user: '个人', group: '群组', chat_id: '群聊', open_id: 'Open ID', union_id: 'Union ID', user_id: '用户 ID', email: '邮箱' } as Record<string, string>)[value] ?? value })) ?? []
  if (values.length && current.value !== '' && !values.some(choice => String(choice.value) === String(current.value))) return [...values, { value: current.value as string | number, label: `当前设置：${String(current.value)}` }]
  return values
})
const isChoice = computed(() => props.field.kind === 'select' || choices.value.length > 0)
const label = computed(() => props.field.label + (props.field.optional ? '（可选）' : ''))
const icon = computed(() => props.field.icon ?? (props.field.kind === 'password' ? 'mdi-lock-outline' : /user|username|account/i.test(props.field.key) ? 'mdi-account-outline' : /url|listen|dir/i.test(props.field.key) ? 'mdi-link-variant' : props.field.kind === 'number' ? 'mdi-clock-outline' : 'mdi-tune'))
function update(next: unknown) {
  error.value = ''; emit('invalid', '')
  if (props.field.kind === 'boolean') { emit('update', Boolean(next)); return }
  const raw = next === null || next === undefined ? '' : String(next)
  if (raw === '' && props.field.optional) { emit('update', undefined); return }
  try {
    if (props.field.kind === 'number') {
      const value = Number(raw)
      if (raw === '' || !Number.isSafeInteger(value) || (props.field.min !== undefined && value < props.field.min) || (props.field.max !== undefined && value > props.field.max)) throw new Error()
      emit('update', value)
    } else if (props.field.kind === 'json') {
      const value: unknown = JSON.parse(raw)
      if (!Array.isArray(value) || value.some(item => typeof item !== 'number' || !Number.isSafeInteger(item) || item < 0)) throw new Error()
      emit('update', value)
    } else if (props.field.kind === 'list') emit('update', raw.split(/[\n,，]/).map(value => value.trim()).filter(Boolean))
    else emit('update', raw)
  } catch {
    error.value = props.field.kind === 'json' ? '请填写非负整数数组。' : '请选择有效数值。'
    emit('invalid', `${props.field.label}：${error.value}`)
  }
}
</script>
<template>
  <div class="config-field">
    <VSwitch v-if="field.kind === 'boolean'" :id="id" :model-value="Boolean(current)" :label="label" :hint="field.hint" :persistent-hint="Boolean(field.hint)" color="primary" inset :error-messages="error" @update:model-value="update" />
    <VSelect v-else-if="isChoice" :id="id" :model-value="current" :items="choices" item-title="label" item-value="value" :label="label" :prepend-inner-icon="icon" :hint="field.hint" :persistent-hint="Boolean(field.hint)" :clearable="field.optional" :error-messages="error" @update:model-value="update" />
    <VTextarea v-else-if="field.kind === 'json' || field.kind === 'list'" :id="id" :model-value="String(displayValue)" :label="label" :prepend-inner-icon="icon" :hint="field.hint" :persistent-hint="Boolean(field.hint)" :error-messages="error" rows="3" auto-grow autocomplete="off" @update:model-value="update" />
    <VTextField v-else :id="id" :model-value="displayValue" :label="label" :prepend-inner-icon="icon" :type="field.kind === 'password' && !revealed ? 'password' : field.kind === 'number' ? 'number' : 'text'" :min="field.min" :max="field.max" :hint="field.hint" :persistent-hint="Boolean(field.hint)" :error-messages="error" :autocomplete="field.kind === 'password' ? 'new-password' : 'off'" @update:model-value="update"><template v-if="field.kind === 'password'" #append-inner><VBtn :icon="revealed ? 'mdi-eye-off-outline' : 'mdi-eye-outline'" variant="text" color="default" density="compact" :aria-label="revealed ? '隐藏此密码' : '显示此密码'" :aria-pressed="revealed" @mousedown.prevent @click.stop="revealed = !revealed" /></template></VTextField>
  </div>
</template>
<style scoped>
.config-field{min-width:0}
</style>
