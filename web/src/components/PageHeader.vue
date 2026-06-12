<script setup lang="ts">
defineProps<{
  title: string;
  lastUpdated: string | null;
}>();

const emit = defineEmits<{
  refresh: [];
}>();
</script>

<template>
  <header class="sticky top-0 z-10 border-b border-slate-200 bg-white">
    <div class="flex min-h-16 flex-wrap items-center justify-between gap-3 px-4 py-3 md:px-7">
      <div>
        <p class="text-xs font-medium uppercase text-slate-400">rps controller</p>
        <h1 class="mt-1 text-xl font-semibold text-slate-900">{{ title }}</h1>
      </div>
      <div class="flex items-center gap-3">
        <span v-if="lastUpdated" class="text-sm text-slate-500">最后更新 {{ lastUpdated }}</span>
        <button
          class="rounded bg-[#18c6a3] px-4 py-2 text-sm font-medium text-white hover:bg-[#13ad8e]"
          type="button"
          @click="emit('refresh')"
        >
          刷新
        </button>
      </div>
    </div>
  </header>
</template>
