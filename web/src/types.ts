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
  rx_bytes: number;
  tx_bytes: number;
};

export type TunnelResponse = {
  id: string;
  client_id: string;
  mode: "tcp" | "udp";
  listen: string;
  target: string | null;
  enabled: boolean;
};

export type CreateTunnelPayload = {
  id: string | null;
  client_id: string;
  mode: "tcp" | "udp";
  listen: string;
  target: string;
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
  active_connections: number;
};

export type MenuKey =
  | "dashboard"
  | "clients"
  | "tcp"
  | "udp"
  | "http"
  | "socks";

export type MenuItem = {
  key: MenuKey;
  label: string;
  to: string;
  icon: Component;
};

export type ConsoleData = {
  status: StatusResponse;
  clients: ClientResponse[];
  tunnels: TunnelResponse[];
  proxy: ProxyResponse;
  proxyAccounts: ProxyAccountResponse[];
};

export type AuthStatusResponse = {
  authenticated: boolean;
  username: string | null;
  two_factor_enabled: boolean;
  security_key_available: boolean;
};

export type LoginPayload = {
  username: string;
  password: string;
  otp_code: string | null;
};

export type LoginResponse = {
  authenticated: boolean;
  requires_2fa: boolean;
  username: string | null;
  security_key_available: boolean;
};

export type CreateClientPayload = {
  psk: string | null;
  remark: string | null;
  enabled: boolean;
};

export type CreateProxyAccountPayload = {
  kind: "http" | "socks5";
  client_id: string;
  username: string | null;
  password: string | null;
  enabled: boolean;
  remark: string | null;
};
