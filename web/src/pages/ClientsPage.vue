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
  vkey: "",
  remark: "",
  enabled: true,
  max_connections: null as number | null,
  compress: false,
  encrypt: false,
});

const onlineClients = computed(() => props.clients.filter((client) => client.online).length);

function submit() {
  emit("create", {
    vkey: form.value.vkey.trim() || null,
    remark: form.value.remark.trim() || null,
    enabled: form.value.enabled,
    max_connections: form.value.max_connections || null,
    compress: form.value.compress,
    encrypt: form.value.encrypt,
  });
  form.value = {
    vkey: "",
    remark: "",
    enabled: true,
    max_connections: null,
    compress: false,
    encrypt: false,
  };
}
</script>

<template>
  <section class="space-y-5">
    <div class="rounded border border-slate-200 bg-white">
      <div class="border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">创建客户端</h2>
      </div>
      <form class="grid gap-4 p-5 lg:grid-cols-4" @submit.prevent="submit">
        <label class="block">
          <span class="text-sm text-slate-600">认证密钥 vkey</span>
          <input
            v-model="form.vkey"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="留空自动生成 UUID"
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">备注</span>
          <input v-model="form.remark" class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm" />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">最大连接数</span>
          <input
            v-model.number="form.max_connections"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
            min="1"
            placeholder="不限"
            type="number"
          />
        </label>
        <div class="flex flex-wrap items-end gap-4">
          <label class="flex items-center gap-2 text-sm"><input v-model="form.enabled" type="checkbox" />启用</label>
          <label class="flex items-center gap-2 text-sm"><input v-model="form.compress" type="checkbox" />压缩</label>
          <label class="flex items-center gap-2 text-sm"><input v-model="form.encrypt" type="checkbox" />加密</label>
          <button
            class="ml-auto rounded bg-[#18c6a3] px-4 py-2 text-sm font-medium text-white hover:bg-[#13ad8e] disabled:bg-slate-400"
            :disabled="creating"
            type="submit"
          >
            {{ creating ? "创建中" : "新增客户端" }}
          </button>
        </div>
        <p v-if="error" class="text-sm text-red-600 lg:col-span-4">{{ error }}</p>
      </form>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="flex items-center justify-between border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">客户端列表</h2>
        <span class="text-sm text-slate-500">在线 {{ onlineClients }} / 总数 {{ clients.length }}</span>
      </div>
      <div class="overflow-x-auto">
        <table class="w-full min-w-[920px] text-left text-sm">
          <thead class="bg-slate-50 text-xs text-slate-500">
            <tr>
              <th class="px-5 py-3">客户端 ID</th>
              <th class="px-5 py-3">vkey</th>
              <th class="px-5 py-3">状态</th>
              <th class="px-5 py-3">备注</th>
              <th class="px-5 py-3">连接上限</th>
              <th class="px-5 py-3">能力</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="client in clients" :key="client.id">
              <td class="px-5 py-3 font-mono text-slate-900">{{ client.id }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ client.vkey }}</td>
              <td class="px-5 py-3">
                <StatusBadge :enabled="client.online" enabled-text="在线" disabled-text="离线" />
              </td>
              <td class="px-5 py-3 text-slate-600">{{ client.remark || "-" }}</td>
              <td class="px-5 py-3 text-slate-600">{{ client.max_connections ?? "不限" }}</td>
              <td class="px-5 py-3 text-slate-600">
                压缩 {{ client.compress ? "开" : "关" }} / 加密 {{ client.encrypt ? "开" : "关" }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
