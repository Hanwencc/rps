<script setup lang="ts">
import type { TunnelResponse } from "../types";

defineProps<{
  title: string;
  tunnels: TunnelResponse[];
}>();
</script>

<template>
  <section class="rounded border border-slate-200 bg-white">
    <div class="border-b border-slate-200 px-5 py-4">
      <h2 class="font-semibold text-slate-900">{{ title }}</h2>
    </div>
    <div class="overflow-x-auto">
      <table class="w-full min-w-[820px] text-left text-sm">
        <thead class="bg-slate-50 text-xs text-slate-500">
          <tr>
            <th class="px-5 py-3">隧道 ID</th>
            <th class="px-5 py-3">监听地址</th>
            <th class="px-5 py-3">目标地址</th>
            <th class="px-5 py-3">客户端</th>
            <th class="px-5 py-3">状态</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-slate-100">
          <tr v-for="tunnel in tunnels" :key="tunnel.id">
            <td class="px-5 py-3 font-mono text-slate-900">{{ tunnel.id }}</td>
            <td class="px-5 py-3 font-mono text-slate-600">{{ tunnel.listen }}</td>
            <td class="px-5 py-3 font-mono text-slate-600">{{ tunnel.target || "-" }}</td>
            <td class="px-5 py-3 font-mono text-slate-600">{{ tunnel.client_id }}</td>
            <td class="px-5 py-3">{{ tunnel.enabled ? "启用" : "停用" }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
