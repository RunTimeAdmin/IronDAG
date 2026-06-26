import { Link } from 'react-router-dom'

export function NotFoundPage() {
  return (
    <div className="flex flex-col items-center justify-center py-24 space-y-4 text-center">
      <p className="text-6xl font-bold text-brand-border">404</p>
      <h1 className="text-2xl font-semibold text-white">Page Not Found</h1>
      <p className="text-brand-muted">That block, transaction, or address doesn't exist here.</p>
      <Link to="/" className="btn-primary mt-4">← Back to Explorer</Link>
    </div>
  )
}
