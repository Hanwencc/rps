<script setup lang="ts">
import { computed, ref } from "vue";
import { Pencil, Save, Trash2, X } from "lucide-vue-next";
import StatusBadge from "../components/StatusBadge.vue";
import type { ClientResponse, CreateClientPayload, UpdateClientPayload } from "../types";

const props = defineProps<{
  clients: ClientResponse[];
  creating: boolean;
  deletingId: string | null;
  savingId: string | null;
  error: string | null;
}>();

const emit = defineEmits<{
  create: [payload: CreateClientPayload];
  update: [id: string, payload: UpdateClientPayload];
  delete: [id: string];
}>();

const form = ref({
  psk: "",
  remark: "",
  enabled: true,
});
const editingId = ref<string | null>(null);
const editForm = ref({
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

function beginEdit(client: ClientResponse) {
  editingId.value = client.id;
  editForm.value = {
    psk: client.psk,
    remark: client.remark || "",
    enabled: client.enabled,
  };
}

function cancelEdit() {
  editingId.value = null;
}

function saveEdit(client: ClientResponse) {
  emit("update", client.id, {
    psk: editForm.value.psk.trim(),
    remark: editForm.value.remark.trim() || null,
    enabled: editForm.value.enabled,
  });
  editingId.value = null;
}

function confirmDelete(client: ClientResponse) {
  const label = client.remark ? `${client.remark} (${client.id})` : client.id;
  if (window.confirm(`确认删除客户端 ${label}？删除前请确保没有隧道或代理账号引用它。`)) {
    emit("delete", client.id);
  }
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
              <th class="px-5 py-3">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <template v-for="client in clients" :key="client.id">
              <tr>
                <td class="px-5 py-3 font-mono text-slate-900">{{ client.id }}</td>
                <td class="max-w-[360px] break-all px-5 py-3 font-mono text-xs text-slate-600">
                  {{ client.psk }}
                </td>
                <td class="px-5 py-3">
                  <div class="space-y-1">
                    <StatusBadge :enabled="client.online" enabled-text="在线" disabled-text="离线" />
                    <div class="text-xs" :class="client.enabled ? 'text-emerald-600' : 'text-slate-500'">
                      {{ client.enabled ? "已启用" : "已停用" }}
                    </div>
                  </div>
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
                <td class="px-5 py-3">
                  <div class="flex flex-wrap gap-2">
                    <button
                      class="inline-flex items-center gap-1 rounded border border-slate-200 px-3 py-1.5 text-xs font-medium text-slate-600 hover:bg-slate-50 disabled:cursor-not-allowed disabled:text-slate-400"
                      :disabled="savingId === client.id"
                      type="button"
                      @click="beginEdit(client)"
                    >
                      <Pencil :size="14" />
                      编辑
                    </button>
                    <button
                      class="inline-flex items-center gap-1 rounded border border-red-200 px-3 py-1.5 text-xs font-medium text-red-600 hover:bg-red-50 disabled:cursor-not-allowed disabled:border-slate-200 disabled:text-slate-400"
                      :disabled="deletingId === client.id || savingId === client.id"
                      type="button"
                      @click="confirmDelete(client)"
                    >
                      <Trash2 :size="14" />
                      {{ deletingId === client.id ? "删除中" : "删除" }}
                    </button>
                  </div>
                </td>
              </tr>
              <tr v-if="editingId === client.id" class="bg-slate-50">
                <td class="px-5 py-4" colspan="6">
                  <form class="grid gap-4 lg:grid-cols-[2fr_1fr_auto]" @submit.prevent="saveEdit(client)">
                    <label class="block">
                      <span class="text-sm text-slate-600">认证密钥 psk</span>
                      <input
                        v-model="editForm.psk"
                        class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
                        required
                      />
                    </label>
                    <label class="block">
                      <span class="text-sm text-slate-600">备注</span>
                      <input
                        v-model="editForm.remark"
                        class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
                      />
                    </label>
                    <div class="flex items-end gap-3">
                      <label class="flex items-center gap-2 pb-2 text-sm">
                        <input v-model="editForm.enabled" type="checkbox" />
                        启用
                      </label>
                      <button
                        class="inline-flex items-center gap-1 rounded bg-[#18c6a3] px-4 py-2 text-sm font-medium text-white hover:bg-[#13ad8e] disabled:bg-slate-400"
                        :disabled="savingId === client.id"
                        type="submit"
                      >
                        <Save :size="15" />
                        {{ savingId === client.id ? "保存中" : "保存" }}
                      </button>
                      <button
                        class="inline-flex items-center gap-1 rounded border border-slate-200 px-4 py-2 text-sm font-medium text-slate-600 hover:bg-white"
                        type="button"
                        @click="cancelEdit"
                      >
                        <X :size="15" />
                        取消
                      </button>
                    </div>
                  </form>
                </td>
              </tr>
            </template>
            <tr v-if="clients.length === 0">
              <td class="px-5 py-8 text-center text-slate-500" colspan="6">暂无客户端</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
