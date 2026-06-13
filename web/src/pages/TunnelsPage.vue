<script setup lang="ts">
import { ref, watch } from "vue";
import { Trash2 } from "lucide-vue-next";
import type { ClientResponse, CreateTunnelPayload, TunnelResponse } from "../types";

const props = defineProps<{
  title: string;
  mode: "tcp" | "udp";
  tunnels: TunnelResponse[];
  clients: ClientResponse[];
  creating: boolean;
  deletingId: string | null;
  error: string | null;
}>();

const emit = defineEmits<{
  create: [payload: CreateTunnelPayload];
  delete: [id: string];
}>();

const form = ref({
  id: "",
  client_id: "",
  listen: "",
  target: "",
  enabled: true,
  expires_at: "",
  traffic_limit_mb: "",
});

watch(
  () => props.clients,
  (clients) => {
    if (!form.value.client_id && clients.length > 0) {
      form.value.client_id = clients[0].id;
    }
  },
  { immediate: true },
);

function submit() {
  if (!form.value.client_id) {
    return;
  }
  emit("create", {
    id: form.value.id.trim() || null,
    client_id: form.value.client_id,
    mode: props.mode,
    listen: form.value.listen.trim(),
    target: form.value.target.trim(),
    enabled: form.value.enabled,
    expires_at: datetimeLocalToUnix(form.value.expires_at),
    traffic_limit_bytes: megabytesToBytes(form.value.traffic_limit_mb),
  });
  form.value.id = "";
  form.value.listen = "";
  form.value.target = "";
  form.value.enabled = true;
  form.value.expires_at = "";
  form.value.traffic_limit_mb = "";
}

function confirmDelete(tunnel: TunnelResponse) {
  const suffix = tunnel.enabled ? "。删除后监听端口会立即停止接收新连接" : "";
  if (window.confirm(`确认删除隧道 ${tunnel.id}？${suffix}`)) {
    emit("delete", tunnel.id);
  }
}

function datetimeLocalToUnix(value: string) {
  if (!value) {
    return null;
  }
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) ? Math.floor(timestamp / 1000) : null;
}

function megabytesToBytes(value: string) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return null;
  }
  return Math.floor(parsed * 1024 * 1024);
}

function formatBytes(bytes: number | null) {
  if (!bytes || bytes <= 0) {
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

function formatTime(value: number | null) {
  return value ? new Date(value * 1000).toLocaleString() : "永不过期";
}

function formatReason(value: string | null) {
  if (value === "expired") return "已到期";
  if (value === "traffic_exhausted") return "流量用完";
  if (value === "manual") return "手动停用";
  return value || "-";
}
</script>

<template>
  <section class="space-y-5">
    <div class="rounded border border-slate-200 bg-white">
      <div class="border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">新增 {{ title }}</h2>
      </div>
      <form class="grid gap-4 p-5 lg:grid-cols-6" @submit.prevent="submit">
        <label class="block">
          <span class="text-sm text-slate-600">隧道 ID</span>
          <input
            v-model="form.id"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="留空自动生成"
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">客户端</span>
          <select
            v-model="form.client_id"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
          >
            <option v-for="client in clients" :key="client.id" :value="client.id">
              {{ client.remark || client.id }}
            </option>
          </select>
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">监听地址</span>
          <input
            v-model="form.listen"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="0.0.0.0:10090"
            required
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">目标地址</span>
          <input
            v-model="form.target"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="127.0.0.1:80"
            required
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">到期时间</span>
          <input
            v-model="form.expires_at"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
            type="datetime-local"
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">流量上限 MB</span>
          <input
            v-model="form.traffic_limit_mb"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
            min="0"
            step="1"
            type="number"
          />
        </label>
        <div class="flex items-end gap-4">
          <label class="flex items-center gap-2 pb-2 text-sm">
            <input v-model="form.enabled" type="checkbox" />
            启用隧道
          </label>
          <button
            class="rounded bg-[#18c6a3] px-4 py-2 text-sm font-medium text-white hover:bg-[#13ad8e] disabled:bg-slate-400"
            :disabled="creating || clients.length === 0"
            type="submit"
          >
            {{ creating ? "创建中" : "创建隧道" }}
          </button>
        </div>
        <p v-if="error" class="text-sm text-red-600 lg:col-span-6">{{ error }}</p>
      </form>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="flex items-center justify-between border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">{{ title }}</h2>
        <span class="text-sm text-slate-500">共 {{ tunnels.length }} 条</span>
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
              <th class="px-5 py-3">流量</th>
              <th class="px-5 py-3">到期时间</th>
              <th class="px-5 py-3">停用原因</th>
              <th class="px-5 py-3">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="tunnel in tunnels" :key="tunnel.id">
              <td class="px-5 py-3 font-mono text-slate-900">{{ tunnel.id }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ tunnel.listen }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ tunnel.target || "-" }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ tunnel.client_id }}</td>
              <td class="px-5 py-3">{{ tunnel.enabled ? "启用" : "停用" }}</td>
              <td class="px-5 py-3 text-slate-600">
                {{ formatBytes(tunnel.rx_bytes + tunnel.tx_bytes) }} /
                {{ tunnel.traffic_limit_bytes ? formatBytes(tunnel.traffic_limit_bytes) : "不限" }}
              </td>
              <td class="px-5 py-3 text-slate-600">{{ formatTime(tunnel.expires_at) }}</td>
              <td class="px-5 py-3 text-slate-600">{{ formatReason(tunnel.disabled_reason) }}</td>
              <td class="px-5 py-3">
                <button
                  class="inline-flex items-center gap-1 rounded border border-red-200 px-3 py-1.5 text-xs font-medium text-red-600 hover:bg-red-50 disabled:cursor-not-allowed disabled:border-slate-200 disabled:text-slate-400"
                  :disabled="deletingId === tunnel.id"
                  type="button"
                  @click="confirmDelete(tunnel)"
                >
                  <Trash2 :size="14" />
                  {{ deletingId === tunnel.id ? "删除中" : "删除" }}
                </button>
              </td>
            </tr>
            <tr v-if="tunnels.length === 0">
              <td class="px-5 py-8 text-center text-slate-500" colspan="9">暂无隧道</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
