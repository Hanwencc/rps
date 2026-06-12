<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  HttpError,
  authStatus,
  createClient,
  createProxyAccount,
  createTunnel,
  loadConsoleData,
  login,
} from "./api";
import PageHeader from "./components/PageHeader.vue";
import Sidebar from "./components/Sidebar.vue";
import ClientsPage from "./pages/ClientsPage.vue";
import DashboardPage from "./pages/DashboardPage.vue";
import LoginPage from "./pages/LoginPage.vue";
import ProxyPage from "./pages/ProxyPage.vue";
import TunnelsPage from "./pages/TunnelsPage.vue";
import type {
  ClientResponse,
  CreateClientPayload,
  CreateProxyAccountPayload,
  CreateTunnelPayload,
  LoginPayload,
  MenuKey,
  ProxyAccountResponse,
  ProxyResponse,
  StatusResponse,
  TunnelResponse,
} from "./types";

const menuLabels: Record<MenuKey, string> = {
  dashboard: "仪表盘",
  clients: "客户端",
  tcp: "TCP 隧道",
  udp: "UDP 隧道",
  http: "HTTP 代理",
  socks: "SOCKS 代理",
};

const activeMenu = ref<MenuKey>("dashboard");
const authenticated = ref(false);
const authLoading = ref(true);
const loginLoading = ref(false);
const loginError = ref<string | null>(null);
const requires2fa = ref(false);
const securityKeyAvailable = ref(false);
const status = ref<StatusResponse | null>(null);
const clients = ref<ClientResponse[]>([]);
const tunnels = ref<TunnelResponse[]>([]);
const proxy = ref<ProxyResponse | null>(null);
const proxyAccounts = ref<ProxyAccountResponse[]>([]);
const loading = ref(true);
const error = ref<string | null>(null);
const createClientError = ref<string | null>(null);
const createTunnelError = ref<string | null>(null);
const createProxyError = ref<string | null>(null);
const creatingClient = ref(false);
const creatingTunnel = ref(false);
const creatingProxy = ref(false);
const lastUpdated = ref<string | null>(null);
let refreshTimer: number | undefined;

const activeTitle = computed(() => menuLabels[activeMenu.value]);
const tcpTunnels = computed(() => tunnels.value.filter((tunnel) => tunnel.mode === "tcp"));
const udpTunnels = computed(() => tunnels.value.filter((tunnel) => tunnel.mode === "udp"));

async function refresh() {
  if (!authenticated.value) {
    return;
  }
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
    if (err instanceof HttpError && err.status === 401) {
      authenticated.value = false;
      stopRefreshTimer();
      return;
    }
    error.value = err instanceof Error ? err.message : "加载控制端状态失败";
  } finally {
    loading.value = false;
  }
}

async function handleLogin(payload: LoginPayload) {
  loginError.value = null;
  loginLoading.value = true;
  try {
    const response = await login(payload);
    securityKeyAvailable.value = response.security_key_available;
    if (response.requires_2fa) {
      requires2fa.value = true;
      return;
    }
    authenticated.value = response.authenticated;
    requires2fa.value = false;
    loading.value = true;
    startRefreshTimer();
    await refresh();
  } catch (err) {
    loginError.value = err instanceof Error ? err.message : "登录失败";
  } finally {
    loginLoading.value = false;
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

async function handleCreateTunnel(payload: CreateTunnelPayload) {
  createTunnelError.value = null;
  creatingTunnel.value = true;
  try {
    await createTunnel(payload);
    await refresh();
  } catch (err) {
    createTunnelError.value = err instanceof Error ? err.message : "创建隧道失败";
  } finally {
    creatingTunnel.value = false;
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

function startRefreshTimer() {
  stopRefreshTimer();
  refreshTimer = window.setInterval(refresh, 5000);
}

function stopRefreshTimer() {
  if (refreshTimer !== undefined) {
    window.clearInterval(refreshTimer);
    refreshTimer = undefined;
  }
}

onMounted(async () => {
  try {
    const auth = await authStatus();
    authenticated.value = auth.authenticated;
    securityKeyAvailable.value = auth.security_key_available;
    if (auth.authenticated) {
      startRefreshTimer();
      await refresh();
    }
  } catch (err) {
    loginError.value = err instanceof Error ? err.message : "读取登录状态失败";
  } finally {
    authLoading.value = false;
    loading.value = false;
  }
});

onUnmounted(stopRefreshTimer);
</script>

<template>
  <LoginPage
    v-if="!authenticated"
    :error="loginError"
    :loading="authLoading || loginLoading"
    :requires2fa="requires2fa"
    :security-key-available="securityKeyAvailable"
    @login="handleLogin"
  />

  <main v-else class="min-h-screen bg-[#eef1f5] text-slate-800">
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
              :clients="clients"
              :creating="creatingTunnel"
              :error="createTunnelError"
              mode="tcp"
              title="TCP 隧道"
              :tunnels="tcpTunnels"
              @create="handleCreateTunnel"
            />
            <TunnelsPage
              v-else-if="activeMenu === 'udp'"
              :clients="clients"
              :creating="creatingTunnel"
              :error="createTunnelError"
              mode="udp"
              title="UDP 隧道"
              :tunnels="udpTunnels"
              @create="handleCreateTunnel"
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
              v-else
              :accounts="proxyAccounts"
              :clients="clients"
              :creating="creatingProxy"
              :error="createProxyError"
              kind="socks5"
              :listener="proxy?.socks5 ?? null"
              @create="handleCreateProxyAccount"
            />
          </template>
        </section>
      </div>
    </div>
  </main>
</template>
