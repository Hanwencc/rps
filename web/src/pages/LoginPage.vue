<script setup lang="ts">
import { computed, ref } from "vue";
import { KeyRound, LockKeyhole, ShieldCheck } from "lucide-vue-next";
import type { LoginPayload } from "../types";

const props = defineProps<{
  loading: boolean;
  error: string | null;
  requires2fa: boolean;
  securityKeyAvailable: boolean;
}>();

const emit = defineEmits<{
  login: [payload: LoginPayload];
}>();

const form = ref({
  username: "admin",
  password: "",
  otp_code: "",
});

const title = computed(() => (props.requires2fa ? "双因素认证" : "登录控制台"));

function submit() {
  emit("login", {
    username: form.value.username.trim(),
    password: form.value.password,
    otp_code: form.value.otp_code.trim() || null,
  });
}
</script>

<template>
  <main class="flex min-h-screen items-center justify-center bg-[#eef1f5] px-4 py-10 text-slate-800">
    <section class="w-full max-w-xl rounded border border-slate-200 bg-white shadow-sm">
      <div class="border-b border-slate-200 bg-slate-50 px-5 py-3 text-center text-lg font-semibold text-slate-900">
        {{ title }}
      </div>

      <form class="space-y-5 p-6" @submit.prevent="submit">
        <div v-if="!requires2fa" class="flex flex-col items-center text-center">
          <LockKeyhole :size="56" class="text-slate-800" stroke-width="1.8" />
          <h1 class="mt-4 text-xl font-semibold text-slate-900">输入管理员账号密码</h1>
          <p class="mt-2 text-sm text-slate-500">登录后才能访问客户端、隧道和代理配置。</p>
        </div>

        <div v-else class="flex flex-col items-center text-center">
          <KeyRound :size="56" class="text-slate-800" stroke-width="1.8" />
          <h1 class="mt-4 text-xl font-semibold text-slate-900">输入你的安全码</h1>
          <p class="mt-2 text-sm text-slate-500">
            输入认证器中的 6 位动态验证码。安全钥匙/WebAuthn 需要 HTTPS 域名后启用。
          </p>
        </div>

        <div v-if="!requires2fa" class="grid gap-4">
          <label class="block">
            <span class="text-sm text-slate-600">账号</span>
            <input
              v-model="form.username"
              autocomplete="username"
              class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
            />
          </label>
          <label class="block">
            <span class="text-sm text-slate-600">密码</span>
            <input
              v-model="form.password"
              autocomplete="current-password"
              class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
              type="password"
            />
          </label>
        </div>

        <label v-else class="block">
          <span class="text-sm text-slate-600">动态验证码</span>
          <input
            v-model="form.otp_code"
            autocomplete="one-time-code"
            class="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-center font-mono text-lg tracking-[0.35em]"
            inputmode="numeric"
            maxlength="6"
            placeholder="000000"
          />
        </label>

        <div
          v-if="requires2fa"
          class="rounded border border-slate-200 bg-slate-50 px-4 py-3 text-sm text-slate-600"
        >
          <div class="flex items-center gap-2 font-medium text-slate-800">
            <ShieldCheck :size="18" class="text-[#18c6a3]" />
            安全钥匙
          </div>
          <p class="mt-2">
            当前页面先支持 TOTP 动态验证码。截图中的安全钥匙/Passkey 属于 WebAuthn，
            浏览器要求 HTTPS 安全上下文和固定 RP ID。
          </p>
          <p v-if="securityKeyAvailable" class="mt-2 text-[#13866f]">安全钥匙已启用。</p>
        </div>

        <p v-if="error" class="rounded border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
          {{ error }}
        </p>

        <button
          class="w-full rounded bg-[#18c6a3] px-4 py-2.5 text-sm font-semibold text-white hover:bg-[#13ad8e] disabled:bg-slate-400"
          :disabled="loading"
          type="submit"
        >
          {{ loading ? "验证中" : requires2fa ? "完成认证" : "登录" }}
        </button>
      </form>
    </section>
  </main>
</template>
