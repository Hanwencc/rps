<script setup lang="ts">
import { Network } from "lucide-vue-next";

defineProps<{
  title: string;
}>();
</script>

<template>
  <section class="rounded border border-slate-200 bg-white p-8">
    <div class="flex max-w-3xl gap-4">
      <Network class="mt-1 text-slate-400" :size="28" />
      <div>
        <h2 class="text-lg font-semibold text-slate-900">{{ title }}</h2>
        <p class="mt-3 leading-7 text-slate-600">
          当前 MVP 已优先实现客户端、TCP/UDP 隧道、HTTP 代理、SOCKS5 代理和基础统计。
          该模块保留导航入口，后续接入对应后端 API 后可直接扩展为完整管理页。
        </p>
      </div>
    </div>
  </section>
</template>
