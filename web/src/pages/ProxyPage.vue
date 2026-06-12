<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Activity, Trash2 } from "lucide-vue-next";
import StatusBadge from "../components/StatusBadge.vue";
import type {
  ClientResponse,
  CreateProxyAccountPayload,
  ProxyAccountResponse,
  ProxyListenConfig,
} from "../types";

const props = defineProps<{
  kind: "http" | "socks5";
  listener: ProxyListenConfig | null;
  clients: ClientResponse[];
  accounts: ProxyAccountResponse[];
  creating: boolean;
  deletingId: string | null;
  error: string | null;
}>();

const emit = defineEmits<{
  create: [payload: CreateProxyAccountPayload];
  delete: [id: string];
}>();

const form = ref({
  client_id: "",
  username: "",
  password: "",
  enabled: true,
  remark: "",
});

const title = computed(() => (props.kind === "http" ? "HTTP 代理" : "SOCKS 代理"));
const protocolName = computed(() => (props.kind === "http" ? "HTTP 正向代理" : "SOCKS5 代理"));
const filteredAccounts = computed(() => props.accounts.filter((account) => account.kind === props.kind));

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
    kind: props.kind,
    client_id: form.value.client_id,
    username: form.value.username.trim() || null,
    password: form.value.password.trim() || null,
    enabled: form.value.enabled,
    remark: form.value.remark.trim() || null,
  });
  form.value.username = "";
  form.value.password = "";
  form.value.remark = "";
  form.value.enabled = true;
}

function confirmDelete(account: ProxyAccountResponse) {
  if (window.confirm(`确认删除代理账号 ${account.username}？删除后新的代理认证会立即失效。`)) {
    emit("delete", account.id);
  }
}
</script>

<template>
  <section class="space-y-5">
    <div class="grid gap-5 xl:grid-cols-[1fr_360px]">
      <div class="rounded border border-slate-200 bg-white">
        <div class="border-b border-slate-200 px-5 py-4">
          <h2 class="font-semibold text-slate-900">{{ title }}</h2>
        </div>
        <div class="overflow-x-auto">
          <table class="w-full min-w-[680px] text-left text-sm">
            <thead class="bg-slate-50 text-xs text-slate-500">
              <tr>
                <th class="px-5 py-3">类型</th>
                <th class="px-5 py-3">监听地址</th>
                <th class="px-5 py-3">默认客户端</th>
                <th class="px-5 py-3">状态</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td class="px-5 py-3 font-semibold text-slate-900">{{ protocolName }}</td>
                <td class="px-5 py-3 font-mono text-slate-600">{{ listener?.listen || "-" }}</td>
                <td class="px-5 py-3 font-mono text-slate-600">{{ listener?.client_id || "-" }}</td>
                <td class="px-5 py-3">
                  <StatusBadge :enabled="Boolean(listener?.enabled)" />
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <div class="rounded border border-slate-200 bg-white p-5">
        <div class="flex items-center gap-3">
          <Activity class="text-[#18c6a3]" :size="22" />
          <h3 class="font-semibold text-slate-900">代理说明</h3>
        </div>
        <p class="mt-4 text-sm leading-6 text-slate-600">
          {{
            kind === "http"
              ? "HTTP 代理使用单一监听端口。新增账号后，访问者需要使用 Basic 代理认证，认证通过后按账号绑定的客户端转发。"
              : "SOCKS5 使用单一监听端口。新增账号后，握手阶段要求 username/password 认证，并支持 CONNECT 和 UDP ASSOCIATE。"
          }}
        </p>
      </div>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">新增{{ title }}账号</h2>
      </div>
      <form class="grid gap-4 p-5 lg:grid-cols-5" @submit.prevent="submit">
        <label class="block">
          <span class="text-sm text-slate-600">绑定客户端</span>
          <select
            v-model="form.client_id"
            class="mt-1 w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm"
          >
            <option v-for="client in clients" :key="client.id" :value="client.id">
              {{ client.id }}
            </option>
          </select>
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">账号</span>
          <input
            v-model="form.username"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="留空随机生成"
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">密码</span>
          <input
            v-model="form.password"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            placeholder="留空随机生成"
          />
        </label>
        <label class="block">
          <span class="text-sm text-slate-600">备注</span>
          <input v-model="form.remark" class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm" />
        </label>
        <div class="flex items-end gap-4">
          <label class="flex items-center gap-2 pb-2 text-sm"><input v-model="form.enabled" type="checkbox" />启用</label>
          <button
            class="ml-auto rounded bg-[#18c6a3] px-4 py-2 text-sm font-medium text-white hover:bg-[#13ad8e] disabled:bg-slate-400"
            :disabled="creating || clients.length === 0"
            type="submit"
          >
            {{ creating ? "创建中" : "新增账号" }}
          </button>
        </div>
        <p v-if="error" class="text-sm text-red-600 lg:col-span-5">{{ error }}</p>
      </form>
    </div>

    <div class="rounded border border-slate-200 bg-white">
      <div class="flex items-center justify-between border-b border-slate-200 px-5 py-4">
        <h2 class="font-semibold text-slate-900">账号列表</h2>
        <span class="text-sm text-slate-500">共 {{ filteredAccounts.length }} 条</span>
      </div>
      <div class="overflow-x-auto">
        <table class="w-full min-w-[860px] text-left text-sm">
          <thead class="bg-slate-50 text-xs text-slate-500">
            <tr>
              <th class="px-5 py-3">账号 ID</th>
              <th class="px-5 py-3">客户端</th>
              <th class="px-5 py-3">账号</th>
              <th class="px-5 py-3">密码</th>
              <th class="px-5 py-3">状态</th>
              <th class="px-5 py-3">当前连接</th>
              <th class="px-5 py-3">备注</th>
              <th class="px-5 py-3">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-if="filteredAccounts.length === 0">
              <td class="px-5 py-6 text-center text-slate-500" colspan="8">
                暂无账号。未创建账号时，该代理端口保持无认证兼容模式。
              </td>
            </tr>
            <tr v-for="account in filteredAccounts" :key="account.id">
              <td class="px-5 py-3 font-mono text-slate-900">{{ account.id }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ account.client_id }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ account.username }}</td>
              <td class="px-5 py-3 font-mono text-slate-600">{{ account.password }}</td>
              <td class="px-5 py-3"><StatusBadge :enabled="account.enabled" /></td>
              <td class="px-5 py-3 font-mono text-slate-900">{{ account.active_connections }}</td>
              <td class="px-5 py-3 text-slate-600">{{ account.remark || "-" }}</td>
              <td class="px-5 py-3">
                <button
                  class="inline-flex items-center gap-1 rounded border border-red-200 px-3 py-1.5 text-xs font-medium text-red-600 hover:bg-red-50 disabled:cursor-not-allowed disabled:border-slate-200 disabled:text-slate-400"
                  :disabled="deletingId === account.id"
                  type="button"
                  @click="confirmDelete(account)"
                >
                  <Trash2 :size="14" />
                  {{ deletingId === account.id ? "删除中" : "删除" }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
