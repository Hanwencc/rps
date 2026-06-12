<script setup lang="ts">
import { Gauge, Layers, Monitor, Repeat2, Server, Shuffle, UserCog } from "lucide-vue-next";
import type { MenuItem, MenuKey } from "../types";

defineProps<{
  activeMenu: MenuKey;
}>();

const emit = defineEmits<{
  select: [key: MenuKey];
}>();

const menuItems: MenuItem[] = [
  { key: "dashboard", label: "仪表盘", icon: Gauge },
  { key: "clients", label: "客户端", icon: Monitor },
  { key: "tcp", label: "TCP 隧道", icon: Repeat2 },
  { key: "udp", label: "UDP 隧道", icon: Shuffle },
  { key: "http", label: "HTTP 代理", icon: Server },
  { key: "socks", label: "SOCKS 代理", icon: Layers },
];
</script>

<template>
  <aside class="fixed inset-y-0 left-0 z-20 hidden w-[220px] bg-[#2f4356] text-[#b9c9df] md:block">
    <div class="flex h-36 flex-col justify-center px-6">
      <UserCog :size="44" class="text-slate-400" stroke-width="1.8" />
      <div class="mt-2 text-sm leading-6 text-slate-300">
        <div>管理员</div>
        <div>系统</div>
      </div>
    </div>
    <nav class="space-y-1">
      <button
        v-for="item in menuItems"
        :key="item.key"
        class="relative flex h-12 w-full items-center gap-3 px-6 text-left text-sm font-semibold transition"
        :class="
          activeMenu === item.key
            ? 'bg-[#263948] text-white before:absolute before:left-0 before:top-0 before:h-full before:w-1 before:bg-[#18c6a3]'
            : 'hover:bg-[#283b4d] hover:text-white'
        "
        type="button"
        @click="emit('select', item.key)"
      >
        <component :is="item.icon" class="text-[#a9c7f1]" :size="20" />
        <span>{{ item.label }}</span>
      </button>
    </nav>
  </aside>

  <div class="flex gap-2 overflow-x-auto border-t border-slate-100 bg-white px-4 py-2 md:hidden">
    <button
      v-for="item in menuItems"
      :key="item.key"
      class="shrink-0 rounded px-3 py-2 text-sm"
      :class="activeMenu === item.key ? 'bg-[#263948] text-white' : 'bg-slate-100 text-slate-700'"
      type="button"
      @click="emit('select', item.key)"
    >
      {{ item.label }}
    </button>
  </div>
</template>
