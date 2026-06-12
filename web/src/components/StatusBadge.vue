<script setup lang="ts">
defineProps<{
  enabled: boolean;
  enabledText?: string;
  disabledText?: string;
}>();
</script>

<template>
  <span
    class="rounded-full px-2 py-1 text-xs font-medium"
    :class="enabled ? 'bg-green-100 text-green-700' : 'bg-slate-100 text-slate-600'"
  >
    {{ enabled ? enabledText || "启用" : disabledText || "停用" }}
  </span>
</template>
