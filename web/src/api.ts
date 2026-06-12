import type {
  AuthStatusResponse,
  ClientResponse,
  ConsoleData,
  CreateClientPayload,
  CreateProxyAccountPayload,
  CreateTunnelPayload,
  LoginPayload,
  LoginResponse,
  ProxyAccountResponse,
  ProxyResponse,
  StatusResponse,
  TunnelResponse,
} from "./types";

export class HttpError extends Error {
  status: number;

  constructor(path: string, status: number, message: string) {
    super(message || `${path} 返回 ${status}`);
    this.status = status;
  }
}

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: {
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });
  if (!response.ok) {
    throw new HttpError(path, response.status, await response.text());
  }
  return (await response.json()) as T;
}

async function fetchEmpty(path: string, init?: RequestInit): Promise<void> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: {
      ...init?.headers,
    },
  });
  if (!response.ok) {
    throw new HttpError(path, response.status, await response.text());
  }
}

export async function authStatus(): Promise<AuthStatusResponse> {
  return fetchJson<AuthStatusResponse>("/api/auth/status");
}

export async function login(payload: LoginPayload): Promise<LoginResponse> {
  return fetchJson<LoginResponse>("/api/auth/login", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export async function logout(): Promise<AuthStatusResponse> {
  return fetchJson<AuthStatusResponse>("/api/auth/logout", {
    method: "POST",
  });
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
  return fetchJson<ClientResponse>("/api/clients", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export async function deleteClient(id: string): Promise<void> {
  await fetchEmpty(`/api/clients/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}

export async function createTunnel(payload: CreateTunnelPayload): Promise<TunnelResponse> {
  return fetchJson<TunnelResponse>("/api/tunnels", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export async function deleteTunnel(id: string): Promise<void> {
  await fetchEmpty(`/api/tunnels/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}

export async function createProxyAccount(
  payload: CreateProxyAccountPayload,
): Promise<ProxyAccountResponse> {
  return fetchJson<ProxyAccountResponse>("/api/proxy-accounts", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
