import type { ReactNode } from 'react'

interface Props {
  label: string
  value: string | number
  sub?: string
  icon?: ReactNode
  accent?: boolean
}

export function StatCard({ label, value, sub, icon, accent }: Props) {
  return (
    <div className={`card flex flex-col gap-1 ${accent ? 'border-brand-orange/40' : ''}`}>
      <div className="flex items-center gap-2">
        {icon && (
          <span className={accent ? 'text-brand-orange' : 'text-brand-muted'}>
            {icon}
          </span>
        )}
        <span className="label">{label}</span>
      </div>
      <p className={`text-2xl font-bold ${accent ? 'text-brand-orange' : 'text-white'}`}>
        {value}
      </p>
      {sub && <p className="text-xs text-brand-muted">{sub}</p>}
    </div>
  )
}
