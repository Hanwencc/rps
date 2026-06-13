import { reactive } from "vue";
import { authStatus } from "./api";

export const authState = reactive({
  checked: false,
  checking: false,
  authenticated: false,
  username: null as string | null,
  twoFactorEnabled: false,
  securityKeyAvailable: false,
});

let pendingCheck: Promise<boolean> | null = null;

export async function ensureAuthStatus(): Promise<boolean> {
  if (authState.checked) {
    return authState.authenticated;
  }
  if (pendingCheck) {
    return pendingCheck;
  }

  authState.checking = true;
  pendingCheck = authStatus()
    .then((status) => {
      applyAuthStatus({
        authenticated: status.authenticated,
        username: status.username,
        twoFactorEnabled: status.two_factor_enabled,
        securityKeyAvailable: status.security_key_available,
      });
      return authState.authenticated;
    })
    .catch(() => {
      clearAuth();
      return false;
    })
    .finally(() => {
      authState.checked = true;
      authState.checking = false;
      pendingCheck = null;
    });

  return pendingCheck;
}

export function applyAuthStatus(input: {
  authenticated: boolean;
  username: string | null;
  twoFactorEnabled?: boolean;
  securityKeyAvailable: boolean;
}) {
  authState.authenticated = input.authenticated;
  authState.username = input.username;
  authState.twoFactorEnabled = input.twoFactorEnabled ?? authState.twoFactorEnabled;
  authState.securityKeyAvailable = input.securityKeyAvailable;
  authState.checked = true;
}

export function clearAuth() {
  authState.authenticated = false;
  authState.username = null;
  authState.securityKeyAvailable = false;
  authState.checked = true;
}
