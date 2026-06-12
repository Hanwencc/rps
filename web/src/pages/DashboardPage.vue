<script setup lang="ts">
import { computed } from "vue";
import { Activity, Database, FileText, Route } from "lucide-vue-next";
import type { ClientResponse, StatusResponse, TunnelResponse } from "../types";

const props = defineProps<{
  status: StatusResponse | null;
  clients: ClientResponse[];
  tunnels: TunnelResponse[];
}>();

const enabledClients = computed(() => props.clients.filter((client) => client.enabled).length);
const tcpTunnels = computed(() => props.tunnels.filter((tunnel) => tunnel.mode === "tcp"));
const udpTunnels = computed(() => props.tunnels.filter((tunnel) => tunnel.mode === "udp"));
const totalTraffic = computed(() =>
  props.clients.reduce((sum, client) => sum + client.rx_bytes + client.tx_bytes, 0),
);

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }

  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value >= 10 || unitIndex === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unitIndex]}`;
}
</script>

<template>
  <section class="space-y-5">
    <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
      <div class="rounded border border-slate-200 bg-white p-5">
        <p class="text-sm text-slate-500">在线客户端</p>
        <p class="mt-2 text-3xl font-semibold text-slate-900">
          {{ status?.online_clients ?? 0 }}/{{ status?.configured_clients ?? 0 }}
        </p>
        <p class="mt-2 text-sm text-slate-500">已启用 {{ enabledClients }} 个客户端</p>
      </div>
      <div class="rounded border border-slate-200 bg-white p-5">
        <p class="text-sm text-slate-500">隧道数量</p>
        <p class="mt-2 text-3xl font-semibold text-slate-900">{{ status?.enabled_tunnels ?? 0 }}</p>
        <p class="mt-2 text-sm text-slate-500">TCP {{ tcpTunnels.length }} / UDP {{ udpTunnels.length }}</p>
      </div>
      <div class="rounded border border-slate-200 bg-white p-5">
        <p class="text-sm text-slate-500">累计流量</p>
        <p class="mt-2 text-3xl font-semibold text-slate-900">{{ formatBytes(totalTraffic) }}</p>
        <p class="mt-2 text-sm text-slate-500">来自所有客户端统计</p>
      </div>
      <div class="rounded border border-slate-200 bg-white p-5">
        <p class="text-sm text-slate-500">Bridge 地址</p>
        <p class="mt-2 break-all font-mono text-lg text-slate-900">{{ status?.bridge_addr }}</p>
        <p class="mt-2 text-sm text-slate-500">agent 主动连接入口</p>
      </div>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">运行概览</h2>
      </div>
      <div class="grid gap-4 p-5 lg:grid-cols-3">
        <div class="flex items-center gap-3 rounded border border-slate-200 p-4">
          <Database class="text-[#18c6a3]" :size="22" />
          <div>
            <p class="font-medium text-slate-900">SQLite 持久化</p>
            <p class="text-sm text-slate-500">clients / tunnels / traffic counters</p>
          </div>
        </div>
        <div class="flex items-center gap-3 rounded border border-slate-200 p-4">
          <Route class="text-[#18c6a3]" :size="22" />
          <div>
            <p class="font-medium text-slate-900">代理链路</p>
            <p class="text-sm text-slate-500">外部访问 -> controller -> agent -> target</p>
          </div>
        </div>
        <div class="flex items-center gap-3 rounded border border-slate-200 p-4">
          <Activity class="text-[#18c6a3]" :size="22" />
          <div>
            <p class="font-medium text-slate-900">Noise + PSK</p>
            <p class="text-sm text-slate-500">bridge 认证和传输加密</p>
          </div>
        </div>
      </div>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">控制台</h2>
      </div>
      <div class="flex items-center gap-3 p-5 text-sm text-slate-600">
        <FileText class="text-[#18c6a3]" :size="22" />
        <span>Web 控制台监听 {{ status?.web_addr || "-" }}</span>
      </div>
    </div>
  </section>
</template>
