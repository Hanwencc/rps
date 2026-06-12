import type {
  ClientResponse,
  ConsoleData,
  CreateClientPayload,
  CreateProxyAccountPayload,
  ProxyAccountResponse,
  ProxyResponse,
  StatusResponse,
  TunnelResponse,
} from "./types";

async function fetchJson<T>(path: string): Promise<T> {
  const response = await fetch(path);
  if (!response.ok) {
    throw new Error(`${path} 返回 ${response.status}`);
  }
  return (await response.json()) as T;
}

export async function loadConsoleData(): Promise<ConsoleData> {
  const [status, clients, tunnels, proxy, proxyAccounts] = await Promise.all([
    fetchJson<StatusResponse>("/api/status"),
    fetchJson<ClientResponse[]>("/api/clients"),
    fetchJson<TunnelResponse[]>("/api/tunnels"),
    fetchJson<ProxyResponse>("/api/proxy"),
    fetchJson<ProxyAccountResponse[]>("/api/proxy-accounts"),
  ]);
  return { status, clients, tunnels, proxy, proxyAccounts };
}

export async function createClient(payload: CreateClientPayload): Promise<ClientResponse> {
  const response = await fetch("/api/clients", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  return (await response.json()) as ClientResponse;
}

export async function createProxyAccount(
  payload: CreateProxyAccountPayload,
): Promise<ProxyAccountResponse> {
  const response = await fetch("/api/proxy-accounts", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  return (await response.json()) as ProxyAccountResponse;
}
