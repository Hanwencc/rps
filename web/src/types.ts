import type { Component } from "vue";

export type StatusResponse = {
  bridge_addr: string;
  web_addr: string;
  online_clients: number;
  configured_clients: number;
  enabled_tunnels: number;
  http_proxy_enabled: boolean;
  socks5_enabled: boolean;
};

export type ClientResponse = {
  id: string;
  psk: string;
  enabled: boolean;
  online: boolean;
  remark: string | null;
  max_connections: number | null;
  compress: boolean;
  encrypt: boolean;
};

export type TunnelResponse = {
  id: string;
  client_id: string;
  mode: "tcp" | "udp";
  listen: string;
  target: string | null;
  enabled: boolean;
};

export type ProxyListenConfig = {
  listen: string;
  client_id: string;
  enabled: boolean;
};

export type ProxyResponse = {
  http_proxy: ProxyListenConfig | null;
  socks5: ProxyListenConfig | null;
};

export type ProxyAccountResponse = {
  id: string;
  kind: "http" | "socks5";
  client_id: string;
  username: string;
  password: string;
  enabled: boolean;
  remark: string | null;
};

export type MenuKey =
  | "dashboard"
  | "clients"
  | "dns"
  | "tcp"
  | "udp"
  | "http"
  | "socks"
  | "secret"
  | "p2p"
  | "files"
  | "settings"
  | "help";

export type MenuItem = {
  key: MenuKey;
  label: string;
  icon: Component;
};

export type ConsoleData = {
  status: StatusResponse;
  clients: ClientResponse[];
  tunnels: TunnelResponse[];
  proxy: ProxyResponse;
  proxyAccounts: ProxyAccountResponse[];
};

export type CreateClientPayload = {
  psk: string | null;
  remark: string | null;
  enabled: boolean;
  max_connections: number | null;
  compress: boolean;
  encrypt: boolean;
};

export type CreateProxyAccountPayload = {
  kind: "http" | "socks5";
  client_id: string;
  username: string | null;
  password: string | null;
  enabled: boolean;
  remark: string | null;
};
