import { createRouter, createWebHistory, type RouteRecordRaw } from "vue-router";
import ClientsPage from "./pages/ClientsPage.vue";
import DashboardPage from "./pages/DashboardPage.vue";
import LoginPage from "./pages/LoginPage.vue";
import ProxyPage from "./pages/ProxyPage.vue";
import TunnelsPage from "./pages/TunnelsPage.vue";
import { ensureAuthStatus } from "./auth";

export const routes: RouteRecordRaw[] = [
  {
    path: "/",
    redirect: "/dashboard",
  },
  {
    path: "/login",
    name: "login",
    component: LoginPage,
    meta: { public: true, title: "登录" },
  },
  {
    path: "/dashboard",
    name: "dashboard",
    component: DashboardPage,
    meta: { requiresAuth: true, title: "仪表盘", menu: "dashboard" },
  },
  {
    path: "/clients",
    name: "clients",
    component: ClientsPage,
    meta: { requiresAuth: true, title: "客户端", menu: "clients" },
  },
  {
    path: "/tunnels/tcp",
    name: "tcp",
    component: TunnelsPage,
    meta: { requiresAuth: true, title: "TCP 隧道", menu: "tcp", mode: "tcp" },
  },
  {
    path: "/tunnels/udp",
    name: "udp",
    component: TunnelsPage,
    meta: { requiresAuth: true, title: "UDP 隧道", menu: "udp", mode: "udp" },
  },
  {
    path: "/proxy/http",
    name: "http",
    component: ProxyPage,
    meta: { requiresAuth: true, title: "HTTP 代理", menu: "http", kind: "http" },
  },
  {
    path: "/proxy/socks5",
    name: "socks",
    component: ProxyPage,
    meta: { requiresAuth: true, title: "SOCKS 代理", menu: "socks", kind: "socks5" },
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

router.beforeEach(async (to) => {
  const authenticated = await ensureAuthStatus();
  if (to.meta.public) {
    if (authenticated && to.name === "login") {
      const redirect = typeof to.query.redirect === "string" ? to.query.redirect : "/dashboard";
      return redirect.startsWith("/") && !redirect.startsWith("/login") ? redirect : "/dashboard";
    }
    return true;
  }

  if (!authenticated) {
    return {
      name: "login",
      query: to.fullPath === "/dashboard" ? {} : { redirect: to.fullPath },
    };
  }

  return true;
});
