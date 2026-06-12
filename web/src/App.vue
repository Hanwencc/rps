<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { createClient, createProxyAccount, loadConsoleData } from "./api";
import PageHeader from "./components/PageHeader.vue";
import Sidebar from "./components/Sidebar.vue";
import ClientsPage from "./pages/ClientsPage.vue";
import DashboardPage from "./pages/DashboardPage.vue";
import PlaceholderPage from "./pages/PlaceholderPage.vue";
import ProxyPage from "./pages/ProxyPage.vue";
import TunnelsPage from "./pages/TunnelsPage.vue";
import type {
  ClientResponse,
  CreateClientPayload,
  CreateProxyAccountPayload,
  MenuKey,
  ProxyAccountResponse,
  ProxyResponse,
  StatusResponse,
  TunnelResponse,
} from "./types";

const menuLabels: Record<MenuKey, string> = {
  dashboard: "仪表盘",
  clients: "客户端",
  dns: "域名解析",
  tcp: "TCP 隧道",
  udp: "UDP 隧道",
  http: "HTTP 代理",
  socks: "SOCKS 代理",
  secret: "私密代理",
  p2p: "P2P 连接",
  files: "文件访问",
  settings: "全局参数",
  help: "使用说明",
};

const activeMenu = ref<MenuKey>("http");
const status = ref<StatusResponse | null>(null);
const clients = ref<ClientResponse[]>([]);
const tunnels = ref<TunnelResponse[]>([]);
const proxy = ref<ProxyResponse | null>(null);
const proxyAccounts = ref<ProxyAccountResponse[]>([]);
const loading = ref(true);
const error = ref<string | null>(null);
const createClientError = ref<string | null>(null);
const createProxyError = ref<string | null>(null);
const creatingClient = ref(false);
const creatingProxy = ref(false);
const lastUpdated = ref<string | null>(null);

const activeTitle = computed(() => menuLabels[activeMenu.value]);
const tcpTunnels = computed(() => tunnels.value.filter((tunnel) => tunnel.mode === "tcp"));
const udpTunnels = computed(() => tunnels.value.filter((tunnel) => tunnel.mode === "udp"));

async function refresh() {
  error.value = null;
  try {
    const data = await loadConsoleData();
    status.value = data.status;
    clients.value = data.clients;
    tunnels.value = data.tunnels;
    proxy.value = data.proxy;
    proxyAccounts.value = data.proxyAccounts;
    lastUpdated.value = new Date().toLocaleTimeString();
  } catch (err) {
    error.value = err instanceof Error ? err.message : "加载控制端状态失败";
  } finally {
    loading.value = false;
  }
}

async function handleCreateClient(payload: CreateClientPayload) {
  createClientError.value = null;
  creatingClient.value = true;
  try {
    await createClient(payload);
    await refresh();
  } catch (err) {
    createClientError.value = err instanceof Error ? err.message : "创建客户端失败";
  } finally {
    creatingClient.value = false;
  }
}

async function handleCreateProxyAccount(payload: CreateProxyAccountPayload) {
  createProxyError.value = null;
  creatingProxy.value = true;
  try {
    await createProxyAccount(payload);
    await refresh();
  } catch (err) {
    createProxyError.value = err instanceof Error ? err.message : "创建代理账号失败";
  } finally {
    creatingProxy.value = false;
  }
}

onMounted(() => {
  void refresh();
  window.setInterval(refresh, 5000);
});
</script>

<template>
  <main class="min-h-screen bg-[#eef1f5] text-slate-800">
    <div class="flex min-h-screen">
      <Sidebar :active-menu="activeMenu" @select="activeMenu = $event" />

      <div class="min-w-0 flex-1 md:pl-[220px]">
        <PageHeader :last-updated="lastUpdated" :title="activeTitle" @refresh="refresh" />

        <section class="p-4 md:p-7">
          <div
            v-if="error"
            class="mb-5 rounded border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
          >
            {{ error }}
          </div>

          <div v-if="loading" class="rounded border border-slate-200 bg-white p-6 text-slate-500">
            正在加载控制端状态...
          </div>

          <template v-else>
            <DashboardPage
              v-if="activeMenu === 'dashboard'"
              :clients="clients"
              :status="status"
              :tunnels="tunnels"
            />
            <ClientsPage
              v-else-if="activeMenu === 'clients'"
              :clients="clients"
              :creating="creatingClient"
              :error="createClientError"
              @create="handleCreateClient"
            />
            <TunnelsPage
              v-else-if="activeMenu === 'tcp'"
              title="TCP 隧道"
              :tunnels="tcpTunnels"
            />
            <TunnelsPage
              v-else-if="activeMenu === 'udp'"
              title="UDP 隧道"
              :tunnels="udpTunnels"
            />
            <ProxyPage
              v-else-if="activeMenu === 'http'"
              :accounts="proxyAccounts"
              :clients="clients"
              :creating="creatingProxy"
              :error="createProxyError"
              kind="http"
              :listener="proxy?.http_proxy ?? null"
              @create="handleCreateProxyAccount"
            />
            <ProxyPage
              v-else-if="activeMenu === 'socks'"
              :accounts="proxyAccounts"
              :clients="clients"
              :creating="creatingProxy"
              :error="createProxyError"
              kind="socks5"
              :listener="proxy?.socks5 ?? null"
              @create="handleCreateProxyAccount"
            />
            <PlaceholderPage v-else :title="activeTitle" />
          </template>
        </section>
      </div>
    </div>
  </main>
</template>
