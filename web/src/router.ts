import { createRouter, createWebHistory, type RouteRecordRaw } from "vue-router";
import ClientsPage from "./pages/ClientsPage.vue";
import DashboardPage from "./pages/DashboardPage.vue";
import ProxyPage from "./pages/ProxyPage.vue";
import TunnelsPage from "./pages/TunnelsPage.vue";

export const routes: RouteRecordRaw[] = [
  {
    path: "/",
    redirect: "/dashboard",
  },
  {
    path: "/dashboard",
    name: "dashboard",
    component: DashboardPage,
    meta: { title: "仪表盘", menu: "dashboard" },
  },
  {
    path: "/clients",
    name: "clients",
    component: ClientsPage,
    meta: { title: "客户端", menu: "clients" },
  },
  {
    path: "/tunnels/tcp",
    name: "tcp",
    component: TunnelsPage,
    meta: { title: "TCP 隧道", menu: "tcp", mode: "tcp" },
  },
  {
    path: "/tunnels/udp",
    name: "udp",
    component: TunnelsPage,
    meta: { title: "UDP 隧道", menu: "udp", mode: "udp" },
  },
  {
    path: "/proxy/http",
    name: "http",
    component: ProxyPage,
    meta: { title: "HTTP 代理", menu: "http", kind: "http" },
  },
  {
    path: "/proxy/socks5",
    name: "socks",
    component: ProxyPage,
    meta: { title: "SOCKS 代理", menu: "socks", kind: "socks5" },
  },
  {
    path: "/:pathMatch(.*)*",
    redirect: "/dashboard",
  },
];

export const router = createRouter({
  history: createWebHistory(),
  routes,
});
