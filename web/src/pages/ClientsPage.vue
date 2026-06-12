<script setup lang="ts">
import { computed, ref } from "vue";
import StatusBadge from "../components/StatusBadge.vue";
import type { ClientResponse, CreateClientPayload } from "../types";

const props = defineProps<{
  clients: ClientResponse[];
  creating: boolean;
  error: string | null;
}>();

const emit = defineEmits<{
  create: [payload: CreateClientPayload];
}>();

const form = ref({
  psk: "",
  remark: "",
  enabled: true,
});

const onlineClients = computed(() => props.clients.filter((client) => client.online).length);

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

function submit() {
  emit("create", {
    psk: form.value.psk.trim() || null,
    remark: form.value.remark.trim() || null,
    enabled: form.value.enabled,
  });
  form.value = {
    psk: "",
    remark: "",
    enabled: true,
  };
}
</script>

<template>
  <section class="space-y-5">
    <div class="rounded border border-slate-200 bg-white">
      <div class="border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">创建客户端</h2>
      </div>
      <form class="grid gap-4 p-5 lg:grid-cols-[2fr_1fr_auto]" @submit.prevent="submit">
        <label class="block">
          <span class="text-sm text-slate-600">认证密钥 psk</span>
          <input
            v-model="form.psk"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="留空自动生成 64 位 hex"
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">备注</span>
          <input
            v-model="form.remark"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
          />
        </label>
        <div class="flex items-end gap-4">
          <label class="flex items-center gap-2 pb-2 text-sm">
            <input v-model="form.enabled" type="checkbox" />
            启用
          </label>
          <button
            class="rounded bg-[#18c6a3] px-4 py-2 text-sm font-medium text-white hover:bg-[#13ad8e] disabled:bg-slate-400"
            :disabled="creating"
            type="submit"
          >
            {{ creating ? "创建中" : "新增客户端" }}
          </button>
        </div>
        <p v-if="error" class="text-sm text-red-600 lg:col-span-3">{{ error }}</p>
      </form>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="flex items-center justify-between border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">客户端列表</h2>
        <span class="text-sm text-slate-500">在线 {{ onlineClients }} / 总数 {{ clients.length }}</span>
      </div>
      <div class="overflow-x-auto">
        <table class="w-full min-w-[980px] text-left text-sm">
          <thead class="bg-slate-50 text-xs text-slate-500">
            <tr>
              <th class="px-5 py-3">客户端 ID</th>
              <th class="px-5 py-3">psk</th>
              <th class="px-5 py-3">状态</th>
              <th class="px-5 py-3">备注</th>
              <th class="px-5 py-3">已用流量</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="client in clients" :key="client.id">
              <td class="px-5 py-3 font-mono text-slate-900">{{ client.id }}</td>
              <td class="max-w-[360px] break-all px-5 py-3 font-mono text-xs text-slate-600">
                {{ client.psk }}
              </td>
              <td class="px-5 py-3">
                <StatusBadge :enabled="client.online" enabled-text="在线" disabled-text="离线" />
              </td>
              <td class="px-5 py-3 text-slate-600">{{ client.remark || "-" }}</td>
              <td class="px-5 py-3 text-slate-600">
                <div class="font-medium text-slate-900">
                  {{ formatBytes(client.rx_bytes + client.tx_bytes) }}
                </div>
                <div class="mt-1 text-xs text-slate-500">
                  接收 {{ formatBytes(client.rx_bytes) }} / 发送 {{ formatBytes(client.tx_bytes) }}
                </div>
              </td>
            </tr>
            <tr v-if="clients.length === 0">
              <td class="px-5 py-8 text-center text-slate-500" colspan="5">暂无客户端</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
