<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RouterView, useRoute } from "vue-router";
import {
  HttpError,
  authStatus,
  createClient,
  createProxyAccount,
  createTunnel,
  deleteClient,
  deleteProxyAccount,
  deleteTunnel,
  loadConsoleData,
  login,
  updateClient,
  updateProxyAccount,
  updateTunnel,
} from "./api";
import PageHeader from "./components/PageHeader.vue";
import Sidebar from "./components/Sidebar.vue";
import LoginPage from "./pages/LoginPage.vue";
import type {
  ClientResponse,
  ConsoleData,
  CreateClientPayload,
  CreateProxyAccountPayload,
  CreateTunnelPayload,
  LoginPayload,
  MenuKey,
  ProxyAccountResponse,
  ProxyResponse,
  StatusResponse,
  TunnelResponse,
  UpdateClientPayload,
  UpdateProxyAccountPayload,
  UpdateTunnelPayload,
} from "./types";

type RouteCreatePayload = CreateClientPayload | CreateTunnelPayload | CreateProxyAccountPayload;
type RouteUpdatePayload = UpdateClientPayload | UpdateTunnelPayload | UpdateProxyAccountPayload;

const menuLabels: Record<MenuKey, string> = {
  dashboard: "仪表盘",
  clients: "客户端",
  tcp: "TCP 隧道",
  udp: "UDP 隧道",
  http: "HTTP 代理",
  socks: "SOCKS 代理",
};

const route = useRoute();
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
const deletingClientId = ref<string | null>(null);
const deletingTunnelId = ref<string | null>(null);
const deletingProxyAccountId = ref<string | null>(null);
const savingClientId = ref<string | null>(null);
const savingTunnelId = ref<string | null>(null);
const savingProxyAccountId = ref<string | null>(null);
const lastUpdated = ref<string | null>(null);
let events: EventSource | null = null;

const activeMenu = computed<MenuKey>(() => {
  const menu = route.meta.menu;
  return typeof menu === "string" && menu in menuLabels ? (menu as MenuKey) : "dashboard";
});
const activeTitle = computed(() => {
  const title = route.meta.title;
  return typeof title === "string" ? title : menuLabels[activeMenu.value];
});
const tcpTunnels = computed(() => tunnels.value.filter((tunnel) => tunnel.mode === "tcp"));
const udpTunnels = computed(() => tunnels.value.filter((tunnel) => tunnel.mode === "udp"));
const pageProps = computed<Record<string, unknown>>(() => {
  switch (activeMenu.value) {
    case "clients":
      return {
        clients: clients.value,
        creating: creatingClient.value,
        deletingId: deletingClientId.value,
        error: createClientError.value,
        savingId: savingClientId.value,
      };
    case "tcp":
      return {
        clients: clients.value,
        creating: creatingTunnel.value,
        deletingId: deletingTunnelId.value,
        error: createTunnelError.value,
        mode: "tcp",
        savingId: savingTunnelId.value,
        title: "TCP 隧道",
        tunnels: tcpTunnels.value,
      };
    case "udp":
      return {
        clients: clients.value,
        creating: creatingTunnel.value,
        deletingId: deletingTunnelId.value,
        error: createTunnelError.value,
        mode: "udp",
        savingId: savingTunnelId.value,
        title: "UDP 隧道",
        tunnels: udpTunnels.value,
      };
    case "http":
      return {
        accounts: proxyAccounts.value,
        clients: clients.value,
        creating: creatingProxy.value,
        deletingId: deletingProxyAccountId.value,
        error: createProxyError.value,
        kind: "http",
        listener: proxy.value?.http_proxy ?? null,
        savingId: savingProxyAccountId.value,
      };
    case "socks":
      return {
        accounts: proxyAccounts.value,
        clients: clients.value,
        creating: creatingProxy.value,
        deletingId: deletingProxyAccountId.value,
        error: createProxyError.value,
        kind: "socks5",
        listener: proxy.value?.socks5 ?? null,
        savingId: savingProxyAccountId.value,
      };
    default:
      return {
        clients: clients.value,
        status: status.value,
        tunnels: tunnels.value,
      };
  }
});

async function refresh() {
  if (!authenticated.value) {
    return;
  }
  error.value = null;
  try {
    const data = await loadConsoleData();
    applyConsoleData(data);
  } catch (err) {
    if (err instanceof HttpError && err.status === 401) {
      authenticated.value = false;
      stopRealtimeEvents();
      return;
    }
    error.value = err instanceof Error ? err.message : "加载控制端状态失败";
  } finally {
    loading.value = false;
  }
}

function applyConsoleData(data: ConsoleData) {
  status.value = data.status;
  clients.value = data.clients;
  tunnels.value = data.tunnels;
  proxy.value = data.proxy;
  proxyAccounts.value = data.proxyAccounts;
  lastUpdated.value = new Date().toLocaleTimeString();
  loading.value = false;
  error.value = null;
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
    startRealtimeEvents();
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

async function handleDeleteClient(id: string) {
  createClientError.value = null;
  deletingClientId.value = id;
  try {
    await deleteClient(id);
    await refresh();
  } catch (err) {
    createClientError.value = err instanceof Error ? err.message : "删除客户端失败";
  } finally {
    deletingClientId.value = null;
  }
}

async function handleDeleteTunnel(id: string) {
  createTunnelError.value = null;
  deletingTunnelId.value = id;
  try {
    await deleteTunnel(id);
    await refresh();
  } catch (err) {
    createTunnelError.value = err instanceof Error ? err.message : "删除隧道失败";
  } finally {
    deletingTunnelId.value = null;
  }
}

async function handleDeleteProxyAccount(id: string) {
  createProxyError.value = null;
  deletingProxyAccountId.value = id;
  try {
    await deleteProxyAccount(id);
    await refresh();
  } catch (err) {
    createProxyError.value = err instanceof Error ? err.message : "删除代理账号失败";
  } finally {
    deletingProxyAccountId.value = null;
  }
}

async function handleUpdateClient(id: string, payload: UpdateClientPayload) {
  createClientError.value = null;
  savingClientId.value = id;
  try {
    const updated = await updateClient(id, payload);
    clients.value = clients.value.map((client) => (client.id === id ? updated : client));
    await refresh();
  } catch (err) {
    createClientError.value = err instanceof Error ? err.message : "保存客户端失败";
  } finally {
    savingClientId.value = null;
  }
}

async function handleUpdateTunnel(id: string, payload: UpdateTunnelPayload) {
  createTunnelError.value = null;
  savingTunnelId.value = id;
  try {
    const updated = await updateTunnel(id, payload);
    tunnels.value = tunnels.value.map((tunnel) => (tunnel.id === id ? updated : tunnel));
    await refresh();
  } catch (err) {
    createTunnelError.value = err instanceof Error ? err.message : "保存隧道失败";
  } finally {
    savingTunnelId.value = null;
  }
}

async function handleUpdateProxyAccount(id: string, payload: UpdateProxyAccountPayload) {
  createProxyError.value = null;
  savingProxyAccountId.value = id;
  try {
    const updated = await updateProxyAccount(id, payload);
    proxyAccounts.value = proxyAccounts.value.map((account) => (account.id === id ? updated : account));
    await refresh();
  } catch (err) {
    createProxyError.value = err instanceof Error ? err.message : "保存代理账号失败";
  } finally {
    savingProxyAccountId.value = null;
  }
}

async function handleRouteCreate(payload: RouteCreatePayload) {
  switch (activeMenu.value) {
    case "clients":
      await handleCreateClient(payload as CreateClientPayload);
      break;
    case "tcp":
    case "udp":
      await handleCreateTunnel(payload as CreateTunnelPayload);
      break;
    case "http":
    case "socks":
      await handleCreateProxyAccount(payload as CreateProxyAccountPayload);
      break;
  }
}

async function handleRouteUpdate(id: string, payload: RouteUpdatePayload) {
  switch (activeMenu.value) {
    case "clients":
      await handleUpdateClient(id, payload as UpdateClientPayload);
      break;
    case "tcp":
    case "udp":
      await handleUpdateTunnel(id, payload as UpdateTunnelPayload);
      break;
    case "http":
    case "socks":
      await handleUpdateProxyAccount(id, payload as UpdateProxyAccountPayload);
      break;
  }
}

async function handleRouteDelete(id: string) {
  switch (activeMenu.value) {
    case "clients":
      await handleDeleteClient(id);
      break;
    case "tcp":
    case "udp":
      await handleDeleteTunnel(id);
      break;
    case "http":
    case "socks":
      await handleDeleteProxyAccount(id);
      break;
  }
}

function startRealtimeEvents() {
  stopRealtimeEvents();
  events = new EventSource("/api/events", { withCredentials: true });
  events.addEventListener("snapshot", (event) => {
    try {
      applyConsoleData(JSON.parse(event.data));
    } catch (err) {
      error.value = err instanceof Error ? err.message : "实时数据解析失败";
    }
  });
  events.addEventListener("auth_expired", () => {
    authenticated.value = false;
    stopRealtimeEvents();
  });
  events.addEventListener("stream_error", (event) => {
    error.value = event.data || "实时数据推送失败";
  });
  events.addEventListener("error", () => {
    if (events?.readyState === EventSource.CLOSED) {
      error.value = "实时连接已断开";
    }
  });
}

function stopRealtimeEvents() {
  events?.close();
  events = null;
}

onMounted(async () => {
  try {
    const auth = await authStatus();
    authenticated.value = auth.authenticated;
    securityKeyAvailable.value = auth.security_key_available;
    if (auth.authenticated) {
      startRealtimeEvents();
      await refresh();
    }
  } catch (err) {
    loginError.value = err instanceof Error ? err.message : "读取登录状态失败";
  } finally {
    authLoading.value = false;
    loading.value = false;
  }
});

onUnmounted(stopRealtimeEvents);
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
      <Sidebar :active-menu="activeMenu" />

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
            <RouterView v-slot="{ Component }">
              <component
                :is="Component"
                v-bind="pageProps"
                @create="handleRouteCreate"
                @delete="handleRouteDelete"
                @update="handleRouteUpdate"
              />
            </RouterView>
          </template>
        </section>
      </div>
    </div>
  </main>
</template>
