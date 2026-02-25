<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useSipRegistration } from '@/composables/useSipRegistration'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Separator } from '@/components/ui/separator'
import { toast } from 'vue-sonner'
import { Settings } from 'lucide-vue-next'

const STORAGE_KEY = 'sip-config'

interface SipConfig {
  serverHost: string
  serverPort: string
  serverTransport: string
  username: string
  password: string
  showProxy: boolean
  proxyHost: string
  proxyPort: string
  proxyTransport: string
}

function loadConfig(): Partial<SipConfig> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

function saveConfig(config: SipConfig) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(config))
}

const router = useRouter()
const { isRegistered, isRegistering, checkRegistered, register } = useSipRegistration()

onMounted(async () => {
  if (await checkRegistered()) {
    await router.push('/dialpad')
  }
})

const saved = loadConfig()
const serverHost = ref(saved.serverHost ?? '')
const serverPort = ref(saved.serverPort ?? '5060')
const serverTransport = ref(saved.serverTransport ?? 'udp')
const username = ref(saved.username ?? '')
const password = ref(saved.password ?? '')
const showProxy = ref(saved.showProxy ?? false)
const proxyHost = ref(saved.proxyHost ?? '')
const proxyPort = ref(saved.proxyPort ?? '5060')
const proxyTransport = ref(saved.proxyTransport ?? 'udp')

// 自动保存配置
watch(
  [serverHost, serverPort, serverTransport, username, password, showProxy, proxyHost, proxyPort, proxyTransport],
  () => {
    saveConfig({
      serverHost: serverHost.value,
      serverPort: serverPort.value,
      serverTransport: serverTransport.value,
      username: username.value,
      password: password.value,
      showProxy: showProxy.value,
      proxyHost: proxyHost.value,
      proxyPort: proxyPort.value,
      proxyTransport: proxyTransport.value,
    })
  },
)

const errors = ref<Record<string, string>>({})

const HOST_RE = /^[a-zA-Z0-9]([a-zA-Z0-9\-]*[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9\-]*[a-zA-Z0-9])?)*$/
const IP_RE = /^(\d{1,3}\.){3}\d{1,3}$/

function isValidHost(host: string): boolean {
  return HOST_RE.test(host) || IP_RE.test(host)
}

function isValidPort(port: string): boolean {
  if (!port) return true // 空值会用默认端口
  const n = Number(port)
  return Number.isInteger(n) && n >= 1 && n <= 65535
}

function validate(): boolean {
  const e: Record<string, string> = {}

  if (!serverHost.value.trim()) {
    e.serverHost = '请输入服务器地址'
  } else if (!isValidHost(serverHost.value.trim())) {
    e.serverHost = '无效的主机名或 IP 地址'
  }

  if (!isValidPort(serverPort.value)) {
    e.serverPort = '端口范围 1-65535'
  }

  if (!username.value.trim()) {
    e.username = '请输入分机号'
  }

  if (!password.value) {
    e.password = '请输入注册码'
  }

  if (showProxy.value && proxyHost.value.trim()) {
    if (!isValidHost(proxyHost.value.trim())) {
      e.proxyHost = '无效的主机名或 IP 地址'
    }
    if (!isValidPort(proxyPort.value)) {
      e.proxyPort = '端口范围 1-65535'
    }
  }

  errors.value = e
  return Object.keys(e).length === 0
}

const canRegister = computed(() => {
  return !!serverHost.value.trim() && !!username.value.trim() && !!password.value
})

function buildSipUri(host: string, port: string, transport: string): string {
  const p = port || '5060'
  const t = transport.toLowerCase()
  const scheme = t === 'tls' || t === 'wss' ? 'sips' : 'sip'
  return `${scheme}:${host}:${p};transport=${t}`
}

async function handleRegister() {
  if (isRegistered.value) {
    await router.push('/dialpad')
    return
  }
  if (!validate()) return

  const server = buildSipUri(serverHost.value.trim(), serverPort.value, serverTransport.value)

  let outboundProxy: string | undefined
  if (showProxy.value && proxyHost.value.trim()) {
    outboundProxy = buildSipUri(proxyHost.value.trim(), proxyPort.value, proxyTransport.value)
  }

  try {
    await register(server, username.value.trim(), password.value, outboundProxy)
    await router.push('/dialpad')
  } catch (e) {
    toast.error(`注册失败: ${e}`)
  }
}
</script>

<template>
  <div class="h-screen overflow-y-auto">
    <div class="flex min-h-full items-center justify-center p-4">
    <Card class="w-full max-w-md relative">
      <Button
        variant="ghost"
        size="icon"
        class="absolute top-2 right-2 text-muted-foreground"
        @click="router.push('/settings')"
      >
        <Settings class="h-4 w-4" />
      </Button>
      <CardContent class="pt-2">
        <form class="space-y-4" @submit.prevent="handleRegister">
          <!-- 服务器配置 -->
          <div class="space-y-3">
            <Label>服务器</Label>
            <div class="space-y-2">
              <div>
                <Input
                  v-model="serverHost"
                  placeholder="sip.example.com"
                  :class="{ 'border-destructive': errors.serverHost }"
                />
                <p v-if="errors.serverHost" class="text-xs text-destructive mt-1">{{ errors.serverHost }}</p>
              </div>
              <div class="grid grid-cols-2 gap-2">
                <div>
                  <Input
                    v-model="serverPort"
                    placeholder="5060"
                    :class="{ 'border-destructive': errors.serverPort }"
                  />
                  <p v-if="errors.serverPort" class="text-xs text-destructive mt-1">{{ errors.serverPort }}</p>
                </div>
                <Select v-model="serverTransport">
                  <SelectTrigger class="w-full">
                    <SelectValue placeholder="协议" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="udp">UDP</SelectItem>
                    <SelectItem value="tcp">TCP</SelectItem>
                    <SelectItem value="tls">TLS</SelectItem>
                    <SelectItem value="ws" disabled>WS（暂不支持）</SelectItem>
                    <SelectItem value="wss" disabled>WSS（暂不支持）</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>

          <div class="space-y-2">
            <Label for="username">分机号</Label>
            <Input
              id="username"
              v-model="username"
              placeholder="1000"
              :class="{ 'border-destructive': errors.username }"
            />
            <p v-if="errors.username" class="text-xs text-destructive mt-1">{{ errors.username }}</p>
          </div>

          <div class="space-y-2">
            <Label for="password">注册码</Label>
            <Input
              id="password"
              v-model="password"
              type="password"
              placeholder="••••••"
              :class="{ 'border-destructive': errors.password }"
            />
            <p v-if="errors.password" class="text-xs text-destructive mt-1">{{ errors.password }}</p>
          </div>

          <Separator />

          <div>
            <button
              type="button"
              class="text-sm text-muted-foreground hover:text-foreground transition-colors"
              @click="showProxy = !showProxy"
            >
              {{ showProxy ? '▼' : '▶' }} Outbound Proxy（可选）
            </button>

            <div v-if="showProxy" class="mt-3 space-y-3 pl-2 border-l-2 border-muted">
              <div class="space-y-2">
                <Label for="proxy-host">代理主机</Label>
                <Input
                  id="proxy-host"
                  v-model="proxyHost"
                  placeholder="proxy.example.com"
                  :class="{ 'border-destructive': errors.proxyHost }"
                />
                <p v-if="errors.proxyHost" class="text-xs text-destructive mt-1">{{ errors.proxyHost }}</p>
              </div>

              <div class="grid grid-cols-2 gap-2">
                <div class="space-y-2">
                  <Label for="proxy-port">端口</Label>
                  <Input
                    id="proxy-port"
                    v-model="proxyPort"
                    placeholder="5060"
                    :class="{ 'border-destructive': errors.proxyPort }"
                  />
                  <p v-if="errors.proxyPort" class="text-xs text-destructive mt-1">{{ errors.proxyPort }}</p>
                </div>
                <div class="space-y-2">
                  <Label>协议</Label>
                  <Select v-model="proxyTransport">
                    <SelectTrigger class="w-full">
                      <SelectValue placeholder="协议" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="udp">UDP</SelectItem>
                      <SelectItem value="tcp">TCP</SelectItem>
                      <SelectItem value="tls">TLS</SelectItem>
                      <SelectItem value="ws" disabled>WS（暂不支持）</SelectItem>
                      <SelectItem value="wss" disabled>WSS（暂不支持）</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </div>
          </div>

          <Button
            type="submit"
            class="w-full"
            :disabled="isRegistering || !canRegister"
          >
            {{ isRegistering ? '注册中...' : '注册' }}
          </Button>
        </form>
      </CardContent>
    </Card>
    </div>
  </div>
</template>
