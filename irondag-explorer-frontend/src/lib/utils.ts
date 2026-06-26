const IDAG = 10n ** 18n

export function formatIdag(attoIdag: bigint, decimals = 4): string {
  const whole = attoIdag / IDAG
  const frac = attoIdag % IDAG
  if (frac === 0n) return `${whole} IDAG`
  const fracStr = frac.toString().padStart(18, '0').slice(0, decimals).replace(/0+$/, '')
  return `${whole}${fracStr ? '.' + fracStr : ''} IDAG`
}

export function formatHex(hex: string): bigint {
  return BigInt(hex)
}

export function shortHash(hash: string, chars = 8): string {
  if (hash.length < chars * 2 + 2) return hash
  return `${hash.slice(0, chars + 2)}…${hash.slice(-chars)}`
}

export function shortAddress(addr: string): string {
  if (addr.length < 12) return addr
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`
}

export function timeAgo(unixSecs: number): string {
  const diff = Math.floor(Date.now() / 1000) - unixSecs
  if (diff < 60) return `${diff}s ago`
  if (diff < 600) return `${Math.floor(diff / 60)}m ${diff % 60}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

export function formatTimestamp(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString()
}

export function hexToDecimal(hex: string): number {
  return parseInt(hex, 16)
}

export function streamLabel(stream: string | undefined): string {
  if (!stream) return '?'
  if (stream.includes('A')) return 'A'
  if (stream.includes('B')) return 'B'
  if (stream.includes('C')) return 'C'
  return stream
}

export function streamClass(stream: string | undefined): string {
  const l = streamLabel(stream)
  if (l === 'A') return 'stream-a'
  if (l === 'B') return 'stream-b'
  if (l === 'C') return 'stream-c'
  return ''
}

export function copyToClipboard(text: string): void {
  navigator.clipboard.writeText(text).catch(() => undefined)
}
