<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { PhysicalPosition, PhysicalSize, currentMonitor, getCurrentWindow } from '@tauri-apps/api/window'
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

const ORB_SIZE = 76
const MENU_WIDTH = 244
const MENU_HEIGHT = 302
const MENU_GAP = 10

const appWindow = getCurrentWindow()
const menuOpen = ref(false)
const desktopVisible = ref(false)
const autostartEnabled = ref(false)
const busy = ref(false)
const dragging = ref(false)
let unlisten: UnlistenFn | undefined
let pointerDown = { x: 0, y: 0 }
let compactPosition = new PhysicalPosition(0, 0)
let menuOpensLeft = true

const statusText = computed(() => desktopVisible.value ? '桌面已临时显示' : '桌面已收纳')

async function refreshState() {
  const state = await invoke<{ desktopVisible: boolean }>('get_desktop_state')
  desktopVisible.value = state.desktopVisible
  autostartEnabled.value = await isEnabled()
}

async function setMenu(open: boolean) {
  if (menuOpen.value === open) return

  if (open) {
    compactPosition = await appWindow.outerPosition()
    const monitor = await currentMonitor()
    if (!monitor) return

    const scale = monitor.scaleFactor
    const menuWidth = Math.round(MENU_WIDTH * scale)
    const menuHeight = Math.round(MENU_HEIGHT * scale)
    const gap = Math.round(MENU_GAP * scale)
    const orbSize = Math.round(ORB_SIZE * scale)
    const rightSpace = monitor.position.x + monitor.size.width - (compactPosition.x + orbSize)
    menuOpensLeft = rightSpace < menuWidth + gap

    const x = menuOpensLeft ? compactPosition.x - menuWidth - gap : compactPosition.x
    const y = Math.max(
      monitor.position.y,
      Math.min(compactPosition.y - Math.round(98 * scale), monitor.position.y + monitor.size.height - menuHeight),
    )

    await appWindow.setPosition(new PhysicalPosition(x, y))
    await appWindow.setSize(new PhysicalSize(menuWidth + orbSize + gap, menuHeight))
    menuOpen.value = true
    await invoke('set_menu_open', { open: true })
    await nextTick()
    return
  }

  menuOpen.value = false
  await invoke('set_menu_open', { open: false })
  await appWindow.setSize(new PhysicalSize(ORB_SIZE, ORB_SIZE))
  await appWindow.setPosition(compactPosition)
}

async function toggleMenu(event: MouseEvent) {
  event.preventDefault()
  await setMenu(!menuOpen.value)
}

function pointerStart(event: PointerEvent) {
  if (event.button !== 0 || menuOpen.value) return
  dragging.value = false
  pointerDown = { x: event.screenX, y: event.screenY }
}

async function pointerMove(event: PointerEvent) {
  if ((event.buttons & 1) !== 1 || menuOpen.value || dragging.value) return
  const distance = Math.hypot(event.screenX - pointerDown.x, event.screenY - pointerDown.y)
  if (distance < 5) return
  dragging.value = true
  await appWindow.startDragging()
}

async function pointerEnd() {
  if (menuOpen.value) return
  if (dragging.value) {
    dragging.value = false
    await snapToEdge()
    return
  }
  if (desktopVisible.value) {
    await hideDesktop()
  } else {
    await showDesktop()
  }
}

async function snapToEdge() {
  const monitor = await currentMonitor()
  if (!monitor) return

  const position = await appWindow.outerPosition()
  const scale = monitor.scaleFactor
  const orbSize = Math.round(ORB_SIZE * scale)
  const margin = Math.round(18 * scale)
  const left = monitor.position.x + margin
  const right = monitor.position.x + monitor.size.width - orbSize - margin
  const top = monitor.position.y + margin
  const bottom = monitor.position.y + monitor.size.height - orbSize - margin
  const distances = [
    { value: Math.abs(position.x - left), x: left, y: Math.max(top, Math.min(position.y, bottom)) },
    { value: Math.abs(position.x - right), x: right, y: Math.max(top, Math.min(position.y, bottom)) },
    { value: Math.abs(position.y - top), x: Math.max(left, Math.min(position.x, right)), y: top },
    { value: Math.abs(position.y - bottom), x: Math.max(left, Math.min(position.x, right)), y: bottom },
  ]
  distances.sort((a, b) => a.value - b.value)
  await appWindow.setPosition(new PhysicalPosition(distances[0].x, distances[0].y))
}

async function hideDesktop() {
  await invoke('hide_desktop')
  desktopVisible.value = false
  await setMenu(false)
}

async function showDesktop() {
  await invoke('show_desktop_temporarily')
  desktopVisible.value = true
  if (menuOpen.value) await setMenu(false)
}

async function toggleAutostart() {
  if (busy.value) return
  busy.value = true
  try {
    if (autostartEnabled.value) await disable()
    else await enable()
    autostartEnabled.value = await isEnabled()
  } finally {
    busy.value = false
  }
}

async function restartApp() {
  await invoke('restart_app')
}

async function quitApp() {
  await invoke('quit_app')
}

onMounted(async () => {
  await refreshState()
  unlisten = await listen<{ desktopVisible: boolean }>('desktop-state-changed', ({ payload }) => {
    desktopVisible.value = payload.desktopVisible
  })
})

onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <main
    class="stage"
    :class="[{ expanded: menuOpen, left: menuOpensLeft }, desktopVisible ? 'is-visible' : 'is-hidden']"
    @contextmenu.prevent
  >
    <section v-if="menuOpen" class="menu" @contextmenu.prevent="setMenu(false)">
      <header class="menu-status">
        <span class="status-mark" />
        <div>
          <strong>{{ statusText }}</strong>
          <small v-if="desktopVisible">静止 10 秒后自动收纳</small>
          <small v-else>壁纸保持显示</small>
        </div>
      </header>

      <div class="rule" />

      <button class="menu-item primary" type="button" @click="hideDesktop">
        <span class="icon">●</span><span>立即隐藏</span>
      </button>
      <button class="menu-item" type="button" @click="showDesktop">
        <span class="icon">○</span><span>临时显示</span>
      </button>

      <div class="rule" />

      <button class="menu-item" type="button" :disabled="busy" @click="toggleAutostart">
        <span class="icon">↻</span><span>开机自启动</span>
        <span class="switch" :class="{ on: autostartEnabled }"><i /></span>
      </button>
      <button class="menu-item" type="button" @click="restartApp">
        <span class="icon">⟳</span><span>重启应用</span>
      </button>

      <div class="rule" />

      <button class="menu-item danger" type="button" @click="quitApp">
        <span class="icon">×</span><span>退出应用</span>
      </button>
    </section>

    <button
      class="orb"
      type="button"
      :title="statusText"
      @pointerdown="pointerStart"
      @pointermove="pointerMove"
      @pointerup="pointerEnd"
      @contextmenu="toggleMenu"
    >
      <span class="orb-glyph">⌁</span>
      <span class="orb-state" />
    </button>
  </main>
</template>

<style scoped>
.stage {
  position: relative;
  width: 100%;
  height: 100%;
}

.orb {
  position: absolute;
  right: 7px;
  top: 7px;
  display: grid;
  width: 62px;
  height: 62px;
  padding: 0;
  place-items: center;
  color: #263041;
  background: #f8fafc;
  border: 2px solid #243249;
  border-radius: 18px;
  box-shadow: 5px 6px 0 #243249;
  cursor: grab;
  touch-action: none;
}

.orb:active { cursor: grabbing; transform: translate(2px, 2px); box-shadow: 3px 4px 0 #243249; }
.orb:focus-visible { outline: 3px solid #79aaf5; outline-offset: 2px; }
.orb-glyph { font-size: 25px; line-height: 1; transform: translateY(-2px); }
.orb-state { position: absolute; right: 7px; bottom: 7px; width: 8px; height: 8px; border: 2px solid #f8fafc; border-radius: 50%; background: #3d7ee8; }
.is-visible .orb-state { background: #51a37d; }

.expanded .orb { top: calc(50% - 31px); }
.expanded:not(.left) .orb { left: 7px; right: auto; }
.expanded.left .orb { right: 7px; }

.menu {
  position: absolute;
  top: 7px;
  width: 232px;
  padding: 10px;
  color: #273142;
  background: #fff;
  border: 2px solid #243249;
  border-radius: 10px;
  box-shadow: 5px 6px 0 #243249;
}

.expanded.left .menu { left: 0; }
.expanded:not(.left) .menu { right: 0; }
.menu-status { display: flex; align-items: center; gap: 10px; min-height: 46px; padding: 4px 9px; }
.menu-status div { display: grid; gap: 3px; }
.menu-status strong { font-size: 13px; font-weight: 700; }
.menu-status small { color: #768095; font-size: 11px; }
.status-mark { width: 11px; height: 11px; border: 2px solid #243249; border-radius: 3px; background: #8eb8f6; }
.is-visible .status-mark { background: #79c6a2; }
.rule { height: 1px; margin: 6px 7px; background: #243249; opacity: .16; }
.menu-item { display: flex; width: 100%; min-height: 36px; padding: 0 10px; align-items: center; gap: 9px; color: inherit; background: transparent; border: 0; border-radius: 7px; cursor: pointer; text-align: left; font-size: 13px; font-weight: 600; }
.menu-item:hover { background: #edf3fb; }
.menu-item.primary { background: #deebff; }
.menu-item.primary:hover { background: #cfe1fd; }
.menu-item.danger:hover { color: #a73333; background: #fdeaea; }
.menu-item:disabled { opacity: .6; cursor: wait; }
.icon { width: 18px; color: #4b5870; text-align: center; font-size: 16px; }
.switch { position: relative; width: 29px; height: 17px; margin-left: auto; border: 2px solid #243249; border-radius: 5px; background: #d6dbe3; }
.switch i { position: absolute; top: 2px; left: 2px; width: 9px; height: 9px; border-radius: 2px; background: #243249; transition: transform .15s ease; }
.switch.on { background: #9fc1f3; }
.switch.on i { transform: translateX(12px); }
</style>
