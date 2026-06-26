import { NavLink, Outlet } from 'react-router-dom'
import { SearchBar } from './SearchBar'

const NAV = [
  { to: '/',             label: 'Home' },
  { to: '/blocks',       label: 'Blocks' },
  { to: '/faucet',       label: 'Faucet' },
  { to: '/dev',          label: 'Dev Tools' },
]

export function Layout() {
  return (
    <div className="min-h-screen flex flex-col">
      {/* Top bar */}
      <header className="border-b border-brand-border bg-brand-card/60 backdrop-blur sticky top-0 z-50">
        <div className="max-w-7xl mx-auto px-4 h-14 flex items-center gap-6">
          {/* Logo */}
          <NavLink to="/" className="flex items-center gap-2 shrink-0">
            <img src="/irondag-logo.png" alt="IronDAG" className="h-7 w-auto"
                 onError={e => (e.currentTarget.style.display = 'none')} />
            <span className="font-bold text-white text-lg tracking-tight">
              Iron<span className="text-brand-orange">DAG</span>
            </span>
          </NavLink>

          {/* Search — fills remaining space */}
          <div className="flex-1 max-w-xl hidden md:block">
            <SearchBar compact />
          </div>

          {/* Nav links */}
          <nav className="flex items-center gap-1 shrink-0">
            {NAV.map(({ to, label }) => (
              <NavLink
                key={to}
                to={to}
                end={to === '/'}
                className={({ isActive }) =>
                  `px-3 py-1.5 rounded-lg text-sm font-medium transition-colors ${
                    isActive
                      ? 'text-brand-orange bg-brand-orange/10'
                      : 'text-gray-400 hover:text-white'
                  }`
                }
              >
                {label}
              </NavLink>
            ))}
          </nav>
        </div>

        {/* Mobile search */}
        <div className="md:hidden px-4 pb-3">
          <SearchBar compact />
        </div>
      </header>

      {/* Page content */}
      <main className="flex-1 max-w-7xl mx-auto w-full px-4 py-8">
        <Outlet />
      </main>

      {/* Footer */}
      <footer className="border-t border-brand-border text-center py-5 text-xs text-brand-muted">
        IronDAG Testnet · Chain ID 11567 · Post-Quantum L1
      </footer>
    </div>
  )
}
