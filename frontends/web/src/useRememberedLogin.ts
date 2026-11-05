import { onMounted, ref, watch } from 'vue'
import { gcm } from '@noble/ciphers/aes.js'

const databaseName = 'ikuai-failover-login'
const storeName = 'credentials'
const preferenceKey = 'ikuai-failover-login'
const recordKey = 'remembered-login'
const additionalData = new TextEncoder().encode('ikuai-failover-remembered-login-v1')
type StoredRecord =
  | { version: 1; mode: 'webcrypto'; key: CryptoKey; iv: Uint8Array<ArrayBuffer>; encrypted: ArrayBuffer }
  | { version: 1; mode: 'portable'; key: Uint8Array<ArrayBuffer>; iv: Uint8Array<ArrayBuffer>; encrypted: Uint8Array<ArrayBuffer> }
interface Credentials { username: string; password: string }
function preference() {
  try {
    const value = JSON.parse(localStorage.getItem(preferenceKey) ?? '{}') as Record<string, unknown>
    return { username: typeof value.username === 'string' ? value.username : '', remember: value.remember !== false }
  } catch { return { username: '', remember: true } }
}
function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(databaseName, 1)
    request.onupgradeneeded = () => { if (!request.result.objectStoreNames.contains(storeName)) request.result.createObjectStore(storeName) }
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(new Error('无法打开本地登录存储。'))
    request.onblocked = () => reject(new Error('本地登录存储被其它页面占用。'))
  })
}
async function readRecord(): Promise<StoredRecord | undefined> {
  const db = await openDatabase()
  try {
    return await new Promise((resolve, reject) => {
      const transaction = db.transaction(storeName, 'readonly')
      const request = transaction.objectStore(storeName).get(recordKey)
      request.onsuccess = () => resolve(request.result as StoredRecord | undefined)
      request.onerror = () => reject(new Error('无法读取已记住的密码。'))
    })
  } finally { db.close() }
}
async function writeRecord(record?: StoredRecord) {
  const db = await openDatabase()
  try {
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(storeName, 'readwrite')
      const store = transaction.objectStore(storeName)
      if (record) store.put(record, recordKey); else store.delete(recordKey)
      transaction.oncomplete = () => resolve()
      transaction.onerror = () => reject(new Error('无法保存本地登录信息。'))
      transaction.onabort = () => reject(new Error('本地登录信息未能保存。'))
    })
  } finally { db.close() }
}
function savePreference(username: string, remember: boolean) {
  try { localStorage.setItem(preferenceKey, JSON.stringify({ username: remember ? username : '', remember })) } catch { /* Storage failures do not prevent logging in. */ }
}
async function encrypt(credentials: Credentials): Promise<StoredRecord> {
  const plaintext = new TextEncoder().encode(JSON.stringify(credentials))
  const iv = crypto.getRandomValues(new Uint8Array(12))
  try {
    if (window.isSecureContext && crypto.subtle) {
      const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'])
      const encrypted = await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData }, key, plaintext)
      return { version: 1, mode: 'webcrypto', key, iv, encrypted }
    }
    // Web Crypto is unavailable on LAN HTTP; credentials still remain ciphertext in IndexedDB.
    const key = crypto.getRandomValues(new Uint8Array(32))
    return { version: 1, mode: 'portable', key, iv, encrypted: gcm(key, iv, additionalData).encrypt(plaintext) }
  } finally { plaintext.fill(0) }
}
async function decrypt(record: StoredRecord): Promise<Credentials> {
  let plaintext: Uint8Array<ArrayBuffer>
  if (record.version !== 1) throw new Error('本地登录信息版本不可用。')
  if (record.mode === 'webcrypto') {
    if (!crypto.subtle) throw new Error('此访问地址无法读取之前保存的登录信息。')
    plaintext = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: record.iv, additionalData }, record.key, record.encrypted))
  } else {
    plaintext = gcm(record.key, record.iv, additionalData).decrypt(record.encrypted)
  }
  try {
    const value = JSON.parse(new TextDecoder().decode(plaintext)) as Record<string, unknown>
    if (typeof value.username !== 'string' || typeof value.password !== 'string') throw new Error('本地登录信息不可用。')
    return { username: value.username, password: value.password }
  } finally { plaintext.fill(0) }
}

/** Passwords are stored only after successful login, and only on this browser origin. */
export function useRememberedLogin() {
  const saved = preference()
  const username = ref(saved.username), password = ref(''), remember = ref(saved.remember)
  const ready = ref(false), storageError = ref('')
  let operation = Promise.resolve()
  let generation = 0
  function enqueue(action: () => Promise<void>) { operation = operation.catch(() => {}).then(action); return operation }
  async function restore(force = false) {
    ready.value = false
    const version = generation
    try {
      if (!remember.value) return
      const record = await readRecord()
      if (!record) return
      const credentials = await decrypt(record)
      if (version !== generation || !remember.value) return
      if (force || !password.value) {
        username.value = credentials.username
        password.value = credentials.password
      }
    } catch { storageError.value = '无法读取本地记住的密码，请重新输入。' }
    finally { ready.value = true }
  }
  async function forget() {
    generation += 1
    savePreference('', false)
    return enqueue(async () => { try { await writeRecord() } catch { storageError.value = '无法清除本地登录信息，请检查浏览器存储设置。' } })
  }
  async function rememberSuccessfulLogin(account: string, secret: string) {
    const version = ++generation
    storageError.value = ''
    const shouldRemember = remember.value
    if (!shouldRemember) savePreference('', false)
    return enqueue(async () => {
      try {
        if (!shouldRemember) { await writeRecord(); return }
        const record = await encrypt({ username: account, password: secret })
        if (version !== generation || !remember.value) return
        await writeRecord(record)
        if (version === generation && remember.value) savePreference(account, true)
      } catch { storageError.value = '本次密码未能记住，请检查浏览器存储设置。' }
    })
  }
  watch(remember, value => { if (!value) void forget() })
  onMounted(() => { void restore() })
  return { username, password, remember, ready, storageError, restore, forget, rememberSuccessfulLogin }
}
